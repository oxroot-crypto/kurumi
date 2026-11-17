//! Strategy backtesting.
//!
//! This crate turns signals and orders into simulated executions: order-book
//! matching, slippage and fee models, and account/position tracking.
//!
//! It is the top layer before the `kurumi` facade, so it may use every lower
//! crate, but nothing in the workspace may depend on it except the facade.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]
