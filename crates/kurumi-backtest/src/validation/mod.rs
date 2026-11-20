//! Strategy validation: objective metrics, admissibility filters, and parallel
//! parameter grid search.
//!
//! - [`objective`] defines [`ObjectiveMetric`], the scalar a sweep maximizes,
//!   and [`ResultFilter`], the constraints a trial must satisfy before it can
//!   rank.
//! - [`grid`] defines [`GridSearchRunner`], which evaluates a caller-supplied
//!   [`StrategySpec`] factory for every parameter combination against one shared
//!   [`MarketSlice`](crate::portfolio::MarketSlice).
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

pub use grid::{
    GridSearchConfig, GridSearchResult, GridSearchRunner, GridTrial, StrategySpec, TrialStatus,
    TrialSummary,
};
pub use objective::{ObjectiveMetric, ResultFilter};
