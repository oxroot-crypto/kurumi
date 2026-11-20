//! The multi-provider fan-out hub.
//!
//! [`MarketDataHub`] registers several connectors, starts one supervised task
//! per provider, merges their event streams into a
//! [`tokio::sync::broadcast`] channel, and routes subscription commands by
//! provider name or by venue. Every consumer owns its broadcast receiver, so a
//! slow consumer can lag without stalling the providers or the other consumers.

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;

use crate::connector::{ConnectorCommand, MarketDataConnector};
use crate::error::FeedError;
use crate::model::{MarketEvent, Subscription, Venue};
use crate::runtime::{ManagedConnector, ReconnectPolicy};

/// Minimum capacity of a per-provider command channel.
const MIN_COMMAND_CAPACITY: usize = 16;

/// A registered connector and its runtime configuration.
struct Registration {
    connector: Arc<dyn MarketDataConnector>,
    venues: Vec<Venue>,
    policy: ReconnectPolicy,
}

/// Merges several market-data providers behind one broadcast stream.
///
/// # Lifecycle
///
/// 1. Register one or more connectors with [`MarketDataHub::register`] or
///    [`MarketDataHub::register_with_policy`].
/// 2. Attach consumers with [`MarketDataHub::subscribe_events`] **before**
///    calling [`MarketDataHub::start`]. A broadcast send with no attached
///    receiver is discarded.
/// 3. Route subscriptions with [`MarketDataHub::subscribe`] by provider name or
///    [`MarketDataHub::subscribe_by_venue`] by venue.
/// 4. Call [`MarketDataHub::shutdown`] to stop every provider and join its
///    task.
///
/// # Backpressure
///
/// Providers write into a bounded per-provider channel that a bridge task
/// drains into the broadcast channel. The bridge never blocks on a consumer:
/// each subscriber has its own cursor, and a subscriber that falls behind sees
/// `tokio::sync::broadcast::error::RecvError::Lagged` with the number of missed
/// events. Other subscribers keep receiving every event.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use kurumi_feed::{MarketDataConnector, MarketDataHub, MockReplayProvider, Venue};
///
/// let mut hub = MarketDataHub::new(256)?;
/// let provider: Arc<dyn MarketDataConnector> = Arc::new(MockReplayProvider::new(
///     "mock",
///     vec![Venue::Nasdaq],
///     Vec::new(),
/// ));
/// hub.register(provider)?;
/// assert_eq!(hub.provider_names(), vec!["mock".to_owned()]);
/// # Ok::<(), kurumi_feed::FeedError>(())
/// ```
pub struct MarketDataHub {
    capacity: usize,
    event_tx: broadcast::Sender<MarketEvent>,
    registrations: BTreeMap<String, Registration>,
    venue_routes: BTreeMap<Venue, Vec<String>>,
    started: bool,
    shutdown_tx: Option<watch::Sender<bool>>,
    commands: BTreeMap<String, mpsc::Sender<ConnectorCommand>>,
    tasks: Vec<JoinHandle<Result<(), FeedError>>>,
}

impl MarketDataHub {
    /// Creates a hub whose broadcast channel buffers `capacity` events.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] when `capacity` is zero.
    pub fn new(capacity: usize) -> Result<Self, FeedError> {
        if capacity == 0 {
            return Err(FeedError::InvalidConfig {
                reason: "event broadcast capacity must be strictly positive".to_owned(),
            });
        }
        let (event_tx, _) = broadcast::channel(capacity);
        Ok(Self {
            capacity,
            event_tx,
            registrations: BTreeMap::new(),
            venue_routes: BTreeMap::new(),
            started: false,
            shutdown_tx: None,
            commands: BTreeMap::new(),
            tasks: Vec::new(),
        })
    }

