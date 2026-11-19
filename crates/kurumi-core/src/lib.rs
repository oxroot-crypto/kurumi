//! Core primitives shared by every Kurumi crate.
//!
//! `kurumi-core` is the lowest layer of the workspace. It defines the vocabulary
//! the rest of Kurumi is built on:
//!
//! - monetary amounts and high-precision decimal values;
//! - calendar dates, business-day conventions, and day-count fractions;
//! - fixed-capacity ring buffers and their O(1) sliding-window aggregates;
//! - the unified [`KurumiError`] type returned by fallible public APIs.
//!
//! The crate has no workspace dependencies, and every other `kurumi-*` crate is
//! allowed to depend on it.
//!
//! # Numerical conventions
//!
//! Money and discrete cashflows use [`rust_decimal::Decimal`]; continuous model
//! parameters use `f64`. Public constructors validate their inputs and return
//! [`KurumiError`] instead of panicking, and reject `NaN`/infinite values at the
//! boundary.
//!
//! # Examples
//!
//! ```
//! use kurumi_core::{Currency, Date, DayCountConvention, Money, Rate};
//! use rust_decimal::Decimal;
//!
//! # fn main() -> Result<(), kurumi_core::KurumiError> {
//! let notional = Money::new(Decimal::new(1_000_000, 0), Currency::Usd);
//! let rate = Rate::new(0.05)?;
//!
//! let start = Date::from_ymd(2024, 1, 1)?;
//! let end = Date::from_ymd(2024, 7, 1)?;
//! let fraction = DayCountConvention::Act360.year_fraction(start, end);
//!
//! let interest = Money::new(Decimal::new(50_000, 0), notional.currency);
//! assert!(interest.amount > Decimal::ZERO);
//! assert!((fraction - 182.0 / 360.0).abs() < 1e-12);
//! assert!(rate.value() > 0.0);
//! # Ok(())
//! # }
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

pub mod error;
pub mod ring_buffer;
pub mod time;
pub mod types;

pub use error::KurumiError;
pub use ring_buffer::{RingBuffer, RollingVariance, RollingWindow};
pub use time::{
    BusinessDayConvention, Calendar, Date, DayCountConvention, Weekday, WeekendCalendar,
};
pub use types::{Currency, Money, Price, Quantity, Rate, Volatility};
