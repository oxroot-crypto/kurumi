//! Target-weight allocation for multi-asset backtests.
//!
//! [`TargetWeightAllocator`] is the bridge between the backtest loop and the
//! portfolio construction layer. At each rebalance the engine hands the
//! allocator an [`AllocationContext`] containing the current timestamp, the
//! instrument universe, and the return history truncated to the configured
//! lookback window. The allocator answers with a [`PortfolioWeights`] target,
//! which the engine then turns into sell-before-buy orders.
//!
//! Three allocators are provided out of the box:
//!
//! - [`StaticWeightAllocator`] holds a fixed target such as a 60/40 portfolio;
//! - [`RollingOptimizerAllocator`] refits any [`CovarianceEstimator`] and
//!   [`PortfolioOptimizer`] on the rolling return history at every rebalance;
//! - [`CustomAllocator`] adapts a closure for discretionary or factor-based
//!   targets.
//!
//! The context never contains future information: the return matrix is built
//! from observations up to and including the signal timestamp only, so the same
//! allocator can be used with either [`ExecutionTiming`](crate::portfolio::ExecutionTiming)
//! convention.
//!
//! # Examples
//!
//! ```
//! use kurumi_backtest::portfolio::{
//!     AllocationContext, StaticWeightAllocator, TargetWeightAllocator,
//! };
//! use rust_decimal::Decimal;
//! use std::collections::BTreeMap;
//!
//! # fn main() -> Result<(), kurumi_backtest::KurumiError> {
//! let mut allocator = StaticWeightAllocator::new(BTreeMap::from([
//!     ("STOCK".to_owned(), 0.6),
//!     ("BOND".to_owned(), 0.4),
//! ]))?;
//! let context = AllocationContext::new(
//!     kurumi_backtest::portfolio::chrono::DateTime::from_timestamp_millis(0)
//!         .expect("epoch"),
//!     vec!["STOCK".to_owned(), "BOND".to_owned()],
//!     None,
//! );
//! let weights = allocator.allocate(&context)?;
//! assert_eq!(weights.weight("STOCK")?, 0.6);
//! # let _ = Decimal::ONE;
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use kurumi_core::KurumiError;
use kurumi_math::CovarianceEstimator;
use kurumi_portfolio::nalgebra::DVector;
use kurumi_portfolio::{PortfolioOptimizer, PortfolioWeights, ReturnMatrix};

/// The history an allocator sees at one rebalance decision.
///
/// `symbols` is the engine's current universe, sorted by symbol. `returns`
/// holds only the assets with at least two price observations so far and is
/// already truncated to [`PortfolioBacktestConfig::lookback_window`](crate::portfolio::PortfolioBacktestConfig::lookback_window)
/// rows; its columns may therefore be a subset of `symbols`. It is `None` until
/// the first return can be computed.
///
/// The matrix is `(T, N)`: rows are chronological return observations and
/// columns are assets. Every observation is at or before `timestamp`, which is
/// what makes rolling optimization free of look-ahead bias.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::AllocationContext;
/// use kurumi_backtest::portfolio::chrono::{DateTime, Utc};
///
/// let timestamp = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
/// let context = AllocationContext::new(timestamp, vec!["AAA".to_owned()], None);
/// assert_eq!(context.symbols, ["AAA"]);
/// assert!(context.returns.is_none());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct AllocationContext {
    /// Timestamp of the observation that triggered the allocation.
    pub timestamp: DateTime<Utc>,
    /// Current instrument universe, sorted by symbol.
    pub symbols: Vec<String>,
    /// Return history through `timestamp`, or `None` before any return exists.
    pub returns: Option<ReturnMatrix>,
}

impl AllocationContext {
    /// Creates an allocation context.
    ///
    /// The constructor deliberately does not validate the return matrix: it is
    /// produced by the engine from validated [`ReturnMatrix`] columns, and
    /// allocators validate whatever they consume.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::AllocationContext;
    /// use kurumi_backtest::portfolio::chrono::{DateTime, Utc};
    ///
    /// let timestamp = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
    /// let context = AllocationContext::new(timestamp, Vec::new(), None);
    /// assert_eq!(context.symbols.len(), 0);
    /// ```
    #[must_use]
    pub fn new(
        timestamp: DateTime<Utc>,
        symbols: Vec<String>,
        returns: Option<ReturnMatrix>,
    ) -> Self {
        Self {
            timestamp,
            symbols,
            returns,
        }
    }

