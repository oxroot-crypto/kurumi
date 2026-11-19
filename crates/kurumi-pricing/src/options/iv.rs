//! Implied-volatility inversion.

use crate::options::black_scholes::evaluate;
use crate::options::contract::{OptionContract, OptionType};
use crate::options::market_data::MarketData;
use kurumi_core::{Date, KurumiError};
use kurumi_math::{RootFinderConfig, brent, newton_raphson};

/// Lowest volatility considered by the inversion.
const MIN_VOLATILITY: f64 = 1e-8;
/// Highest volatility considered by the inversion (500% annualized).
const MAX_VOLATILITY: f64 = 5.0;
/// Tolerance used to recognise a price on the no-arbitrage bound.
const BOUND_TOLERANCE: f64 = 1e-12;

/// Returns the implied volatility that reproduces `target_price`.
///
/// The time to expiry is derived from the contract's expiry and `as_of` using
/// Actual/365 Fixed. The option must be European; the inversion uses the
/// Black-Scholes-Merton model.
///
/// # Method
///
/// The inversion first tries Newton-Raphson with the Black-Scholes vega as the
/// derivative, starting from the market volatility (or 20% when the market
/// volatility is zero). Newton is accepted only when it converges inside the
/// economically meaningful range and the repriced option matches the target.
/// Otherwise the routine falls back to Brent's method on
/// `[1e-8, 5.0]`, which is safeguarded and does not depend on a derivative.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when the contract is American, the
/// target price is outside the no-arbitrage bounds, or the inputs are invalid,
/// and [`KurumiError::ConvergenceFailed`] when no root can be bracketed.
///
/// # Examples
///
/// ```
/// use kurumi_core::Date;
/// use kurumi_pricing::{
///     BlackScholesMertonEngine, ExerciseStyle, MarketData, OptionContract, OptionType,
///     PricingEngine, implied_volatility,
/// };
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let market = MarketData::new(100.0, 0.05, 0.0, 0.20)?;
/// let call = OptionContract::new(
///     OptionType::Call,
///     100.0,
///     Date::from_ymd(2025, 1, 1)?,
///     ExerciseStyle::European,
/// )?;
/// let as_of = Date::from_ymd(2024, 1, 1)?;
/// let price = BlackScholesMertonEngine::new(market)
///     .price(&call, as_of)?
///     .npv;
/// let volatility = implied_volatility(price, &call, &market, as_of)?;
/// assert!((volatility - 0.20).abs() < 1e-6);
/// # Ok(())
/// # }
/// ```
pub fn implied_volatility(
    target_price: f64,
    contract: &OptionContract,
    market: &MarketData,
    as_of: Date,
) -> Result<f64, KurumiError> {
    let time_to_expiry = contract.time_to_expiry(as_of)?;
    implied_volatility_with_time_to_expiry(target_price, contract, market, time_to_expiry)
}

