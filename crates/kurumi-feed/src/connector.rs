//! The provider-agnostic connector abstraction.
//!
//! A connector owns one wire protocol. It receives the initial subscription
//! set, an event sender, a command receiver, and a shutdown signal, and returns
//! when the session ends. Everything above it — reconnect backoff, command
//! bookkeeping, fan-out — is implemented once by
//! [`crate::ManagedConnector`] and [`crate::MarketDataHub`], so a new provider
//! only implements [`MarketDataConnector`].

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;

use tokio::sync::{mpsc, watch};

use crate::error::FeedError;
use crate::model::{MarketEvent, Subscription, Venue};

/// A boxed future returned by [`MarketDataConnector::connect`].
///
/// The alias keeps the trait object-safe: the future borrows the connector and
/// is `Send`, so it can be awaited on any Tokio worker.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A control message delivered to a running connector.
///
/// The resilience runtime keeps its own authoritative subscription set and
/// forwards every command to the live session. When the session is replaced,
/// the full set is passed back as the initial subscription of the next
/// connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectorCommand {
    /// Add subscriptions to the active set.
    Subscribe(Vec<Subscription>),
    /// Remove subscriptions from the active set.
    Unsubscribe(Vec<Subscription>),
}

/// A source of real-time market data for one provider.
///
/// # Contract
///
/// - [`MarketDataConnector::connect`] must send events through `tx` and process
///   [`ConnectorCommand`] values from `cmd_rx` until either side closes or
///   `shutdown` becomes `true`.
/// - A graceful end of stream (including shutdown) returns `Ok(())`; the
///   runtime then stops instead of reconnecting.
/// - An unexpected loss of the transport returns an error such as
///   [`FeedError::Disconnected`]; the runtime decides whether to retry based on
///   [`FeedError::is_retryable`].
/// - Implementations must not block a Tokio worker and must not hold a
///   non-async lock across an `await`.
pub trait MarketDataConnector: Send + Sync {
    /// Returns the stable provider name used for routing and diagnostics.
    fn name(&self) -> &str;

    /// Returns the venues this connector can serve.
    fn supported_venues(&self) -> &[Venue];

    /// Runs one connection session.
    ///
    /// The method is called again with a fresh command channel after a
    /// reconnect; the runtime passes the complete active subscription set as
    /// `initial_subs` on every attempt.
    fn connect(
        &self,
        initial_subs: Vec<Subscription>,
        tx: mpsc::Sender<MarketEvent>,
        cmd_rx: mpsc::Receiver<ConnectorCommand>,
        shutdown: watch::Receiver<bool>,
    ) -> BoxFuture<'_, Result<(), FeedError>>;
}

/// Applies a control command to an in-memory subscription set.
///
/// The runtime and the mock provider share this helper so command semantics
/// cannot drift between them.
pub(crate) fn apply_command(active: &mut BTreeSet<Subscription>, command: &ConnectorCommand) {
    match command {
        ConnectorCommand::Subscribe(subscriptions) => {
            active.extend(subscriptions.iter().cloned());
        }
        ConnectorCommand::Unsubscribe(subscriptions) => {
            for subscription in subscriptions {
                active.remove(subscription);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChannelKind, InstrumentId};

    #[test]
    fn commands_add_and_remove_subscriptions() {
        let instrument = InstrumentId::new("AAPL", Venue::Nasdaq);
        let subscription = Subscription::new(instrument, vec![ChannelKind::Trades]);
        let mut active = BTreeSet::new();

        apply_command(
            &mut active,
            &ConnectorCommand::Subscribe(vec![subscription.clone()]),
        );
        assert!(active.contains(&subscription));

        apply_command(
            &mut active,
            &ConnectorCommand::Unsubscribe(vec![subscription.clone()]),
        );
        assert!(!active.contains(&subscription));
    }
}