    /// Returns the number of historical return observations available.
    ///
    /// This is `0` when [`AllocationContext::returns`] is `None`, which lets a
    /// rolling allocator compare directly against its minimum lookback.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::AllocationContext;
    /// use kurumi_backtest::portfolio::chrono::{DateTime, Utc};
    ///
    /// let timestamp = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
    /// let context = AllocationContext::new(timestamp, Vec::new(), None);
    /// assert_eq!(context.return_count(), 0);
    /// ```
    #[must_use]
    #[inline]
    pub fn return_count(&self) -> usize {
        self.returns
            .as_ref()
            .map_or(0, |matrix| matrix.dimensions().0)
    }
}

/// Produces a target portfolio allocation at a rebalance decision.
///
/// Implementations receive the truncated history in [`AllocationContext`] and
/// must return a validated [`PortfolioWeights`]. The engine accepts long-only
/// weights (a small negative round-off is clamped to zero) and scales the
/// target to the tradable investable fraction of equity.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::{
///     AllocationContext, CustomAllocator, TargetWeightAllocator,
/// };
/// use kurumi_portfolio::{nalgebra::DVector, PortfolioWeights};
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let mut allocator = CustomAllocator::new(|context: &AllocationContext| {
///     let weight = 1.0 / context.symbols.len() as f64;
///     PortfolioWeights::new(
///         context.symbols.clone(),
///         DVector::from_element(context.symbols.len(), weight),
///     )
/// });
/// let context = AllocationContext::new(
///     kurumi_backtest::portfolio::chrono::DateTime::from_timestamp_millis(0)
///         .expect("epoch"),
///     vec!["AAA".to_owned(), "BBB".to_owned()],
///     None,
/// );
/// let weights = allocator.allocate(&context)?;
/// assert!((weights.weight("AAA")? - 0.5).abs() < 1e-15);
/// # Ok(())
/// # }
/// ```
pub trait TargetWeightAllocator {
    /// Returns the target weights for the current observation.
    ///
    /// # Errors
    ///
    /// Returns a typed [`KurumiError`] when the allocator cannot produce a
    /// valid allocation for the supplied history.
    fn allocate(&mut self, ctx: &AllocationContext) -> Result<PortfolioWeights, KurumiError>;
}

/// A fixed target allocation, such as a 60/40 stock/bond portfolio.
///
/// Symbols outside the configured map receive a zero target, and configured
/// symbols missing from the current universe are ignored. The target is
/// validated to be long-only and to sum to at most one, so the residual is held
/// as cash.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::{AllocationContext, StaticWeightAllocator, TargetWeightAllocator};
/// use std::collections::BTreeMap;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let mut allocator = StaticWeightAllocator::new(BTreeMap::from([
///     ("STOCK".to_owned(), 0.6),
///     ("BOND".to_owned(), 0.4),
/// ]))?;
/// let context = AllocationContext::new(
///     kurumi_backtest::portfolio::chrono::DateTime::from_timestamp_millis(0)
///         .expect("epoch"),
///     vec!["STOCK".to_owned(), "BOND".to_owned()],
///     None,
/// );
/// let weights = allocator.allocate(&context)?;
/// assert_eq!(weights.weight("BOND")?, 0.4);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct StaticWeightAllocator {
    weights: BTreeMap<String, f64>,
}

impl StaticWeightAllocator {
    /// Creates a fixed-weight allocator.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the map is empty, a
    /// symbol is empty, a weight is negative or not finite, or the weights sum
    /// to more than one (within `1e-9` rounding tolerance).
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::StaticWeightAllocator;
    /// use std::collections::BTreeMap;
    ///
    /// assert!(StaticWeightAllocator::new(BTreeMap::new()).is_err());
    /// assert!(StaticWeightAllocator::new(BTreeMap::from([
    ///     ("AAA".to_owned(), 0.6),
    ///     ("BBB".to_owned(), 0.4),
    /// ]))
    /// .is_ok());
    /// assert!(StaticWeightAllocator::new(BTreeMap::from([
    ///     ("AAA".to_owned(), 0.7),
    ///     ("BBB".to_owned(), 0.4),
    /// ]))
    /// .is_err());
    /// ```
    pub fn new(weights: BTreeMap<String, f64>) -> Result<Self, KurumiError> {
        if weights.is_empty() {
            return Err(KurumiError::InvalidParameter {
                parameter: "weights",
                reason: "must contain at least one symbol".to_owned(),
            });
        }
        let mut total = 0.0;
        for (symbol, weight) in &weights {
            if symbol.trim().is_empty() {
                return Err(KurumiError::InvalidParameter {
                    parameter: "weights",
                    reason: "symbols must not be empty".to_owned(),
                });
            }
            if !weight.is_finite() || *weight < 0.0 {
                return Err(KurumiError::InvalidParameter {
                    parameter: "weights",
                    reason: format!("weight of `{symbol}` must be finite and non-negative"),
                });
            }
            total += *weight;
        }
        if !total.is_finite() || total > 1.0 + 1e-9 {
            return Err(KurumiError::InvalidParameter {
                parameter: "weights",
                reason: format!("weights sum to {total}, which exceeds one"),
            });
        }
        Ok(Self { weights })
    }

    /// Returns the configured fixed weights.
    #[must_use]
    #[inline]
    pub fn weights(&self) -> &BTreeMap<String, f64> {
        &self.weights
    }
}

impl TargetWeightAllocator for StaticWeightAllocator {
    fn allocate(&mut self, ctx: &AllocationContext) -> Result<PortfolioWeights, KurumiError> {
        let weights = ctx
            .symbols
            .iter()
            .map(|symbol| self.weights.get(symbol).copied().unwrap_or(0.0))
            .collect::<Vec<_>>();
        PortfolioWeights::new(ctx.symbols.clone(), DVector::from_vec(weights))
    }
}

/// What a rolling allocator does before it has enough history.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::InsufficientDataPolicy;
///
/// assert_eq!(
///     InsufficientDataPolicy::default(),
///     InsufficientDataPolicy::EqualWeight
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InsufficientDataPolicy {
    /// Allocate equally across the universe.
    #[default]
    EqualWeight,
    /// Hold everything in cash until the lookback requirement is met.
    Cash,
}

impl InsufficientDataPolicy {
    /// Returns the fallback target for `symbols`.
    fn fallback(self, symbols: &[String]) -> Result<PortfolioWeights, KurumiError> {
        if symbols.is_empty() {
            return Err(KurumiError::InsufficientData {
                required: 1,
                available: 0,
            });
        }
        let weight = match self {
            Self::EqualWeight => 1.0 / symbols.len() as f64,
            Self::Cash => 0.0,
        };
        PortfolioWeights::new(
            symbols.to_vec(),
            DVector::from_element(symbols.len(), weight),
        )
    }
}

/// A rolling optimizer over any covariance estimator and portfolio optimizer.
///
/// At every rebalance the allocator estimates the `(N, N)` covariance from the
/// truncated `(T, N)` return history, computes the mean return of each asset,
/// and solves the optimizer. When fewer than [`RollingOptimizerAllocator::min_lookback_periods`]
/// returns are available it falls back according to
/// [`InsufficientDataPolicy`].
///
/// The generic parameters keep the estimator and optimizer monomorphised, so
/// this allocator accepts the same implementors that
/// [`PortfolioOptimizer`] accepts — for example
/// `LedoitWolfShrinkage` with `HierarchicalRiskParity` or `SampleCovariance`
/// with `EqualWeight`.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::{
///     AllocationContext, InsufficientDataPolicy, RollingOptimizerAllocator,
///     TargetWeightAllocator,
/// };
/// use kurumi_backtest::portfolio::chrono::{DateTime, Utc};
/// use kurumi_math::SampleCovariance;
/// use kurumi_portfolio::{nalgebra::DMatrix, EqualWeight, ReturnMatrix};
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let timestamps = (0..3)
///     .map(|day| DateTime::<Utc>::from_timestamp_millis(day * 86_400_000).expect("day"))
///     .collect::<Vec<_>>();
/// let returns = ReturnMatrix::new(
///     vec!["AAA".to_owned(), "BBB".to_owned()],
///     timestamps.clone(),
///     DMatrix::from_row_slice(3, 2, &[0.01, 0.02, 0.02, 0.01, -0.01, 0.03]),
/// )?;
/// let context = AllocationContext::new(
///     *timestamps.last().expect("last timestamp"),
///     vec!["AAA".to_owned(), "BBB".to_owned()],
///     Some(returns),
/// );
/// let mut allocator = RollingOptimizerAllocator::new(SampleCovariance, EqualWeight::new())
///     .with_min_lookback_periods(2)?
///     .with_insufficient_data_policy(InsufficientDataPolicy::EqualWeight);
/// let weights = allocator.allocate(&context)?;
/// assert!((weights.sum() - 1.0).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RollingOptimizerAllocator<E, O> {
    estimator: E,
    optimizer: O,
    min_lookback_periods: usize,
    insufficient_data: InsufficientDataPolicy,
}

