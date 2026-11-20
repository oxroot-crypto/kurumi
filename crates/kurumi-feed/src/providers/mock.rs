//! An offline, deterministic connector used for tests and examples.
//!
//! [`MockReplayProvider`] replays a fixed sequence of [`MarketEvent`]s. It only
//! emits an event once the event's instrument and channel are subscribed, so a
//! [`crate::MarketDataHub`] or [`crate::ManagedConnector`] can establish its
//! subscriptions before the replay starts. The replay position is shared across
//! reconnects: a reconnect resumes with the next unconsumed event instead of
//! duplicating history.
//!
//! The provider can inject disconnects after chosen message counts. That is the
//! mechanism the resilience tests use to prove that
//! [`crate::ManagedConnector`] reconnects with an exponential backoff and
//! replays the complete active subscription set.

use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard};

use tokio::sync::{mpsc, watch};

use crate::connector::{BoxFuture, ConnectorCommand, MarketDataConnector, apply_command};
use crate::error::FeedError;
use crate::model::{ChannelKind, InstrumentId, MarketEvent, Subscription, Venue};

/// Shared state that survives reconnects.
#[derive(Debug, Default)]
struct MockState {
    /// Index of the next event to replay.
    position: usize,
    /// Number of events successfully sent across all sessions.
    sent: usize,
    /// Number of `connect` calls, including the initial one.
    connect_calls: usize,
    /// Subscription set passed to each `connect` call.
    initial_subscriptions: Vec<Vec<Subscription>>,
}

/// A connector that replays a preset event sequence entirely in memory.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use kurumi_feed::{MarketDataConnector, MockReplayProvider, Venue};
///
/// let provider = MockReplayProvider::new("mock", vec![Venue::Nasdaq], Vec::new())
///     .with_disconnects(vec![2, 5]);
/// let connector: Arc<dyn MarketDataConnector> = Arc::new(provider);
/// assert_eq!(connector.name(), "mock");
/// ```
#[derive(Debug)]
pub struct MockReplayProvider {
    name: String,
    venues: Vec<Venue>,
    events: Vec<MarketEvent>,
    disconnect_after: Vec<usize>,
    state: Mutex<MockState>,
}

impl MockReplayProvider {
    /// Creates a provider that replays `events` in order.
    ///
    /// `name` must be non-empty and unique inside a hub; the hub validates that
    /// when the connector is registered.
    #[must_use]
    pub fn new(name: impl Into<String>, venues: Vec<Venue>, events: Vec<MarketEvent>) -> Self {
        Self {
            name: name.into(),
            venues,
            events,
            disconnect_after: Vec::new(),
            state: Mutex::new(MockState::default()),
        }
    }

    /// Configures the provider to fail after the given cumulative message counts.
    ///
    /// The counts are global across reconnects: `vec![2, 5]` drops the
    /// connection after the second and the fifth successfully sent message. The
    /// replay resumes at the next event on each reconnect.
    #[must_use]
    pub fn with_disconnects(mut self, after_messages: Vec<usize>) -> Self {
        self.disconnect_after = after_messages;
        self
    }

    /// Returns how many times [`MarketDataConnector::connect`] was invoked.
    ///
    /// The initial connection counts as one, so a provider that reconnected
    /// twice reports `3`.
    #[must_use]
    pub fn connect_calls(&self) -> usize {
        match self.state.lock() {
            Ok(state) => state.connect_calls,
            Err(_) => 0,
        }
    }

    /// Returns the subscription set recorded at every `connect` call.
    #[must_use]
    pub fn initial_subscriptions(&self) -> Vec<Vec<Subscription>> {
        match self.state.lock() {
            Ok(state) => state.initial_subscriptions.clone(),
            Err(_) => Vec::new(),
        }
    }

    /// Locks the shared state, translating poisoning into a typed error.
    fn lock_state(&self) -> Result<MutexGuard<'_, MockState>, FeedError> {
        self.state.lock().map_err(|_| FeedError::ProtocolError {
            provider: self.name.clone(),
            reason: "mock replay state mutex is poisoned".to_owned(),
        })
    }
}

