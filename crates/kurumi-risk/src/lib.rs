//! Risk and performance analytics.
//!
//! This crate aggregates pricing and market data into risk measures: Value at
//! Risk, CVaR/expected shortfall, Greeks aggregation, the Sharpe ratio, and
//! drawdown statistics.
//!
//! It depends on `kurumi-core`, `kurumi-math`, `kurumi-ta`, and
//! `kurumi-pricing`; it must not depend on `kurumi-backtest` or the `kurumi`
//! facade.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]
