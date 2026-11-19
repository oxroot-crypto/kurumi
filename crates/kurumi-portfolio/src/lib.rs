//! Multi-asset data alignment, portfolio optimization, performance analytics,
//! and rebalancing.
//!
//! The crate is built around five stages:
//!
//! 1. [`panel`] aligns independently observed price or return series onto a
//!    common time axis and turns them into a `(T, N)` [`ReturnMatrix`]: rows are
//!    chronological observations and columns are assets.
//! 2. [`analytics`] validates portfolio weights and computes expected return,
//!    volatility, annualized Sharpe ratio, diversification ratio, Euler risk
//!    contributions, and parametric or historical portfolio VaR/CVaR.
//! 3. [`optimize`] solves constrained allocations with equal-weight,
//!    inverse-volatility, mean-variance (GMV, target return, maximum Sharpe, and
//!    the efficient frontier), risk-parity, and hierarchical risk-parity
//!    strategies.
//! 4. [`rebalance`] turns a target allocation and current `Decimal` holdings
//!    into buy and sell trade suggestions.
//! 5. [`rolling`] projects rolling betas and correlations against a benchmark
//!    column of a return matrix.
//!
//! Multi-asset matrices follow the workspace convention: all return matrices
//! are `(T, N)` and all covariance/correlation matrices are symmetric `(N, N)`.
//! Sample moments use Bessel's correction. Series are aligned only by the
//! explicit [`AlignmentPolicy`] supplied by the caller; nothing is
//! forward-filled silently.
//!
//! # Examples
//!
//! ```
//! use kurumi_portfolio::chrono::{DateTime, Utc};
//! use kurumi_portfolio::nalgebra::DVector;
//! use kurumi_portfolio::{
//!     AssetSeries, AlignmentPolicy, PortfolioWeights, ReturnMatrix, ReturnMethod,
//!     risk_decomposition,
//! };
//!
//! # fn at(day: i64) -> DateTime<Utc> {
//! #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
//! # }
//! # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
//! let a = AssetSeries::new(
//!     "A",
//!     vec![(at(0), 100.0), (at(1), 110.0), (at(2), 121.0)],
//! )?;
//! let b = AssetSeries::new(
//!     "B",
//!     vec![(at(0), 50.0), (at(1), 49.0), (at(2), 51.0)],
//! )?;
//! let returns = ReturnMatrix::from_price_series(
//!     &[a, b],
//!     AlignmentPolicy::InnerJoin,
//!     ReturnMethod::Simple,
//! )?;
//!
//! let weights = PortfolioWeights::new(
//!     vec!["A".to_owned(), "B".to_owned()],
//!     DVector::from_vec(vec![0.6, 0.4]),
//! )?;
//! let covariance = returns.covariance_matrix()?;
//! let decomposition = risk_decomposition(&weights, &covariance)?;
//! assert!(decomposition.portfolio_volatility > 0.0);
//! assert_eq!(covariance.shape(), (2, 2));
//! # Ok(())
//! # }
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

pub mod analytics;
pub mod optimize;
pub mod panel;
pub mod rebalance;
pub mod rolling;

pub use analytics::{
    PortfolioVar, PortfolioWeights, RiskDecomposition, annualized_sharpe_ratio,
    diversification_ratio, historical_portfolio_var, parametric_portfolio_var,
    portfolio_expected_return, portfolio_volatility, risk_decomposition,
};
pub use optimize::{
    EqualRiskContribution, EqualWeight, GlobalMinVariance, HierarchicalRiskParity, HrpLinkage,
    InverseVolatility, MaxSharpe, OptimizationConstraints, OptimizationResult, PortfolioOptimizer,
    TargetReturn, efficient_frontier,
};
pub use panel::{AlignmentPolicy, AssetSeries, ReturnMatrix, ReturnMethod};
pub use rebalance::{Holding, RebalanceAction, RebalanceSide, generate_rebalance_actions};
pub use rolling::{
    RollingBetaPanel, RollingPanel, rolling_beta_and_correlation, rolling_betas,
    rolling_correlations,
};

/// The `chrono` crate, re-exported so callers can construct time series
/// timestamps without adding a separate dependency.
pub use chrono;

/// The shared Kurumi error type returned by every fallible API in this crate.
pub use kurumi_core::KurumiError;

/// The `nalgebra` crate, re-exported for the matrix and vector types in the
/// public API.
pub use nalgebra;

/// The `rust_decimal` crate, re-exported for the [`Decimal`](rust_decimal::Decimal)
/// amounts used by the rebalancing API.
pub use rust_decimal;
