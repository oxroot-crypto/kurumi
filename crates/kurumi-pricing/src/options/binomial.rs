//! The Cox-Ross-Rubinstein binomial tree engine.

use crate::options::contract::OptionContract;
use crate::options::market_data::MarketData;
use crate::traits::{PricingEngine, ValuationResult};
use kurumi_core::{Date, KurumiError};

/// The maximum number of tree steps accepted by the engine.
pub const MAX_STEPS: usize = 100_000;

/// A Cox-Ross-Rubinstein (CRR) binomial tree engine.
///
/// The tree divides the time to expiry into `steps` intervals of length
/// `dt = T / steps` and uses
///
/// ```text
/// u = exp(sigma sqrt(dt)),  d = 1 / u,
/// p = (exp((r - q) dt) - d) / (u - d),
/// ```
///
/// with per-step discount factor `exp(-r dt)`. Backward induction replaces each
/// node's continuation value with the maximum of the continuation value and the
/// immediate exercise value when the contract is American, which captures early
/// exercise. European contracts skip the exercise comparison.
///
/// The engine uses `O(steps)` memory and `O(steps^2)` time. Prices converge to
/// the Black-Scholes-Merton value as `steps` grows, with the usual CRR
/// oscillation.
///
/// # Examples
///
/// ```
/// use kurumi_core::Date;
/// use kurumi_pricing::{
///     CrrBinomialEngine, ExerciseStyle, MarketData, OptionContract, OptionType, PricingEngine,
/// };
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let market = MarketData::new(50.0, 0.10, 0.0, 0.40)?;
/// let engine = CrrBinomialEngine::new(market, 100)?;
/// let put = OptionContract::new(
///     OptionType::Put,
///     50.0,
///     Date::from_ymd(2025, 1, 1)?,
///     ExerciseStyle::American,
/// )?;
/// let value = engine.price(&put, Date::from_ymd(2024, 8, 3)?)?;
/// assert!(value.greeks.is_none());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrrBinomialEngine {
    market: MarketData,
    steps: usize,
}

