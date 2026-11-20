//! Objective metrics and result filters for strategy validation.
//!
//! A grid search needs two decisions per parameter combination: how good the
//! resulting [`PortfolioBacktestReport`] is, and whether it is admissible at
//! all. [`ObjectiveMetric`] answers the first by mapping a report to a scalar
//! that is always **maximized**; [`ResultFilter`] answers the second by rejecting
//! degenerate results such as zero-trade or excessively levered parameter sets.
//!
//! Undefined statistics never compete for the best parameter: a score that is
//! `NaN` or infinite is reported as `None` and is ignored by ranking.
//!
//! # Examples
//!
//! ```
//! use kurumi_backtest::portfolio::{
//!     CalendarTrigger, MultiAssetFeed, PortfolioBacktestConfig, PortfolioBacktestEngine,
//!     StaticWeightAllocator,
//! };
//! use kurumi_backtest::validation::{ObjectiveMetric, ResultFilter};
//! use kurumi_backtest::{NoFee, NoSlippage};
//! use kurumi_ta::OhlcvBar;
//! use rust_decimal::Decimal;
//! use std::collections::BTreeMap;
//!
//! # fn main() -> Result<(), kurumi_backtest::KurumiError> {
//! let mut series = BTreeMap::new();
//! series.insert(
//!     "AAA".to_owned(),
//!     (0..6)
//!         .map(|day| {
//!             OhlcvBar::new(
//!                 day,
//!                 100.0 + day as f64,
//!                 101.0 + day as f64,
//!                 99.0 + day as f64,
//!                 100.0 + day as f64,
//!                 1_000.0,
//!                 100_000.0,
//!             )
//!         })
//!         .collect::<Result<Vec<_>, _>>()?,
//! );
//! let feed = MultiAssetFeed::new(series)?;
//! let config = PortfolioBacktestConfig::new(Decimal::from(10_000))?;
//! let mut engine = PortfolioBacktestEngine::new(config, Box::new(NoFee), Box::new(NoSlippage))?;
//! let mut allocator = StaticWeightAllocator::new(BTreeMap::from([("AAA".to_owned(), 1.0)]))?;
//! let report = engine.run(feed, &CalendarTrigger::Monthly, &mut allocator)?;
//!
//! assert!(ObjectiveMetric::TotalReturn.score(&report).is_some());
//! let filter = ResultFilter {
//!     min_rebalance_count: Some(1),
//!     ..ResultFilter::default()
//! };
//! assert!(filter.passes(&report));
//! # Ok(())
//! # }
//! ```

use std::fmt;
use std::sync::Arc;

use crate::portfolio::PortfolioBacktestReport;

/// A scalar objective that a grid search maximizes.
///
/// Every variant maps a [`PortfolioBacktestReport`] to one `f64`; larger is
/// always better. [`ObjectiveMetric::MinMaxDrawdown`] negates the maximum
/// drawdown internally so a smaller drawdown produces a larger score.
///
/// [`ObjectiveMetric::score`] returns `None` for a statistic that the report
/// could not compute (for example a Sharpe ratio with zero volatility) and for
/// any non-finite value returned by a custom closure. Such trials are never
/// selected as the best parameter.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::validation::ObjectiveMetric;
///
/// let metric = ObjectiveMetric::Custom(std::sync::Arc::new(|_report| 1.5));
/// assert_eq!(metric.name(), "Custom");
/// ```
#[derive(Clone)]
pub enum ObjectiveMetric {
    /// Annualized Sharpe ratio; larger is better.
    SharpeRatio,
    /// Annualized Sortino ratio; larger is better.
    SortinoRatio,
    /// Annualized return divided by the maximum drawdown; larger is better.
    CalmarRatio,
    /// Cumulative simple return over the evaluated segment; larger is better.
    TotalReturn,
    /// Geometric annualized return over the evaluated segment; larger is
    /// better.
    AnnualizedReturn,
    /// Negative maximum drawdown, so maximizing the score minimizes the
    /// drawdown. The underlying value is a non-negative fraction.
    MinMaxDrawdown,
    /// A caller-supplied score, which must be finite to be ranked.
    ///
    /// The closure receives the complete report and is shared through [`Arc`],
    /// so the metric stays cheap to clone.
    Custom(Arc<dyn Fn(&PortfolioBacktestReport) -> f64 + Send + Sync>),
}

