//! The Black-Scholes-Merton analytic engine for European options.

use crate::options::contract::{OptionContract, OptionType};
use crate::options::market_data::MarketData;
use crate::traits::{Greeks, PricingEngine, ValuationResult};
use kurumi_core::{Date, KurumiError};
use kurumi_math::{standard_normal_cdf, standard_normal_pdf};

/// The Black-Scholes-Merton analytic engine.
///
/// The engine owns a [`MarketData`] snapshot. Prices and Greeks follow the
/// standard European formulas with a continuous dividend yield `q`:
///
/// ```text
/// d1 = (ln(S / K) + (r - q + sigma^2 / 2) T) / (sigma sqrt(T))
/// d2 = d1 - sigma sqrt(T)
/// C  = S e^(-qT) N(d1) - K e^(-rT) N(d2)
/// P  = K e^(-rT) N(-d2) - S e^(-qT) N(-d1)
/// ```
///
/// where `N` is the standard normal cumulative distribution and `T` is the time
/// to expiry in years. Greeks are reported per unit of the respective input:
/// vega is the change per unit of volatility (a move from 20% to 21% changes the
/// value by one hundredth of the reported vega) and rho is the change per unit
/// of rate.
///
/// # Examples
///
/// ```
/// use kurumi_core::Date;
/// use kurumi_pricing::{
///     BlackScholesMertonEngine, ExerciseStyle, MarketData, OptionContract, OptionType,
///     PricingEngine,
/// };
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let market = MarketData::new(100.0, 0.05, 0.0, 0.20)?;
/// let engine = BlackScholesMertonEngine::new(market);
/// let call = OptionContract::new(
///     OptionType::Call,
///     100.0,
///     Date::from_ymd(2025, 1, 1)?,
///     ExerciseStyle::European,
/// )?;
/// let value = engine.price(&call, Date::from_ymd(2024, 1, 1)?)?;
/// assert!(value.greeks.is_some());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlackScholesMertonEngine {
    market: MarketData,
}