    /// Registers a connector with [`ReconnectPolicy::default`].
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] when the hub has already started,
    /// the connector name is empty or already registered, or the default policy
    /// is invalid.
    pub fn register(&mut self, connector: Arc<dyn MarketDataConnector>) -> Result<(), FeedError> {
        self.register_with_policy(connector, ReconnectPolicy::default())
    }

    /// Registers a connector with an explicit reconnect policy.
    ///
    /// Venue routing uses registration order: the first registered provider
    /// that supports a venue receives venue-routed subscriptions for it.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] when the hub has already started,
    /// the connector name is empty or already registered, or `policy` fails
    /// [`ReconnectPolicy::validate`].
    pub fn register_with_policy(
        &mut self,
        connector: Arc<dyn MarketDataConnector>,
        policy: ReconnectPolicy,
    ) -> Result<(), FeedError> {
        if self.started {
            return Err(FeedError::InvalidConfig {
                reason: "cannot register a connector after the hub has started".to_owned(),
            });
        }
        policy.validate()?;
        let name = connector.name().to_owned();
        if name.is_empty() {
            return Err(FeedError::InvalidConfig {
                reason: "connector name must not be empty".to_owned(),
            });
        }
        if self.registrations.contains_key(&name) {
            return Err(FeedError::InvalidConfig {
                reason: format!("connector `{name}` is already registered"),
            });
        }

        let venues = connector.supported_venues().to_vec();
        for venue in &venues {
            let route = self.venue_routes.entry(venue.clone()).or_default();
            if !route.contains(&name) {
                route.push(name.clone());
            }
        }
        self.registrations.insert(
            name,
            Registration {
                connector,
                venues,
                policy,
            },
        );
        Ok(())
    }

    /// Returns the registered provider names in deterministic order.
    #[must_use]
    pub fn provider_names(&self) -> Vec<String> {
        self.registrations.keys().cloned().collect()
    }

    /// Returns the venues a registered provider supports.
    #[must_use]
    pub fn supported_venues(&self, provider: &str) -> Option<&[Venue]> {
        self.registrations
            .get(provider)
            .map(|registration| registration.venues.as_slice())
    }

    /// Returns the first registered provider that supports `venue`.
    #[must_use]
    pub fn provider_for_venue(&self, venue: &Venue) -> Option<&str> {
        self.venue_routes
            .get(venue)
            .and_then(|providers| providers.first())
            .map(String::as_str)
    }

    /// Starts one supervised task per registered provider.
    ///
    /// Every provider connects with an empty initial subscription set; call
    /// [`MarketDataHub::subscribe`] after `start` to begin streaming.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] when the hub was already started.
    pub fn start(&mut self) -> Result<(), FeedError> {
        if self.started {
            return Err(FeedError::InvalidConfig {
                reason: "the hub has already been started".to_owned(),
            });
        }
        let (shutdown_tx, _) = watch::channel(false);
        let providers: Vec<(String, Arc<dyn MarketDataConnector>, ReconnectPolicy)> = self
            .registrations
            .iter()
            .map(|(name, registration)| {
                (
                    name.clone(),
                    Arc::clone(&registration.connector),
                    registration.policy,
                )
            })
            .collect();

        let mut tasks = Vec::with_capacity(providers.len());
        for (name, connector, policy) in providers {
            let (provider_event_tx, mut provider_event_rx) =
                mpsc::channel::<MarketEvent>(self.capacity);
            let (command_tx, command_rx) =
                mpsc::channel::<ConnectorCommand>(self.capacity.max(MIN_COMMAND_CAPACITY));
            self.commands.insert(name.clone(), command_tx);

            let broadcast_tx = self.event_tx.clone();
            let shutdown_rx = shutdown_tx.subscribe();
            let provider = name.clone();
            let task = tokio::spawn(async move {
                let bridge = tokio::spawn(async move {
                    while let Some(event) = provider_event_rx.recv().await {
                        // No receivers is a valid state; the event is dropped
                        // for late subscribers, not for existing ones.
                        let _ = broadcast_tx.send(event);
                    }
                });
                let managed = ManagedConnector::new(connector, policy)?;
                let result = managed
                    .run(Vec::new(), provider_event_tx, command_rx, shutdown_rx)
                    .await;
                let _ = bridge.await;
                if let Err(error) = &result {
                    tracing::warn!(
                        provider = %provider,
                        error = %error,
                        "market-data provider task stopped"
                    );
                }
                result
            });
            tasks.push(task);
        }

        self.shutdown_tx = Some(shutdown_tx);
        self.tasks = tasks;
        self.started = true;
        Ok(())
    }

    /// Subscribes to the merged event stream.
    ///
    /// Attach the returned receiver before [`MarketDataHub::start`] to avoid
    /// missing events emitted at start-up.
    #[must_use]
    pub fn subscribe_events(&self) -> broadcast::Receiver<MarketEvent> {
        self.event_tx.subscribe()
    }

    /// Sends subscription commands to one provider by name.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] when the hub is not started or
    /// `provider` is unknown, and [`FeedError::ChannelClosed`] when the
    /// provider task has exited.
    pub async fn subscribe(
        &self,
        provider: &str,
        subscriptions: Vec<Subscription>,
    ) -> Result<(), FeedError> {
        self.send_command(provider, ConnectorCommand::Subscribe(subscriptions))
            .await
    }

    /// Sends unsubscribe commands to one provider by name.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] when the hub is not started or
    /// `provider` is unknown, and [`FeedError::ChannelClosed`] when the
    /// provider task has exited.
    pub async fn unsubscribe(
        &self,
        provider: &str,
        subscriptions: Vec<Subscription>,
    ) -> Result<(), FeedError> {
        self.send_command(provider, ConnectorCommand::Unsubscribe(subscriptions))
            .await
    }

    /// Routes subscriptions to the first registered provider for `venue`.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::SubscriptionFailed`] when no registered provider
    /// supports the venue, plus the errors documented by
    /// [`MarketDataHub::subscribe`].
    pub async fn subscribe_by_venue(
        &self,
        venue: &Venue,
        subscriptions: Vec<Subscription>,
    ) -> Result<(), FeedError> {
        let provider = self
            .provider_for_venue(venue)
            .map(str::to_owned)
            .ok_or_else(|| FeedError::SubscriptionFailed {
                provider: venue.to_string(),
                reason: "no registered provider supports the venue".to_owned(),
            })?;
        self.subscribe(&provider, subscriptions).await
    }

    /// Routes unsubscriptions to the first registered provider for `venue`.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::SubscriptionFailed`] when no registered provider
    /// supports the venue, plus the errors documented by
    /// [`MarketDataHub::unsubscribe`].
    pub async fn unsubscribe_by_venue(
        &self,
        venue: &Venue,
        subscriptions: Vec<Subscription>,
    ) -> Result<(), FeedError> {
        let provider = self
            .provider_for_venue(venue)
            .map(str::to_owned)
            .ok_or_else(|| FeedError::SubscriptionFailed {
                provider: venue.to_string(),
                reason: "no registered provider supports the venue".to_owned(),
            })?;
        self.unsubscribe(&provider, subscriptions).await
    }

    /// Sends one control command to a provider.
    async fn send_command(
        &self,
        provider: &str,
        command: ConnectorCommand,
    ) -> Result<(), FeedError> {
        if !self.started {
            return Err(FeedError::InvalidConfig {
                reason: "the hub is not started".to_owned(),
            });
        }
        let command_tx = self
            .commands
            .get(provider)
            .ok_or_else(|| FeedError::InvalidConfig {
                reason: format!("unknown provider `{provider}`"),
            })?;
        command_tx
            .send(command)
            .await
            .map_err(|_| FeedError::ChannelClosed {
                provider: provider.to_owned(),
            })
    }

    /// Stops every provider and waits for its task to finish.
    ///
    /// Providers receive the shutdown signal and drop their subscriptions; the
    /// method then joins every task. The first provider error, if any, is
    /// returned after all tasks have been joined.
    ///
    /// # Errors
    ///
    /// Returns the first terminal error reported by a provider task.
    pub async fn shutdown(&mut self) -> Result<(), FeedError> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(true);
        }
        self.commands.clear();
        let tasks = std::mem::take(&mut self.tasks);
        let mut first_error: Option<FeedError> = None;
        for task in tasks {
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
                Err(join_error) => {
                    if first_error.is_none() {
                        first_error = Some(FeedError::ProtocolError {
                            provider: "market-data-hub".to_owned(),
                            reason: format!("provider task failed: {join_error}"),
                        });
                    }
                }
            }
        }
        self.started = false;
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for MarketDataHub {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::MockReplayProvider;

    #[test]
    fn zero_capacity_is_rejected() {
        assert!(matches!(
            MarketDataHub::new(0),
            Err(FeedError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn duplicate_provider_names_are_rejected() {
        let mut hub = MarketDataHub::new(16).expect("valid capacity");
        let first: Arc<dyn MarketDataConnector> = Arc::new(MockReplayProvider::new(
            "mock",
            vec![Venue::Nasdaq],
            Vec::new(),
        ));
        let second: Arc<dyn MarketDataConnector> = Arc::new(MockReplayProvider::new(
            "mock",
            vec![Venue::Nyse],
            Vec::new(),
        ));
        assert!(hub.register(first).is_ok());
        assert!(hub.register(second).is_err());
    }

    #[test]
    fn venue_routing_prefers_the_first_registration() {
        let mut hub = MarketDataHub::new(16).expect("valid capacity");
        let first: Arc<dyn MarketDataConnector> = Arc::new(MockReplayProvider::new(
            "first",
            vec![Venue::Nasdaq],
            Vec::new(),
        ));
        let second: Arc<dyn MarketDataConnector> = Arc::new(MockReplayProvider::new(
            "second",
            vec![Venue::Nasdaq],
            Vec::new(),
        ));
        hub.register(first).expect("first registers");
        hub.register(second).expect("second registers");
        assert_eq!(hub.provider_for_venue(&Venue::Nasdaq), Some("first"));
        assert_eq!(hub.provider_for_venue(&Venue::Nyse), None);
        assert_eq!(hub.supported_venues("first").map(<[Venue]>::len), Some(1));
    }

    #[tokio::test]
    async fn subscribing_before_start_is_rejected() {
        let hub = MarketDataHub::new(16).expect("valid capacity");
        let result = hub.subscribe("missing", Vec::new()).await;
        assert!(matches!(result, Err(FeedError::InvalidConfig { .. })));
    }
}
