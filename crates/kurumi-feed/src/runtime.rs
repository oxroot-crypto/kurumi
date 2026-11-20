//! The resilience runtime that keeps a connector alive.
//!
//! [`ManagedConnector`] wraps any [`MarketDataConnector`] with an exponential
//! backoff policy, heartbeat supervision, command forwarding, and automatic
//! re-subscription. It owns the authoritative active subscription set, so a
//! reconnect can never lose a subscription that a consumer added while the
//! transport was down.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use crate::connector::{ConnectorCommand, MarketDataConnector, apply_command};
use crate::error::FeedError;
use crate::model::{ConnectionState, FeedStatusEvent, MarketEvent, Subscription};
use crate::util::now_utc;

/// Capacity of the per-session connector event channel.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Capacity of the per-session connector command channel.
const COMMAND_CHANNEL_CAPACITY: usize = 64;

/// Retry and liveness policy applied by [`ManagedConnector`].
///
/// Backoff starts at [`ReconnectPolicy::initial_backoff`], is multiplied by
/// [`ReconnectPolicy::backoff_multiplier`] after every failed attempt, and is
/// capped at [`ReconnectPolicy::max_backoff`]. A reconnect is attempted only
/// for errors that [`FeedError::is_retryable`] classifies as transient.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use kurumi_feed::ReconnectPolicy;
///
/// let policy = ReconnectPolicy {
///     initial_backoff: Duration::from_millis(100),
///     max_backoff: Duration::from_secs(5),
///     backoff_multiplier: 2.0,
///     max_retries: Some(5),
///     heartbeat_timeout: Duration::from_secs(30),
/// };
/// assert!(policy.validate().is_ok());
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReconnectPolicy {
    /// Delay before the first retry.
    pub initial_backoff: Duration,
    /// Upper bound on the exponentially growing delay.
    pub max_backoff: Duration,
    /// Multiplier applied to the delay after every failed attempt.
    pub backoff_multiplier: f64,
    /// Maximum number of reconnect attempts, or `None` for unlimited retries.
    pub max_retries: Option<usize>,
    /// Maximum time without an event before the session is considered dead.
    ///
    /// A zero duration disables heartbeat supervision.
    pub heartbeat_timeout: Duration,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            max_retries: None,
            heartbeat_timeout: Duration::from_secs(30),
        }
    }
}

impl ReconnectPolicy {
    /// Validates the policy.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] when `initial_backoff` is zero,
    /// `max_backoff` is smaller than `initial_backoff`, or
    /// `backoff_multiplier` is not a finite value greater than or equal to
    /// `1.0`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use kurumi_feed::ReconnectPolicy;
    ///
    /// let invalid = ReconnectPolicy {
    ///     initial_backoff: Duration::ZERO,
    ///     ..ReconnectPolicy::default()
    /// };
    /// assert!(invalid.validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<(), FeedError> {
        if self.initial_backoff.is_zero() {
            return Err(FeedError::InvalidConfig {
                reason: "initial_backoff must be strictly positive".to_owned(),
            });
        }
        if self.max_backoff < self.initial_backoff {
            return Err(FeedError::InvalidConfig {
                reason: "max_backoff must be greater than or equal to initial_backoff".to_owned(),
            });
        }
        if !self.backoff_multiplier.is_finite() || self.backoff_multiplier < 1.0 {
            return Err(FeedError::InvalidConfig {
                reason: "backoff_multiplier must be a finite value greater than or equal to 1.0"
                    .to_owned(),
            });
        }
        Ok(())
    }

    /// Returns the next backoff delay, capped at `max_backoff`.
    fn next_backoff(&self, current: Duration) -> Duration {
        if self.backoff_multiplier <= 1.0 {
            return current.min(self.max_backoff);
        }
        let cap = self.max_backoff.as_secs_f64();
        let scaled = (current.as_secs_f64() * self.backoff_multiplier).min(cap);
        Duration::from_secs_f64(scaled)
    }
}

/// The outcome of one connector session.
enum AttemptOutcome {
    /// The connector reported a graceful end of stream.
    Completed,
    /// The runtime was asked to stop.
    Shutdown,
    /// The session failed and may be retried.
    Failed(FeedError),
}

/// The outcome of waiting between reconnect attempts.
enum WaitOutcome {
    /// The backoff elapsed and another attempt may start.
    Elapsed,
    /// A shutdown request arrived while waiting.
    Shutdown,
}