impl BlackScholesMertonEngine {
    /// Creates an engine from a market snapshot.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::{BlackScholesMertonEngine, MarketData};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let market = MarketData::new(100.0, 0.05, 0.0, 0.2)?;
    /// let engine = BlackScholesMertonEngine::new(market);
    /// assert!((engine.market().spot() - 100.0).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn new(market: MarketData) -> Self {
        Self { market }
    }

    /// Returns the market snapshot used by the engine.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::{BlackScholesMertonEngine, MarketData};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let market = MarketData::new(100.0, 0.05, 0.0, 0.2)?;
    /// let engine = BlackScholesMertonEngine::new(market);
    /// assert!((engine.market().volatility() - 0.2).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn market(&self) -> &MarketData {
        &self.market
    }

    /// Prices a European option using an explicit time to expiry in years.
    ///
    /// This bypasses the calendar-date conversion and is useful when the caller
    /// already has a year fraction from another convention. The result includes
    /// all five Greeks.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `time_to_expiry` is not
    /// finite and positive, when the contract is American, or when a Greek is
    /// not finite. Returns [`KurumiError::NumericalOverflow`] when an
    /// intermediate value is not representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::{
    ///     BlackScholesMertonEngine, ExerciseStyle, MarketData, OptionContract, OptionType,
    /// };
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let engine = BlackScholesMertonEngine::new(MarketData::new(100.0, 0.05, 0.0, 0.20)?);
    /// let call = OptionContract::new(
    ///     OptionType::Call,
    ///     100.0,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     ExerciseStyle::European,
    /// )?;
    /// let value = engine.price_with_time_to_expiry(&call, 1.0)?;
    /// assert!((value.npv - 10.450_583_572_185_6).abs() < 1e-9);
    /// # Ok(())
    /// # }
    /// ```
    pub fn price_with_time_to_expiry(
        &self,
        contract: &OptionContract,
        time_to_expiry: f64,
    ) -> Result<ValuationResult, KurumiError> {
        if contract.is_american() {
            return Err(KurumiError::InvalidParameter {
                parameter: "exercise_style",
                reason: "the Black-Scholes-Merton engine prices European options only".to_owned(),
            });
        }
        let (npv, greeks) = evaluate(
            contract,
            &self.market,
            time_to_expiry,
            self.market.volatility(),
        )?;
        ValuationResult::with_greeks(npv, greeks)
    }

    /// Returns the implied volatility that reproduces `target_price` as of
    /// `as_of`.
    ///
    /// The time to expiry is derived from the contract date with Actual/365
    /// Fixed. See [`crate::options::implied_volatility`] for the numerical
    /// method.
    ///
    /// # Errors
    ///
    /// Propagates the errors of [`crate::options::implied_volatility`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::{
    ///     BlackScholesMertonEngine, ExerciseStyle, MarketData, OptionContract, OptionType,
    ///     PricingEngine,
    /// };
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let market = MarketData::new(100.0, 0.05, 0.0, 0.20)?;
    /// let engine = BlackScholesMertonEngine::new(market);
    /// let call = OptionContract::new(
    ///     OptionType::Call,
    ///     100.0,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     ExerciseStyle::European,
    /// )?;
    /// let price = engine.price(&call, Date::from_ymd(2024, 1, 1)?)?.npv;
    /// let volatility = engine.implied_volatility(&call, price, Date::from_ymd(2024, 1, 1)?)?;
    /// assert!((volatility - 0.20).abs() < 1e-6);
    /// # Ok(())
    /// # }
    /// ```
    pub fn implied_volatility(
        &self,
        contract: &OptionContract,
        target_price: f64,
        as_of: Date,
    ) -> Result<f64, KurumiError> {
        crate::options::iv::implied_volatility(target_price, contract, &self.market, as_of)
    }

    /// Returns the implied volatility for an explicit time to expiry in years.
    ///
    /// # Errors
    ///
    /// Propagates the errors of
    /// [`crate::options::implied_volatility_with_time_to_expiry`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::{
    ///     BlackScholesMertonEngine, ExerciseStyle, MarketData, OptionContract, OptionType,
    /// };
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let engine = BlackScholesMertonEngine::new(MarketData::new(100.0, 0.05, 0.0, 0.20)?);
    /// let call = OptionContract::new(
    ///     OptionType::Call,
    ///     100.0,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     ExerciseStyle::European,
    /// )?;
    /// let price = engine.price_with_time_to_expiry(&call, 1.0)?.npv;
    /// let volatility = engine.implied_volatility_with_time_to_expiry(&call, price, 1.0)?;
    /// assert!((volatility - 0.20).abs() < 1e-9);
    /// # Ok(())
    /// # }
    /// ```
    pub fn implied_volatility_with_time_to_expiry(
        &self,
        contract: &OptionContract,
        target_price: f64,
        time_to_expiry: f64,
    ) -> Result<f64, KurumiError> {
        crate::options::iv::implied_volatility_with_time_to_expiry(
            target_price,
            contract,
            &self.market,
            time_to_expiry,
        )
    }
}

impl PricingEngine<OptionContract> for BlackScholesMertonEngine {
    fn price(
        &self,
        instrument: &OptionContract,
        as_of: Date,
    ) -> Result<ValuationResult, KurumiError> {
        let time_to_expiry = instrument.time_to_expiry(as_of)?;
        self.price_with_time_to_expiry(instrument, time_to_expiry)
    }
}

