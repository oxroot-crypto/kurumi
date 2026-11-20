//! Parallel parameter grid search over an immutable market timeline.
//!
//! [`GridSearchRunner`] evaluates a caller-supplied strategy factory for every
//! parameter combination and ranks the resulting [`PortfolioBacktestReport`]s
//! with an [`ObjectiveMetric`] and a [`ResultFilter`].
//!
//! # Shared, zero-copy market data
//!
//! Every trial borrows the same [`MarketSlice`]; the historical snapshots are
//! never cloned per parameter combination. The runner only hands each trial a
//! fresh [`PortfolioBacktestEngine`] and the strategy's own allocator.
//!
//! # Feature gating
//!
//! Trials run through [`rayon`](https://docs.rs/rayon) when the `parallel`
//! feature is enabled, which is the crate default. Without the feature the same
//! iterator pipeline runs sequentially, so the two configurations produce the
//! same trials in the same order.
//!
//! # Determinism
//!
//! Trials are stored in original parameter order. Ranking ignores failed
//! trials, trials that fail the [`ResultFilter`], and trials whose score is
//! `None` or non-finite; the winner is the largest finite score, with ties
//! broken by the original grid index (smaller first). [`GridSearchResult::best`]
//! and [`GridSearchResult::top_k`] apply the same rule, so a parallel run and a
//! manual sequential run select identical winners.
//!
//! A single trial that cannot run — for example because its parameter set fails
//! configuration validation — is recorded as [`TrialStatus::Failed`] with the
//! formatted error; it never aborts the rest of the sweep.

use std::cmp::Ordering;
use std::fmt;

use kurumi_core::KurumiError;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

use crate::fee::{FeeModel, NoFee};
use crate::portfolio::{
    MarketSlice, PortfolioBacktestConfig, PortfolioBacktestEngine, PortfolioBacktestReport,
    RebalanceTrigger, TargetWeightAllocator,
};
use crate::slippage::{NoSlippage, SlippageModel};

use super::objective::{ObjectiveMetric, ResultFilter};

/// The complete recipe for one backtest inside a parameter grid.
///
/// A [`GridSearchRunner`] calls the caller's factory once per parameter
/// combination and receives a `StrategySpec`, so each trial gets its own engine,
/// trigger, and allocator while sharing the immutable [`MarketSlice`].
///
/// The default cost models are [`NoFee`] and [`NoSlippage`] (zero-cost fills).
/// Use [`StrategySpec::with_fee_model`] and
/// [`StrategySpec::with_slippage_model`] to supply the same models the engine
/// accepts directly.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::{
///     CalendarTrigger, PortfolioBacktestConfig, StaticWeightAllocator,
/// };
/// use kurumi_backtest::validation::StrategySpec;
/// use rust_decimal::Decimal;
/// use std::collections::BTreeMap;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let spec = StrategySpec::new(
///     PortfolioBacktestConfig::new(Decimal::from(10_000))?,
///     Box::new(CalendarTrigger::Monthly),
///     Box::new(StaticWeightAllocator::new(BTreeMap::from([(
///         "AAA".to_owned(),
///         1.0,
///     )]))?),
/// );
/// assert_eq!(spec.config().initial_cash, Decimal::from(10_000));
/// # Ok(())
/// # }
/// ```
pub struct StrategySpec {
    config: PortfolioBacktestConfig,
    trigger: Box<dyn RebalanceTrigger>,
    allocator: Box<dyn TargetWeightAllocator>,
    fee_model: Option<Box<dyn FeeModel>>,
    slippage_model: Option<Box<dyn SlippageModel>>,
}

impl StrategySpec {
    /// Creates a strategy specification with zero-cost fills.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::{
    ///     CalendarTrigger, PortfolioBacktestConfig, StaticWeightAllocator,
    /// };
    /// use kurumi_backtest::validation::StrategySpec;
    /// use rust_decimal::Decimal;
    /// use std::collections::BTreeMap;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let _spec = StrategySpec::new(
    ///     PortfolioBacktestConfig::new(Decimal::from(10_000))?,
    ///     Box::new(CalendarTrigger::Monthly),
    ///     Box::new(StaticWeightAllocator::new(BTreeMap::from([(
    ///         "AAA".to_owned(),
    ///         1.0,
    ///     )]))?),
    /// );
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn new(
        config: PortfolioBacktestConfig,
        trigger: Box<dyn RebalanceTrigger>,
        allocator: Box<dyn TargetWeightAllocator>,
    ) -> Self {
        Self {
            config,
            trigger,
            allocator,
            fee_model: None,
            slippage_model: None,
        }
    }

