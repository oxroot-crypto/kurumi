//! Portfolio performance and benchmark-relative attribution reports.
//!
//! [`PortfolioBacktestReport`] combines the `kurumi-risk` performance metrics
//! with the portfolio-specific audit trail produced by the engine:
//!
//! - the equity curve and per-observation weight snapshots;
//! - every [`RebalanceEvent`] with its cause, target, realized weights, turnover,
//!   fees, and slippage;
//! - annualized turnover and the transaction-cost drag in basis points.
//!
//! [`PortfolioBacktestReport::with_benchmark`] aligns a benchmark price series
//! on the report's equity timestamps and delegates to the `kurumi-risk::relative`
//! functions for beta, Jensen's alpha, tracking error, information ratio, and the
//! cumulative excess return.
//!
//! # Examples
//!
//! See the [engine module documentation](crate::portfolio::engine) for a complete
//! backtest that produces a report.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use kurumi_core::KurumiError;
use kurumi_risk::{
    MetricsConfig, PerformanceMetrics, beta, information_ratio, jensen_alpha, tracking_error,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::portfolio::trigger::RebalanceCause;

/// One rebalance recorded by the engine.
///
/// `before_weights` are the portfolio weights when the trigger fired,
/// `target_weights` are the normalized allocation handed to the order generator,
/// and `after_weights` are the weights observed after execution (marked at the
/// execution observation's close). All maps are keyed by symbol and omit cash.
///
/// `turnover` is the one-way turnover `0.5 * sum_i |target_i - before_i|` in
/// weight units. `fees` and `slippage` are the costs of the fills actually
/// executed for this event, in the account currency.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::RebalanceCause;
/// use std::collections::BTreeMap;
///
/// let event = kurumi_backtest::portfolio::RebalanceEvent {
///     timestamp: kurumi_backtest::portfolio::chrono::DateTime::from_timestamp_millis(0)
///         .expect("epoch"),
///     cause: RebalanceCause::Initial,
///     before_weights: BTreeMap::new(),
///     target_weights: BTreeMap::from([("AAA".to_owned(), 1.0)]),
///     after_weights: BTreeMap::from([("AAA".to_owned(), 1.0)]),
///     turnover: 0.0,
///     fees: rust_decimal::Decimal::ZERO,
///     slippage: rust_decimal::Decimal::ZERO,
///     sell_count: 0,
///     buy_count: 1,
///     frozen_symbols: Vec::new(),
/// };
/// assert_eq!(event.cause, RebalanceCause::Initial);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RebalanceEvent {
    /// Timestamp of the observation on which the fills happened.
    pub timestamp: DateTime<Utc>,
    /// Why the engine rebalanced.
    pub cause: RebalanceCause,
    /// Weights observed when the trigger fired.
    pub before_weights: BTreeMap<String, f64>,
    /// Normalized target weights used to generate the orders.
    pub target_weights: BTreeMap<String, f64>,
    /// Weights observed after execution.
    pub after_weights: BTreeMap<String, f64>,
    /// One-way turnover of the rebalance, in weight units.
    pub turnover: f64,
    /// Fees charged by the executed fills.
    pub fees: Decimal,
    /// Slippage cost embedded in the executed fills.
    pub slippage: Decimal,
    /// Number of sell fills executed.
    pub sell_count: usize,
    /// Number of buy fills executed.
    pub buy_count: usize,
    /// Symbols that were frozen because the signal snapshot had no quote.
    pub frozen_symbols: Vec<String>,
}

