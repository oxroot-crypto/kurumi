//! Asset and derivative pricing.
//!
//! This crate models cashflows, discount factors, yield curves, fixed-income
//! instruments, and option pricing engines. Dates and day-count conventions come
//! from `kurumi-core`; numerical routines come from `kurumi-math`.
//!
//! The crate is organized as follows:
//!
//! - [`traits`] defines [`Instrument`], [`PricingEngine`], [`ValuationResult`],
//!   and the [`Greeks`] sensitivity set;
//! - [`yield_curve`] provides [`Compounding`] and [`ZeroCurve`];
//! - [`fixed_income`] provides cashflow discounting, NPV/IRR, and
//!   [`FixedRateBond`] analytics;
//! - [`options`] provides [`OptionContract`], [`MarketData`],
//!   [`BlackScholesMertonEngine`], [`CrrBinomialEngine`], and
//!   [`implied_volatility`].
//!
//! Discrete money amounts use `Decimal`; continuous model values use `f64`.
//! Invalid model inputs (non-positive volatility, out-of-range probabilities,
//! `NaN`/infinite values) are rejected with typed errors at the public boundary.
//!
//! # Examples
//!
//! ```
//! use kurumi_core::{Date, DayCountConvention};
//! use kurumi_pricing::{
//!     BlackScholesMertonEngine, Compounding, ExerciseStyle, MarketData, OptionContract,
//!     OptionType, PricingEngine, ZeroCurve,
//! };
//!
//! # fn main() -> Result<(), kurumi_pricing::KurumiError> {
//! let reference = Date::from_ymd(2024, 1, 1)?;
//! let curve = ZeroCurve::flat(
//!     reference,
//!     0.05,
//!     Compounding::Continuous,
//!     DayCountConvention::Act365Fixed,
//! )?;
//! assert!(curve.discount_factor(Date::from_ymd(2025, 1, 1)?)? < 1.0);
//!
//! let market = MarketData::new(100.0, 0.05, 0.0, 0.20)?;
//! let call = OptionContract::new(
//!     OptionType::Call,
//!     100.0,
//!     Date::from_ymd(2025, 1, 1)?,
//!     ExerciseStyle::European,
//! )?;
//! let value = BlackScholesMertonEngine::new(market).price(&call, reference)?;
//! assert!(value.npv > 0.0);
//! assert!(value.greeks.is_some());
//! # Ok(())
//! # }
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

pub mod fixed_income;
pub mod options;
pub mod traits;
pub mod yield_curve;

pub use fixed_income::{
    CashFlow, DiscountedCashFlowEngine, FixedRateBond, discounted_cash_flow,
    internal_rate_of_return, net_present_value, yield_to_maturity,
};
pub use options::{
    BlackScholesMertonEngine, CrrBinomialEngine, ExerciseStyle, MAX_STEPS, MarketData,
    OptionContract, OptionType, implied_volatility, implied_volatility_with_time_to_expiry,
};
pub use traits::{Greeks, Instrument, PricingEngine, ValuationResult};
pub use yield_curve::{Compounding, ZeroCurve};

pub use kurumi_core::KurumiError;
