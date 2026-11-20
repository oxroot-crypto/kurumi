//! Walk-forward analysis (WFA) over an immutable, aligned market timeline.
//!
//! Walk-forward analysis estimates how a parameterized strategy would have
//! performed out of sample. The timeline is divided into sequential folds: on
//! every fold the best parameter set is selected on an in-sample (IS) window and
//! that exact parameter set is then evaluated on the following, untouched
//! out-of-sample (OOS) window. The OOS equity curves are compounded into one
//! continuous stitched curve, which is the only input to the reported OOS
//! statistics.
//!
//! # Window layout
//!
//! ```text
//! fold k, Rolling mode
//!
//!  is_warmup_start    is_start            is_end      oos_start        oos_end
//!        |                |                  |  embargo    |              |
//!        v                v                  v <--------> v              v
//! -------+----------------+------------------+-----------+--------------+------>
//!        |<--- warmup --->|<-- in_sample --->|           |<- out_of_sample ->|
//!                         |<---- folds advance by out_of_sample_bars ----------->|
//! ```
//!
//! - [`WalkForwardConfig::in_sample_bars`] is the fixed IS length.
//! - [`WalkForwardConfig::out_of_sample_bars`] is both the OOS length and the
//!   forward step between folds.
//! - [`WalkForwardConfig::embargo_bars`] are skipped between the end of the IS
//!   window and the start of the OOS window, so a leaky multi-period label
//!   cannot overlap the test segment.
//! - [`WalkForwardConfig::warmup_bars`] are borrowed from the immediately
//!   preceding history for every IS and OOS backtest. Warmup bars populate
//!   indicator and allocator history without placing trades or recording equity,
//!   so the evaluated segment never starts cold. Near the start of the timeline
//!   the borrow is clamped to the bars that exist.
//!
//! [`WalkForwardMode::Rolling`] keeps a fixed IS length and rolls its start
//! forward; [`WalkForwardMode::Anchored`] keeps `is_start = 0` and grows the IS
//! window instead.
//!
//! # No look-ahead
//!
//! An IS slice always ends at `is_end <= oos_start`, so parameter selection
//! never observes any bar at or after the OOS start. The OOS windows of
//! consecutive folds are adjacent: their timestamps do not overlap and no OOS
//! observation between them is skipped, so the stitched curve is a complete
//! record of the out-of-sample test.
//!
//! # Compounded OOS stitching
//!
//! Each fold is backtested from a fresh account whose opening equity is
//! `C_k = report_k.initial_equity`. With `S_0 = C_0`, the stitched value of the
//! observation at time `t` in fold `k` is
//!
//! ```text
//! S_k(t) = S_{k-1} * E_k(t) / C_k
//! ```
//!
//! computed entirely in [`Decimal`] arithmetic so the compounded level carries
//! no floating-point drift. `S_{k-1}` is the last stitched value of the previous
//! fold. The stitched curve is converted to periodic returns and passed to
//! `kurumi-risk`, which produces the OOS annualized return, annualized
//! volatility, Sharpe, Sortino, Calmar and maximum drawdown statistics.
//!
//! # Overfitting diagnostics
//!
//! [`WalkForwardDiagnostics`] contrasts the IS winners with the OOS outcome:
//!
//! - **Walk-forward efficiency** = OOS annualized return divided by the mean IS
//!   best annualized return, or `None` when that denominator is not positive.
//! - **Sharpe degradation** = stitched OOS Sharpe minus the mean IS best Sharpe,
//!   or `None` when either side is undefined. A more negative value means the
//!   risk-adjusted result deteriorated out of sample.
//! - **Profitable fold ratio** = fraction of folds whose OOS end equity exceeds
//!   its opening equity.
//! - **Parameter stability** = fraction of adjacent fold pairs that selected
//!   exactly the same best parameter set.
//!
//! # Folds without an admissible winner
//!
//! [`GridSearchRunner`] ranks a trial only when
//! it succeeded, passed the [`ResultFilter`](super::objective::ResultFilter), and
//! produced a finite objective score. When a whole fold has no ranked trial,
//! [`WalkForwardAnalyzer::run`] returns [`KurumiError::InvalidParameter`] unless
//! [`WalkForwardConfig::fallback_param_index`] names a parameter set to use
//! instead. A fallback fold is excluded from the IS-side averages of
//! [`WalkForwardDiagnostics`], because it is not an in-sample optimum.
//!
//! # Determinism
//!
//! Windows are generated deterministically from the timeline length, folds are
//! evaluated in ascending order, parameter ranking keeps
//! [`GridSearchRunner`]'s deterministic
//! tie-breaking, and stitching is exact decimal arithmetic. A parallel and a
//! sequential build therefore produce the same report for the same inputs.
//!
//! # Examples
//!
//! ```
//! use kurumi_backtest::validation::{
//!     GridSearchConfig, ObjectiveMetric, WalkForwardConfig, WalkForwardMode, generate_folds,
//! };
//!
//! # fn main() -> Result<(), kurumi_backtest::KurumiError> {
//! let config = WalkForwardConfig::new(
//!     250,
//!     100,
//!     GridSearchConfig::new(ObjectiveMetric::SharpeRatio),
//! )
//! .with_mode(WalkForwardMode::Rolling)
//! .with_embargo_bars(5)
//! .with_warmup_bars(60);
//!
//! let folds = generate_folds(1_000, &config)?;
//! assert_eq!(folds.len(), 7);
//! assert_eq!(folds.first().map(|fold| fold.oos_start), Some(255));
//! assert_eq!(folds.last().map(|fold| fold.oos_end), Some(955));
//! # Ok(())
//! # }
//! ```

use chrono::{DateTime, Utc};
use kurumi_core::KurumiError;
use kurumi_risk::{MetricsConfig, PerformanceMetrics};
use rust_decimal::Decimal;

use super::grid::{
    GridSearchConfig, GridSearchResult, GridSearchRunner, StrategySpec, TrialStatus, TrialSummary,
};
use crate::portfolio::report::curve_returns;
use crate::portfolio::{AlignedMarketData, PortfolioBacktestReport, RebalanceCause};

/// The minimum number of evaluated observations a backtest needs to produce
/// performance metrics.
///
/// [`PerformanceMetrics::from_returns`] requires at least two returns, which the
/// engine draws from at least three equity observations.
const MIN_EVALUATED_BARS: usize = 3;

/// The annualization convention used by [`WalkForwardConfig::new`].
const DEFAULT_PERIODS_PER_YEAR: f64 = 252.0;

/// How the in-sample window moves from one fold to the next.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::validation::WalkForwardMode;
///
/// assert_eq!(WalkForwardMode::default(), WalkForwardMode::Rolling);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WalkForwardMode {
    /// Keep a fixed `in_sample_bars` length and roll its start forward by
    /// `out_of_sample_bars` on every fold.
    #[default]
    Rolling,
    /// Keep the in-sample start at index zero and extend the in-sample end by
    /// `out_of_sample_bars` on every fold, so the IS window grows.
    Anchored,
}