    /// Returns the strategy configuration.
    #[must_use]
    #[inline]
    pub fn config(&self) -> &PortfolioBacktestConfig {
        &self.config
    }

    /// Supplies the fee model used by the trial; defaults to [`NoFee`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::{
    ///     CalendarTrigger, PortfolioBacktestConfig, StaticWeightAllocator,
    /// };
    /// use kurumi_backtest::validation::StrategySpec;
    /// use kurumi_backtest::NoFee;
    /// use rust_decimal::Decimal;
    /// use std::collections::BTreeMap;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let spec = StrategySpec::new(
    ///     PortfolioBacktestConfig::new(Decimal::from(10_000))?,
    ///     Box::new(CalendarTrigger::Monthly),
    ///     Box::new(StaticWeightAllocator::new(BTreeMap::from([(
    ///         "AAA".to_owned(),
    ///         1.0,
    ///     )]))?),
    /// )
    /// .with_fee_model(Box::new(NoFee));
    /// assert_eq!(spec.config().initial_cash, Decimal::from(10_000));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_fee_model(mut self, fee_model: Box<dyn FeeModel>) -> Self {
        self.fee_model = Some(fee_model);
        self
    }

    /// Supplies the slippage model used by the trial; defaults to
    /// [`NoSlippage`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::{
    ///     CalendarTrigger, PortfolioBacktestConfig, StaticWeightAllocator,
    /// };
    /// use kurumi_backtest::validation::StrategySpec;
    /// use kurumi_backtest::NoSlippage;
    /// use rust_decimal::Decimal;
    /// use std::collections::BTreeMap;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let spec = StrategySpec::new(
    ///     PortfolioBacktestConfig::new(Decimal::from(10_000))?,
    ///     Box::new(CalendarTrigger::Monthly),
    ///     Box::new(StaticWeightAllocator::new(BTreeMap::from([(
    ///         "AAA".to_owned(),
    ///         1.0,
    ///     )]))?),
    /// )
    /// .with_slippage_model(Box::new(NoSlippage));
    /// assert_eq!(spec.config().initial_cash, Decimal::from(10_000));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_slippage_model(mut self, slippage_model: Box<dyn SlippageModel>) -> Self {
        self.slippage_model = Some(slippage_model);
        self
    }
}

impl fmt::Debug for StrategySpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StrategySpec")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// Whether a single grid trial produced a report.
///
/// A failed trial carries the formatted error that stopped it and never aborts
/// the remaining parameter combinations.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::validation::TrialStatus;
///
/// let status = TrialStatus::Failed("lookback_window must be at least one".to_owned());
/// assert!(matches!(status, TrialStatus::Failed(_)));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrialStatus {
    /// The backtest completed and produced a report.
    Succeeded,
    /// The backtest could not run; the string is the formatted error.
    Failed(String),
}

impl TrialStatus {
    /// Returns `true` when the trial failed.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::TrialStatus;
    ///
    /// assert!(!TrialStatus::Succeeded.is_failed());
    /// assert!(TrialStatus::Failed("boom".to_owned()).is_failed());
    /// ```
    #[must_use]
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

/// The compact performance summary retained for every successful trial.
///
/// These are the fields a wide sweep keeps when
/// [`GridSearchConfig::retain_all_reports`] is `false`; the complete report is
/// kept only for the winning parameter set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrialSummary {
    /// Number of return observations behind the performance metrics.
    pub observations: usize,
    /// Cumulative simple return over the evaluated segment.
    pub total_return: f64,
    /// Geometric annualized return over the evaluated segment.
    pub annualized_return: f64,
    /// Annualized Sharpe ratio; `None` when volatility is zero.
    pub sharpe_ratio: Option<f64>,
    /// Annualized Sortino ratio; `None` when downside deviation is zero.
    pub sortino_ratio: Option<f64>,
    /// Annualized return divided by the maximum drawdown.
    pub calmar_ratio: Option<f64>,
    /// Largest peak-to-trough decline as a non-negative fraction.
    pub max_drawdown: f64,
    /// Annualized one-way turnover.
    pub annualized_turnover: f64,
    /// Number of rebalances recorded.
    pub rebalance_count: usize,
    /// Equity at the last evaluated observation.
    pub final_equity: rust_decimal::Decimal,
    /// Cumulative fees charged.
    pub total_fees: rust_decimal::Decimal,
    /// Cumulative slippage cost.
    pub total_slippage: rust_decimal::Decimal,
}