/// Wraps a [`MarketDataConnector`] with reconnection and supervision.
///
/// # Lifecycle
///
/// [`ManagedConnector::run`] calls the connector once, forwards events to the
/// caller's bounded channel, and forwards subscription commands to the live
/// session. If the session fails with a retryable error, the runtime waits for
/// the backoff delay, then reconnects with the complete active subscription set.
/// A graceful `Ok(())` from the connector, a shutdown signal, or an exhausted
/// retry budget ends the loop.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use std::time::Duration;
/// use kurumi_feed::{
///     ChannelKind, InstrumentId, MarketDataConnector, MockReplayProvider,
///     ReconnectPolicy, Subscription, Venue,
/// };
///
/// let instrument = InstrumentId::new("AAPL", Venue::Nasdaq);
/// let provider = Arc::new(MockReplayProvider::new(
///     "mock",
///     vec![Venue::Nasdaq],
///     Vec::new(),
/// ));
/// let connector: Arc<dyn MarketDataConnector> = provider;
/// let policy = ReconnectPolicy {
///     initial_backoff: Duration::from_millis(10),
///     ..ReconnectPolicy::default()
/// };
/// let runtime = kurumi_feed::ManagedConnector::new(connector, policy)?;
/// let _ = runtime;
/// let _ = Subscription::new(instrument, vec![ChannelKind::Trades]);
/// # Ok::<(), kurumi_feed::FeedError>(())
/// ```
pub struct ManagedConnector {
    connector: Arc<dyn MarketDataConnector>,
    policy: ReconnectPolicy,
}

impl std::fmt::Debug for ManagedConnector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedConnector")
            .field("provider", &self.connector.name())
            .field("policy", &self.policy)
            .finish()
    }
}

impl ManagedConnector {
    /// Creates a managed runtime around `connector`.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] when `policy` fails
    /// [`ReconnectPolicy::validate`].
    pub fn new(
        connector: Arc<dyn MarketDataConnector>,
        policy: ReconnectPolicy,
    ) -> Result<Self, FeedError> {
        policy.validate()?;
        Ok(Self { connector, policy })
    }

