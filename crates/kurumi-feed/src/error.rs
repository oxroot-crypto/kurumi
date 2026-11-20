//! The typed error returned by every fallible `kurumi-feed` operation.
//!
//! Connectors translate transport, protocol, and authentication failures into
//! [`FeedError`] instead of leaking a networking dependency's error type. The
//! runtime uses [`FeedError::is_retryable`] to decide whether a broken
//! connection should be re-established.

use thiserror::Error;

/// A failure raised while connecting to, or streaming from, a market-data
/// provider.
///
/// Every variant keeps enough context to diagnose the failure without exposing
/// `tokio`, `tungstenite`, or `reqwest` error types through the public API. The
/// enum is `#[non_exhaustive]`, so new variants can be added without breaking
/// downstream matches.
///
/// # Examples
///
/// ```
/// use kurumi_feed::FeedError;
///
/// let error = FeedError::InvalidConfig {
///     reason: "event-channel capacity must be positive".to_owned(),
/// };
/// assert!(!error.is_retryable());
/// assert_eq!(
///     error.to_string(),
///     "invalid configuration: event-channel capacity must be positive"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum FeedError {
    /// The initial transport connection could not be established.
    #[error("failed to connect to `{provider}`: {reason}")]
    ConnectionFailed {
        /// Provider or connector that failed to connect.
        provider: String,
        /// Human-readable cause of the failure.
        reason: String,
    },

    /// The provider rejected the supplied credentials or session token.
    #[error("authentication with `{provider}` failed: {reason}")]
    AuthenticationFailed {
        /// Provider that rejected the credentials.
        provider: String,
        /// Human-readable cause of the rejection.
        reason: String,
    },

    /// A subscription request could not be applied by the provider.
    #[error("subscription failed for `{provider}`: {reason}")]
    SubscriptionFailed {
        /// Provider that rejected the subscription.
        provider: String,
        /// Human-readable cause of the rejection.
        reason: String,
    },

    /// The provider sent a frame that violates the expected wire protocol.
    #[error("protocol error in `{provider}`: {reason}")]
    ProtocolError {
        /// Provider or local component that detected the violation.
        provider: String,
        /// Human-readable description of the violation.
        reason: String,
    },

    /// The provider asked the client to slow down.
    #[error("`{provider}` rate limited the request; retry after {retry_after_ms} ms")]
    RateLimited {
        /// Provider that applied the limit.
        provider: String,
        /// Minimum delay the provider asked the client to wait.
        retry_after_ms: u64,
    },

    /// A connection or heartbeat deadline elapsed.
    #[error("`{provider}` timed out: {reason}")]
    Timeout {
        /// Provider or runtime component that timed out.
        provider: String,
        /// Human-readable description of the missed deadline.
        reason: String,
    },

    /// An established connection was lost unexpectedly.
    #[error("`{provider}` disconnected: {reason}")]
    Disconnected {
        /// Provider whose connection was lost.
        provider: String,
        /// Human-readable cause of the disconnect.
        reason: String,
    },

    /// The receiving end of an event or command channel was dropped.
    #[error("`{provider}` channel closed")]
    ChannelClosed {
        /// Provider whose channel closed.
        provider: String,
    },

    /// The caller supplied an invalid feed configuration.
    #[error("invalid configuration: {reason}")]
    InvalidConfig {
        /// Human-readable description of the violated requirement.
        reason: String,
    },
}

impl FeedError {
    /// Returns `true` when retrying the connection may succeed.
    ///
    /// Transport interruptions, timeouts, rate limits, and protocol glitches
    /// are treated as transient. Authentication failures, invalid
    /// configuration, and a closed consumer channel are terminal: reconnecting
    /// cannot repair them.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_feed::FeedError;
    ///
    /// assert!(
    ///     (FeedError::Disconnected {
    ///         provider: "mock".to_owned(),
    ///         reason: "socket reset".to_owned(),
    ///     })
    ///     .is_retryable()
    /// );
    /// assert!(
    ///     !(FeedError::AuthenticationFailed {
    ///         provider: "mock".to_owned(),
    ///         reason: "bad token".to_owned(),
    ///     })
    ///     .is_retryable()
    /// );
    /// ```
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::ConnectionFailed { .. }
            | Self::Timeout { .. }
            | Self::Disconnected { .. }
            | Self::RateLimited { .. }
            | Self::ProtocolError { .. }
            | Self::SubscriptionFailed { .. } => true,
            Self::AuthenticationFailed { .. }
            | Self::ChannelClosed { .. }
            | Self::InvalidConfig { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_failures_are_retryable() {
        for error in [
            FeedError::ConnectionFailed {
                provider: "p".to_owned(),
                reason: "refused".to_owned(),
            },
            FeedError::Timeout {
                provider: "p".to_owned(),
                reason: "heartbeat".to_owned(),
            },
            FeedError::Disconnected {
                provider: "p".to_owned(),
                reason: "reset".to_owned(),
            },
            FeedError::RateLimited {
                provider: "p".to_owned(),
                retry_after_ms: 250,
            },
            FeedError::ProtocolError {
                provider: "p".to_owned(),
                reason: "bad frame".to_owned(),
            },
            FeedError::SubscriptionFailed {
                provider: "p".to_owned(),
                reason: "unknown symbol".to_owned(),
            },
        ] {
            assert!(error.is_retryable(), "{error} should be retryable");
        }
    }

    #[test]
    fn terminal_failures_are_not_retryable() {
        for error in [
            FeedError::AuthenticationFailed {
                provider: "p".to_owned(),
                reason: "bad token".to_owned(),
            },
            FeedError::ChannelClosed {
                provider: "p".to_owned(),
            },
            FeedError::InvalidConfig {
                reason: "zero capacity".to_owned(),
            },
        ] {
            assert!(!error.is_retryable(), "{error} should be terminal");
        }
    }
}
