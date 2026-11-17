//! Time-series structures and technical indicators.
//!
//! The crate owns candle/OHLCV structures and exposes each indicator in two
//! forms:
//!
//! - an O(1) streaming form for live data, updated one observation at a time;
//! - a vectorized batch form for research, historical analysis, and backtests.
//!
//! Both forms share one definition of the formula and must agree within the
//! documented tolerance. This crate sits above `kurumi-core` and `kurumi-math`,
//! and below `kurumi-risk`, `kurumi-backtest`, and the `kurumi` facade.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]
