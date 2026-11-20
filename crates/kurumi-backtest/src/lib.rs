//! Strategy backtesting.
//!
//! This crate turns signals and orders into simulated executions: order
//! matching, slippage and fee models, account and position tracking, and a
//! performance report built on `kurumi-risk`.
//!
//! - [`order`] defines [`Order`], [`OrderKind`], [`Side`], and [`Fill`].
//! - [`fee`] and [`slippage`] provide the transaction-cost models.
//! - [`account`] tracks cash, signed positions, average cost, realized and
//!   unrealized profit and loss, and the equity curve.
//! - [`engine`] matches pending orders bar by bar and marks the account to the
//!   close.
//! - [`report`] converts the equity curve into returns and produces a
//!   [`BacktestReport`] with the full `kurumi-risk`
//!   metric set.
//! - [`portfolio`] adds deterministic multi-asset feeds, accounts, rebalance
//!   triggers, allocators, the portfolio backtest engine, and attribution
//!   reports without changing the single-asset APIs.
//! - [`validation`] adds objective metrics, result filters, a Rayon-parallel
//!   parameter grid search over shared, zero-copy market slices, and walk-forward
//!   analysis with compounded out-of-sample stitching.
//!
//! The `parallel` feature (enabled by default) gates `rayon` usage; without it
//! the grid search falls back to sequential iteration.
//!
//! It is the top layer before the `kurumi` facade, so it may use every lower
//! crate, but nothing in the workspace may depend on it except the facade.
//!
//! # Examples
//!
//! ```
//! use kurumi_backtest::{
//!     BacktestEngine, FixedBpsSlippage, FixedRateFee, Order, Side,
//! };
//! use kurumi_core::{Currency, Quantity};
//! use kurumi_risk::MetricsConfig;
//! use kurumi_ta::OhlcvBar;
//! use rust_decimal::Decimal;
//!
//! # fn main() -> Result<(), kurumi_backtest::KurumiError> {
//! let mut engine = BacktestEngine::with_cash(
//!     Decimal::from(100_000),
//!     Currency::Usd,
//!     Box::new(FixedRateFee::new(Decimal::new(5, 4))?),
//!     Box::new(FixedBpsSlippage::new(Decimal::from(2))?),
//! )?;
//! engine.submit(Order::market(Side::Buy, Quantity::new(Decimal::from(100))?))?;
//! for (index, price) in [100.0, 102.0, 104.0].iter().enumerate() {
//!     let bar = OhlcvBar::new(
//!         index as i64,
//!         *price,
//!         price + 1.0,
//!         price - 1.0,
//!         *price,
//!         1_000.0,
//!         price * 1_000.0,
//!     )?;
//!     engine.on_bar(&bar)?;
//! }
//! let report = engine.report(MetricsConfig::default())?;
//! assert_eq!(report.fill_count, 1);
//! # Ok(())
//! # }
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

pub mod account;
pub mod engine;
pub mod fee;
pub mod order;
pub mod portfolio;
pub mod report;
pub mod slippage;
pub mod validation;

pub use account::{EquityPoint, PortfolioAccount};
pub use engine::BacktestEngine;
pub use fee::{FeeModel, FixedRateFee, Liquidity, MakerTakerFee, NoFee};
pub use order::{Fill, Order, OrderKind, Side};
pub use portfolio::{
    AlignedMarketData, AllocationContext, AssetPosition, BenchmarkComparison, CalendarTrigger,
    CompositeTrigger, CustomAllocator, DriftTrigger, ExecutionTiming, InsufficientDataPolicy,
    MarketSlice, MarketSnapshot, MultiAssetAccount, MultiAssetFeed, MultiAssetFill,
    PortfolioBacktestConfig, PortfolioBacktestEngine, PortfolioBacktestReport, RebalanceCause,
    RebalanceEvent, RebalanceTrigger, RollingOptimizerAllocator, StaticWeightAllocator,
    TargetWeightAllocator, TriggerContext,
};
pub use report::{BacktestReport, equity_returns};
pub use slippage::{FixedBpsSlippage, NoSlippage, SlippageModel};
pub use validation::{
    FoldWindow, GridSearchConfig, GridSearchResult, GridSearchRunner, GridTrial, ObjectiveMetric,
    ResultFilter, StrategySpec, TrialStatus, TrialSummary, WalkForwardAnalyzer, WalkForwardConfig,
    WalkForwardDiagnostics, WalkForwardFold, WalkForwardMode, WalkForwardReport, generate_folds,
};

pub use kurumi_core::KurumiError;