impl TrialSummary {
    fn from_report(report: &PortfolioBacktestReport) -> Self {
        Self {
            observations: report.metrics.observations,
            total_return: report.metrics.total_return,
            annualized_return: report.metrics.annualized_return,
            sharpe_ratio: report.metrics.sharpe_ratio,
            sortino_ratio: report.metrics.sortino_ratio,
            calmar_ratio: report.metrics.calmar_ratio,
            max_drawdown: report.metrics.max_drawdown,
            annualized_turnover: report.annualized_turnover,
            rebalance_count: report.rebalance_count,
            final_equity: report.final_equity,
            total_fees: report.total_fees,
            total_slippage: report.total_slippage,
        }
    }
}

/// One parameter combination and its outcome.
///
/// Successful trials always carry a summary; the full
/// [`PortfolioBacktestReport`] is present when
/// [`GridSearchConfig::retain_all_reports`] is `true` or when this trial is the
/// winner.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::validation::{GridTrial, TrialStatus};
///
/// let trial = GridTrial::<usize> {
///     param_index: 0,
///     params: 7,
///     status: TrialStatus::Succeeded,
///     score: Some(1.0),
///     passed_filter: true,
///     summary: None,
///     report: None,
/// };
/// assert!(trial.is_ranked());
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct GridTrial<P> {
    /// Original zero-based position of the parameter combination in the grid.
    pub param_index: usize,
    /// The parameter combination evaluated by this trial.
    pub params: P,
    /// Whether the backtest ran.
    pub status: TrialStatus,
    /// Finite objective value, or `None` when the metric is undefined.
    pub score: Option<f64>,
    /// Whether the report satisfied the [`ResultFilter`].
    pub passed_filter: bool,
    /// Key performance statistics of a successful trial.
    pub summary: Option<TrialSummary>,
    /// Full report, retained for every trial or only for the winner.
    pub report: Option<PortfolioBacktestReport>,
}

impl<P> GridTrial<P> {
    /// Returns `true` when this trial can compete for the best parameter.
    ///
    /// A trial is ranked only when it succeeded, passed the filter, and has a
    /// finite score.
    #[must_use]
    pub fn is_ranked(&self) -> bool {
        matches!(&self.status, TrialStatus::Succeeded)
            && self.passed_filter
            && self.score.is_some_and(f64::is_finite)
    }

    fn failed(param_index: usize, params: P, error: &KurumiError) -> Self {
        Self {
            param_index,
            params,
            status: TrialStatus::Failed(error.to_string()),
            score: None,
            passed_filter: false,
            summary: None,
            report: None,
        }
    }
}

/// Configuration of a parameter grid search.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::validation::{GridSearchConfig, ObjectiveMetric, ResultFilter};
///
/// let config = GridSearchConfig::new(ObjectiveMetric::SharpeRatio)
///     .with_filter(ResultFilter::new().with_max_drawdown_limit(0.25))
///     .with_warmup_bars(60)
///     .with_retain_all_reports(false);
/// assert_eq!(config.warmup_bars, 60);
/// ```
#[derive(Debug, Clone)]
pub struct GridSearchConfig {
    /// Ranking objective; larger scores win.
    pub objective: ObjectiveMetric,
    /// Admissibility constraints applied before ranking.
    pub filter: ResultFilter,
    /// Number of leading observations used to warm up indicator history.
    pub warmup_bars: usize,
    /// Whether to keep every successful trial's full report.
    ///
    /// When `false`, each successful trial keeps only a [`TrialSummary`] and the
    /// winner keeps its complete [`PortfolioBacktestReport`], which bounds
    /// memory for large sweeps.
    pub retain_all_reports: bool,
}

impl GridSearchConfig {
    /// Creates a configuration with no warmup, a permissive filter, and compact
    /// report retention.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::{GridSearchConfig, ObjectiveMetric};
    ///
    /// let config = GridSearchConfig::new(ObjectiveMetric::TotalReturn);
    /// assert_eq!(config.warmup_bars, 0);
    /// assert!(!config.retain_all_reports);
    /// ```
    #[must_use]
    pub fn new(objective: ObjectiveMetric) -> Self {
        Self {
            objective,
            filter: ResultFilter::default(),
            warmup_bars: 0,
            retain_all_reports: false,
        }
    }