/// One walk-forward fold: the in-sample training range and the out-of-sample
/// test range, including their warmup borrows.
///
/// All indices are zero-based positions on the [`AlignedMarketData`] timeline
/// and the second index of every range is exclusive. Windows are produced by
/// [`generate_folds`], which guarantees
/// `is_warmup_start <= is_start < is_end <= oos_start < oos_end`.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::validation::{
///     GridSearchConfig, ObjectiveMetric, WalkForwardConfig, generate_folds,
/// };
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let config = WalkForwardConfig::new(
///     250,
///     100,
///     GridSearchConfig::new(ObjectiveMetric::TotalReturn),
/// )
/// .with_embargo_bars(5)
/// .with_warmup_bars(60);
/// let folds = generate_folds(1_000, &config)?;
///
/// let first = folds.first().copied();
/// assert_eq!(first.map(|fold| fold.is_start), Some(0));
/// assert_eq!(first.map(|fold| fold.is_end), Some(250));
/// assert_eq!(first.map(|fold| fold.oos_start), Some(255));
/// assert_eq!(first.map(|fold| fold.oos_warmup_start), Some(195));
/// assert_eq!(first.map(|fold| fold.fold_id), Some(0));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoldWindow {
    /// Zero-based fold position on the timeline.
    pub fold_id: usize,
    /// First bar borrowed as warmup for the in-sample backtest.
    pub is_warmup_start: usize,
    /// First evaluated in-sample bar; parameter selection starts here.
    pub is_start: usize,
    /// One past the last in-sample bar; always at most [`FoldWindow::oos_start`].
    pub is_end: usize,
    /// First bar borrowed as warmup for the out-of-sample backtest.
    pub oos_warmup_start: usize,
    /// First evaluated out-of-sample bar.
    pub oos_start: usize,
    /// One past the last evaluated out-of-sample bar.
    pub oos_end: usize,
}

impl FoldWindow {
    /// Returns the number of evaluated in-sample bars.
    ///
    /// Rolling folds always return [`WalkForwardConfig::in_sample_bars`];
    /// anchored folds grow from that length as `is_end` advances.
    #[must_use]
    #[inline]
    pub const fn is_bars(&self) -> usize {
        self.is_end.saturating_sub(self.is_start)
    }

    /// Returns the number of evaluated out-of-sample bars.
    ///
    /// This is [`WalkForwardConfig::out_of_sample_bars`] for every fold.
    #[must_use]
    #[inline]
    pub const fn oos_bars(&self) -> usize {
        self.oos_end.saturating_sub(self.oos_start)
    }

    /// Returns the number of warmup bars borrowed for the in-sample backtest.
    ///
    /// At the start of the timeline there may be fewer bars than configured, so
    /// this is `min(warmup_bars, is_start)`.
    #[must_use]
    #[inline]
    pub const fn is_warmup(&self) -> usize {
        self.is_start.saturating_sub(self.is_warmup_start)
    }

    /// Returns the number of warmup bars borrowed for the out-of-sample
    /// backtest.
    ///
    /// This is `min(warmup_bars, oos_start)`.
    #[must_use]
    #[inline]
    pub const fn oos_warmup(&self) -> usize {
        self.oos_start.saturating_sub(self.oos_warmup_start)
    }
}

/// Configuration of a walk-forward analysis.
///
/// Use [`WalkForwardConfig::new`] and the builder methods to describe the window
/// plan, the annualization convention, and the grid search applied to every
/// in-sample window. [`generate_folds`] and [`WalkForwardAnalyzer::run`] validate
/// the configuration before using it.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::validation::{
///     GridSearchConfig, ObjectiveMetric, WalkForwardConfig, WalkForwardMode,
/// };
///
/// let config = WalkForwardConfig::new(250, 100, GridSearchConfig::new(ObjectiveMetric::SharpeRatio))
///     .with_mode(WalkForwardMode::Anchored)
///     .with_embargo_bars(5)
///     .with_warmup_bars(60)
///     .with_periods_per_year(252.0)
///     .with_risk_free_rate(0.02);
///
/// assert_eq!(config.mode, WalkForwardMode::Anchored);
/// assert_eq!(config.embargo_bars, 5);
/// ```
#[derive(Debug, Clone)]
pub struct WalkForwardConfig {
    /// How the in-sample window advances across folds.
    pub mode: WalkForwardMode,
    /// Number of evaluated in-sample bars per fold; must be at least three.
    pub in_sample_bars: usize,
    /// Number of evaluated out-of-sample bars per fold and the forward step
    /// between folds; must be at least three.
    pub out_of_sample_bars: usize,
    /// Bars skipped between the end of the in-sample window and the start of the
    /// out-of-sample window. Defaults to zero.
    pub embargo_bars: usize,
    /// Bars borrowed before each IS and OOS backtest to warm up indicator and
    /// allocator history without trading. Defaults to zero.
    ///
    /// The borrow is clamped to the bars available before the evaluated segment,
    /// so the first fold may borrow fewer than this many bars.
    pub warmup_bars: usize,
    /// Grid search applied to every in-sample window.
    ///
    /// The grid's own [`GridSearchConfig::warmup_bars`] is replaced per fold by
    /// the window's computed warmup; configure warmup through
    /// [`WalkForwardConfig::warmup_bars`] instead.
    pub grid_config: GridSearchConfig,
    /// Number of return periods in a year used by the OOS risk metrics.
    pub periods_per_year: f64,
    /// Annual risk-free rate used by the OOS Sharpe and Sortino statistics.
    pub risk_free_rate: f64,
    /// Parameter index used when a fold has no admissible in-sample winner.
    ///
    /// When `None` (the default), such a fold is an
    /// [`KurumiError::InvalidParameter`] instead of silently running a parameter
    /// set the grid could not justify.
    pub fallback_param_index: Option<usize>,
}

impl WalkForwardConfig {
    /// Creates a rolling configuration with zero embargo, zero warmup, daily
    /// annualization, a zero risk-free rate, and no fallback.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::{GridSearchConfig, ObjectiveMetric, WalkForwardConfig};
    ///
    /// let config = WalkForwardConfig::new(250, 100, GridSearchConfig::new(ObjectiveMetric::SharpeRatio));
    /// assert_eq!(config.in_sample_bars, 250);
    /// assert_eq!(config.out_of_sample_bars, 100);
    /// assert_eq!(config.embargo_bars, 0);
    /// assert_eq!(config.warmup_bars, 0);
    /// assert_eq!(config.fallback_param_index, None);
    /// ```
    #[must_use]
    pub fn new(
        in_sample_bars: usize,
        out_of_sample_bars: usize,
        grid_config: GridSearchConfig,
    ) -> Self {
        Self {
            mode: WalkForwardMode::Rolling,
            in_sample_bars,
            out_of_sample_bars,
            embargo_bars: 0,
            warmup_bars: 0,
            grid_config,
            periods_per_year: DEFAULT_PERIODS_PER_YEAR,
            risk_free_rate: 0.0,
            fallback_param_index: None,
        }
    }

    /// Sets the window mode.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::{
    ///     GridSearchConfig, ObjectiveMetric, WalkForwardConfig, WalkForwardMode,
    /// };
    ///
    /// let config = WalkForwardConfig::new(250, 100, GridSearchConfig::new(ObjectiveMetric::SharpeRatio))
    ///     .with_mode(WalkForwardMode::Anchored);
    /// assert_eq!(config.mode, WalkForwardMode::Anchored);
    /// ```
    #[must_use]
    pub fn with_mode(mut self, mode: WalkForwardMode) -> Self {
        self.mode = mode;
        self
    }