impl ObjectiveMetric {
    /// Creates a [`ObjectiveMetric::Custom`] from a closure.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::ObjectiveMetric;
    ///
    /// let metric = ObjectiveMetric::custom(|report| report.rebalance_count as f64);
    /// assert_eq!(metric.name(), "Custom");
    /// ```
    #[must_use]
    pub fn custom<F>(objective: F) -> Self
    where
        F: Fn(&PortfolioBacktestReport) -> f64 + Send + Sync + 'static,
    {
        Self::Custom(Arc::new(objective))
    }

    /// Returns the human-readable variant name.
    ///
    /// This is useful for logs and reports where printing the whole enum,
    /// including a custom closure, would be noisy.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::ObjectiveMetric;
    ///
    /// assert_eq!(ObjectiveMetric::SharpeRatio.name(), "SharpeRatio");
    /// assert_eq!(ObjectiveMetric::MinMaxDrawdown.name(), "MinMaxDrawdown");
    /// ```
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::SharpeRatio => "SharpeRatio",
            Self::SortinoRatio => "SortinoRatio",
            Self::CalmarRatio => "CalmarRatio",
            Self::TotalReturn => "TotalReturn",
            Self::AnnualizedReturn => "AnnualizedReturn",
            Self::MinMaxDrawdown => "MinMaxDrawdown",
            Self::Custom(_) => "Custom",
        }
    }

    /// Scores `report`, returning `None` when the statistic is undefined or not
    /// finite.
    ///
    /// `MinMaxDrawdown` returns `-max_drawdown`: because the raw drawdown is a
    /// non-negative fraction, maximizing the returned score minimizes the
    /// drawdown.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::{
    ///     CalendarTrigger, MultiAssetFeed, PortfolioBacktestConfig, PortfolioBacktestEngine,
    ///     StaticWeightAllocator,
    /// };
    /// use kurumi_backtest::validation::ObjectiveMetric;
    /// use kurumi_backtest::{NoFee, NoSlippage};
    /// use kurumi_ta::OhlcvBar;
    /// use rust_decimal::Decimal;
    /// use std::collections::BTreeMap;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let mut series = BTreeMap::new();
    /// series.insert(
    ///     "AAA".to_owned(),
    ///     (0..6)
    ///         .map(|day| {
    ///             OhlcvBar::new(
    ///                 day,
    ///                 100.0 + day as f64,
    ///                 101.0 + day as f64,
    ///                 99.0 + day as f64,
    ///                 100.0 + day as f64,
    ///                 1_000.0,
    ///                 100_000.0,
    ///             )
    ///         })
    ///         .collect::<Result<Vec<_>, _>>()?,
    /// );
    /// let feed = MultiAssetFeed::new(series)?;
    /// let config = PortfolioBacktestConfig::new(Decimal::from(10_000))?;
    /// let mut engine = PortfolioBacktestEngine::new(config, Box::new(NoFee), Box::new(NoSlippage))?;
    /// let mut allocator = StaticWeightAllocator::new(BTreeMap::from([("AAA".to_owned(), 1.0)]))?;
    /// let report = engine.run(feed, &CalendarTrigger::Monthly, &mut allocator)?;
    ///
    /// let drawdown = ObjectiveMetric::MinMaxDrawdown.score(&report).expect("defined");
    /// assert!(drawdown <= 0.0);
    /// let broken = ObjectiveMetric::custom(|_report| f64::NAN);
    /// assert!(broken.score(&report).is_none());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn score(&self, report: &PortfolioBacktestReport) -> Option<f64> {
        let value = match self {
            Self::SharpeRatio => report.metrics.sharpe_ratio,
            Self::SortinoRatio => report.metrics.sortino_ratio,
            Self::CalmarRatio => report.metrics.calmar_ratio,
            Self::TotalReturn => Some(report.metrics.total_return),
            Self::AnnualizedReturn => Some(report.metrics.annualized_return),
            Self::MinMaxDrawdown => Some(-report.metrics.max_drawdown),
            Self::Custom(objective) => Some(objective.as_ref()(report)),
        }?;
        value.is_finite().then_some(value)
    }
}

impl fmt::Debug for ObjectiveMetric {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ObjectiveMetric")
            .field(&self.name())
            .finish()
    }
}

