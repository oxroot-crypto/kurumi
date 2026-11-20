//! Strategy validation: objective metrics, admissibility filters, parallel
//! parameter grid search, and walk-forward analysis.
//!
//! - [`objective`] defines [`ObjectiveMetric`], the scalar a sweep maximizes,
//!   and [`ResultFilter`], the constraints a trial must satisfy before it can
//!   rank.
//! - [`grid`] defines [`GridSearchRunner`], which evaluates a caller-supplied
//!   [`StrategySpec`] factory for every parameter combination against one shared
//!   [`MarketSlice`](crate::portfolio::MarketSlice).
//! - [`walk_forward`] defines [`WalkForwardAnalyzer`], which selects parameters
//!   on rolling or anchored in-sample windows, evaluates them on the following
//!   out-of-sample windows, and compounds the OOS equity into one stitched curve
//!   with overfitting diagnostics.
//!
//! All trials borrow the same immutable market timeline, so a parameter sweep
//! never clones historical bars per combination. Ranking filters out failed,
//! inadmissible, and non-finite trials and breaks score ties by the original
//! grid index, which keeps parallel and sequential runs equivalent.
//!
//! # Examples
//!
//! ```
//! use kurumi_backtest::validation::{GridSearchConfig, ObjectiveMetric, ResultFilter};
//!
//! let config = GridSearchConfig::new(ObjectiveMetric::SharpeRatio)
//!     .with_filter(ResultFilter::new().with_max_drawdown_limit(0.25))
//!     .with_warmup_bars(60);
//! assert_eq!(config.warmup_bars, 60);
//! ```

pub mod grid;
pub mod objective;
pub mod walk_forward;

pub use grid::{
    GridSearchConfig, GridSearchResult, GridSearchRunner, GridTrial, StrategySpec, TrialStatus,
    TrialSummary,
};
pub use objective::{ObjectiveMetric, ResultFilter};
pub use walk_forward::{
    FoldWindow, WalkForwardAnalyzer, WalkForwardConfig, WalkForwardDiagnostics, WalkForwardFold,
    WalkForwardMode, WalkForwardReport, generate_folds,
};