    /// Sets the admissibility constraints.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::{GridSearchConfig, ObjectiveMetric, ResultFilter};
    ///
    /// let config = GridSearchConfig::new(ObjectiveMetric::SharpeRatio)
    ///     .with_filter(ResultFilter::new().with_min_rebalance_count(2));
    /// assert_eq!(config.filter.min_rebalance_count, Some(2));
    /// ```
    #[must_use]
    pub fn with_filter(mut self, filter: ResultFilter) -> Self {
        self.filter = filter;
        self
    }

    /// Sets the warmup prefix length, in observations.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::{GridSearchConfig, ObjectiveMetric};
    ///
    /// let config = GridSearchConfig::new(ObjectiveMetric::SharpeRatio).with_warmup_bars(60);
    /// assert_eq!(config.warmup_bars, 60);
    /// ```
    #[must_use]
    pub fn with_warmup_bars(mut self, warmup_bars: usize) -> Self {
        self.warmup_bars = warmup_bars;
        self
    }

    /// Sets whether every successful trial keeps its full report.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::{GridSearchConfig, ObjectiveMetric};
    ///
    /// let config = GridSearchConfig::new(ObjectiveMetric::SharpeRatio).with_retain_all_reports(true);
    /// assert!(config.retain_all_reports);
    /// ```
    #[must_use]
    pub fn with_retain_all_reports(mut self, retain_all_reports: bool) -> Self {
        self.retain_all_reports = retain_all_reports;
        self
    }
}

/// Runs a parameter grid over shared market data.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::validation::{GridSearchConfig, GridSearchRunner, ObjectiveMetric};
///
/// let runner = GridSearchRunner::new(GridSearchConfig::new(ObjectiveMetric::SharpeRatio));
/// assert_eq!(runner.config().warmup_bars, 0);
/// ```
#[derive(Debug, Clone)]
pub struct GridSearchRunner {
    config: GridSearchConfig,
}

impl GridSearchRunner {
    /// Creates a runner from a validated-at-use configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::{GridSearchConfig, GridSearchRunner, ObjectiveMetric};
    ///
    /// let runner = GridSearchRunner::new(GridSearchConfig::new(ObjectiveMetric::SharpeRatio));
    /// assert_eq!(runner.config().warmup_bars, 0);
    /// ```
    #[must_use]
    pub fn new(config: GridSearchConfig) -> Self {
        Self { config }
    }

    /// Returns the runner configuration.
    #[must_use]
    #[inline]
    pub fn config(&self) -> &GridSearchConfig {
        &self.config
    }

