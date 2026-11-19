//! Risk and performance analytics.
//!
//! This crate aggregates return series and market data into risk measures:
//! Value at Risk, CVaR/expected shortfall, performance and drawdown statistics,
//! and position-sizing rules.
//!
//! - [`metrics`] computes annualized return and volatility, Sharpe, Sortino and
//!   Calmar ratios, maximum drawdown and its duration, win rate, and the
//!   profit/loss ratio.
//! - [`var`] provides historical and parametric Value at Risk together with
//!   Expected Shortfall.
//! - [`sizing`] provides the Kelly criterion and a volatility-target position
//!   calculator.
//! - [`relative`] provides benchmark-relative metrics: beta, conditional betas,
//!   Jensen's alpha, tracking error, information and Treynor ratios, and
//!   capture ratios.
//!
//! All public functions consume finite `f64` inputs, reject `NaN` and infinite
//! values at the boundary, and return the shared [`KurumiError`] type.
//!
//! It depends on `kurumi-core`, `kurumi-math`, `kurumi-ta`, and
//! `kurumi-pricing`; it must not depend on `kurumi-backtest` or the `kurumi`
//! facade.
//!
//! # Examples
//!
//! ```
//! use kurumi_risk::{MetricsConfig, PerformanceMetrics, historical_var};
//!
//! # fn main() -> Result<(), kurumi_risk::KurumiError> {
//! let returns = [0.01, -0.02, 0.015, 0.005, -0.01, 0.02];
//! let metrics = PerformanceMetrics::from_returns(&returns, MetricsConfig::default())?;
//! assert!(metrics.sharpe_ratio.is_some());
//!
//! let var = historical_var(&returns, 0.95)?;
//! assert!(var >= 0.0);
//! # Ok(())
//! # }
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

pub mod metrics;
pub mod relative;
pub mod sizing;
pub mod var;

pub use metrics::{
    DrawdownStats, MetricsConfig, PerformanceMetrics, annualized_return, annualized_volatility,
    calmar_ratio, drawdown_stats, max_drawdown, profit_loss_ratio, sharpe_ratio, sortino_ratio,
    total_return, win_rate,
};
pub use relative::{
    CaptureRatios, beta, capture_ratios, downside_beta, information_ratio, jensen_alpha,
    tracking_error, treynor_ratio, upside_beta,
};
pub use sizing::{VolatilityTargetSizer, kelly_fraction, volatility_target_fraction};
pub use var::{
    historical_expected_shortfall, historical_var, parametric_expected_shortfall, parametric_var,
};

pub use kurumi_core::KurumiError;