    /// Sets the number of embargo bars between the IS and OOS windows.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::{GridSearchConfig, ObjectiveMetric, WalkForwardConfig};
    ///
    /// let config = WalkForwardConfig::new(250, 100, GridSearchConfig::new(ObjectiveMetric::SharpeRatio))
    ///     .with_embargo_bars(5);
    /// assert_eq!(config.embargo_bars, 5);
    /// ```
    #[must_use]
    pub fn with_embargo_bars(mut self, embargo_bars: usize) -> Self {
        self.embargo_bars = embargo_bars;
        self
    }

    /// Sets the number of warmup bars borrowed before each IS and OOS backtest.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::{GridSearchConfig, ObjectiveMetric, WalkForwardConfig};
    ///
    /// let config = WalkForwardConfig::new(250, 100, GridSearchConfig::new(ObjectiveMetric::SharpeRatio))
    ///     .with_warmup_bars(60);
    /// assert_eq!(config.warmup_bars, 60);
    /// ```
    #[must_use]
    pub fn with_warmup_bars(mut self, warmup_bars: usize) -> Self {
        self.warmup_bars = warmup_bars;
        self
    }

    /// Sets the number of return periods in a year used for the OOS metrics.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::{GridSearchConfig, ObjectiveMetric, WalkForwardConfig};
    ///
    /// let config = WalkForwardConfig::new(250, 100, GridSearchConfig::new(ObjectiveMetric::SharpeRatio))
    ///     .with_periods_per_year(12.0);
    /// assert!((config.periods_per_year - 12.0).abs() < 1e-15);
    /// ```
    #[must_use]
    pub fn with_periods_per_year(mut self, periods_per_year: f64) -> Self {
        self.periods_per_year = periods_per_year;
        self
    }

    /// Sets the annual risk-free rate used for the OOS Sharpe and Sortino
    /// statistics.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::{GridSearchConfig, ObjectiveMetric, WalkForwardConfig};
    ///
    /// let config = WalkForwardConfig::new(250, 100, GridSearchConfig::new(ObjectiveMetric::SharpeRatio))
    ///     .with_risk_free_rate(0.02);
    /// assert!((config.risk_free_rate - 0.02).abs() < 1e-15);
    /// ```
    #[must_use]
    pub fn with_risk_free_rate(mut self, risk_free_rate: f64) -> Self {
        self.risk_free_rate = risk_free_rate;
        self
    }

    /// Sets the parameter index used when a fold has no admissible winner.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::{GridSearchConfig, ObjectiveMetric, WalkForwardConfig};
    ///
    /// let config = WalkForwardConfig::new(250, 100, GridSearchConfig::new(ObjectiveMetric::SharpeRatio))
    ///     .with_fallback_param_index(Some(0));
    /// assert_eq!(config.fallback_param_index, Some(0));
    /// ```
    #[must_use]
    pub fn with_fallback_param_index(mut self, fallback_param_index: Option<usize>) -> Self {
        self.fallback_param_index = fallback_param_index;
        self
    }

    /// Validates the window plan and annualization inputs.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when either window is shorter
    /// than three evaluated bars (the minimum needed for
    /// [`PerformanceMetrics`]), or when an annualization input is not finite
    /// with a positive period count.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::{GridSearchConfig, ObjectiveMetric, WalkForwardConfig};
    ///
    /// let valid = WalkForwardConfig::new(250, 100, GridSearchConfig::new(ObjectiveMetric::SharpeRatio));
    /// assert!(valid.validate().is_ok());
    ///
    /// let invalid = WalkForwardConfig::new(2, 100, GridSearchConfig::new(ObjectiveMetric::SharpeRatio));
    /// assert!(invalid.validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<(), KurumiError> {
        if self.in_sample_bars < MIN_EVALUATED_BARS {
            return Err(KurumiError::InvalidParameter {
                parameter: "in_sample_bars",
                reason: format!(
                    "must be at least {MIN_EVALUATED_BARS} evaluated bars to produce performance metrics"
                ),
            });
        }
        if self.out_of_sample_bars < MIN_EVALUATED_BARS {
            return Err(KurumiError::InvalidParameter {
                parameter: "out_of_sample_bars",
                reason: format!(
                    "must be at least {MIN_EVALUATED_BARS} evaluated bars to produce performance metrics"
                ),
            });
        }
        if !self.periods_per_year.is_finite() || self.periods_per_year <= 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "periods_per_year",
                reason: "must be finite and strictly positive".to_owned(),
            });
        }
        if !self.risk_free_rate.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "risk_free_rate",
                reason: "must be finite".to_owned(),
            });
        }
        Ok(())
    }
}

/// Plans the walk-forward folds for a timeline of `total_bars` observations.
///
/// Folds are emitted in ascending fold order. A fold is complete only when its
/// whole out-of-sample window fits before `total_bars`; the first incomplete
/// fold stops the plan. In [`WalkForwardMode::Rolling`] the in-sample start of
/// fold `k` is `k * out_of_sample_bars`; in [`WalkForwardMode::Anchored`] every
/// in-sample start is zero and the in-sample end is
/// `in_sample_bars + k * out_of_sample_bars`.
///
/// Production folds are adjacent: `folds[k].is_start == k * out_of_sample_bars`
/// in rolling mode, and `folds[k].oos_end == folds[k + 1].oos_start` in both
/// modes, so the stitched OOS curve has no gap and no duplicate observation.
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] when the timeline cannot hold one
/// complete fold, [`KurumiError::InvalidParameter`] when the configuration is
/// invalid or a generated window violates the boundary invariants, and
/// [`KurumiError::NumericalOverflow`] when the window arithmetic overflows
/// `usize`.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::validation::{
///     GridSearchConfig, ObjectiveMetric, WalkForwardConfig, WalkForwardMode, generate_folds,
/// };
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let config = WalkForwardConfig::new(250, 100, GridSearchConfig::new(ObjectiveMetric::TotalReturn));
/// let rolling = generate_folds(1_000, &config)?;
/// assert_eq!(rolling.len(), 7);
/// assert_eq!(rolling.get(1).map(|fold| fold.is_start), Some(100));
///
/// let anchored = WalkForwardConfig::new(250, 100, GridSearchConfig::new(ObjectiveMetric::TotalReturn))
///     .with_mode(WalkForwardMode::Anchored);
/// let anchored_folds = generate_folds(1_000, &anchored)?;
/// assert_eq!(anchored_folds.get(1).map(|fold| fold.is_start), Some(0));
/// assert_eq!(anchored_folds.get(1).map(|fold| fold.is_end), Some(350));
///
/// assert!(generate_folds(10, &config).is_err());
/// # Ok(())
/// # }
/// ```
pub fn generate_folds(
    total_bars: usize,
    config: &WalkForwardConfig,
) -> Result<Vec<FoldWindow>, KurumiError> {
    config.validate()?;

    let mut folds = Vec::new();
    let mut fold_id = 0usize;
    loop {
        let offset = fold_id.checked_mul(config.out_of_sample_bars).ok_or(
            KurumiError::NumericalOverflow {
                operation: "generate_folds",
            },
        )?;
        let (is_start, is_end) = match config.mode {
            WalkForwardMode::Rolling => (
                offset,
                offset.checked_add(config.in_sample_bars).ok_or(
                    KurumiError::NumericalOverflow {
                        operation: "generate_folds",
                    },
                )?,
            ),
            WalkForwardMode::Anchored => (
                0,
                config.in_sample_bars.checked_add(offset).ok_or(
                    KurumiError::NumericalOverflow {
                        operation: "generate_folds",
                    },
                )?,
            ),
        };
        let oos_start =
            is_end
                .checked_add(config.embargo_bars)
                .ok_or(KurumiError::NumericalOverflow {
                    operation: "generate_folds",
                })?;
        let oos_end = oos_start.checked_add(config.out_of_sample_bars).ok_or(
            KurumiError::NumericalOverflow {
                operation: "generate_folds",
            },
        )?;
        if oos_end > total_bars {
            break;
        }

        let window = FoldWindow {
            fold_id,
            is_warmup_start: is_start.saturating_sub(config.warmup_bars),
            is_start,
            is_end,
            oos_warmup_start: oos_start.saturating_sub(config.warmup_bars),
            oos_start,
            oos_end,
        };
        validate_window(&window, total_bars, config)?;
        folds.push(window);
        fold_id = fold_id
            .checked_add(1)
            .ok_or(KurumiError::NumericalOverflow {
                operation: "generate_folds",
            })?;
    }

    if folds.is_empty() {
        return Err(KurumiError::InsufficientData {
            required: config
                .in_sample_bars
                .saturating_add(config.embargo_bars)
                .saturating_add(config.out_of_sample_bars),
            available: total_bars,
        });
    }
    Ok(folds)
}