/// Benchmark-relative attribution produced by
/// [`PortfolioBacktestReport::with_benchmark`].
///
/// `beta`, `tracking_error`, and `information_ratio` follow their usual
/// annualization; `jensen_alpha` is per period, matching
/// [`kurumi_risk::jensen_alpha`]. `excess_return` is the cumulative portfolio
/// return minus the cumulative benchmark return over the aligned sample.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::BenchmarkComparison;
///
/// let comparison = BenchmarkComparison {
///     observations: 3,
///     beta: 1.0,
///     jensen_alpha: 0.0,
///     tracking_error: 0.0,
///     information_ratio: None,
///     portfolio_return: 0.1,
///     benchmark_return: 0.1,
///     excess_return: 0.0,
/// };
/// assert_eq!(comparison.observations, 3);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BenchmarkComparison {
    /// Number of aligned periodic returns used in the comparison.
    pub observations: usize,
    /// Portfolio beta relative to the benchmark.
    pub beta: f64,
    /// Per-period Jensen's alpha.
    pub jensen_alpha: f64,
    /// Annualized tracking error.
    pub tracking_error: f64,
    /// Annualized information ratio; `None` when the tracking error is zero.
    pub information_ratio: Option<f64>,
    /// Cumulative portfolio return over the aligned sample.
    pub portfolio_return: f64,
    /// Cumulative benchmark return over the aligned sample.
    pub benchmark_return: f64,
    /// `portfolio_return - benchmark_return`.
    pub excess_return: f64,
}

/// The complete result of a multi-asset portfolio backtest.
///
/// # Examples
///
/// See the [engine module documentation](crate::portfolio::engine) for a
/// runnable backtest that returns this type.
#[derive(Debug, Clone, PartialEq)]
pub struct PortfolioBacktestReport {
    /// Risk and performance statistics computed from the equity returns.
    pub metrics: PerformanceMetrics,
    /// Opening equity of the account.
    pub initial_equity: Decimal,
    /// Equity at the last recorded observation.
    pub final_equity: Decimal,
    /// Number of rebalances recorded by the engine.
    pub rebalance_count: usize,
    /// Annualized one-way turnover, `sum(turnover) / periods * periods_per_year`.
    pub annualized_turnover: f64,
    /// Cumulative fees charged during the backtest.
    pub total_fees: Decimal,
    /// Cumulative slippage cost embedded in execution prices.
    pub total_slippage: Decimal,
    /// Annualized transaction-cost drag in basis points of average equity.
    pub cost_drag_bps: f64,
    /// End-of-observation equity curve.
    pub equity_curve: Vec<(DateTime<Utc>, Decimal)>,
    /// Per-observation portfolio weights, keyed by symbol.
    pub weight_history: Vec<(DateTime<Utc>, BTreeMap<String, f64>)>,
    /// Rebalance audit trail in chronological order.
    pub rebalance_events: Vec<RebalanceEvent>,
    /// Lowest cash balance observed after a fill.
    pub min_cash: Decimal,
    /// Number of return periods in a year used for the metrics.
    pub periods_per_year: f64,
}