    /// Evaluates `factory` for every parameter in `params`.
    ///
    /// The market data is borrowed for the whole sweep, so no historical bar is
    /// cloned per parameter combination. A factory or backtest error marks only
    /// the affected trial as [`TrialStatus::Failed`] and keeps the remaining
    /// trials running.
    ///
    /// When [`GridSearchConfig::retain_all_reports`] is `false`, only summaries
    /// are kept during the sweep and the winning trial is re-evaluated once so
    /// its complete report can be retained, which doubles the cost of that one
    /// trial while keeping peak memory bounded.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InsufficientData`] for an empty slice,
    /// [`KurumiError::InvalidParameter`] when
    /// [`GridSearchConfig::warmup_bars`] is not smaller than the slice length,
    /// and propagates no other error: individual trial failures are recorded in
    /// the result.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::{
    ///     AlignedMarketData, CalendarTrigger, MultiAssetFeed, PortfolioBacktestConfig,
    ///     StaticWeightAllocator,
    /// };
    /// use kurumi_backtest::validation::{
    ///     GridSearchConfig, GridSearchRunner, ObjectiveMetric, ResultFilter, StrategySpec,
    /// };
    /// use kurumi_ta::OhlcvBar;
    /// use rust_decimal::Decimal;
    /// use std::collections::BTreeMap;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let mut series = BTreeMap::new();
    /// series.insert(
    ///     "AAA".to_owned(),
    ///     (0..12)
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
    /// let data = AlignedMarketData::from_feed(MultiAssetFeed::new(series)?);
    ///
    /// let runner = GridSearchRunner::new(
    ///     GridSearchConfig::new(ObjectiveMetric::TotalReturn)
    ///         .with_filter(ResultFilter::new().with_min_rebalance_count(1))
    ///         .with_warmup_bars(2),
    /// );
    /// let params = vec![2usize, 3usize, 4usize];
    /// let result = runner.run(data.slice(), &params, |lookback| {
    ///     let mut config = PortfolioBacktestConfig::new(Decimal::from(100_000))?;
    ///     config.lookback_window = *lookback;
    ///     let allocator = StaticWeightAllocator::new(BTreeMap::from([(
    ///         "AAA".to_owned(),
    ///         1.0,
    ///     )]))?;
    ///     Ok(StrategySpec::new(
    ///         config,
    ///         Box::new(CalendarTrigger::Monthly),
    ///         Box::new(allocator),
    ///     ))
    /// })?;
    ///
    /// assert_eq!(result.trials.len(), 3);
    /// assert!(result.best().is_some());
    /// # Ok(())
    /// # }
    /// ```
    pub fn run<P, F>(
        &self,
        data: MarketSlice<'_>,
        params: &[P],
        factory: F,
    ) -> Result<GridSearchResult<P>, KurumiError>
    where
        P: Clone + Send + Sync,
        F: Fn(&P) -> Result<StrategySpec, KurumiError> + Sync,
    {
        if data.is_empty() {
            return Err(KurumiError::InsufficientData {
                required: 1,
                available: 0,
            });
        }
        if self.config.warmup_bars >= data.len() {
            return Err(KurumiError::InvalidParameter {
                parameter: "warmup_bars",
                reason: format!(
                    "must be smaller than the {} observations in the market slice",
                    data.len()
                ),
            });
        }

        let config = &self.config;
        let factory = &factory;

        #[cfg(feature = "parallel")]
        let trials = params
            .par_iter()
            .enumerate()
            .map(|(param_index, params)| {
                evaluate_trial(param_index, params.clone(), data, config, factory)
            })
            .collect::<Vec<_>>();

        #[cfg(not(feature = "parallel"))]
        let trials = params
            .iter()
            .enumerate()
            .map(|(param_index, params)| {
                evaluate_trial(param_index, params.clone(), data, config, factory)
            })
            .collect::<Vec<_>>();

        let mut result = GridSearchResult {
            best_index: select_best(&trials),
            trials,
        };

        if !config.retain_all_reports {
            if let Some(best_index) = result.best_index {
                let retained = params.get(best_index).and_then(|params| {
                    let spec = factory(params).ok()?;
                    run_strategy(spec, data, config.warmup_bars).ok()
                });
                if let Some(trial) = result
                    .trials
                    .iter_mut()
                    .find(|trial| trial.param_index == best_index)
                {
                    trial.report = retained;
                }
            }
        }

        Ok(result)
    }
}

/// The outcome of a completed parameter grid search.
///
/// `trials` preserves the original parameter order. `best_index` is the
/// `param_index` of the winning trial, or `None` when no trial produced a finite,
/// admissible score.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::validation::{GridSearchResult, GridTrial, TrialStatus};
///
/// let trial = |param_index: usize, score: Option<f64>, passed_filter: bool| GridTrial {
///     param_index,
///     params: param_index,
///     status: TrialStatus::Succeeded,
///     score,
///     passed_filter,
///     summary: None,
///     report: None,
/// };
/// let result = GridSearchResult {
///     trials: vec![
///         trial(0, Some(2.0), true),
///         trial(1, Some(2.0), true),
///         trial(2, Some(1.0), true),
///         trial(3, None, false),
///     ],
///     best_index: Some(0),
/// };
/// assert_eq!(result.best().map(|trial| trial.param_index), Some(0));
/// let top = result.top_k(2);
/// assert_eq!(
///     top.iter().map(|trial| trial.param_index).collect::<Vec<_>>(),
///     vec![0, 1],
/// );
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct GridSearchResult<P> {
    /// Every trial in original parameter order, including failures.
    pub trials: Vec<GridTrial<P>>,
    /// `param_index` of the winning trial, if any trial was ranked.
    pub best_index: Option<usize>,
}

impl<P> GridSearchResult<P> {
    /// Returns the winning trial, or `None` when no trial was ranked.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::GridSearchResult;
    ///
    /// let result: GridSearchResult<usize> = GridSearchResult {
    ///     trials: Vec::new(),
    ///     best_index: None,
    /// };
    /// assert!(result.best().is_none());
    /// ```
    #[must_use]
    pub fn best(&self) -> Option<&GridTrial<P>> {
        let best_index = self.best_index?;
        self.trials
            .iter()
            .find(|trial| trial.param_index == best_index && trial.is_ranked())
    }

