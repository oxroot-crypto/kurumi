//! The unified error type of the execution kernel.
//!
//! [`ExecError`] covers order-state, bookkeeping, risk, broker, journal, and
//! strategy failures. Every fallible public API in `kurumi-exec` returns it, so
//! callers can handle execution failures without matching on the lower-level
//! [`KurumiError`] of the supporting crates.
//!
//! # Examples
//!
//! ```
//! use kurumi_exec::ExecError;
//! use rust_decimal::Decimal;
//!
//! let error = ExecError::InsufficientFunds {
//!     required: Decimal::from(1_000),
//!     available: Decimal::from(500),
//! };
//! assert!(error.to_string().contains("insufficient funds"));
//! ```

use rust_decimal::Decimal;
use thiserror::Error;

use kurumi_core::KurumiError;

/// The error returned by every fallible execution operation.
///
/// The first nine variants are the domain errors of the execution kernel; the
/// final two are extensions used when an order request itself is malformed or a
/// supporting crate fails with a [`KurumiError`].
///
/// # Examples
///
/// ```
/// use kurumi_exec::ExecError;
///
/// let error: ExecError = ExecError::OrderNotFound {
///     client_order_id: "missing".to_owned(),
/// };
/// assert!(error.to_string().contains("missing"));
/// ```
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ExecError {
    /// An execution report was inconsistent with the order's current state,
    /// such as a fill for an order that is already terminal.
    #[error("invalid order state transition: {reason}")]
    InvalidStateTransition {
        /// Human-readable description of the illegal transition.
        reason: String,
    },

    /// A pre-trade risk rule rejected the order.
    #[error("risk rule `{rule}` rejected the order: {reason}")]
    RiskRejected {
        /// Name of the rule that rejected the order.
        rule: String,
        /// Human-readable rejection reason.
        reason: String,
    },

    /// The account does not have enough unreserved cash for a buy order.
    #[error("insufficient funds: required {required}, available {available}")]
    InsufficientFunds {
        /// Cash required by the operation.
        required: Decimal,
        /// Unreserved cash currently available.
        available: Decimal,
    },

    /// The account does not have enough unreserved quantity for a sell order.
    #[error("insufficient position for `{symbol}`: required {required}, available {available}")]
    InsufficientPosition {
        /// Instrument whose position is too small.
        symbol: String,
        /// Quantity required by the operation.
        required: Decimal,
        /// Unreserved long quantity currently available.
        available: Decimal,
    },

    /// A client order identifier has already been used.
    #[error("duplicate client order id `{client_order_id}`")]
    DuplicateClientOrderId {
        /// Identifier that was submitted twice.
        client_order_id: String,
    },

    /// The requested order does not exist in the ledger.
    #[error("order `{client_order_id}` not found")]
    OrderNotFound {
        /// Identifier that was not found.
        client_order_id: String,
    },

    /// A broker-facing operation failed.
    #[error("broker error: {message}")]
    BrokerError {
        /// Description of the broker failure.
        message: String,
    },

    /// Writing the audit journal failed.
    #[error("journal error: {message}")]
    JournalError {
        /// Description of the journaling failure.
        message: String,
    },

    /// Strategy code, or the data handed to it, was invalid.
    #[error("strategy error: {message}")]
    StrategyError {
        /// Description of the strategy failure.
        message: String,
    },

    /// An order request is structurally invalid, for example a non-positive
    /// quantity or a non-positive limit price.
    #[error("invalid order: {reason}")]
    InvalidOrder {
        /// Human-readable description of the invalid request.
        reason: String,
    },

    /// A supporting crate returned an error.
    #[error(transparent)]
    Core(#[from] KurumiError),
}

impl ExecError {
    /// Builds an [`ExecError::InvalidStateTransition`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::ExecError;
    ///
    /// let error = ExecError::invalid_transition("fill after terminal state");
    /// assert!(matches!(
    ///     error,
    ///     ExecError::InvalidStateTransition { .. }
    /// ));
    /// ```
    #[must_use]
    pub fn invalid_transition(reason: impl Into<String>) -> Self {
        Self::InvalidStateTransition {
            reason: reason.into(),
        }
    }

    /// Builds an [`ExecError::RiskRejected`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::ExecError;
    ///
    /// let error = ExecError::risk_rejected("MaxOrderNotional", "too large");
    /// assert!(error.to_string().contains("MaxOrderNotional"));
    /// ```
    #[must_use]
    pub fn risk_rejected(rule: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::RiskRejected {
            rule: rule.into(),
            reason: reason.into(),
        }
    }

    /// Builds an [`ExecError::StrategyError`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::ExecError;
    ///
    /// let error = ExecError::strategy("no market data");
    /// assert!(error.to_string().contains("no market data"));
    /// ```
    #[must_use]
    pub fn strategy(message: impl Into<String>) -> Self {
        Self::StrategyError {
            message: message.into(),
        }
    }

    /// Builds an [`ExecError::InvalidOrder`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::ExecError;
    ///
    /// let error = ExecError::invalid_order("quantity must be positive");
    /// assert!(error.to_string().contains("quantity"));
    /// ```
    #[must_use]
    pub fn invalid_order(reason: impl Into<String>) -> Self {
        Self::InvalidOrder {
            reason: reason.into(),
        }
    }
}
