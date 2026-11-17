//! Core primitives shared by every Kurumi crate.
//!
//! `kurumi-core` is the lowest layer of the workspace. It defines the vocabulary
//! the rest of Kurumi is built on:
//!
//! - monetary amounts and high-precision decimal values;
//! - calendar dates, business-day conventions, and day-count fractions;
//! - the unified `KurumiError` type returned by fallible public APIs.
//!
//! The crate has no workspace dependencies, and every other `kurumi-*` crate is
//! allowed to depend on it.
//!
//! # Numerical conventions
//!
//! Money and discrete cashflows use `rust_decimal::Decimal`; continuous model
//! parameters use `f64`. Public constructors validate their inputs and return
//! `KurumiError` instead of panicking, and reject `NaN`/infinite values at the
//! boundary.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]