    /// Returns the wrapped connector's name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.connector.name()
    }

    /// Returns the retry policy in force.
    #[must_use]
    pub fn policy(&self) -> ReconnectPolicy {
        self.policy
    }

    /// Returns the wrapped connector.
    #[must_use]
    pub fn connector(&self) -> &Arc<dyn MarketDataConnector> {
        &self.connector
    }

    /// Runs the connector until it completes, shutdown is requested, or the
    /// retry budget is exhausted.
    ///
    /// # Errors
    ///
    /// Returns the terminal connector error when the failure is not retryable
    /// or no reconnect attempts remain. Graceful completion and shutdown return
    /// `Ok(())`.
    pub async fn run(
        &self,
        initial_subs: Vec<Subscription>,
        event_tx: mpsc::Sender<MarketEvent>,
        mut cmd_rx: mpsc::Receiver<ConnectorCommand>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), FeedError> {
        let mut active: BTreeSet<Subscription> = initial_subs.into_iter().collect();
        let mut attempt = 0usize;
        let mut backoff = self.policy.initial_backoff;

        loop {
            match self
                .run_attempt(&mut active, &event_tx, &mut cmd_rx, &mut shutdown, attempt)
                .await
            {
                AttemptOutcome::Completed | AttemptOutcome::Shutdown => return Ok(()),
                AttemptOutcome::Failed(error) => {
                    if *shutdown.borrow() || !error.is_retryable() {
                        return Err(error);
                    }
                    if let Some(max_retries) = self.policy.max_retries {
                        if attempt >= max_retries {
                            return Err(error);
                        }
                    }
                    tracing::warn!(
                        provider = %self.connector.name(),
                        error = %error,
                        attempt = attempt.saturating_add(1),
                        "market-data connection lost; scheduling a reconnect"
                    );
                    match wait_backoff(backoff, &mut active, &mut cmd_rx, &mut shutdown).await {
                        WaitOutcome::Elapsed => {}
                        WaitOutcome::Shutdown => return Ok(()),
                    }
                    backoff = self.policy.next_backoff(backoff);
                    attempt = attempt.saturating_add(1);
                }
            }
        }
    }

    /// Runs exactly one connector session.
    async fn run_attempt(
        &self,
        active: &mut BTreeSet<Subscription>,
        event_tx: &mpsc::Sender<MarketEvent>,
        cmd_rx: &mut mpsc::Receiver<ConnectorCommand>,
        shutdown: &mut watch::Receiver<bool>,
        attempt: usize,
    ) -> AttemptOutcome {
        if *shutdown.borrow() {
            return AttemptOutcome::Shutdown;
        }

        let provider = self.connector.name().to_owned();
        let state = if attempt == 0 {
            ConnectionState::Connecting
        } else {
            ConnectionState::Reconnecting
        };
        if !send_event(
            event_tx,
            MarketEvent::Status(FeedStatusEvent {
                provider: provider.clone(),
                state,
                message: None,
                ts: now_utc(),
            }),
            shutdown,
        )
        .await
        {
            return AttemptOutcome::Shutdown;
        }

        let (connector_tx, mut connector_rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
        let (command_tx, command_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let initial: Vec<Subscription> = active.iter().cloned().collect();
        let future = self
            .connector
            .connect(initial, connector_tx, command_rx, shutdown.clone());
        tokio::pin!(future);

        let heartbeat = self.policy.heartbeat_timeout;
        let mut deadline = deadline_after(heartbeat);
        let mut connected = false;
        let mut events_open = true;

        loop {
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        try_send_status(event_tx, &provider, ConnectionState::Disconnected, Some("shutdown requested".to_owned()));
                        return AttemptOutcome::Shutdown;
                    }
                }
                command = cmd_rx.recv() => {
                    match command {
                        Some(command) => {
                            apply_command(active, &command);
                            if !forward_command(&command_tx, command, shutdown).await {
                                return AttemptOutcome::Shutdown;
                            }
                        }
                        None => {
                            try_send_status(event_tx, &provider, ConnectionState::Disconnected, Some("command channel closed".to_owned()));
                            return AttemptOutcome::Shutdown;
                        }
                    }
                }
                event = connector_rx.recv(), if events_open => {
                    match event {
                        Some(event) => {
                            if !connected {
                                connected = true;
                                if !send_event(
                                    event_tx,
                                    MarketEvent::Status(FeedStatusEvent {
                                        provider: provider.clone(),
                                        state: ConnectionState::Connected,
                                        message: None,
                                        ts: now_utc(),
                                    }),
                                    shutdown,
                                )
                                .await
                                {
                                    return AttemptOutcome::Shutdown;
                                }
                            }
                            deadline = deadline_after(heartbeat);
                            if !send_event(event_tx, event, shutdown).await {
                                return AttemptOutcome::Shutdown;
                            }
                        }
                        None => {
                            events_open = false;
                        }
                    }
                }
                result = &mut future => {
                    // A connector may fill its bounded event channel and finish
                    // in the same poll, so forward everything it buffered
                    // before the session outcome is handled.
                    while let Ok(event) = connector_rx.try_recv() {
                        if !connected {
                            connected = true;
                            if !send_event(
                                event_tx,
                                MarketEvent::Status(FeedStatusEvent {
                                    provider: provider.clone(),
                                    state: ConnectionState::Connected,
                                    message: None,
                                    ts: now_utc(),
                                }),
                                shutdown,
                            )
                            .await
                            {
                                return AttemptOutcome::Shutdown;
                            }
                        }
                        if !send_event(event_tx, event, shutdown).await {
                            return AttemptOutcome::Shutdown;
                        }
                    }
                    return match result {
                        Ok(()) => {
                            try_send_status(
                                event_tx,
                                &provider,
                                ConnectionState::Disconnected,
                                Some("provider stream completed".to_owned()),
                            );
                            AttemptOutcome::Completed
                        }
                        Err(error) => {
                            try_send_status(
                                event_tx,
                                &provider,
                                ConnectionState::Disconnected,
                                Some(error.to_string()),
                            );
                            AttemptOutcome::Failed(error)
                        }
                    };
                }
                () = tokio::time::sleep_until(deadline), if !heartbeat.is_zero() => {
                    let error = FeedError::Timeout {
                        provider: provider.clone(),
                        reason: format!("no events within {} ms", heartbeat.as_millis()),
                    };
                    try_send_status(event_tx, &provider, ConnectionState::Disconnected, Some(error.to_string()));
                    return AttemptOutcome::Failed(error);
                }
            }
        }
    }
}