    /// Returns up to `k` ranked trials in descending score order.
    ///
    /// Failed trials, filtered trials, and trials without a finite score are
    /// excluded. Equal scores are ordered by ascending `param_index`, so the
    /// result is a strict total order and therefore reproducible.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::{GridSearchResult, GridTrial, TrialStatus};
    ///
    /// let trial = |param_index: usize, score: Option<f64>| GridTrial {
    ///     param_index,
    ///     params: (),
    ///     status: TrialStatus::Succeeded,
    ///     score,
    ///     passed_filter: true,
    ///     summary: None,
    ///     report: None,
    /// };
    /// let result = GridSearchResult {
    ///     trials: vec![trial(0, Some(1.0)), trial(1, Some(3.0)), trial(2, Some(3.0))],
    ///     best_index: Some(1),
    /// };
    /// assert_eq!(
    ///     result.top_k(2).iter().map(|trial| trial.param_index).collect::<Vec<_>>(),
    ///     vec![1, 2],
    /// );
    /// ```
    #[must_use]
    pub fn top_k(&self, k: usize) -> Vec<&GridTrial<P>> {
        let mut ranked = self
            .trials
            .iter()
            .filter(|trial| trial.is_ranked())
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.param_index.cmp(&right.param_index))
        });
        ranked.truncate(k);
        ranked
    }
}

fn select_best<P>(trials: &[GridTrial<P>]) -> Option<usize> {
    trials
        .iter()
        .filter(|trial| trial.is_ranked())
        .min_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.param_index.cmp(&right.param_index))
        })
        .map(|trial| trial.param_index)
}

fn evaluate_trial<P, F>(
    param_index: usize,
    params: P,
    data: MarketSlice<'_>,
    config: &GridSearchConfig,
    factory: &F,
) -> GridTrial<P>
where
    P: Clone,
    F: Fn(&P) -> Result<StrategySpec, KurumiError> + Sync,
{
    let spec = match factory(&params) {
        Ok(spec) => spec,
        Err(error) => return GridTrial::failed(param_index, params, &error),
    };
    match run_strategy(spec, data, config.warmup_bars) {
        Ok(report) => {
            let score = config.objective.score(&report);
            let passed_filter = score.is_some() && config.filter.passes(&report);
            let summary = Some(TrialSummary::from_report(&report));
            let retained = config.retain_all_reports.then_some(report);
            GridTrial {
                param_index,
                params,
                status: TrialStatus::Succeeded,
                score,
                passed_filter,
                summary,
                report: retained,
            }
        }
        Err(error) => GridTrial::failed(param_index, params, &error),
    }
}