fn validate_window(
    window: &FoldWindow,
    total_bars: usize,
    config: &WalkForwardConfig,
) -> Result<(), KurumiError> {
    let in_sample_length = window.is_end.checked_sub(window.is_start);
    let out_of_sample_length = window.oos_end.checked_sub(window.oos_start);
    let in_sample_ok = match config.mode {
        WalkForwardMode::Rolling => in_sample_length == Some(config.in_sample_bars),
        WalkForwardMode::Anchored => {
            window.is_start == 0
                && in_sample_length.is_some_and(|length| length >= config.in_sample_bars)
        }
    };
    let valid = window.is_warmup_start <= window.is_start
        && window.is_start < window.is_end
        && window.is_end <= window.oos_start
        && window.oos_start < window.oos_end
        && window.oos_end <= total_bars
        && in_sample_ok
        && out_of_sample_length == Some(config.out_of_sample_bars);
    if !valid {
        return Err(KurumiError::InvalidParameter {
            parameter: "fold_windows",
            reason: format!(
                "generated window for fold {} violates the walk-forward boundary invariants",
                window.fold_id
            ),
        });
    }
    Ok(())
}

/// One evaluated walk-forward fold.
///
/// The fold keeps the complete in-sample grid result so callers can audit every
/// competing parameter set, the parameter set actually used out of sample, and
/// the full out-of-sample backtest report including its audit trail.
///
/// `is_best_summary` is the compact summary of the admissible in-sample winner.
/// It is `None` when [`WalkForwardConfig::fallback_param_index`] supplied the
/// parameters, because a fallback is not an in-sample optimum; such a fold is
/// therefore excluded from the IS-side diagnostic averages.
///
/// See [`WalkForwardAnalyzer::run`] for a complete example.
#[derive(Debug, Clone, PartialEq)]
pub struct WalkForwardFold<P> {
    /// Window that produced this fold.
    pub window: FoldWindow,
    /// Complete in-sample parameter grid result.
    pub is_result: GridSearchResult<P>,
    /// Parameter combination evaluated out of sample.
    pub best_params: P,
    /// Whether `best_params` came from the configured fallback instead of an
    /// admissible in-sample winner.
    pub used_fallback: bool,
    /// Compact summary of the in-sample winner; `None` for a fallback fold.
    pub is_best_summary: Option<TrialSummary>,
    /// Full out-of-sample backtest report.
    pub oos_report: PortfolioBacktestReport,
    /// This fold's out-of-sample equity on the stitched level.
    pub stitched_equity_curve: Vec<(DateTime<Utc>, Decimal)>,
}

/// Overfitting diagnostics contrasting in-sample winners with the out-of-sample
/// outcome.
///
/// Ratios that are undefined for the sample are `None` rather than `NaN`, which
/// matches [`PerformanceMetrics`] and [`TrialSummary`]. Undefined means either
/// side of the difference or quotient is undefined: for example a fold whose
/// Sharpe ratio is undefined because its volatility is zero.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::validation::WalkForwardDiagnostics;
///
/// let diagnostics = WalkForwardDiagnostics {
///     walk_forward_efficiency: Some(0.6),
///     sharpe_degradation: Some(-0.4),
///     oos_profitable_fold_ratio: 0.75,
///     parameter_stability_ratio: 1.0,
///     mean_is_best_annualized_return: Some(0.2),
///     mean_is_best_sharpe: Some(1.5),
/// };
/// assert_eq!(diagnostics.oos_profitable_fold_ratio, 0.75);
/// assert_eq!(diagnostics.parameter_stability_ratio, 1.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WalkForwardDiagnostics {
    /// Stitched OOS annualized return divided by the mean IS best annualized
    /// return; `None` when that mean is not strictly positive or a value is not
    /// finite.
    pub walk_forward_efficiency: Option<f64>,
    /// Stitched OOS Sharpe ratio minus the mean IS best Sharpe ratio; `None`
    /// when either side is undefined. Negative values indicate degradation.
    pub sharpe_degradation: Option<f64>,
    /// Fraction of folds whose out-of-sample end equity exceeds its opening
    /// equity, in `[0, 1]`.
    pub oos_profitable_fold_ratio: f64,
    /// Fraction of adjacent fold pairs that selected exactly the same best
    /// parameter set, in `[0, 1]`. Reported as `0.0` when only one fold exists,
    /// because there is no adjacent pair.
    pub parameter_stability_ratio: f64,
    /// Mean IS best annualized return across admissible folds, or `None` when no
    /// fold has an admissible winner.
    pub mean_is_best_annualized_return: Option<f64>,
    /// Mean IS best Sharpe ratio across admissible folds that define it, or
    /// `None` when none does.
    pub mean_is_best_sharpe: Option<f64>,
}

/// The complete result of a walk-forward analysis.
///
/// See [`WalkForwardAnalyzer::run`] for a complete example.
#[derive(Debug, Clone, PartialEq)]
pub struct WalkForwardReport<P> {
    /// Window mode used to generate the folds.
    pub mode: WalkForwardMode,
    /// Evaluated folds in ascending order.
    pub folds: Vec<WalkForwardFold<P>>,
    /// Compounded out-of-sample equity curve, one observation per evaluated OOS
    /// bar of every fold in ascending time order.
    pub stitched_oos_equity_curve: Vec<(DateTime<Utc>, Decimal)>,
    /// Opening equity of the first fold, the starting level of the stitched
    /// curve.
    pub oos_initial_equity: Decimal,
    /// Last value of the stitched out-of-sample curve.
    pub oos_final_equity: Decimal,
    /// Risk and performance statistics computed from the stitched curve with
    /// [`WalkForwardConfig::periods_per_year`] and
    /// [`WalkForwardConfig::risk_free_rate`].
    pub oos_metrics: PerformanceMetrics,
    /// Sum of the one-way turnover recorded by the out-of-sample folds, in
    /// weight units.
    ///
    /// Each fold's initial build is excluded: every fold starts from cash, so
    /// that setup trade is not continuing out-of-sample turnover.
    pub oos_total_turnover: f64,
    /// `oos_total_turnover` annualized over the stitched observations, in turns
    /// per year. Initial fold builds are excluded, matching
    /// [`WalkForwardReport::oos_total_turnover`].
    pub oos_annualized_turnover: f64,
    /// Overfitting diagnostics.
    pub diagnostics: WalkForwardDiagnostics,
}