impl CrrBinomialEngine {
    /// Creates a CRR engine with the given number of time steps.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `steps` is zero or exceeds
    /// [`MAX_STEPS`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::{CrrBinomialEngine, MAX_STEPS, MarketData};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let market = MarketData::new(100.0, 0.05, 0.0, 0.2)?;
    /// assert!(CrrBinomialEngine::new(market, 500).is_ok());
    /// assert!(CrrBinomialEngine::new(market, 0).is_err());
    /// assert!(CrrBinomialEngine::new(market, MAX_STEPS + 1).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(market: MarketData, steps: usize) -> Result<Self, KurumiError> {
        if steps == 0 || steps > MAX_STEPS {
            return Err(KurumiError::InvalidParameter {
                parameter: "steps",
                reason: format!("must be in 1..={MAX_STEPS}"),
            });
        }
        Ok(Self { market, steps })
    }

    /// Returns the market snapshot used by the engine.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::{CrrBinomialEngine, MarketData};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let market = MarketData::new(100.0, 0.05, 0.0, 0.2)?;
    /// let engine = CrrBinomialEngine::new(market, 100)?;
    /// assert!((engine.market().spot() - 100.0).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn market(&self) -> &MarketData {
        &self.market
    }

    /// Returns the number of time steps.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::{CrrBinomialEngine, MarketData};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let engine = CrrBinomialEngine::new(MarketData::new(100.0, 0.05, 0.0, 0.2)?, 100)?;
    /// assert_eq!(engine.steps(), 100);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn steps(&self) -> usize {
        self.steps
    }

    /// Prices an option using an explicit time to expiry in years.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `time_to_expiry` is not
    /// finite and positive, volatility is not strictly positive, or the
    /// risk-neutral probability falls outside `[0, 1]`. Returns
    /// [`KurumiError::NumericalOverflow`] when a tree value is not
    /// representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::{
    ///     CrrBinomialEngine, ExerciseStyle, MarketData, OptionContract, OptionType,
    /// };
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let engine = CrrBinomialEngine::new(MarketData::new(50.0, 0.10, 0.0, 0.40)?, 5)?;
    /// let put = OptionContract::new(
    ///     OptionType::Put,
    ///     50.0,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     ExerciseStyle::American,
    /// )?;
    /// let value = engine.price_with_time_to_expiry(&put, 5.0 / 12.0)?;
    /// assert!((value.npv - 4.488_458_534_725_916).abs() < 1e-9);
    /// # Ok(())
    /// # }
    /// ```
    pub fn price_with_time_to_expiry(
        &self,
        contract: &OptionContract,
        time_to_expiry: f64,
    ) -> Result<ValuationResult, KurumiError> {
        if !time_to_expiry.is_finite() || time_to_expiry <= 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "time_to_expiry",
                reason: "must be finite and strictly positive".to_owned(),
            });
        }
        let volatility = self.market.volatility();
        if !volatility.is_finite() || volatility <= 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "volatility",
                reason: "the CRR tree requires strictly positive volatility".to_owned(),
            });
        }

        let steps = self.steps;
        let dt = time_to_expiry / steps as f64;
        let sqrt_dt = dt.sqrt();
        let up = (volatility * sqrt_dt).exp();
        let down = 1.0 / up;
        let growth = ((self.market.risk_free_rate() - self.market.dividend_yield()) * dt).exp();
        let denominator = up - down;
        if !denominator.is_finite() || denominator <= 0.0 {
            return Err(KurumiError::NumericalOverflow {
                operation: "CRR tree parameters",
            });
        }
        let probability = (growth - down) / denominator;
        if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
            return Err(KurumiError::InvalidParameter {
                parameter: "risk_neutral_probability",
                reason: "fell outside [0, 1]; reduce the step size or check the inputs".to_owned(),
            });
        }
        let discount = (-self.market.risk_free_rate() * dt).exp();
        if !discount.is_finite() {
            return Err(KurumiError::NumericalOverflow {
                operation: "CRR tree discount factor",
            });
        }

        let spot = self.market.spot();
        let ratio = up / down;
        let mut values = Vec::with_capacity(steps + 1);
        let mut terminal_spot = spot
            * down.powi(
                i32::try_from(steps).map_err(|_| KurumiError::InvalidParameter {
                    parameter: "steps",
                    reason: "must fit in an i32".to_owned(),
                })?,
            );
        for _ in 0..=steps {
            values.push(contract.payoff(terminal_spot)?);
            terminal_spot *= ratio;
        }

        for level in (0..steps).rev() {
            let mut node_spot = spot
                * down.powi(
                    i32::try_from(level).map_err(|_| KurumiError::InvalidParameter {
                        parameter: "steps",
                        reason: "must fit in an i32".to_owned(),
                    })?,
                );
            for node in 0..=level {
                let lower = values
                    .get(node)
                    .copied()
                    .ok_or(KurumiError::NumericalOverflow {
                        operation: "CRR backward induction",
                    })?;
                let upper =
                    values
                        .get(node + 1)
                        .copied()
                        .ok_or(KurumiError::NumericalOverflow {
                            operation: "CRR backward induction",
                        })?;
                let continuation = discount * (probability * upper + (1.0 - probability) * lower);
                let value = if contract.is_american() {
                    continuation.max(contract.payoff(node_spot)?)
                } else {
                    continuation
                };
                if let Some(slot) = values.get_mut(node) {
                    *slot = value;
                } else {
                    return Err(KurumiError::NumericalOverflow {
                        operation: "CRR backward induction",
                    });
                }
                node_spot *= ratio;
            }
        }

        let npv = values
            .first()
            .copied()
            .ok_or(KurumiError::NumericalOverflow {
                operation: "CRR tree result",
            })?;
        ValuationResult::new(npv)
    }
}