impl<E, O> RollingOptimizerAllocator<E, O> {
    /// Creates a rolling allocator with a two-period minimum lookback and an
    /// equal-weight fallback.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::RollingOptimizerAllocator;
    /// use kurumi_math::SampleCovariance;
    /// use kurumi_portfolio::EqualWeight;
    ///
    /// let allocator = RollingOptimizerAllocator::new(SampleCovariance, EqualWeight::new());
    /// assert_eq!(allocator.min_lookback_periods(), 2);
    /// ```
    #[must_use]
    pub fn new(estimator: E, optimizer: O) -> Self {
        Self {
            estimator,
            optimizer,
            min_lookback_periods: 2,
            insufficient_data: InsufficientDataPolicy::EqualWeight,
        }
    }

    /// Sets the minimum number of return observations required to optimize.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `periods` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::RollingOptimizerAllocator;
    /// use kurumi_math::SampleCovariance;
    /// use kurumi_portfolio::EqualWeight;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let allocator = RollingOptimizerAllocator::new(SampleCovariance, EqualWeight::new())
    ///     .with_min_lookback_periods(60)?;
    /// assert_eq!(allocator.min_lookback_periods(), 60);
    /// assert!(
    ///     RollingOptimizerAllocator::new(SampleCovariance, EqualWeight::new())
    ///         .with_min_lookback_periods(0)
    ///         .is_err()
    /// );
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_min_lookback_periods(mut self, periods: usize) -> Result<Self, KurumiError> {
        if periods == 0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "min_lookback_periods",
                reason: "must be at least one".to_owned(),
            });
        }
        self.min_lookback_periods = periods;
        Ok(self)
    }

    /// Sets the fallback used before enough history exists.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::{
    ///     InsufficientDataPolicy, RollingOptimizerAllocator,
    /// };
    /// use kurumi_math::SampleCovariance;
    /// use kurumi_portfolio::EqualWeight;
    ///
    /// let allocator = RollingOptimizerAllocator::new(SampleCovariance, EqualWeight::new())
    ///     .with_insufficient_data_policy(InsufficientDataPolicy::Cash);
    /// assert_eq!(
    ///     allocator.insufficient_data_policy(),
    ///     InsufficientDataPolicy::Cash
    /// );
    /// ```
    #[must_use]
    pub fn with_insufficient_data_policy(mut self, policy: InsufficientDataPolicy) -> Self {
        self.insufficient_data = policy;
        self
    }

    /// Returns the minimum number of return observations required to optimize.
    #[must_use]
    #[inline]
    pub fn min_lookback_periods(&self) -> usize {
        self.min_lookback_periods
    }

    /// Returns the fallback policy used before enough history exists.
    #[must_use]
    #[inline]
    pub fn insufficient_data_policy(&self) -> InsufficientDataPolicy {
        self.insufficient_data
    }
}

