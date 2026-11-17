//! Asset and derivative pricing.
//!
//! This crate models cashflows, discount factors, yield curves, fixed-income
//! instruments, and option pricing engines. Dates and day-count conventions come
//! from `kurumi-core`; numerical routines come from `kurumi-math`.
//!
//! Discrete money amounts use `Decimal`; continuous model values use `f64`.
//! Invalid model inputs (non-positive volatility, out-of-range probabilities,
//! `NaN`/infinite values) are rejected with typed errors at the public boundary.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]