pub(crate) fn evaluate(
    contract: &OptionContract,
    market: &MarketData,
    time_to_expiry: f64,
    volatility: f64,
) -> Result<(f64, Greeks), KurumiError> {
    if !time_to_expiry.is_finite() || time_to_expiry <= 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "time_to_expiry",
            reason: "must be finite and strictly positive".to_owned(),
        });
    }
    if !volatility.is_finite() || volatility < 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "volatility",
            reason: "must be finite and non-negative".to_owned(),
        });
    }

    let spot = market.spot();
    let strike = contract.strike();
    let rate = market.risk_free_rate();
    let dividend = market.dividend_yield();
    let discount_r = (-rate * time_to_expiry).exp();
    let discount_q = (-dividend * time_to_expiry).exp();
    if !discount_r.is_finite() || !discount_q.is_finite() {
        return Err(KurumiError::NumericalOverflow {
            operation: "Black-Scholes-Merton discount factor",
        });
    }

    if volatility <= 0.0 {
        return deterministic(contract, market, time_to_expiry);
    }

    let sqrt_time = time_to_expiry.sqrt();
    let sigma_sqrt_time = volatility * sqrt_time;
    let d1 = ((spot / strike).ln()
        + (rate - dividend + 0.5 * volatility * volatility) * time_to_expiry)
        / sigma_sqrt_time;
    let d2 = d1 - sigma_sqrt_time;
    if !d1.is_finite() || !d2.is_finite() {
        return Err(KurumiError::NumericalOverflow {
            operation: "Black-Scholes-Merton d1/d2",
        });
    }

    let n_d1 = standard_normal_cdf(d1)?;
    let n_d2 = standard_normal_cdf(d2)?;
    let phi_d1 = standard_normal_pdf(d1)?;

    let (npv, delta, theta, rho) = match contract.option_type() {
        OptionType::Call => {
            let npv = spot * discount_q * n_d1 - strike * discount_r * n_d2;
            let delta = discount_q * n_d1;
            let theta = -spot * discount_q * phi_d1 * volatility / (2.0 * sqrt_time)
                - rate * strike * discount_r * n_d2
                + dividend * spot * discount_q * n_d1;
            let rho = strike * time_to_expiry * discount_r * n_d2;
            (npv, delta, theta, rho)
        }
        OptionType::Put => {
            let npv = strike * discount_r * standard_normal_cdf(-d2)?
                - spot * discount_q * standard_normal_cdf(-d1)?;
            let delta = -discount_q * standard_normal_cdf(-d1)?;
            let theta = -spot * discount_q * phi_d1 * volatility / (2.0 * sqrt_time)
                + rate * strike * discount_r * standard_normal_cdf(-d2)?
                - dividend * spot * discount_q * standard_normal_cdf(-d1)?;
            let rho = -strike * time_to_expiry * discount_r * standard_normal_cdf(-d2)?;
            (npv, delta, theta, rho)
        }
    };

    let gamma = discount_q * phi_d1 / (spot * sigma_sqrt_time);
    let vega = spot * discount_q * phi_d1 * sqrt_time;
    let greeks = Greeks::new(delta, gamma, vega, theta, rho)?;
    if npv.is_finite() {
        Ok((npv, greeks))
    } else {
        Err(KurumiError::NumericalOverflow {
            operation: "Black-Scholes-Merton price",
        })
    }
}

