//! Backtest performance reports produced from the equity curve.
//!
//! The report is the bridge from the execution kernel to `kurumi-risk`: the
//! equity curve is converted into a periodic return series and
//! [`PerformanceMetrics::from_returns`] computes the annualized return and
//! volatility, Sharpe, Sortino and Calmar ratios, drawdown statistics, win rate,
//! and profit/loss ratio.

use kurumi_core::{Currency, KurumiError};
use kurumi_risk::{MetricsConfig, PerformanceMetrics};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::account::EquityPoint;

/// Converts an equity curve into periodic simple returns.
///
/// The return for the interval ending at observation `i` is
/// `equity_i / equity_{i-1} - 1`. Equity must be strictly positive at every
/// observation so the ratio and the subsequent geometric annualization are
/// defined.
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for fewer than two observations and
/// [`KurumiError::InvalidParameter`] when an equity value is not strictly
/// positive or cannot be represented as `f64`.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::{equity_returns, PortfolioAccount};
/// use kurumi_core::{Currency, Price};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let mut account = PortfolioAccount::new(Decimal::from(100), Currency::Usd)?;
/// account.record_equity(0, Price::new(Decimal::from(10))?)?;
/// account.record_equity(1, Price::new(Decimal::from(11))?)?;
/// let returns = equity_returns(account.equity_curve())?;
/// assert_eq!(returns.len(), 1);
/// assert!(returns.first().expect("one return").abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn equity_returns(curve: &[EquityPoint]) -> Result<Vec<f64>, KurumiError> {
    if curve.len() < 2 {
        return Err(KurumiError::InsufficientData {
            required: 2,
            available: curve.len(),
        });
    }
    let mut returns = Vec::with_capacity(curve.len() - 1);
    for pair in curve.windows(2) {
        let previous = pair
            .first()
            .ok_or(KurumiError::InsufficientData {
                required: 2,
                available: 1,
            })?
            .equity;
        let current = pair
            .get(1)
            .ok_or(KurumiError::InsufficientData {
                required: 2,
                available: 1,
            })?
            .equity;
        if previous <= Decimal::ZERO || current <= Decimal::ZERO {
            return Err(KurumiError::InvalidParameter {
                parameter: "equity",
                reason: "must be strictly positive at every observation".to_owned(),
            });
        }
        let previous_value = previous.to_f64().ok_or(KurumiError::InvalidParameter {
            parameter: "equity",
            reason: "cannot be represented as f64".to_owned(),
        })?;
        let current_value = current.to_f64().ok_or(KurumiError::InvalidParameter {
            parameter: "equity",
            reason: "cannot be represented as f64".to_owned(),
        })?;
        returns.push(current_value / previous_value - 1.0);
    }
    Ok(returns)
}

/// The performance report of a completed backtest.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::{BacktestReport, PortfolioAccount};
/// use kurumi_core::{Currency, Price};
/// use kurumi_risk::MetricsConfig;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let mut account = PortfolioAccount::new(Decimal::from(1_000), Currency::Usd)?;
/// account.record_equity(0, Price::new(Decimal::from(100))?)?;
/// account.record_equity(1, Price::new(Decimal::from(100))?)?;
/// account.record_equity(2, Price::new(Decimal::from(100))?)?;
/// let report = BacktestReport::from_equity_curve(
///     account.equity_curve(),
///     account.initial_cash(),
///     account.total_fees(),
///     0,
///     account.currency(),
///     MetricsConfig::default(),
/// )?;
/// assert_eq!(report.final_equity, Decimal::from(1_000));
/// assert_eq!(report.metrics.observations, 2);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BacktestReport {
    /// Risk and performance statistics computed from the equity returns.
    pub metrics: PerformanceMetrics,
    /// Opening equity of the account.
    pub initial_equity: Decimal,
    /// Equity at the last recorded observation.
    pub final_equity: Decimal,
    /// Cumulative fees charged during the backtest.
    pub total_fees: Decimal,
    /// Number of simulated fills.
    pub fill_count: usize,
    /// Currency of the account.
    pub currency: Currency,
}