/// Returns the implied volatility for an explicit time to expiry in years.
///
/// This is the model-level entry point used by the analytic engine; see
/// [`implied_volatility`] for the method and errors.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when the contract is American, the
/// target price is outside the no-arbitrage bounds, or the inputs are invalid,
/// and [`KurumiError::ConvergenceFailed`] when no root can be bracketed.
///
/// # Examples
///
/// ```
/// use kurumi_core::Date;
/// use kurumi_pricing::{
///     BlackScholesMertonEngine, ExerciseStyle, MarketData, OptionContract, OptionType,
///     implied_volatility_with_time_to_expiry,
/// };
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let market = MarketData::new(100.0, 0.05, 0.0, 0.20)?;
/// let call = OptionContract::new(
///     OptionType::Call,
///     100.0,
///     Date::from_ymd(2025, 1, 1)?,
///     ExerciseStyle::European,
/// )?;
/// let price = BlackScholesMertonEngine::new(market)
///     .price_with_time_to_expiry(&call, 1.0)?
///     .npv;
/// let volatility =
///     implied_volatility_with_time_to_expiry(price, &call, &market, 1.0)?;
/// assert!((volatility - 0.20).abs() < 1e-9);
/// # Ok(())
/// # }
/// ```
pub fn implied_volatility_with_time_to_expiry(
    target_price: f64,
    contract: &OptionContract,
    market: &MarketData,
    time_to_expiry: f64,
) -> Result<f64, KurumiError> {
    if contract.is_american() {
        return Err(KurumiError::InvalidParameter {
            parameter: "exercise_style",
            reason: "implied volatility requires a European option".to_owned(),
        });
    }
    if !target_price.is_finite() || target_price < 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "target_price",
            reason: "must be finite and non-negative".to_owned(),
        });
    }
    if !time_to_expiry.is_finite() || time_to_expiry <= 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "time_to_expiry",
            reason: "must be finite and strictly positive".to_owned(),
        });
    }

    let spot = market.spot();
    let strike = contract.strike();
    let rate = market.risk_free_rate();
    let dividend = market.dividend_yield();
    let discount_r = (-rate * time_to_expiry).exp();
    let discount_q = (-dividend * time_to_expiry).exp();
    let forward_value = spot * discount_q;
    let strike_value = strike * discount_r;
    if !forward_value.is_finite() || !strike_value.is_finite() {
        return Err(KurumiError::NumericalOverflow {
            operation: "implied volatility no-arbitrage bounds",
        });
    }
    let (lower_bound, upper_bound) = match contract.option_type() {
        OptionType::Call => ((forward_value - strike_value).max(0.0), forward_value),
        OptionType::Put => ((strike_value - forward_value).max(0.0), strike_value),
    };
    if target_price < lower_bound - BOUND_TOLERANCE || target_price > upper_bound + BOUND_TOLERANCE
    {
        return Err(KurumiError::InvalidParameter {
            parameter: "target_price",
            reason: format!(
                "price {target_price} is outside the no-arbitrage bounds [{lower_bound}, {upper_bound}]"
            ),
        });
    }
    if target_price <= lower_bound + BOUND_TOLERANCE {
        return Ok(0.0);
    }
    if target_price >= upper_bound - BOUND_TOLERANCE {
        return Err(KurumiError::InvalidParameter {
            parameter: "target_price",
            reason: "price is too close to the upper no-arbitrage bound; volatility is unbounded"
                .to_owned(),
        });
    }

    let objective = |volatility: f64| -> f64 {
        match evaluate(contract, market, time_to_expiry, volatility) {
            Ok((price, _)) => price - target_price,
            Err(_) => f64::NAN,
        }
    };
    let derivative = |volatility: f64| -> f64 {
        match evaluate(contract, market, time_to_expiry, volatility) {
            Ok((_, greeks)) => greeks.vega,
            Err(_) => f64::NAN,
        }
    };

    let newton_config = RootFinderConfig::new(100, 1e-12)?;
    let guess = if market.volatility() > 0.0 {
        market.volatility()
    } else {
        0.2
    };
    if let Ok(volatility) = newton_raphson(objective, derivative, guess, newton_config) {
        if volatility > 0.0 && volatility < MAX_VOLATILITY {
            if let Ok((price, _)) = evaluate(contract, market, time_to_expiry, volatility) {
                if (price - target_price).abs() <= 1e-9 {
                    return Ok(volatility);
                }
            }
        }
    }

    let low = MIN_VOLATILITY;
    let high = MAX_VOLATILITY;
    let low_value = objective(low);
    let high_value = objective(high);
    if !low_value.is_finite() || !high_value.is_finite() {
        return Err(KurumiError::NumericalOverflow {
            operation: "implied volatility bracket",
        });
    }
    if low_value.abs() <= f64::EPSILON {
        return Ok(low);
    }
    if high_value.abs() <= f64::EPSILON {
        return Ok(high);
    }
    if low_value.is_sign_positive() == high_value.is_sign_positive() {
        return Err(KurumiError::ConvergenceFailed {
            method: "implied_volatility",
            iterations: newton_config.max_iterations,
        });
    }
    let brent_config = RootFinderConfig::new(200, 1e-12)?;
    brent(objective, low, high, brent_config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::contract::ExerciseStyle;
    use crate::traits::PricingEngine;
    use approx::assert_relative_eq;

    fn date(year: i32, month: u32, day: u32) -> Date {
        Date::from_ymd(year, month, day).expect("valid date")
    }

    fn contract(option_type: OptionType) -> OptionContract {
        OptionContract::new(option_type, 50.0, date(2025, 1, 1), ExerciseStyle::European)
            .expect("valid contract")
    }

    #[test]
    fn hull_example_implied_volatility_round_trips_to_the_input() {
        // Hull, Chapter 15: S = 49, K = 50, r = 5%, T = 20/52, sigma = 20%.
        let market = MarketData::new(49.0, 0.05, 0.0, 0.20).expect("market data");
        let call = contract(OptionType::Call);
        let volatility = implied_volatility_with_time_to_expiry(
            2.400_527_323_271_71,
            &call,
            &market,
            20.0 / 52.0,
        )
        .expect("implied volatility");
        assert_relative_eq!(volatility, 0.20, epsilon = 1e-9);

        let put = contract(OptionType::Put);
        let put_volatility = implied_volatility_with_time_to_expiry(
            2.448_175_441_281_85,
            &put,
            &market,
            20.0 / 52.0,
        )
        .expect("implied volatility");
        assert_relative_eq!(put_volatility, 0.20, epsilon = 1e-9);
    }

    #[test]
    fn date_based_inversion_recovers_the_market_volatility() {
        let market = MarketData::new(100.0, 0.05, 0.02, 0.35).expect("market data");
        let option = OptionContract::new(
            OptionType::Put,
            105.0,
            date(2025, 6, 30),
            ExerciseStyle::European,
        )
        .expect("contract");
        let as_of = date(2024, 1, 1);
        let price = crate::options::black_scholes::BlackScholesMertonEngine::new(market)
            .price(&option, as_of)
            .expect("price")
            .npv;
        let implied = implied_volatility(price, &option, &market, as_of).expect("implied");
        assert_relative_eq!(implied, 0.35, epsilon = 1e-6);
    }

    #[test]
    fn low_volatility_options_are_inverted_robustly() {
        let market = MarketData::new(100.0, 0.05, 0.0, 0.02).expect("market data");
        let option = OptionContract::new(
            OptionType::Call,
            100.0,
            date(2025, 1, 1),
            ExerciseStyle::European,
        )
        .expect("contract");
        let price = crate::options::black_scholes::BlackScholesMertonEngine::new(market)
            .price_with_time_to_expiry(&option, 0.25)
            .expect("price")
            .npv;
        let implied =
            implied_volatility_with_time_to_expiry(price, &option, &market, 0.25).expect("implied");
        assert_relative_eq!(implied, 0.02, epsilon = 1e-8);
    }

    #[test]
    fn prices_outside_the_no_arbitrage_bounds_are_rejected() {
        let market = MarketData::new(100.0, 0.05, 0.0, 0.20).expect("market data");
        let call = contract(OptionType::Call);
        // A call cannot be worth more than the discounted spot.
        assert!(implied_volatility_with_time_to_expiry(200.0, &call, &market, 1.0).is_err());
        // A deep in-the-money call cannot trade below its intrinsic floor.
        let in_the_money = OptionContract::new(
            OptionType::Call,
            50.0,
            date(2025, 1, 1),
            ExerciseStyle::European,
        )
        .expect("contract");
        assert!(implied_volatility_with_time_to_expiry(10.0, &in_the_money, &market, 1.0).is_err());
    }

    #[test]
    fn american_contracts_are_rejected() {
        let market = MarketData::new(100.0, 0.05, 0.0, 0.20).expect("market data");
        let american = OptionContract::new(
            OptionType::Put,
            100.0,
            date(2025, 1, 1),
            ExerciseStyle::American,
        )
        .expect("contract");
        assert!(implied_volatility_with_time_to_expiry(5.0, &american, &market, 1.0).is_err());
    }

    #[test]
    fn invalid_targets_are_rejected() {
        let market = MarketData::new(100.0, 0.05, 0.0, 0.20).expect("market data");
        let call = contract(OptionType::Call);
        assert!(implied_volatility_with_time_to_expiry(-1.0, &call, &market, 1.0).is_err());
        assert!(implied_volatility_with_time_to_expiry(f64::NAN, &call, &market, 1.0).is_err());
        assert!(implied_volatility_with_time_to_expiry(5.0, &call, &market, 0.0).is_err());
    }
}