/// Computes a monotonic deadline, falling back to a short horizon when the
/// configured heartbeat duration is too large to represent.
fn deadline_after(heartbeat: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(heartbeat)
        .or_else(|| now.checked_add(Duration::from_secs(3_600)))
        .unwrap_or(now)
}

/// Sends one event without blocking shutdown.
///
/// Returns `false` when shutdown was requested or the consumer dropped the
/// receiver.
async fn send_event(
    tx: &mpsc::Sender<MarketEvent>,
    event: MarketEvent,
    shutdown: &mut watch::Receiver<bool>,
) -> bool {
    if *shutdown.borrow() {
        return false;
    }
    let permit = tokio::select! {
        biased;
        _ = shutdown.changed() => {
            // A shutdown request always wins over a pending send; the runtime
            // only ever sends `true`, so there is no state to retry for.
            return false;
        }
        permit = tx.reserve() => permit,
    };
    match permit {
        Ok(permit) => {
            permit.send(event);
            true
        }
        Err(_) => false,
    }
}

/// Forwards a command to a live connector session without blocking shutdown.
async fn forward_command(
    tx: &mpsc::Sender<ConnectorCommand>,
    command: ConnectorCommand,
    shutdown: &mut watch::Receiver<bool>,
) -> bool {
    if *shutdown.borrow() {
        return false;
    }
    let permit = tokio::select! {
        biased;
        _ = shutdown.changed() => {
            // A shutdown request always wins over a pending send; the runtime
            // only ever sends `true`, so there is no state to retry for.
            return false;
        }
        permit = tx.reserve() => permit,
    };
    match permit {
        Ok(permit) => {
            permit.send(command);
            true
        }
        Err(_) => false,
    }
}

/// Queues a lifecycle notification without waiting for capacity.
fn try_send_status(
    tx: &mpsc::Sender<MarketEvent>,
    provider: &str,
    state: ConnectionState,
    message: Option<String>,
) {
    let _ = tx.try_send(MarketEvent::Status(FeedStatusEvent {
        provider: provider.to_owned(),
        state,
        message,
        ts: now_utc(),
    }));
}

/// Waits for the backoff delay while still accepting subscription commands.
async fn wait_backoff(
    delay: Duration,
    active: &mut BTreeSet<Subscription>,
    cmd_rx: &mut mpsc::Receiver<ConnectorCommand>,
    shutdown: &mut watch::Receiver<bool>,
) -> WaitOutcome {
    let sleep = tokio::time::sleep(delay);
    tokio::pin!(sleep);
    loop {
        if *shutdown.borrow() {
            return WaitOutcome::Shutdown;
        }
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return WaitOutcome::Shutdown;
                }
            }
            command = cmd_rx.recv() => {
                match command {
                    Some(command) => apply_command(active, &command),
                    None => return WaitOutcome::Shutdown,
                }
            }
            () = &mut sleep => return WaitOutcome::Elapsed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_validation_rejects_zero_initial_backoff() {
        let policy = ReconnectPolicy {
            initial_backoff: Duration::ZERO,
            ..ReconnectPolicy::default()
        };
        assert!(matches!(
            policy.validate(),
            Err(FeedError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn policy_validation_rejects_shrinking_backoff() {
        let policy = ReconnectPolicy {
            initial_backoff: Duration::from_secs(2),
            max_backoff: Duration::from_secs(1),
            ..ReconnectPolicy::default()
        };
        assert!(policy.validate().is_err());
    }

    #[test]
    fn policy_validation_rejects_a_multiplier_below_one() {
        let policy = ReconnectPolicy {
            backoff_multiplier: 0.5,
            ..ReconnectPolicy::default()
        };
        assert!(policy.validate().is_err());
    }

    #[test]
    fn backoff_grows_to_the_cap() {
        let policy = ReconnectPolicy {
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_millis(250),
            backoff_multiplier: 2.0,
            ..ReconnectPolicy::default()
        };
        let first = policy.next_backoff(policy.initial_backoff);
        assert_eq!(first, Duration::from_millis(200));
        let second = policy.next_backoff(first);
        assert_eq!(second, Duration::from_millis(250));
    }

    #[test]
    fn default_policy_is_valid() {
        assert!(ReconnectPolicy::default().validate().is_ok());
    }
}