/// Admissibility constraints applied before a trial can be ranked.
///
/// The filter is a cheap guard against parameter sets that look good only
/// because they barely traded or took extreme risk. Every constraint is
/// optional; the default filter admits every successful backtest.
///
/// Comparisons are inclusive: a value exactly equal to its limit passes. A
/// configured limit that is negative or not finite rejects every result
/// (fail-closed) rather than silently disabling the guard.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::validation::ResultFilter;
///
/// let filter = ResultFilter {
///     min_rebalance_count: Some(4),
///     max_drawdown_limit: Some(0.25),
///     max_annualized_turnover: Some(12.0),
/// };
/// assert_eq!(filter.max_drawdown_limit, Some(0.25));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ResultFilter {
    /// Minimum number of rebalances the strategy must record.
    pub min_rebalance_count: Option<usize>,
    /// Maximum tolerated drawdown, as a non-negative fraction of equity.
    ///
    /// For example `Some(0.25)` rejects any parameter set whose maximum
    /// drawdown exceeds 25%.
    pub max_drawdown_limit: Option<f64>,
    /// Maximum tolerated annualized one-way turnover, in turns per year.
    pub max_annualized_turnover: Option<f64>,
}

impl ResultFilter {
    /// Creates the default, permissive filter.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::ResultFilter;
    ///
    /// let filter = ResultFilter::new();
    /// assert!(filter.min_rebalance_count.is_none());
    /// assert_eq!(filter, ResultFilter::default());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the minimum number of rebalances.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::ResultFilter;
    ///
    /// assert_eq!(ResultFilter::new().with_min_rebalance_count(3).min_rebalance_count, Some(3));
    /// ```
    #[must_use]
    pub fn with_min_rebalance_count(mut self, minimum: usize) -> Self {
        self.min_rebalance_count = Some(minimum);
        self
    }

    /// Sets the maximum tolerated drawdown fraction.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::ResultFilter;
    ///
    /// assert_eq!(ResultFilter::new().with_max_drawdown_limit(0.25).max_drawdown_limit, Some(0.25));
    /// ```
    #[must_use]
    pub fn with_max_drawdown_limit(mut self, limit: f64) -> Self {
        self.max_drawdown_limit = Some(limit);
        self
    }

    /// Sets the maximum tolerated annualized turnover.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::ResultFilter;
    ///
    /// assert_eq!(
    ///     ResultFilter::new().with_max_annualized_turnover(12.0).max_annualized_turnover,
    ///     Some(12.0),
    /// );
    /// ```
    #[must_use]
    pub fn with_max_annualized_turnover(mut self, limit: f64) -> Self {
        self.max_annualized_turnover = Some(limit);
        self
    }