/// Runs walk-forward analysis over an immutable, aligned market timeline.
///
/// The analyzer is stateless between runs: every call creates fresh accounts for
/// the folds, so it can be reused for several datasets.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::{
///     AlignedMarketData, CalendarTrigger, MarketSnapshot, MultiAssetFeed,
///     PortfolioBacktestConfig, StaticWeightAllocator,
/// };
/// use kurumi_backtest::validation::{
///     GridSearchConfig, ObjectiveMetric, StrategySpec, WalkForwardAnalyzer,
///     WalkForwardConfig,
/// };
/// use kurumi_ta::OhlcvBar;
/// use rust_decimal::Decimal;
/// use std::collections::BTreeMap;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let day = 86_400_000_i64;
/// let mut series = BTreeMap::new();
/// let bars = (0..64)
///     .map(|index| {
///         let close = 100.0 + 0.4 * index as f64 + ((index as f64) * 0.7).sin();
///         OhlcvBar::new(
///             index * day,
///             close,
///             close + 1.0,
///             close - 1.0,
///             close,
///             1_000.0,
///             100_000.0,
///         )
///     })
///     .collect::<Result<Vec<_>, _>>()?;
/// series.insert("AAA".to_owned(), bars);
/// let data = AlignedMarketData::from_feed(MultiAssetFeed::new(series)?);
///
/// let config = WalkForwardConfig::new(
///     20,
///     8,
///     GridSearchConfig::new(ObjectiveMetric::TotalReturn),
/// )
/// .with_embargo_bars(2)
/// .with_warmup_bars(5);
/// let params = vec![5usize, 7usize];
/// let report = WalkForwardAnalyzer::new(config).run(&data, &params, |bars| {
///     let mut backtest = PortfolioBacktestConfig::new(Decimal::from(100_000))?;
///     backtest.allow_fractional = true;
///     let allocator =
///         StaticWeightAllocator::new(BTreeMap::from([("AAA".to_owned(), 1.0)]))?;
///     Ok(StrategySpec::new(
///         backtest,
///         Box::new(CalendarTrigger::fixed_interval(*bars)?),
///         Box::new(allocator),
///     ))
/// })?;
///
/// assert_eq!(report.folds.len(), 5);
/// assert_eq!(report.stitched_oos_equity_curve.len(), 40);
/// assert_eq!(report.oos_metrics.observations, 39);
/// assert!(report.oos_final_equity > Decimal::ZERO);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct WalkForwardAnalyzer {
    config: WalkForwardConfig,
}

impl WalkForwardAnalyzer {
    /// Creates an analyzer from a walk-forward configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::validation::{
    ///     GridSearchConfig, ObjectiveMetric, WalkForwardAnalyzer, WalkForwardConfig,
    /// };
    ///
    /// let config =
    ///     WalkForwardConfig::new(250, 100, GridSearchConfig::new(ObjectiveMetric::SharpeRatio));
    /// let analyzer = WalkForwardAnalyzer::new(config);
    /// assert_eq!(analyzer.config().in_sample_bars, 250);
    /// ```
    #[must_use]
    pub fn new(config: WalkForwardConfig) -> Self {
        Self { config }
    }

    /// Returns the analyzer configuration.
    #[must_use]
    #[inline]
    pub fn config(&self) -> &WalkForwardConfig {
        &self.config
    }

    /// Runs the walk-forward analysis over `data`.
    ///
    /// For each fold the method
    ///
    /// 1. runs the parameter grid over `data[is_warmup_start..is_end]` with the
    ///    fold's computed warmup, selecting the best admissible parameter set;
    /// 2. runs only that parameter set over `data[oos_warmup_start..oos_end]`
    ///    with the OOS warmup, retaining its complete report;
    /// 3. compounds the report's equity curve onto the stitched OOS level with
    ///    exact [`Decimal`] arithmetic.
    ///
    /// The complete stitched curve then feeds `kurumi-risk`. The market data is
    /// borrowed for the whole analysis and each fold borrows a zero-copy
    /// [`MarketSlice`](crate::portfolio::MarketSlice): no history is cloned per
    /// parameter combination.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`WalkForwardConfig::validate`] and
    /// [`generate_folds`], [`KurumiError::InsufficientData`] for an empty
    /// parameter grid, [`KurumiError::InvalidParameter`] when
    /// [`WalkForwardConfig::fallback_param_index`] is out of range or a fold has
    /// no admissible winner and no fallback is configured, when a fold's OOS
    /// backtest fails or loses observations, and when the stitched curve is not
    /// a strictly increasing time series. Propagates the strategy factory and
    /// backtest errors that stop a fold.
    pub fn run<P, F>(
        &self,
        data: &AlignedMarketData,
        params: &[P],
        factory: F,
    ) -> Result<WalkForwardReport<P>, KurumiError>
    where
        P: Clone + PartialEq + Send + Sync,
        F: Fn(&P) -> Result<StrategySpec, KurumiError> + Sync,
    {
        self.config.validate()?;
        if params.is_empty() {
            return Err(KurumiError::InsufficientData {
                required: 1,
                available: 0,
            });
        }
        if let Some(index) = self.config.fallback_param_index {
            if params.get(index).is_none() {
                return Err(KurumiError::InvalidParameter {
                    parameter: "fallback_param_index",
                    reason: format!(
                        "index {index} is outside the {} parameter combinations",
                        params.len()
                    ),
                });
            }
        }

        let windows = generate_folds(data.len(), &self.config)?;
        let factory = &factory;
        let mut folds = Vec::with_capacity(windows.len());
        let mut stitched: Vec<(DateTime<Utc>, Decimal)> = Vec::new();
        let mut level: Option<Decimal> = None;
        let mut total_turnover = 0.0_f64;

        for window in windows {
            let in_sample_result = self.evaluate_in_sample(&window, data, params, factory)?;
            let best_summary = in_sample_result.best().and_then(|trial| trial.summary);
            let (best_params, used_fallback) = match in_sample_result.best() {
                Some(trial) => (trial.params.clone(), false),
                None => {
                    let fallback = self.config.fallback_param_index.ok_or_else(|| {
                        KurumiError::InvalidParameter {
                            parameter: "grid_config",
                            reason: format!(
                                "fold {} found no admissible in-sample parameter set; configure \
                                 `fallback_param_index` or relax the result filter",
                                window.fold_id
                            ),
                        }
                    })?;
                    let params =
                        params
                            .get(fallback)
                            .ok_or_else(|| KurumiError::InvalidParameter {
                                parameter: "fallback_param_index",
                                reason: format!(
                                    "index {fallback} is outside the {} parameter combinations",
                                    params.len()
                                ),
                            })?;
                    (params.clone(), true)
                }
            };
            let is_best_summary = if used_fallback { None } else { best_summary };

            let oos_report = self.evaluate_out_of_sample(&window, data, &best_params, factory)?;
            if oos_report.equity_curve.len() != window.oos_bars() {
                return Err(KurumiError::InvalidParameter {
                    parameter: "oos_report",
                    reason: format!(
                        "fold {} recorded {} out-of-sample observations but the window has {} bars",
                        window.fold_id,
                        oos_report.equity_curve.len(),
                        window.oos_bars()
                    ),
                });
            }

            let initial_equity = oos_report.initial_equity;
            let base_level = level.unwrap_or(initial_equity);
            let fold_curve = stitch_fold(base_level, initial_equity, &oos_report.equity_curve)?;
            if let (Some((last_timestamp, _)), Some((first_timestamp, _))) =
                (stitched.last(), fold_curve.first())
            {
                if first_timestamp <= last_timestamp {
                    return Err(KurumiError::InvalidParameter {
                        parameter: "stitched_oos_equity_curve",
                        reason: format!(
                            "fold {} starts at {first_timestamp}, which does not follow {last_timestamp}",
                            window.fold_id
                        ),
                    });
                }
            }
            stitched.extend(fold_curve.iter().copied());
            level = fold_curve.last().map(|(_, equity)| *equity);

            // Each fold starts from cash, so its first rebalance is a setup
            // trade that re-enters the book. Counting it would make the OOS
            // turnover depend on the number of folds rather than on ongoing
            // trading, so only the continuing rebalances contribute here.
            total_turnover += oos_report
                .rebalance_events
                .iter()
                .filter(|event| event.cause != RebalanceCause::Initial)
                .map(|event| event.turnover)
                .sum::<f64>();

            folds.push(WalkForwardFold {
                window,
                is_result: in_sample_result,
                best_params,
                used_fallback,
                is_best_summary,
                oos_report,
                stitched_equity_curve: fold_curve,
            });
        }

        let returns = curve_returns(&stitched)?;
        let metrics_config =
            MetricsConfig::new(self.config.periods_per_year, self.config.risk_free_rate)?;
        let oos_metrics = PerformanceMetrics::from_returns(&returns, metrics_config)?;
        let periods = returns.len() as f64;
        let oos_annualized_turnover = total_turnover / periods * self.config.periods_per_year;
        if !oos_total_turnover_is_finite(total_turnover) || !oos_annualized_turnover.is_finite() {
            return Err(KurumiError::NumericalOverflow {
                operation: "WalkForwardAnalyzer::turnover",
            });
        }
        let oos_initial_equity = folds
            .first()
            .map(|fold| fold.oos_report.initial_equity)
            .ok_or(KurumiError::InsufficientData {
                required: 1,
                available: 0,
            })?;
        let oos_final_equity = level.ok_or(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        })?;
        let diagnostics = build_diagnostics(&folds, &oos_metrics);

