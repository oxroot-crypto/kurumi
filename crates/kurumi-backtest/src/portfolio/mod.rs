//! Multi-asset portfolio backtesting infrastructure.
//!
//! This module is additive to the single-asset backtest kernel and lives beside
//! it:
//!
//! - [`feed`] provides [`MarketSnapshot`] and [`MultiAssetFeed`], the
//!   deterministic union timeline of several OHLCV series;
//! - [`account`] provides [`AssetPosition`], [`MultiAssetFill`], and
//!   [`MultiAssetAccount`], the `Decimal` ledger for several instruments;
//! - [`trigger`] provides [`TriggerContext`], the [`RebalanceTrigger`] trait,
//!   and calendar, drift, and composite implementations.
//! - [`allocator`] bridges the loop to `kurumi-portfolio` with static, rolling
//!   optimizer, and closure-backed target-weight allocators.
//! - [`engine`] runs the backtest: signal timing, halt handling,
//!   sell-before-buy execution, and the audit trail.
//! - [`report`] turns the equity curve into risk metrics and benchmark-relative
//!   attribution, keeping every rebalance event and weight snapshot.
//!
//! Every position, snapshot, and price map is a [`std::collections::BTreeMap`](https://doc.rust-lang.org/std/collections/struct.BTreeMap.html),
//! so iteration order and backtest results are deterministic. Money and
//! quantities stay in [`rust_decimal::Decimal`]; `f64` appears only in weights,
//! which the portfolio optimizers consume.
//!
//! # Examples
//!
//! ```
//! use kurumi_backtest::portfolio::{
//!     CalendarTrigger, MultiAssetAccount, MultiAssetFeed, MultiAssetFill, RebalanceTrigger,
//!     TriggerContext,
//! };
//! use kurumi_backtest::Side;
//! use kurumi_core::{Price, Quantity};
//! use kurumi_ta::OhlcvBar;
//! use rust_decimal::Decimal;
//! use std::collections::BTreeMap;
//!
//! # fn main() -> Result<(), kurumi_backtest::KurumiError> {
//! let mut series = BTreeMap::new();
//! series.insert(
//!     "AAA".to_owned(),
//!     vec![OhlcvBar::new(0, 10.0, 11.0, 9.0, 10.0, 1_000.0, 10_000.0)?],
//! );
//! let feed = MultiAssetFeed::new(series)?;
//!
//! let mut account = MultiAssetAccount::new(Decimal::from(10_000))?;
//! let snapshot = feed.iter().next().expect("one snapshot");
//! account.update_market_prices(snapshot)?;
//!
//! let fill = MultiAssetFill::new(
//!     "AAA",
//!     Side::Buy,
//!     Price::new(Decimal::from(10))?,
//!     Quantity::new(Decimal::from(100))?,
//!     Decimal::ONE,
//!     Decimal::ZERO,
//! )?;
//! account.apply_fill(&fill)?;
//! assert_eq!(account.position("AAA").map(|p| p.quantity()), Some(Decimal::from(100)));
//!
//! let context = TriggerContext {
//!     current_timestamp: snapshot.timestamp(),
//!     last_rebalance_timestamp: None,
//!     bars_advanced: 0,
//!     last_rebalance_bar: None,
//!     current_weights: account.current_weights()?,
//!     target_weights: BTreeMap::new(),
//! };
//! assert!(CalendarTrigger::Monthly.should_rebalance(&context));
//! # Ok(())
//! # }
//! ```

pub mod account;
pub mod allocator;
pub mod engine;
pub mod feed;
pub mod report;
pub mod trigger;

pub use account::{AssetPosition, MultiAssetAccount, MultiAssetFill};
pub use allocator::{
    AllocationContext, CustomAllocator, InsufficientDataPolicy, RollingOptimizerAllocator,
    StaticWeightAllocator, TargetWeightAllocator,
};
pub use engine::{ExecutionTiming, PortfolioBacktestConfig, PortfolioBacktestEngine};
pub use feed::{MarketSnapshot, MultiAssetFeed};
pub use report::{BenchmarkComparison, PortfolioBacktestReport, RebalanceEvent};
pub use trigger::{
    CalendarTrigger, CompositeTrigger, DriftTrigger, RebalanceCause, RebalanceTrigger,
    TriggerContext,
};

/// The `chrono` crate, re-exported so callers can build timestamps without
/// adding a separate dependency.
pub use chrono;