    /// Returns `true` when `report` satisfies every configured constraint.
    ///
    /// Non-finite report statistics never satisfy a configured constraint.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::{
    ///     CalendarTrigger, MultiAssetFeed, PortfolioBacktestConfig, PortfolioBacktestEngine,
    ///     StaticWeightAllocator,
    /// };
    /// use kurumi_backtest::validation::ResultFilter;
    /// use kurumi_backtest::{NoFee, NoSlippage};
    /// use kurumi_ta::OhlcvBar;
    /// use rust_decimal::Decimal;
    /// use std::collections::BTreeMap;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let mut series = BTreeMap::new();
    /// series.insert(
    ///     "AAA".to_owned(),
    ///     (0..6)
    ///         .map(|day| {
    ///             OhlcvBar::new(
    ///                 day,
    ///                 100.0 + day as f64,
    ///                 101.0 + day as f64,
    ///                 99.0 + day as f64,
    ///                 100.0 + day as f64,
    ///                 1_000.0,
    ///                 100_000.0,
    ///             )
    ///         })
    ///         .collect::<Result<Vec<_>, _>>()?,
    /// );
    /// let feed = MultiAssetFeed::new(series)?;
    /// let config = PortfolioBacktestConfig::new(Decimal::from(10_000))?;
    /// let mut engine = PortfolioBacktestEngine::new(config, Box::new(NoFee), Box::new(NoSlippage))?;
    /// let mut allocator = StaticWeightAllocator::new(BTreeMap::from([("AAA".to_owned(), 1.0)]))?;
    /// let report = engine.run(feed, &CalendarTrigger::Monthly, &mut allocator)?;
    ///
    /// assert!(ResultFilter::new().passes(&report));
    /// assert!(!ResultFilter::new().with_max_drawdown_limit(-1.0).passes(&report));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn passes(&self, report: &PortfolioBacktestReport) -> bool {
        if let Some(minimum) = self.min_rebalance_count {
            if report.rebalance_count < minimum {
                return false;
            }
        }
        if let Some(limit) = self.max_drawdown_limit {
            if !limit.is_finite() || limit < 0.0 {
                return false;
            }
            if !report.metrics.max_drawdown.is_finite() || report.metrics.max_drawdown > limit {
                return false;
            }
        }
        if let Some(limit) = self.max_annualized_turnover {
            if !limit.is_finite() || limit < 0.0 {
                return false;
            }
            if !report.annualized_turnover.is_finite() || report.annualized_turnover > limit {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use rust_decimal::Decimal;

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
    fn objective_metrics_score_reports_and_negate_drawdown() {
        let report = report(&[1_000, 1_020, 990, 1_030]);
        assert_eq!(
            ObjectiveMetric::TotalReturn.score(&report),
            Some(report.metrics.total_return)
        );
        assert_eq!(
            ObjectiveMetric::AnnualizedReturn.score(&report),
            Some(report.metrics.annualized_return)
        );
        assert_eq!(
            ObjectiveMetric::MinMaxDrawdown.score(&report),
            Some(-report.metrics.max_drawdown)
        );
        assert_eq!(
            ObjectiveMetric::SharpeRatio.score(&report),
            report.metrics.sharpe_ratio
        );
        assert_eq!(
            ObjectiveMetric::SortinoRatio.score(&report),
            report.metrics.sortino_ratio
        );
        assert_eq!(
            ObjectiveMetric::CalmarRatio.score(&report),
            report.metrics.calmar_ratio
        );
    }

    #[test]
    fn objective_names_and_custom_scores_are_reported() {
        assert_eq!(ObjectiveMetric::SharpeRatio.name(), "SharpeRatio");
        assert_eq!(ObjectiveMetric::SortinoRatio.name(), "SortinoRatio");
        assert_eq!(ObjectiveMetric::CalmarRatio.name(), "CalmarRatio");
        assert_eq!(ObjectiveMetric::TotalReturn.name(), "TotalReturn");
        assert_eq!(ObjectiveMetric::AnnualizedReturn.name(), "AnnualizedReturn");
        assert_eq!(ObjectiveMetric::MinMaxDrawdown.name(), "MinMaxDrawdown");
        assert_eq!(
            ObjectiveMetric::custom(|report| report.metrics.total_return).name(),
            "Custom"
        );
    }

    #[test]
    fn non_finite_custom_scores_are_not_ranked() {
        let report = report(&[1_000, 1_010, 1_020, 1_030]);
        assert!(
            ObjectiveMetric::custom(|_report| f64::NAN)
                .score(&report)
                .is_none()
        );
        assert!(
            ObjectiveMetric::custom(|_report| f64::INFINITY)
                .score(&report)
                .is_none()
        );
        assert_eq!(
            ObjectiveMetric::custom(|_report| 2.5).score(&report),
            Some(2.5)
        );
    }

    #[test]
    fn result_filter_enforces_every_configured_boundary_inclusively() {
        let report = report(&[1_000, 1_020, 990, 1_030]);
        assert!(ResultFilter::new().passes(&report));
        assert!(
            ResultFilter::new()
                .with_min_rebalance_count(0)
                .passes(&report)
        );
        assert!(
            !ResultFilter::new()
                .with_min_rebalance_count(1)
                .passes(&report)
        );
        assert!(
            ResultFilter::new()
                .with_max_drawdown_limit(report.metrics.max_drawdown)
                .passes(&report)
        );
        assert!(
            !ResultFilter::new()
                .with_max_drawdown_limit(report.metrics.max_drawdown - 1e-12)
                .passes(&report)
        );
        assert!(
            !ResultFilter::new()
                .with_max_drawdown_limit(f64::NAN)
                .passes(&report)
        );
        assert!(
            !ResultFilter::new()
                .with_max_annualized_turnover(-1.0)
                .passes(&report)
        );
    }

    #[test]
    fn non_finite_report_statistics_never_pass_a_configured_filter() {
        let mut report = report(&[1_000, 1_010, 1_020, 1_030]);
        report.metrics.max_drawdown = f64::NAN;
        assert!(
            !ResultFilter::new()
                .with_max_drawdown_limit(1.0)
                .passes(&report)
        );
        report.metrics.max_drawdown = 0.0;
        report.annualized_turnover = f64::INFINITY;
        assert!(
            !ResultFilter::new()
                .with_max_annualized_turnover(1.0)
                .passes(&report)
        );
    }
}