fn deterministic(
    contract: &OptionContract,
    market: &MarketData,
    time_to_expiry: f64,
) -> Result<(f64, Greeks), KurumiError> {
    let spot = market.spot();
    let strike = contract.strike();
    let rate = market.risk_free_rate();
    let dividend = market.dividend_yield();
    let discount_r = (-rate * time_to_expiry).exp();
    let discount_q = (-dividend * time_to_expiry).exp();
    let forward_value = spot * discount_q;
    let strike_value = strike * discount_r;
    let (npv, delta, theta, rho) = match contract.option_type() {
        OptionType::Call => {
            if forward_value > strike_value {
                (
                    forward_value - strike_value,
                    discount_q,
                    dividend * forward_value - rate * strike_value,
                    strike * time_to_expiry * discount_r,
                )
            } else {
                (0.0, 0.0, 0.0, 0.0)
            }
        }
        OptionType::Put => {
            if strike_value > forward_value {
                (
                    strike_value - forward_value,
                    -discount_q,
                    rate * strike_value - dividend * forward_value,
                    -strike * time_to_expiry * discount_r,
                )
            } else {
                (0.0, 0.0, 0.0, 0.0)
            }
        }
    };
    let greeks = Greeks::new(delta, 0.0, 0.0, theta, rho)?;
    Ok((npv, greeks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn date(year: i32, month: u32, day: u32) -> Date {
        Date::from_ymd(year, month, day).expect("valid date")
    }

    fn contract(option_type: OptionType, strike: f64) -> OptionContract {
        OptionContract::new(
            option_type,
            strike,
            date(2025, 1, 1),
            crate::options::ExerciseStyle::European,
        )
        .expect("valid contract")
    }

    fn engine(spot: f64, rate: f64, dividend: f64, volatility: f64) -> BlackScholesMertonEngine {
        BlackScholesMertonEngine::new(
            MarketData::new(spot, rate, dividend, volatility).expect("valid market data"),
        )
    }

    #[test]
    fn hull_call_and_put_prices_match_the_textbook() {
        // Hull, Options, Futures, and Other Derivatives, Chapter 15:
        // S = 42, K = 40, r = 10%, q = 0, T = 0.5, sigma = 20%.
        let engine = engine(42.0, 0.10, 0.0, 0.20);
        let call = engine
            .price_with_time_to_expiry(&contract(OptionType::Call, 40.0), 0.5)
            .expect("call price");
        let put = engine
            .price_with_time_to_expiry(&contract(OptionType::Put, 40.0), 0.5)
            .expect("put price");
        assert_relative_eq!(call.npv, 4.759_422_392_871_54, epsilon = 1e-9);
        assert_relative_eq!(put.npv, 0.808_599_372_900_096, epsilon = 1e-9);
    }

    #[test]
    fn hull_greeks_match_the_textbook() {
        // Hull, Chapter 15, Table 15.3: S = 49, K = 50, r = 5%, T = 20/52,
        // sigma = 20%, q = 0.
        let engine = engine(49.0, 0.05, 0.0, 0.20);
        let time = 20.0 / 52.0;
        let call = engine
            .price_with_time_to_expiry(&contract(OptionType::Call, 50.0), time)
            .expect("call price");
        let greeks = call.greeks.expect("call greeks");
        assert_relative_eq!(call.npv, 2.400_527_323_271_71, epsilon = 1e-9);
        assert_relative_eq!(greeks.delta, 0.521_604_661_066_396, epsilon = 1e-9);
        assert_relative_eq!(greeks.gamma, 0.065_544_039_347_844_4, epsilon = 1e-9);
        assert_relative_eq!(greeks.vega, 12.105_479_882_628_8, epsilon = 1e-9);
        assert_relative_eq!(greeks.theta, -4.305_329_822_932_57, epsilon = 1e-9);
        assert_relative_eq!(greeks.rho, 8.906_961_949_608_35, epsilon = 1e-9);

        let put = engine
            .price_with_time_to_expiry(&contract(OptionType::Put, 50.0), time)
            .expect("put price");
        let put_greeks = put.greeks.expect("put greeks");
        assert_relative_eq!(put.npv, 2.448_175_441_281_85, epsilon = 1e-9);
        assert_relative_eq!(put_greeks.delta, -0.478_395_338_933_604, epsilon = 1e-9);
        assert_relative_eq!(put_greeks.theta, -1.852_947_417_032_07, epsilon = 1e-9);
        assert_relative_eq!(put_greeks.rho, -9.957_518_095_780_16, epsilon = 1e-9);
    }

    #[test]
    fn at_the_money_black_scholes_matches_the_reference_example() {
        // S = K = 100, r = 5%, q = 0, T = 1, sigma = 20%.
        let engine = engine(100.0, 0.05, 0.0, 0.20);
        let call = engine
            .price_with_time_to_expiry(&contract(OptionType::Call, 100.0), 1.0)
            .expect("call price");
        let put = engine
            .price_with_time_to_expiry(&contract(OptionType::Put, 100.0), 1.0)
            .expect("put price");
        assert_relative_eq!(call.npv, 10.450_583_572_185_6, epsilon = 1e-9);
        assert_relative_eq!(put.npv, 5.573_526_022_256_97, epsilon = 1e-9);

        let greeks = call.greeks.expect("call greeks");
        assert_relative_eq!(greeks.delta, 0.636_830_651_175_619, epsilon = 1e-9);
        assert_relative_eq!(greeks.gamma, 0.018_762_017_345_846_9, epsilon = 1e-9);
        assert_relative_eq!(greeks.vega, 37.524_034_691_693_8, epsilon = 1e-9);
        assert_relative_eq!(greeks.theta, -6.414_027_546_438_2, epsilon = 1e-9);
        assert_relative_eq!(greeks.rho, 53.232_481_545_376_3, epsilon = 1e-9);
    }

    #[test]
    fn dividend_yield_lowers_call_values_and_raises_put_values() {
        let without_dividend = engine(100.0, 0.05, 0.0, 0.25);
        let with_dividend = engine(100.0, 0.05, 0.03, 0.25);
        let call_without = without_dividend
            .price_with_time_to_expiry(&contract(OptionType::Call, 100.0), 0.5)
            .expect("call");
        let call_with = with_dividend
            .price_with_time_to_expiry(&contract(OptionType::Call, 100.0), 0.5)
            .expect("call");
        let put_without = without_dividend
            .price_with_time_to_expiry(&contract(OptionType::Put, 100.0), 0.5)
            .expect("put");
        let put_with = with_dividend
            .price_with_time_to_expiry(&contract(OptionType::Put, 100.0), 0.5)
            .expect("put");
        assert!(call_with.npv < call_without.npv);
        assert!(put_with.npv > put_without.npv);
        // Reference values from the same formulas with q = 3%.
        assert_relative_eq!(call_with.npv, 7.404_935_111_103_54, epsilon = 1e-9);
        assert_relative_eq!(put_with.npv, 6.424_732_353_630_55, epsilon = 1e-9);
    }

    #[test]
    fn zero_volatility_reduces_to_the_discounted_forward_payoff() {
        let engine = engine(100.0, 0.05, 0.0, 0.0);
        let call = engine
            .price_with_time_to_expiry(&contract(OptionType::Call, 100.0), 1.0)
            .expect("deterministic call");
        let expected = 100.0 - 100.0 * (-0.05_f64).exp();
        assert_relative_eq!(call.npv, expected, epsilon = 1e-12);
        let greeks = call.greeks.expect("greeks");
        assert_relative_eq!(greeks.delta, 1.0, epsilon = 1e-15);
        assert_relative_eq!(greeks.gamma, 0.0, epsilon = 1e-15);
        assert_relative_eq!(greeks.vega, 0.0, epsilon = 1e-15);
        assert_relative_eq!(
            greeks.theta,
            -0.05 * 100.0 * (-0.05_f64).exp(),
            epsilon = 1e-12
        );
        assert_relative_eq!(greeks.rho, 100.0 * (-0.05_f64).exp(), epsilon = 1e-12);
    }

    #[test]
    fn date_based_pricing_uses_actual_over_365() {
        let engine = engine(100.0, 0.05, 0.0, 0.20);
        let as_of = date(2024, 1, 1);
        let option = contract(OptionType::Call, 100.0);
        let time = option.time_to_expiry(as_of).expect("time");
        let by_date = engine.price(&option, as_of).expect("date price");
        let by_time = engine
            .price_with_time_to_expiry(&option, time)
            .expect("time price");
        assert_relative_eq!(by_date.npv, by_time.npv, epsilon = 1e-15);
    }

    #[test]
    fn american_contracts_are_rejected_by_the_analytic_engine() {
        let engine = engine(100.0, 0.05, 0.0, 0.20);
        let american = OptionContract::new(
            OptionType::Put,
            100.0,
            date(2025, 1, 1),
            crate::options::ExerciseStyle::American,
        )
        .expect("valid contract");
        assert!(engine.price_with_time_to_expiry(&american, 1.0).is_err());
    }

    #[test]
    fn evaluate_rejects_invalid_time_and_volatility() {
        let market = MarketData::new(100.0, 0.05, 0.0, 0.2).expect("market");
        let option = contract(OptionType::Call, 100.0);
        assert!(evaluate(&option, &market, 0.0, 0.2).is_err());
        assert!(evaluate(&option, &market, f64::NAN, 0.2).is_err());
        assert!(evaluate(&option, &market, 1.0, -0.1).is_err());
        assert!(evaluate(&option, &market, 1.0, f64::INFINITY).is_err());
    }
}