impl BacktestReport {
    /// Builds a report from an equity curve and the account's summary values.
    ///
    /// At least three equity observations are required: the first pair produces
    /// the first return and the metrics need at least two returns.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`equity_returns`] and
    /// [`PerformanceMetrics::from_returns`], and
    /// [`KurumiError::InsufficientData`] when the curve has no observations.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{BacktestReport, PortfolioAccount};
    /// use kurumi_core::{Currency, Price};
    /// use kurumi_risk::MetricsConfig;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let mut account = PortfolioAccount::new(Decimal::from(1_000), Currency::Usd)?;
    /// account.record_equity(0, Price::new(Decimal::from(100))?)?;
    /// account.record_equity(1, Price::new(Decimal::from(101))?)?;
    /// account.record_equity(2, Price::new(Decimal::from(102))?)?;
    /// let report = BacktestReport::from_equity_curve(
    ///     account.equity_curve(),
    ///     account.initial_cash(),
    ///     account.total_fees(),
    ///     0,
    ///     account.currency(),
    ///     MetricsConfig::default(),
    /// )?;
    /// assert_eq!(report.metrics.observations, 2);
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_equity_curve(
        curve: &[EquityPoint],
        initial_equity: Decimal,
        total_fees: Decimal,
        fill_count: usize,
        currency: Currency,
        config: MetricsConfig,
    ) -> Result<Self, KurumiError> {
        let final_equity = curve
            .last()
            .ok_or(KurumiError::InsufficientData {
                required: 1,
                available: 0,
            })?
            .equity;
        let returns = equity_returns(curve)?;
        let metrics = PerformanceMetrics::from_returns(&returns, config)?;
        Ok(Self {
            metrics,
            initial_equity,
            final_equity,
            total_fees,
            fill_count,
            currency,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(equities: &[i64]) -> Vec<EquityPoint> {
        equities
            .iter()
            .enumerate()
            .map(|(index, equity)| EquityPoint {
                timestamp: index as i64,
                price: Decimal::from(100),
                cash: Decimal::from(*equity),
                position: Decimal::ZERO,
                equity: Decimal::from(*equity),
            })
            .collect()
    }

    #[test]
    fn equity_returns_are_period_over_period_simple_returns() {
        let points = curve(&[1_000, 1_100, 1_210]);
        let returns = equity_returns(&points).expect("valid returns");
        assert_eq!(returns.len(), 2);
        assert!((returns.first().copied().unwrap_or_default() - 0.10).abs() < 1e-12);
        assert!((returns.get(1).copied().unwrap_or_default() - 0.10).abs() < 1e-12);
    }

    #[test]
    fn equity_returns_require_two_observations() {
        assert!(matches!(
            equity_returns(&[]),
            Err(KurumiError::InsufficientData { .. })
        ));
        assert!(equity_returns(&curve(&[1_000])).is_err());
    }

    #[test]
    fn equity_returns_reject_non_positive_equity() {
        let mut points = curve(&[1_000, 900]);
        if let Some(point) = points.get_mut(1) {
            point.equity = Decimal::ZERO;
        }
        assert!(equity_returns(&points).is_err());
    }

    #[test]
    fn report_matches_direct_metric_computation() {
        let points = curve(&[1_000, 1_010, 1_005, 1_030]);
        let returns = equity_returns(&points).expect("valid returns");
        let config = MetricsConfig::default();
        let report = BacktestReport::from_equity_curve(
            &points,
            Decimal::from(1_000),
            Decimal::from(3),
            3,
            Currency::Usd,
            config,
        )
        .expect("valid report");
        let expected = PerformanceMetrics::from_returns(&returns, config).expect("valid metrics");
        assert_eq!(report.metrics, expected);
        assert_eq!(report.final_equity, Decimal::from(1_030));
        assert_eq!(report.initial_equity, Decimal::from(1_000));
        assert_eq!(report.fill_count, 3);
        assert_eq!(report.total_fees, Decimal::from(3));
    }

    #[test]
    fn report_requires_at_least_two_returns() {
        let points = curve(&[1_000, 1_010]);
        assert!(
            BacktestReport::from_equity_curve(
                &points,
                Decimal::from(1_000),
                Decimal::ZERO,
                0,
                Currency::Usd,
                MetricsConfig::default(),
            )
            .is_err()
        );
    }
}
