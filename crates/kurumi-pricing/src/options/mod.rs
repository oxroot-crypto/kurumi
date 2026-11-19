//! Option pricing: contracts, market data, analytic and lattice engines, and
//! implied-volatility inversion.
//!
//! The module is organized around the generic [`PricingEngine`](crate::PricingEngine)
//! abstraction:
//!
//! - [`BlackScholesMertonEngine`] values European options analytically and
//!   returns the full first-order and second-order Greeks;
//! - [`CrrBinomialEngine`] values European and American options on a
//!   Cox-Ross-Rubinstein tree, so American early exercise is captured;
//! - [`implied_volatility`] inverts a European price back to volatility with
//!   Newton-Raphson and a Brent fallback.

mod binomial;
mod black_scholes;
mod contract;
mod iv;
mod market_data;

pub use binomial::{CrrBinomialEngine, MAX_STEPS};
pub use black_scholes::BlackScholesMertonEngine;
pub use contract::{ExerciseStyle, OptionContract, OptionType};
pub use iv::{implied_volatility, implied_volatility_with_time_to_expiry};
pub use market_data::MarketData;