        Ok(WalkForwardReport {
            mode: self.config.mode,
            folds,
            stitched_oos_equity_curve: stitched,
            oos_initial_equity,
            oos_final_equity,
            oos_metrics,
            oos_total_turnover: total_turnover,
            oos_annualized_turnover,
            diagnostics,
        })
    }

    fn evaluate_in_sample<P, F>(
        &self,
        window: &FoldWindow,
        data: &AlignedMarketData,
        params: &[P],
        factory: &F,
    ) -> Result<GridSearchResult<P>, KurumiError>
    where
        P: Clone + Send + Sync,
        F: Fn(&P) -> Result<StrategySpec, KurumiError> + Sync,
    {
        let slice = data.range(window.is_warmup_start..window.is_end)?;
        let mut config = self.config.grid_config.clone();
        config.warmup_bars = window.is_warmup();
        GridSearchRunner::new(config).run(slice, params, factory)
    }

    fn evaluate_out_of_sample<P, F>(
        &self,
        window: &FoldWindow,
        data: &AlignedMarketData,
        best_params: &P,
        factory: &F,
    ) -> Result<PortfolioBacktestReport, KurumiError>
    where
        P: Clone + Send + Sync,
        F: Fn(&P) -> Result<StrategySpec, KurumiError> + Sync,
    {
        let slice = data.range(window.oos_warmup_start..window.oos_end)?;
        let mut config = self.config.grid_config.clone();
        config.warmup_bars = window.oos_warmup();
        config.retain_all_reports = true;
        let result =
            GridSearchRunner::new(config).run(slice, std::slice::from_ref(best_params), factory)?;
        let trial = result
            .trials
            .into_iter()
            .next()
            .ok_or(KurumiError::InsufficientData {
                required: 1,
                available: 0,
            })?;
        trial.report.ok_or_else(|| KurumiError::InvalidParameter {
            parameter: "oos_report",
            reason: match &trial.status {
                TrialStatus::Succeeded => format!(
                    "fold {} out-of-sample evaluation produced no report",
                    window.fold_id
                ),
                TrialStatus::Failed(error) => format!(
                    "fold {} out-of-sample evaluation failed: {error}",
                    window.fold_id
                ),
            },
        })
    }
}

/// Scales `curve` onto `base_level` by the fold's growth relative to its own
/// opening equity.
fn stitch_fold(
    base_level: Decimal,
    initial_equity: Decimal,
    curve: &[(DateTime<Utc>, Decimal)],
) -> Result<Vec<(DateTime<Utc>, Decimal)>, KurumiError> {
    if initial_equity <= Decimal::ZERO {
        return Err(KurumiError::InvalidParameter {
            parameter: "initial_equity",
            reason: "must be strictly positive to stitch an out-of-sample curve".to_owned(),
        });
    }
    let mut stitched = Vec::with_capacity(curve.len());
    for (timestamp, equity) in curve {
        let scaled = base_level
            .checked_mul(*equity)
            .ok_or(KurumiError::NumericalOverflow {
                operation: "WalkForwardAnalyzer::stitched_oos_equity",
            })?
            .checked_div(initial_equity)
            .ok_or(KurumiError::NumericalOverflow {
                operation: "WalkForwardAnalyzer::stitched_oos_equity",
            })?;
        if scaled <= Decimal::ZERO {
            return Err(KurumiError::InvalidParameter {
                parameter: "equity_curve",
                reason: "every stitched observation must be strictly positive".to_owned(),
            });
        }
        stitched.push((*timestamp, scaled));
    }
    Ok(stitched)
}

fn oos_total_turnover_is_finite(total_turnover: f64) -> bool {
    total_turnover.is_finite() && total_turnover >= 0.0
}

fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let value = values.iter().sum::<f64>() / values.len() as f64;
    value.is_finite().then_some(value)
}

fn fold_best_annualized_return<P>(fold: &WalkForwardFold<P>) -> Option<f64> {
    fold.is_best_summary
        .map(|summary| summary.annualized_return)
        .filter(|value| value.is_finite())
}

fn fold_best_sharpe<P>(fold: &WalkForwardFold<P>) -> Option<f64> {
    fold.is_best_summary
        .and_then(|summary| summary.sharpe_ratio)
        .filter(|value| value.is_finite())
}