impl PortfolioBacktestReport {
    /// Computes benchmark-relative metrics on the timestamps shared by the
    /// report and the benchmark price series.
    ///
    /// The benchmark must provide strictly positive finite prices. Alignment is
    /// an inner join on exact timestamps; duplicated benchmark timestamps are
    /// rejected, and the library never forward-fills. At least three aligned
    /// observations are required so that beta has two periodic returns.
    ///
    /// `risk_free_rate` is annual and is converted to a per-period rate before
    /// calling [`kurumi_risk::jensen_alpha`].
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] for a non-positive or non-finite
    /// benchmark price, a duplicate timestamp, or invalid annualization inputs,
    /// and [`KurumiError::InsufficientData`] when fewer than three observations
    /// align. Propagates `kurumi-risk` errors such as a zero-variance benchmark.
    pub fn with_benchmark(
        &self,
        benchmark_prices: &[(DateTime<Utc>, f64)],
        periods_per_year: f64,
        risk_free_rate: f64,
    ) -> Result<BenchmarkComparison, KurumiError> {
        let config = MetricsConfig::new(periods_per_year, risk_free_rate)?;
        let mut benchmarks = BTreeMap::new();
        for (timestamp, price) in benchmark_prices {
            if !price.is_finite() || *price <= 0.0 {
                return Err(KurumiError::InvalidParameter {
                    parameter: "benchmark_prices",
                    reason: format!("price at {timestamp} must be finite and strictly positive"),
                });
            }
            if benchmarks.insert(*timestamp, *price).is_some() {
                return Err(KurumiError::InvalidParameter {
                    parameter: "benchmark_prices",
                    reason: format!("duplicate benchmark timestamp {timestamp}"),
                });
            }
        }

        let mut equities = BTreeMap::new();
        for (timestamp, equity) in &self.equity_curve {
            if *equity <= Decimal::ZERO {
                return Err(KurumiError::InvalidParameter {
                    parameter: "equity_curve",
                    reason: "every equity observation must be strictly positive".to_owned(),
                });
            }
            equities.insert(*timestamp, *equity);
        }

        let mut aligned = Vec::new();
        for (timestamp, price) in &benchmarks {
            if let Some(equity) = equities.get(timestamp) {
                aligned.push((*equity, *price));
            }
        }
        if aligned.len() < 3 {
            return Err(KurumiError::InsufficientData {
                required: 3,
                available: aligned.len(),
            });
        }

        let mut portfolio_returns = Vec::with_capacity(aligned.len() - 1);
        let mut benchmark_returns = Vec::with_capacity(aligned.len() - 1);
        for pair in aligned.windows(2) {
            let (Some(previous), Some(current)) = (pair.first(), pair.get(1)) else {
                return Err(KurumiError::InsufficientData {
                    required: 2,
                    available: pair.len(),
                });
            };
            let previous_equity =
                previous
                    .0
                    .to_f64()
                    .ok_or_else(|| KurumiError::InvalidParameter {
                        parameter: "equity_curve",
                        reason: "equity cannot be represented as f64".to_owned(),
                    })?;
            let current_equity =
                current
                    .0
                    .to_f64()
                    .ok_or_else(|| KurumiError::InvalidParameter {
                        parameter: "equity_curve",
                        reason: "equity cannot be represented as f64".to_owned(),
                    })?;
            portfolio_returns.push(current_equity / previous_equity - 1.0);
            benchmark_returns.push(current.1 / previous.1 - 1.0);
        }

        let (Some(first), Some(last)) = (aligned.first(), aligned.last()) else {
            return Err(KurumiError::InsufficientData {
                required: 2,
                available: aligned.len(),
            });
        };
        let first_equity = first
            .0
            .to_f64()
            .ok_or_else(|| KurumiError::InvalidParameter {
                parameter: "equity_curve",
                reason: "equity cannot be represented as f64".to_owned(),
            })?;
        let last_equity = last
            .0
            .to_f64()
            .ok_or_else(|| KurumiError::InvalidParameter {
                parameter: "equity_curve",
                reason: "equity cannot be represented as f64".to_owned(),
            })?;
        let portfolio_return = last_equity / first_equity - 1.0;
        let benchmark_return = last.1 / first.1 - 1.0;
        let excess_return = portfolio_return - benchmark_return;
        let risk_free_per_period = config.risk_free_rate() / config.periods_per_year();
        let portfolio_beta = beta(&portfolio_returns, &benchmark_returns)?;
        let alpha = jensen_alpha(&portfolio_returns, &benchmark_returns, risk_free_per_period)?;
        let tracking = tracking_error(
            &portfolio_returns,
            &benchmark_returns,
            config.periods_per_year(),
        )?;
        let ratio = if tracking == 0.0 {
            None
        } else {
            Some(information_ratio(
                &portfolio_returns,
                &benchmark_returns,
                config.periods_per_year(),
            )?)
        };
        for value in [
            portfolio_return,
            benchmark_return,
            excess_return,
            portfolio_beta,
            alpha,
            tracking,
        ] {
            if !value.is_finite() {
                return Err(KurumiError::NumericalOverflow {
                    operation: "PortfolioBacktestReport::with_benchmark",
                });
            }
        }

        Ok(BenchmarkComparison {
            observations: portfolio_returns.len(),
            beta: portfolio_beta,
            jensen_alpha: alpha,
            tracking_error: tracking,
            information_ratio: ratio,
            portfolio_return,
            benchmark_return,
            excess_return,
        })
    }

    // Internal constructor: each argument is a field of the resulting report.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_backtest(
        initial_equity: Decimal,
        final_equity: Decimal,
        equity_curve: Vec<(DateTime<Utc>, Decimal)>,
        weight_history: Vec<(DateTime<Utc>, BTreeMap<String, f64>)>,
        rebalance_events: Vec<RebalanceEvent>,
        total_fees: Decimal,
        total_slippage: Decimal,
        min_cash: Decimal,
        total_turnover: f64,
        periods_per_year: f64,
        risk_free_rate: f64,
    ) -> Result<Self, KurumiError> {
        let returns = curve_returns(&equity_curve)?;
        let config = MetricsConfig::new(periods_per_year, risk_free_rate)?;
        let metrics = PerformanceMetrics::from_returns(&returns, config)?;
        let periods = returns.len() as f64;
        let average_equity = equity_curve
            .iter()
            .map(|(_, equity)| *equity)
            .sum::<Decimal>()
            .checked_div(Decimal::from(equity_curve.len() as u64))
            .ok_or(KurumiError::NumericalOverflow {
                operation: "PortfolioBacktestReport::average_equity",
            })?;
        let average_equity =
            average_equity
                .to_f64()
                .ok_or_else(|| KurumiError::InvalidParameter {
                    parameter: "equity_curve",
                    reason: "average equity cannot be represented as f64".to_owned(),
                })?;
        let total_cost = total_fees
            .checked_add(total_slippage)
            .ok_or(KurumiError::NumericalOverflow {
                operation: "PortfolioBacktestReport::total_cost",
            })?
            .to_f64()
            .ok_or_else(|| KurumiError::InvalidParameter {
                parameter: "total_cost",
                reason: "cannot be represented as f64".to_owned(),
            })?;
        let annualized_turnover = total_turnover / periods * periods_per_year;
        let cost_drag_bps = if average_equity > 0.0 {
            total_cost / average_equity * (periods_per_year / periods) * 10_000.0
        } else {
            0.0
        };
        if !annualized_turnover.is_finite() || !cost_drag_bps.is_finite() {
            return Err(KurumiError::NumericalOverflow {
                operation: "PortfolioBacktestReport::cost_metrics",
            });
        }
        let rebalance_count = rebalance_events.len();
        Ok(Self {
            metrics,
            initial_equity,
            final_equity,
            rebalance_count,
            annualized_turnover,
            total_fees,
            total_slippage,
            cost_drag_bps,
            equity_curve,
            weight_history,
            rebalance_events,
            min_cash,
            periods_per_year,
        })
    }
}