impl MarketDataConnector for MockReplayProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn supported_venues(&self) -> &[Venue] {
        &self.venues
    }

    fn connect(
        &self,
        initial_subs: Vec<Subscription>,
        tx: mpsc::Sender<MarketEvent>,
        cmd_rx: mpsc::Receiver<ConnectorCommand>,
        shutdown: watch::Receiver<bool>,
    ) -> BoxFuture<'_, Result<(), FeedError>> {
        Box::pin(self.replay(initial_subs, tx, cmd_rx, shutdown))
    }
}

impl MockReplayProvider {
    /// Replays events until the sequence is exhausted, a shutdown arrives, or a
    /// configured disconnect fires.
    async fn replay(
        &self,
        initial_subs: Vec<Subscription>,
        tx: mpsc::Sender<MarketEvent>,
        mut cmd_rx: mpsc::Receiver<ConnectorCommand>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), FeedError> {
        {
            let mut state = self.lock_state()?;
            state.connect_calls = state.connect_calls.saturating_add(1);
            state.initial_subscriptions.push(initial_subs.clone());
        }
        let mut active: BTreeSet<Subscription> = initial_subs.into_iter().collect();

        loop {
            if *shutdown.borrow() {
                return Ok(());
            }

            // Apply every command that is already queued before choosing the
            // next event, so subscribe/unsubscribe calls are not delayed until
            // the next matching event.
            while let Ok(command) = cmd_rx.try_recv() {
                apply_command(&mut active, &command);
            }

            let event = {
                let state = self.lock_state()?;
                self.events.get(state.position).cloned()
            };
            let Some(event) = event else {
                return Ok(());
            };

            while !is_subscribed(&active, &event) {
                if *shutdown.borrow() {
                    return Ok(());
                }
                tokio::select! {
                    biased;
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            return Ok(());
                        }
                    }
                    command = cmd_rx.recv() => {
                        match command {
                            Some(command) => apply_command(&mut active, &command),
                            None => return Ok(()),
                        }
                    }
                }
            }

            let send = tx.send(event);
            tokio::pin!(send);
            loop {
                tokio::select! {
                    biased;
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            return Ok(());
                        }
                    }
                    result = &mut send => {
                        if result.is_err() {
                            return Err(FeedError::ChannelClosed {
                                provider: self.name.clone(),
                            });
                        }
                        break;
                    }
                }
            }

            let sent = {
                let mut state = self.lock_state()?;
                state.position = state.position.saturating_add(1);
                state.sent = state.sent.saturating_add(1);
                state.sent
            };
            if self.disconnect_after.contains(&sent) {
                return Err(FeedError::Disconnected {
                    provider: self.name.clone(),
                    reason: format!("injected disconnect after {sent} messages"),
                });
            }
        }
    }
}

/// Returns `true` when `active` selects the channel carried by `event`.
fn is_subscribed(active: &BTreeSet<Subscription>, event: &MarketEvent) -> bool {
    match event {
        MarketEvent::Trade(tick) => {
            channel_subscribed(active, &tick.instrument, &ChannelKind::Trades)
        }
        MarketEvent::Quote(tick) => {
            channel_subscribed(active, &tick.instrument, &ChannelKind::Quotes)
        }
        MarketEvent::OrderBookDelta(delta) => {
            channel_subscribed(active, &delta.instrument, &ChannelKind::OrderBook)
        }
        MarketEvent::Bar(bar) => channel_subscribed(
            active,
            &bar.instrument,
            &ChannelKind::Bars {
                interval_secs: bar.interval_secs,
            },
        ),
        MarketEvent::Status(_) | MarketEvent::Lagged { .. } => true,
    }
}

/// Returns `true` when any subscription selects `channel` for `instrument`.
fn channel_subscribed(
    active: &BTreeSet<Subscription>,
    instrument: &InstrumentId,
    channel: &ChannelKind,
) -> bool {
    active.iter().any(|subscription| {
        subscription.instrument == *instrument && subscription.channels.contains(channel)
    })
}
