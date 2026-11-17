//! Numerical methods for Kurumi.
//!
//! This crate provides the reusable computational building blocks used by
//! pricing, risk, and indicator code:
//!
//! - interpolation (linear, log-linear, and cubic splines);
//! - root finding (bisection, Newton-Raphson, and Brent);
//! - random number generation and stochastic path construction;
//! - summary statistics and linear-algebra helpers.
//!
//! It sits directly above `kurumi-core` and must not depend on any higher-layer
//! crate.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]