fn curve_returns(curve: &[(DateTime<Utc>, Decimal)]) -> Result<Vec<f64>, KurumiError> {
    if curve.len() < 3 {
        return Err(KurumiError::InsufficientData {
            required: 3,
            available: curve.len(),
        });
    }
    let mut returns = Vec::with_capacity(curve.len() - 1);
    for pair in curve.windows(2) {
        let (Some(previous), Some(current)) = (pair.first(), pair.get(1)) else {
            return Err(KurumiError::InsufficientData {
                required: 2,
                available: pair.len(),
            });
        };
        if previous.1 <= Decimal::ZERO || current.1 <= Decimal::ZERO {
            return Err(KurumiError::InvalidParameter {
                parameter: "equity_curve",
                reason: "equity must be strictly positive at every observation".to_owned(),
            });
        }
        let previous_value = previous
            .1
            .to_f64()
            .ok_or_else(|| KurumiError::InvalidParameter {
                parameter: "equity_curve",
                reason: "equity cannot be represented as f64".to_owned(),
            })?;
        let current_value = current
            .1
            .to_f64()
            .ok_or_else(|| KurumiError::InvalidParameter {
                parameter: "equity_curve",
                reason: "equity cannot be represented as f64".to_owned(),
            })?;
        returns.push(current_value / previous_value - 1.0);
    }
    Ok(returns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    fn timestamp(day: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp_millis(day * 86_400_000).expect("valid day")
    }

    fn report(equities: &[i64]) -> PortfolioBacktestReport {
        let curve = equities
            .iter()
            .enumerate()
            .map(|(index, equity)| (timestamp(index as i64), Decimal::from(*equity)))
            .collect::<Vec<_>>();
        PortfolioBacktestReport::from_backtest(
            Decimal::from(*equities.first().expect("non-empty")),
            Decimal::from(*equities.last().expect("non-empty")),
            curve,
            Vec::new(),
            Vec::new(),
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::from(*equities.first().expect("non-empty")),
            0.0,
            252.0,
            0.0,
        )
        .expect("valid report")
    }

    #[test]
    fn report_computes_metrics_turnover_and_cost_drag() {
        let report = report(&[1_000, 1_010, 1_020, 1_030]);
        assert_eq!(report.metrics.observations, 3);
        assert!((report.metrics.total_return - 0.03).abs() < 1e-12);
        assert_eq!(report.rebalance_count, 0);
        assert!(report.cost_drag_bps >= 0.0);
        assert!(report.annualized_turnover.is_finite());
    }

    #[test]
    fn benchmark_comparison_aligns_timestamps_and_computes_relative_metrics() {
        let report = report(&[1_000, 1_010, 1_020, 1_030]);
        let benchmark = vec![
            (timestamp(0), 100.0),
            (timestamp(1), 100.5),
            (timestamp(2), 101.0),
            (timestamp(3), 101.5),
        ];
        let comparison = report
            .with_benchmark(&benchmark, 252.0, 0.0)
            .expect("valid comparison");
        assert_eq!(comparison.observations, 3);
        assert!(comparison.beta.is_finite());
        assert!(comparison.jensen_alpha.is_finite());
        assert!(comparison.tracking_error >= 0.0);
        assert!(comparison.excess_return.is_finite());
    }

    #[test]
    fn benchmark_alignment_requires_three_shared_observations() {
        let report = report(&[1_000, 1_010, 1_020, 1_030]);
        let benchmark = vec![(timestamp(0), 100.0), (timestamp(99), 101.0)];
        assert!(report.with_benchmark(&benchmark, 252.0, 0.0).is_err());
    }

    #[test]
    fn benchmark_rejects_duplicate_and_non_positive_prices() {
        let report = report(&[1_000, 1_010, 1_020, 1_030]);
        let duplicate = vec![
            (timestamp(0), 100.0),
            (timestamp(0), 101.0),
            (timestamp(1), 102.0),
            (timestamp(2), 103.0),
        ];
        assert!(report.with_benchmark(&duplicate, 252.0, 0.0).is_err());
        let zero = vec![
            (timestamp(0), 0.0),
            (timestamp(1), 101.0),
            (timestamp(2), 102.0),
        ];
        assert!(report.with_benchmark(&zero, 252.0, 0.0).is_err());
    }

    #[test]
    fn constant_benchmark_is_rejected_because_beta_is_undefined() {
        let report = report(&[1_000, 1_010, 1_020, 1_030]);
        let benchmark = vec![
            (timestamp(0), 100.0),
            (timestamp(1), 100.0),
            (timestamp(2), 100.0),
            (timestamp(3), 100.0),
        ];
        assert!(report.with_benchmark(&benchmark, 252.0, 0.0).is_err());
    }
}