fn build_diagnostics<P: PartialEq>(
    folds: &[WalkForwardFold<P>],
    oos_metrics: &PerformanceMetrics,
) -> WalkForwardDiagnostics {
    let is_annualized = folds
        .iter()
        .filter_map(fold_best_annualized_return)
        .collect::<Vec<_>>();
    let mean_is_best_annualized_return = mean(&is_annualized);
    let walk_forward_efficiency = match mean_is_best_annualized_return {
        Some(denominator) if denominator > 0.0 => {
            let value = oos_metrics.annualized_return / denominator;
            value.is_finite().then_some(value)
        }
        _ => None,
    };

    let is_sharpe = folds
        .iter()
        .filter_map(fold_best_sharpe)
        .collect::<Vec<_>>();
    let mean_is_best_sharpe = mean(&is_sharpe);
    let sharpe_degradation = match (oos_metrics.sharpe_ratio, mean_is_best_sharpe) {
        (Some(oos), Some(is)) => {
            let value = oos - is;
            value.is_finite().then_some(value)
        }
        _ => None,
    };

    let profitable = folds
        .iter()
        .filter(|fold| fold.oos_report.final_equity > fold.oos_report.initial_equity)
        .count();
    let fold_count = folds.len();
    let oos_profitable_fold_ratio = if fold_count == 0 {
        0.0
    } else {
        profitable as f64 / fold_count as f64
    };

    let stable_pairs = folds
        .windows(2)
        .filter(|pair| match (pair.first(), pair.get(1)) {
            (Some(left), Some(right)) => left.best_params == right.best_params,
            _ => false,
        })
        .count();
    let parameter_stability_ratio = if fold_count < 2 {
        0.0
    } else {
        stable_pairs as f64 / (fold_count - 1) as f64
    };
    debug_assert!((0.0..=1.0).contains(&parameter_stability_ratio));

    WalkForwardDiagnostics {
        walk_forward_efficiency,
        sharpe_degradation,
        oos_profitable_fold_ratio,
        parameter_stability_ratio,
        mean_is_best_annualized_return,
        mean_is_best_sharpe,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::{
        AllocationContext, CalendarTrigger, MultiAssetFeed, PortfolioBacktestConfig,
        StaticWeightAllocator, TargetWeightAllocator,
    };
    use crate::validation::{ObjectiveMetric, ResultFilter};
    use kurumi_portfolio::PortfolioWeights;
    use kurumi_ta::OhlcvBar;
    use rust_decimal::Decimal;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    const DAY: i64 = 86_400_000;
    const BARS: usize = 120;

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
        .expect("valid test bar")
    }

    fn data(bars: usize) -> AlignedMarketData {
        let mut series = BTreeMap::new();
        series.insert(
            "AAA".to_owned(),
            (0..bars)
                .map(|day| {
                    bar(
                        day as i64,
                        100.0 + 0.3 * day as f64 + ((day as f64) * 0.9).sin(),
                    )
                })
                .collect::<Vec<_>>(),
        );
        series.insert(
            "BBB".to_owned(),
            (0..bars)
                .map(|day| {
                    bar(
                        day as i64,
                        80.0 + 0.2 * day as f64 + ((day as f64) * 0.4).cos(),
                    )
                })
                .collect::<Vec<_>>(),
        );
        AlignedMarketData::from_feed(MultiAssetFeed::new(series).expect("valid feed"))
    }

    fn grid_config() -> GridSearchConfig {
        GridSearchConfig::new(ObjectiveMetric::TotalReturn)
    }

    fn scenario_config() -> WalkForwardConfig {
        WalkForwardConfig::new(250, 100, grid_config())
            .with_embargo_bars(5)
            .with_warmup_bars(60)
    }

    fn small_config() -> WalkForwardConfig {
        WalkForwardConfig::new(20, 8, grid_config())
            .with_embargo_bars(2)
            .with_warmup_bars(5)
    }

    fn factory(interval: &usize) -> Result<StrategySpec, KurumiError> {
        let mut config = PortfolioBacktestConfig::new(Decimal::from(100_000))?;
        config.allow_fractional = true;
        config.lookback_window = 10;
        let allocator = StaticWeightAllocator::new(BTreeMap::from([("AAA".to_owned(), 1.0)]))?;
        Ok(StrategySpec::new(
            config,
            Box::new(CalendarTrigger::fixed_interval(*interval)?),
            Box::new(allocator),
        ))
    }

    #[test]
    fn rolling_folds_are_contiguous_and_respect_embargo() {
        let folds = generate_folds(1_000, &scenario_config()).expect("valid plan");
        assert_eq!(folds.len(), 7);

        let first = folds.first().copied().expect("first fold");
        assert_eq!(first.fold_id, 0);
        assert_eq!(first.is_warmup_start, 0);
        assert_eq!(first.is_start, 0);
        assert_eq!(first.is_end, 250);
        assert_eq!(first.is_warmup(), 0);
        assert_eq!(first.is_bars(), 250);
        assert_eq!(first.oos_warmup_start, 195);
        assert_eq!(first.oos_start, 255);
        assert_eq!(first.oos_warmup(), 60);
        assert_eq!(first.oos_end, 355);
        assert_eq!(first.oos_bars(), 100);

        for (index, fold) in folds.iter().enumerate() {
            assert_eq!(fold.fold_id, index);
            assert_eq!(fold.is_bars(), 250);
            assert_eq!(fold.is_start, index * 100);
            assert_eq!(fold.is_warmup(), if index == 0 { 0 } else { 60 });
            assert!(fold.is_end <= fold.oos_start);
            if let Some(previous) = index
                .checked_sub(1)
                .and_then(|previous| folds.get(previous))
            {
                assert_eq!(fold.oos_start, previous.oos_end);
            }
        }
        assert_eq!(folds.last().map(|fold| fold.oos_end), Some(955));
    }

    #[test]
    fn anchored_folds_keep_the_start_and_grow_the_window() {
        let config = scenario_config().with_mode(WalkForwardMode::Anchored);
        let folds = generate_folds(1_000, &config).expect("valid plan");
        assert_eq!(folds.len(), 7);
        for (index, fold) in folds.iter().enumerate() {
            assert_eq!(fold.is_start, 0);
            assert_eq!(fold.is_warmup(), 0);
            assert_eq!(fold.is_end, 250 + index * 100);
            if let Some(previous) = index
                .checked_sub(1)
                .and_then(|previous| folds.get(previous))
            {
                assert!(previous.is_end < fold.is_end);
                assert_eq!(previous.oos_end, fold.oos_start);
            }
        }
    }

    #[test]
    fn an_incomplete_plan_is_rejected_before_any_fold() {
        let config = small_config();
        assert!(matches!(
            generate_folds(29, &config),
            Err(KurumiError::InsufficientData {
                required: 30,
                available: 29
            })
        ));
        assert!(matches!(
            generate_folds(30, &config),
            Ok(folds) if folds.len() == 1
        ));
        assert!(matches!(
            WalkForwardConfig::new(2, 8, grid_config()).validate(),
            Err(KurumiError::InvalidParameter {
                parameter: "in_sample_bars",
                ..
            })
        ));
        assert!(matches!(
            WalkForwardConfig::new(20, 2, grid_config()).validate(),
            Err(KurumiError::InvalidParameter {
                parameter: "out_of_sample_bars",
                ..
            })
        ));
        assert!(matches!(
            WalkForwardConfig::new(20, 8, grid_config())
                .with_periods_per_year(0.0)
                .validate(),
            Err(KurumiError::InvalidParameter {
                parameter: "periods_per_year",
                ..
            })
        ));
    }

    #[test]
    fn analyzer_stitches_fold_curves_and_reports_oos_statistics() {
        let data = data(BARS);
        let report = WalkForwardAnalyzer::new(small_config())
            .run(&data, &[3usize, 5usize], factory)
            .expect("walk-forward run");

        assert_eq!(report.mode, WalkForwardMode::Rolling);
        assert_eq!(report.folds.len(), 12);
        assert_eq!(report.stitched_oos_equity_curve.len(), 96);
        assert_eq!(report.oos_metrics.observations, 95);
        assert!(report.oos_initial_equity > Decimal::ZERO);
        assert!(report.oos_final_equity > Decimal::ZERO);
        assert!(report.oos_total_turnover >= 0.0);
        assert!(report.oos_annualized_turnover.is_finite());
        let initial_builds = report
            .folds
            .iter()
            .filter(|fold| {
                fold.oos_report
                    .rebalance_events
                    .iter()
                    .any(|event| event.cause == RebalanceCause::Initial)
            })
            .count();
        assert_eq!(initial_builds, report.folds.len());
        let expected_turnover = report
            .folds
            .iter()
            .flat_map(|fold| fold.oos_report.rebalance_events.iter())
            .filter(|event| event.cause != RebalanceCause::Initial)
            .map(|event| event.turnover)
            .sum::<f64>();
        assert!((report.oos_total_turnover - expected_turnover).abs() < 1e-12);
        assert!((report.oos_annualized_turnover - expected_turnover / 95.0 * 252.0).abs() < 1e-12);
        assert!(
            report.oos_total_turnover
                < report
                    .folds
                    .iter()
                    .flat_map(|fold| fold.oos_report.rebalance_events.iter())
                    .map(|event| event.turnover)
                    .sum::<f64>(),
            "the initial builds must be excluded from the OOS turnover"
        );
        assert_eq!(
            report
                .stitched_oos_equity_curve
                .last()
                .map(|(_, equity)| *equity),
            Some(report.oos_final_equity)
        );
        assert!(
            report
                .diagnostics
                .mean_is_best_annualized_return
                .is_some_and(f64::is_finite)
        );
        assert!(report.diagnostics.parameter_stability_ratio.is_finite());
        assert!(report.diagnostics.oos_profitable_fold_ratio <= 1.0);

        let mut level = report.oos_initial_equity;
        for fold in &report.folds {
            assert_eq!(fold.oos_report.equity_curve.len(), 8);
            let initial = fold.oos_report.initial_equity;
            for (index, (_, equity)) in fold.oos_report.equity_curve.iter().enumerate() {
                let expected = level
                    .checked_mul(*equity)
                    .and_then(|value| value.checked_div(initial))
                    .expect("scaled level");
                assert_eq!(
                    fold.stitched_equity_curve
                        .get(index)
                        .map(|(_, value)| *value),
                    Some(expected)
                );
            }
            level = fold
                .stitched_equity_curve
                .last()
                .map(|(_, value)| *value)
                .expect("last level");
        }

        let mut expected_timestamps = Vec::new();
        for fold in &report.folds {
            for (timestamp, _) in &fold.oos_report.equity_curve {
                if let Some(previous) = expected_timestamps.last() {
                    assert!(timestamp > previous, "timestamps must strictly increase");
                }
                expected_timestamps.push(*timestamp);
            }
        }
        assert_eq!(
            report
                .stitched_oos_equity_curve
                .iter()
                .map(|(timestamp, _)| *timestamp)
                .collect::<Vec<_>>(),
            expected_timestamps
        );
    }

    /// Records the number of return rows visible to each allocation, so a test
    /// can prove the OOS warmup prefix is consumed rather than dropped.
    type AllocationRecord = (DateTime<Utc>, Option<usize>);
    type SharedAllocationRecords = Arc<Mutex<Vec<AllocationRecord>>>;

    #[derive(Debug)]
    struct RecordingAllocator {
        records: SharedAllocationRecords,
        inner: StaticWeightAllocator,
    }

    impl TargetWeightAllocator for RecordingAllocator {
        fn allocate(&mut self, ctx: &AllocationContext) -> Result<PortfolioWeights, KurumiError> {
            let rows = ctx.returns.as_ref().map(|matrix| matrix.dimensions().0);
            self.records
                .lock()
                .expect("recording allocator mutex")
                .push((ctx.timestamp, rows));
            self.inner.allocate(ctx)
        }
    }

    #[test]
    fn oos_warmup_prefix_is_consumed_before_the_first_oos_allocation() {
        let data = data(BARS);
        let config = small_config();
        let folds = generate_folds(data.len(), &config).expect("valid plan");
        let last = folds.last().copied().expect("last fold");
        let oos_start_timestamp = data
            .snapshots()
            .get(last.oos_start)
            .expect("OOS start snapshot")
            .timestamp();
        let records = Arc::new(Mutex::new(Vec::new()));
        let factory = {
            let records = Arc::clone(&records);
            move |interval: &usize| -> Result<StrategySpec, KurumiError> {
                let mut portfolio = PortfolioBacktestConfig::new(Decimal::from(100_000))?;
                portfolio.allow_fractional = true;
                portfolio.lookback_window = 10;
                let inner = StaticWeightAllocator::new(BTreeMap::from([
                    ("AAA".to_owned(), 0.6),
                    ("BBB".to_owned(), 0.4),
                ]))?;
                Ok(StrategySpec::new(
                    portfolio,
                    Box::new(CalendarTrigger::fixed_interval(*interval)?),
                    Box::new(RecordingAllocator {
                        records: Arc::clone(&records),
                        inner,
                    }),
                ))
            }
        };

        WalkForwardAnalyzer::new(config)
            .run(&data, &[3usize, 5usize], factory)
            .expect("walk-forward run");

        let recorded = records.lock().expect("recording allocator mutex");
        let observed = recorded
            .iter()
            .filter(|(timestamp, _)| *timestamp == oos_start_timestamp)
            .filter_map(|(_, rows)| *rows)
            .max();
        assert_eq!(
            observed,
            Some(last.oos_warmup()),
            "the first OOS allocation must see the whole warmup prefix"
        );
    }

    #[test]
    fn a_fold_without_an_admissible_winner_errors_or_uses_the_fallback() {
        let strict_grid = GridSearchConfig::new(ObjectiveMetric::TotalReturn)
            .with_filter(ResultFilter::new().with_min_rebalance_count(usize::MAX));
        let strict = WalkForwardConfig::new(20, 8, strict_grid)
            .with_embargo_bars(2)
            .with_warmup_bars(5);
        let data = data(BARS);
        let analyzer = WalkForwardAnalyzer::new(strict);
        assert!(matches!(
            analyzer.run(&data, &[3usize, 5usize], factory),
            Err(KurumiError::InvalidParameter {
                parameter: "grid_config",
                ..
            })
        ));

        let fallback = analyzer.config().clone().with_fallback_param_index(Some(1));
        let report = WalkForwardAnalyzer::new(fallback)
            .run(&data, &[3usize, 5usize], factory)
            .expect("fallback run");
        assert!(report.folds.iter().all(|fold| fold.used_fallback));
        assert!(report.folds.iter().all(|fold| fold.best_params == 5));
        assert!(report.diagnostics.walk_forward_efficiency.is_none());
        assert!(report.diagnostics.mean_is_best_annualized_return.is_none());
        assert!((report.diagnostics.parameter_stability_ratio - 1.0).abs() < 1e-15);

        let out_of_range = analyzer.config().clone().with_fallback_param_index(Some(9));
        assert!(matches!(
            WalkForwardAnalyzer::new(out_of_range).run(&data, &[3usize, 5usize], factory),
            Err(KurumiError::InvalidParameter {
                parameter: "fallback_param_index",
                ..
            })
        ));
    }

    #[test]
    fn empty_parameter_grids_are_rejected() {
        let data = data(BARS);
        let analyzer = WalkForwardAnalyzer::new(small_config());
        let params: [usize; 0] = [];
        assert!(matches!(
            analyzer.run(&data, &params, factory),
            Err(KurumiError::InsufficientData {
                required: 1,
                available: 0
            })
        ));
        assert!(analyzer.run(&data, &[3usize], factory).is_ok());
    }
}
