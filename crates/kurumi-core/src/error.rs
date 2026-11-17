//! The unified error type returned by every fallible Kurumi API.
//!
//! Downstream crates must not expose their own error enums through public
//! boundaries: convert internal failures into [`KurumiError`] and keep the
//! originating context in a descriptive variant.

use crate::types::Currency;
use thiserror::Error;

/// The single public error type of the Kurumi workspace.
///
/// Variants carry enough context to diagnose a failure without leaking any
/// dependency's error type. The enum is `#[non_exhaustive]`, so new variants can
/// be added without breaking downstream matchers.
///
/// # Examples
///
/// ```
/// use kurumi_core::{Currency, KurumiError, Money};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), KurumiError> {
/// let usd = Money::new(Decimal::new(100, 2), Currency::Usd);
/// let eur = Money::new(Decimal::new(100, 2), Currency::Eur);
/// assert!(matches!(
///     usd.checked_add(eur),
///     Err(KurumiError::CurrencyMismatch { .. })
/// ));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum KurumiError {
    /// A caller supplied an argument outside the domain of the operation.
    ///
    /// `parameter` names the argument and `reason` states the violated
    /// requirement, for example `"volatility"` / `"must be finite"`.
    #[error("invalid parameter `{parameter}`: {reason}")]
    InvalidParameter {
        /// Name of the offending parameter.
        parameter: &'static str,
        /// Human-readable description of the violated requirement.
        reason: String,
    },

    /// A checked arithmetic operation exceeded the representable range.
    ///
    /// `operation` names the operation that overflowed, for example
    /// `"Money::checked_add"`. Decimal and integer overflow is reported here
    /// rather than wrapping or panicking.
    #[error("numerical overflow in `{operation}`")]
    NumericalOverflow {
        /// Name of the operation that overflowed.
        operation: &'static str,
    },

    /// An iterative numerical method exhausted its iteration budget.
    ///
    /// Used by root finders and other solvers configured with a maximum number
    /// of iterations.
    #[error("`{method}` failed to converge after {iterations} iterations")]
    ConvergenceFailed {
        /// Name of the iterative method.
        method: &'static str,
        /// Iteration budget that was exhausted.
        iterations: usize,
    },

    /// An arithmetic operation mixed two different currencies.
    ///
    /// Kurumi never converts currencies implicitly; callers must convert to a
    /// common currency before combining [`Money`](crate::Money) values.
    #[error("currency mismatch: expected {expected}, found {actual}")]
    CurrencyMismatch {
        /// Currency of the left-hand operand.
        expected: Currency,
        /// Currency of the right-hand operand.
        actual: Currency,
    },

    /// A calendar date or date operation was invalid.
    ///
    /// Examples include an out-of-range month, a day beyond the end of the
    /// month, or a date outside the supported year range.
    #[error("invalid date: {reason}")]
    DateError {
        /// Human-readable description of the invalid date.
        reason: String,
    },

    /// There were too few observations for the requested statistic.
    ///
    /// For example, a sample variance needs at least two observations.
    #[error("insufficient data: need at least {required}, got {available}")]
    InsufficientData {
        /// Minimum number of observations required.
        required: usize,
        /// Number of observations supplied.
        available: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_parameter_message_names_the_parameter() {
        let error = KurumiError::InvalidParameter {
            parameter: "volatility",
            reason: "must be finite".to_owned(),
        };
        assert_eq!(
            error.to_string(),
            "invalid parameter `volatility`: must be finite"
        );
    }

    #[test]
    fn currency_mismatch_message_reports_both_currencies() {
        let error = KurumiError::CurrencyMismatch {
            expected: Currency::Usd,
            actual: Currency::Btc,
        };
        assert_eq!(
            error.to_string(),
            "currency mismatch: expected USD, found BTC"
        );
    }

    #[test]
    fn insufficient_data_message_reports_counts() {
        let error = KurumiError::InsufficientData {
            required: 2,
            available: 1,
        };
        assert_eq!(
            error.to_string(),
            "insufficient data: need at least 2, got 1"
        );
    }
}