impl PricingEngine<OptionContract> for CrrBinomialEngine {
    fn price(
        &self,
        instrument: &OptionContract,
        as_of: Date,
    ) -> Result<ValuationResult, KurumiError> {
        let time_to_expiry = instrument.time_to_expiry(as_of)?;
        self.price_with_time_to_expiry(instrument, time_to_expiry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::black_scholes::BlackScholesMertonEngine;
    use crate::options::contract::{ExerciseStyle, OptionType};
    use approx::assert_relative_eq;

    fn date(year: i32, month: u32, day: u32) -> Date {
        Date::from_ymd(year, month, day).expect("valid date")
    }

    fn contract(option_type: OptionType, exercise: ExerciseStyle) -> OptionContract {
        OptionContract::new(option_type, 50.0, date(2025, 1, 1), exercise).expect("valid contract")
    }

    #[test]
    fn hull_american_put_tree_values_match_the_reference() {
        // Hull, Chapter 21: S = 50, K = 50, r = 10%, q = 0, T = 5/12,
        // sigma = 40%. Hull reports 4.49 for a 5-step tree.
        let market = MarketData::new(50.0, 0.10, 0.0, 0.40).expect("market data");
        let put = contract(OptionType::Put, ExerciseStyle::American);

        let five = CrrBinomialEngine::new(market, 5)
            .expect("engine")
            .price_with_time_to_expiry(&put, 5.0 / 12.0)
            .expect("5-step price");
        assert_relative_eq!(five.npv, 4.488_458_534_725_916, epsilon = 1e-9);

        let five_hundred = CrrBinomialEngine::new(market, 500)
            .expect("engine")
            .price_with_time_to_expiry(&put, 5.0 / 12.0)
            .expect("500-step price");
        assert_relative_eq!(five_hundred.npv, 4.283_021_276_450_517, epsilon = 1e-9);
    }

    #[test]
    fn american_put_exceeds_the_european_put() {
        let market = MarketData::new(50.0, 0.10, 0.0, 0.40).expect("market data");
        let american = CrrBinomialEngine::new(market, 500)
            .expect("engine")
            .price_with_time_to_expiry(
                &contract(OptionType::Put, ExerciseStyle::American),
                5.0 / 12.0,
            )
            .expect("american");
        let european = CrrBinomialEngine::new(market, 500)
            .expect("engine")
            .price_with_time_to_expiry(
                &contract(OptionType::Put, ExerciseStyle::European),
                5.0 / 12.0,
            )
            .expect("european");
        assert!(american.npv > european.npv);
        assert_relative_eq!(european.npv, 4.073_434_834_660_33, epsilon = 1e-9);
    }

    #[test]
    fn american_call_without_dividends_equals_the_european_call() {
        let market = MarketData::new(100.0, 0.05, 0.0, 0.20).expect("market data");
        let call = OptionContract::new(
            OptionType::Call,
            100.0,
            date(2025, 1, 1),
            ExerciseStyle::American,
        )
        .expect("contract");
        let european = OptionContract::new(
            OptionType::Call,
            100.0,
            date(2025, 1, 1),
            ExerciseStyle::European,
        )
        .expect("contract");
        let engine = CrrBinomialEngine::new(market, 500).expect("engine");
        let american_value = engine
            .price_with_time_to_expiry(&call, 1.0)
            .expect("american call");
        let european_value = engine
            .price_with_time_to_expiry(&european, 1.0)
            .expect("european call");
        assert_relative_eq!(american_value.npv, european_value.npv, epsilon = 1e-12);
        // Hull's Black-Scholes value for this case is 10.4506.
        assert!((american_value.npv - 10.450_583_572_185_6).abs() < 5e-3);
    }

    #[test]
    fn european_tree_price_converges_to_black_scholes() {
        let market = MarketData::new(100.0, 0.05, 0.0, 0.20).expect("market data");
        let contract = OptionContract::new(
            OptionType::Call,
            100.0,
            date(2025, 1, 1),
            ExerciseStyle::European,
        )
        .expect("contract");
        let tree = CrrBinomialEngine::new(market, 1_000).expect("engine");
        let analytic = BlackScholesMertonEngine::new(market);
        let tree_value = tree
            .price_with_time_to_expiry(&contract, 1.0)
            .expect("tree price");
        let analytic_value = analytic
            .price_with_time_to_expiry(&contract, 1.0)
            .expect("analytic price");
        assert!((tree_value.npv - analytic_value.npv).abs() < 1e-2);
    }

    #[test]
    fn tree_rejects_invalid_parameters() {
        let market = MarketData::new(100.0, 0.05, 0.0, 0.20).expect("market data");
        assert!(CrrBinomialEngine::new(market, 0).is_err());
        assert!(CrrBinomialEngine::new(market, MAX_STEPS + 1).is_err());

        let zero_volatility = MarketData::new(100.0, 0.05, 0.0, 0.0).expect("market data");
        let engine = CrrBinomialEngine::new(zero_volatility, 100).expect("engine");
        assert!(
            engine
                .price_with_time_to_expiry(
                    &contract(OptionType::Call, ExerciseStyle::European),
                    1.0
                )
                .is_err()
        );

        let engine = CrrBinomialEngine::new(market, 100).expect("engine");
        assert!(
            engine
                .price_with_time_to_expiry(
                    &contract(OptionType::Call, ExerciseStyle::European),
                    0.0
                )
                .is_err()
        );
    }

    #[test]
    fn date_based_pricing_uses_actual_over_365() {
        let market = MarketData::new(100.0, 0.05, 0.0, 0.20).expect("market data");
        let contract = OptionContract::new(
            OptionType::Put,
            100.0,
            date(2025, 1, 1),
            ExerciseStyle::American,
        )
        .expect("contract");
        let as_of = date(2024, 1, 1);
        let engine = CrrBinomialEngine::new(market, 200).expect("engine");
        let by_date = engine.price(&contract, as_of).expect("date price");
        let time = contract.time_to_expiry(as_of).expect("time");
        let by_time = engine
            .price_with_time_to_expiry(&contract, time)
            .expect("time price");
        assert_relative_eq!(by_date.npv, by_time.npv, epsilon = 1e-15);
    }
}