impl<E, O> TargetWeightAllocator for RollingOptimizerAllocator<E, O>
where
    E: CovarianceEstimator,
    O: PortfolioOptimizer,
{
    fn allocate(&mut self, ctx: &AllocationContext) -> Result<PortfolioWeights, KurumiError> {
        let Some(returns) = ctx.returns.as_ref() else {
            return self.insufficient_data.fallback(&ctx.symbols);
        };
        if returns.dimensions().0 < self.min_lookback_periods {
            return self.insufficient_data.fallback(&ctx.symbols);
        }
        let covariance = self.estimator.estimate(returns.data())?;
        let expected_returns = returns.mean_returns()?;
        let result = self.optimizer.optimize(&expected_returns, &covariance)?;
        result.to_weights(returns.symbols().to_vec())
    }
}

/// A closure-backed allocator for custom factor or timing logic.
///
/// The closure is called with the same [`AllocationContext`] the engine passes
/// to every allocator and must return a validated [`PortfolioWeights`].
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::{AllocationContext, CustomAllocator, TargetWeightAllocator};
/// use kurumi_portfolio::{nalgebra::DVector, PortfolioWeights};
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let mut allocator = CustomAllocator::new(|context: &AllocationContext| {
///     PortfolioWeights::new(
///         context.symbols.clone(),
///         DVector::from_element(context.symbols.len(), 0.0),
///     )
/// });
/// let context = AllocationContext::new(
///     kurumi_backtest::portfolio::chrono::DateTime::from_timestamp_millis(0)
///         .expect("epoch"),
///     vec!["AAA".to_owned()],
///     None,
/// );
/// assert_eq!(allocator.allocate(&context)?.weight("AAA")?, 0.0);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct CustomAllocator<F> {
    allocate: F,
}

impl<F> CustomAllocator<F> {
    /// Creates a custom allocator from a closure.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::CustomAllocator;
    /// use kurumi_portfolio::PortfolioWeights;
    ///
    /// let _allocator = CustomAllocator::new(|_context: &kurumi_backtest::portfolio::AllocationContext| {
    ///     PortfolioWeights::new(
    ///         vec!["AAA".to_owned()],
    ///         kurumi_portfolio::nalgebra::DVector::from_vec(vec![1.0]),
    ///     )
    /// });
    /// ```
    #[must_use]
    pub fn new(allocate: F) -> Self {
        Self { allocate }
    }
}

impl<F> TargetWeightAllocator for CustomAllocator<F>
where
    F: FnMut(&AllocationContext) -> Result<PortfolioWeights, KurumiError>,
{
    fn allocate(&mut self, ctx: &AllocationContext) -> Result<PortfolioWeights, KurumiError> {
        (self.allocate)(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use kurumi_math::SampleCovariance;
    use kurumi_portfolio::nalgebra::DMatrix;
    use kurumi_portfolio::{EqualWeight, ReturnMatrix};

    fn timestamp(day: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp_millis(day * 86_400_000).expect("valid day")
    }

    fn returns_matrix(rows: &[&[f64]]) -> ReturnMatrix {
        let timestamps = (0..rows.len() as i64).map(timestamp).collect::<Vec<_>>();
        let columns = rows.first().map_or(0, |row| row.len());
        let values = rows
            .iter()
            .flat_map(|row| row.iter().copied())
            .collect::<Vec<_>>();
        ReturnMatrix::new(
            vec!["AAA".to_owned(), "BBB".to_owned()],
            timestamps,
            DMatrix::from_row_slice(rows.len(), columns, &values),
        )
        .expect("valid matrix")
    }

    #[test]
    fn static_weights_follow_the_configured_target_and_zero_others() {
        let mut allocator = StaticWeightAllocator::new(BTreeMap::from([
            ("AAA".to_owned(), 0.6),
            ("BBB".to_owned(), 0.4),
        ]))
        .expect("valid weights");
        let context = AllocationContext::new(
            timestamp(10),
            vec!["AAA".to_owned(), "BBB".to_owned(), "CCC".to_owned()],
            None,
        );
        let weights = allocator.allocate(&context).expect("allocation");
        assert!((weights.weight("AAA").expect("AAA") - 0.6).abs() < 1e-15);
        assert!((weights.weight("BBB").expect("BBB") - 0.4).abs() < 1e-15);
        assert_eq!(weights.weight("CCC").expect("CCC"), 0.0);
    }

    #[test]
    fn static_weights_validate_their_domain() {
        assert!(StaticWeightAllocator::new(BTreeMap::new()).is_err());
        assert!(StaticWeightAllocator::new(BTreeMap::from([("AAA".to_owned(), 1.5)])).is_err());
        assert!(
            StaticWeightAllocator::new(BTreeMap::from([("AAA".to_owned(), f64::NAN)])).is_err()
        );
        assert!(StaticWeightAllocator::new(BTreeMap::from([("  ".to_owned(), 0.5)])).is_err());
    }

    #[test]
    fn rolling_allocator_optimizes_once_enough_history_exists() {
        let returns =
            returns_matrix(&[&[0.01, 0.02], &[0.02, 0.01], &[-0.01, 0.03], &[0.03, -0.01]]);
        let context = AllocationContext::new(
            timestamp(10),
            vec!["AAA".to_owned(), "BBB".to_owned()],
            Some(returns),
        );
        let mut allocator = RollingOptimizerAllocator::new(SampleCovariance, EqualWeight::new())
            .with_min_lookback_periods(3)
            .expect("valid lookback");
        let weights = allocator.allocate(&context).expect("allocation");
        assert!((weights.sum() - 1.0).abs() < 1e-12);
        assert_eq!(weights.symbols(), ["AAA", "BBB"]);
    }

    #[test]
    fn rolling_allocator_falls_back_when_history_is_short() {
        let context =
            AllocationContext::new(timestamp(0), vec!["AAA".to_owned(), "BBB".to_owned()], None);
        let mut equal = RollingOptimizerAllocator::new(SampleCovariance, EqualWeight::new());
        let weights = equal.allocate(&context).expect("equal fallback");
        assert!((weights.weight("AAA").expect("AAA") - 0.5).abs() < 1e-15);

        let mut cash = RollingOptimizerAllocator::new(SampleCovariance, EqualWeight::new())
            .with_insufficient_data_policy(InsufficientDataPolicy::Cash);
        let weights = cash.allocate(&context).expect("cash fallback");
        assert_eq!(weights.weight("AAA").expect("AAA"), 0.0);
    }

    #[test]
    fn custom_allocator_adapts_a_closure() {
        let mut allocator = CustomAllocator::new(|context: &AllocationContext| {
            let value = 1.0 / context.symbols.len() as f64;
            PortfolioWeights::new(
                context.symbols.clone(),
                DVector::from_element(context.symbols.len(), value),
            )
        });
        let context = AllocationContext::new(
            timestamp(0),
            vec!["AAA".to_owned(), "BBB".to_owned(), "CCC".to_owned()],
            None,
        );
        let weights = allocator.allocate(&context).expect("allocation");
        assert!((weights.sum() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn allocation_context_reports_the_truncated_return_count() {
        let returns = returns_matrix(&[&[0.01, 0.02], &[0.02, 0.01]]);
        let context = AllocationContext::new(
            timestamp(1),
            vec!["AAA".to_owned(), "BBB".to_owned()],
            Some(returns),
        );
        assert_eq!(context.return_count(), 2);
        assert_eq!(
            AllocationContext::new(timestamp(0), Vec::new(), None).return_count(),
            0
        );
    }

    #[test]
    fn rolling_allocator_rejects_zero_minimum_lookback() {
        assert!(
            RollingOptimizerAllocator::new(SampleCovariance, EqualWeight::new())
                .with_min_lookback_periods(0)
                .is_err()
        );
    }
}