fn run_strategy(
    spec: StrategySpec,
    data: MarketSlice<'_>,
    warmup_bars: usize,
) -> Result<PortfolioBacktestReport, KurumiError> {
    let StrategySpec {
        config,
        trigger,
        mut allocator,
        fee_model,
        slippage_model,
    } = spec;
    let fee_model = fee_model.unwrap_or_else(|| Box::new(NoFee));
    let slippage_model = slippage_model.unwrap_or_else(|| Box::new(NoSlippage));
    let mut engine = PortfolioBacktestEngine::new(config, fee_model, slippage_model)?;
    engine.run_on_slice(data, warmup_bars, trigger.as_ref(), allocator.as_mut())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::{
        AlignedMarketData, CalendarTrigger, MultiAssetFeed, StaticWeightAllocator,
    };
    use kurumi_ta::OhlcvBar;
    use rust_decimal::Decimal;
    use std::collections::BTreeMap;

    const DAY: i64 = 86_400_000;
    const WARMUP_BARS: usize = 5;

    fn bar(day: i64, close: f64) -> OhlcvBar {
        OhlcvBar::new(
            day * DAY,
            close,
            close + 1.0,
            close - 1.0,
            close,
            1_000.0,
            close * 1_000.0,
        )
        .expect("valid bar")
    }

    fn data(bars: usize) -> AlignedMarketData {
        let mut series = BTreeMap::new();
        series.insert(
            "AAA".to_owned(),
            (0..bars)
                .map(|day| bar(day as i64, 100.0 + (day % 5) as f64))
                .collect::<Vec<_>>(),
        );
        series.insert(
            "BBB".to_owned(),
            (0..bars)
                .map(|day| bar(day as i64, if day % 2 == 0 { 50.0 } else { 55.0 }))
                .collect::<Vec<_>>(),
        );
        AlignedMarketData::from_feed(MultiAssetFeed::new(series).expect("valid feed"))
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Parameters {
        lookback_window: usize,
        allow_fractional: bool,
    }

    fn parameters() -> Vec<Parameters> {
        let mut params = Vec::new();
        for lookback_window in 2..=13 {
            for allow_fractional in [false, true] {
                params.push(Parameters {
                    lookback_window,
                    allow_fractional,
                });
            }
        }
        params
    }

    fn config_for(params: &Parameters) -> PortfolioBacktestConfig {
        let mut config =
            PortfolioBacktestConfig::new(Decimal::from(100_000)).expect("valid config");
        config.lookback_window = params.lookback_window;
        config.allow_fractional = params.allow_fractional;
        config
    }

    fn allocator_for() -> StaticWeightAllocator {
        StaticWeightAllocator::new(BTreeMap::from([
            ("AAA".to_owned(), 0.6),
            ("BBB".to_owned(), 0.4),
        ]))
        .expect("valid allocator")
    }

    fn trigger() -> CalendarTrigger {
        CalendarTrigger::fixed_interval(7).expect("valid interval")
    }

    fn objective() -> ObjectiveMetric {
        ObjectiveMetric::MinMaxDrawdown
    }

    fn filter() -> ResultFilter {
        ResultFilter::new()
            .with_min_rebalance_count(1)
            .with_max_drawdown_limit(0.99)
    }

    fn runner_config(retain_all_reports: bool) -> GridSearchConfig {
        GridSearchConfig::new(objective())
            .with_filter(filter())
            .with_warmup_bars(WARMUP_BARS)
            .with_retain_all_reports(retain_all_reports)
    }

    fn manual_run(data: MarketSlice<'_>, params: &Parameters) -> PortfolioBacktestReport {
        let mut engine =
            PortfolioBacktestEngine::new(config_for(params), Box::new(NoFee), Box::new(NoSlippage))
                .expect("valid engine");
        let mut allocator = allocator_for();
        engine
            .run_on_slice(data, WARMUP_BARS, &trigger(), &mut allocator)
            .expect("backtest")
    }

    fn factory(params: &Parameters) -> Result<StrategySpec, KurumiError> {
        Ok(StrategySpec::new(
            config_for(params),
            Box::new(trigger()),
            Box::new(allocator_for()),
        ))
    }

    #[test]
    fn parallel_and_sequential_runs_are_identical_over_a_24_point_grid() {
        let params = parameters();
        assert_eq!(params.len(), 24);
        let data = data(40);
        let slice = data.slice();

        let runner = GridSearchRunner::new(runner_config(true));
        let result = runner
            .run(slice, &params, factory)
            .expect("grid search succeeds");

        let mut expected_scores = Vec::with_capacity(params.len());
        let mut expected_best: Option<(f64, usize)> = None;
        for (index, params) in params.iter().enumerate() {
            let report = manual_run(slice, params);
            let score = objective().score(&report);
            let passed_filter = score.is_some() && filter().passes(&report);
            let trial = result.trials.get(index).expect("trial in order");

            assert_eq!(trial.param_index, index);
            assert_eq!(trial.params, *params);
            assert_eq!(trial.status, TrialStatus::Succeeded);
            assert_eq!(trial.score, score, "trial {index} score differs");
            assert_eq!(
                trial.passed_filter, passed_filter,
                "trial {index} filter verdict differs"
            );
            assert_eq!(
                trial.summary.map(|summary| summary.rebalance_count),
                Some(report.rebalance_count)
            );
            assert_eq!(
                trial.report.as_ref(),
                Some(&report),
                "trial {index} report differs"
            );
            expected_scores.push((score, passed_filter, index));

            if passed_filter {
                if let Some(score) = score {
                    let better = expected_best.is_none_or(|(best, _)| score > best);
                    if better {
                        expected_best = Some((score, index));
                    }
                }
            }
        }
        let expected_best_index = expected_best.map(|(_, index)| index);
        assert_eq!(result.best_index, expected_best_index);
        assert_eq!(
            result.best().map(|trial| trial.param_index),
            expected_best_index
        );

        let mut expected_order = expected_scores
            .iter()
            .filter_map(|(score, passed, index)| {
                if !passed {
                    return None;
                }
                score
                    .filter(|value| value.is_finite())
                    .map(|value| (value, *index))
            })
            .collect::<Vec<_>>();
        expected_order.sort_by(|left, right| {
            right
                .0
                .partial_cmp(&left.0)
                .expect("scores are finite")
                .then_with(|| left.1.cmp(&right.1))
        });
        let actual_order = result
            .top_k(params.len())
            .iter()
            .map(|trial| (trial.score.expect("ranked score"), trial.param_index))
            .collect::<Vec<_>>();
        assert_eq!(actual_order, expected_order);
    }

    #[test]
    fn failed_trials_are_isolated_and_the_best_valid_trial_wins() {
        let params = vec![
            Parameters {
                lookback_window: 0,
                allow_fractional: false,
            },
            Parameters {
                lookback_window: 3,
                allow_fractional: false,
            },
            Parameters {
                lookback_window: 6,
                allow_fractional: true,
            },
        ];
        let data = data(40);
        let runner = GridSearchRunner::new(runner_config(true));
        let result = runner
            .run(data.slice(), &params, factory)
            .expect("grid search succeeds");

        let failed = result.trials.first().expect("invalid trial");
        assert!(failed.status.is_failed());
        assert!(matches!(failed.status, TrialStatus::Failed(_)));
        assert!(failed.score.is_none());
        assert!(!failed.passed_filter);
        assert!(failed.report.is_none());

        assert_eq!(
            result
                .trials
                .iter()
                .filter(|trial| trial.status == TrialStatus::Succeeded)
                .count(),
            2
        );
        let best_index = result.best_index.expect("a valid winner");
        assert_ne!(best_index, 0);
        let best = result.best().expect("best trial");
        assert_eq!(best.param_index, best_index);
        assert!(best.report.is_some());
    }

    #[test]
    fn compact_mode_keeps_full_reports_only_for_the_winner() {
        let params = parameters();
        let data = data(40);
        let runner = GridSearchRunner::new(runner_config(false));
        let result = runner
            .run(data.slice(), &params, factory)
            .expect("grid search succeeds");

        let best_index = result.best_index.expect("a valid winner");
        for trial in &result.trials {
            if trial.param_index == best_index {
                assert!(trial.report.is_some(), "winner keeps its full report");
            } else {
                assert!(
                    trial.report.is_none(),
                    "non-winning trial {} unexpectedly kept a report",
                    trial.param_index
                );
            }
            if trial.status == TrialStatus::Succeeded {
                assert!(trial.summary.is_some());
            }
        }
        assert!(
            result
                .best()
                .and_then(|trial| trial.report.as_ref())
                .is_some()
        );
    }

    #[test]
    fn empty_grid_and_invalid_warmup_are_reported() {
        let data = data(10);
        let runner = GridSearchRunner::new(runner_config(true));
        let empty = runner
            .run(data.slice(), &[] as &[Parameters], factory)
            .expect("empty grid is valid");
        assert!(empty.trials.is_empty());
        assert!(empty.best_index.is_none());
        assert!(empty.best().is_none());

        let mut invalid = runner_config(true);
        invalid.warmup_bars = data.len();
        let invalid_runner = GridSearchRunner::new(invalid);
        assert!(
            invalid_runner
                .run(data.slice(), &parameters(), factory)
                .is_err()
        );

        let empty_data =
            AlignedMarketData::from_feed(MultiAssetFeed::new(BTreeMap::new()).expect("empty feed"));
        assert!(
            runner
                .run(empty_data.slice(), &parameters(), factory)
                .is_err()
        );
    }

    #[test]
    fn top_k_breaks_ties_by_grid_index_and_skips_unranked_trials() {
        let trial = |param_index: usize, score: Option<f64>, passed_filter: bool| GridTrial {
            param_index,
            params: param_index,
            status: TrialStatus::Succeeded,
            score,
            passed_filter,
            summary: None,
            report: None,
        };
        let result = GridSearchResult {
            trials: vec![
                trial(0, Some(2.0), false),
                trial(1, Some(2.0), true),
                trial(2, Some(2.0), true),
                trial(3, Some(f64::NAN), true),
                trial(4, Some(1.0), true),
            ],
            best_index: Some(1),
        };
        assert_eq!(
            result
                .top_k(3)
                .iter()
                .map(|trial| trial.param_index)
                .collect::<Vec<_>>(),
            vec![1, 2, 4]
        );
        assert_eq!(result.best().map(|trial| trial.param_index), Some(1));

        let failed = GridTrial::<usize> {
            param_index: 5,
            params: 5,
            status: TrialStatus::Failed("boom".to_owned()),
            score: Some(99.0),
            passed_filter: true,
            summary: None,
            report: None,
        };
        assert!(!failed.is_ranked());
    }
}
