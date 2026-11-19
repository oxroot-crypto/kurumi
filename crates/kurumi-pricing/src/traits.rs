//! Instrument metadata and the generic pricing-engine abstraction.
//!
//! Every priced instrument implements [`Instrument`], which exposes the metadata
//! an engine needs before it looks at the instrument's cashflows: the maturity
//! date and whether the instrument has expired at a given valuation date.
//!
//! A [`PricingEngine`] turns an instrument into a [`ValuationResult`]. The result
//! carries the net present value and, when the engine computes them, the
//! first-order and second-order [`Greeks`]. Engines own their market data, so
//! [`PricingEngine::price`] only needs the instrument and the valuation date.

use kurumi_core::{Date, KurumiError};

/// Metadata shared by every instrument that can be valued.
///
/// # Examples
///
/// ```
/// use kurumi_core::{Date, KurumiError};
/// use kurumi_pricing::Instrument;
///
/// struct Deposit {
///     maturity: Date,
/// }
///
/// impl Instrument for Deposit {
///     fn maturity(&self) -> Date {
///         self.maturity
///     }
/// }
///
/// # fn main() -> Result<(), KurumiError> {
/// let deposit = Deposit { maturity: Date::from_ymd(2024, 12, 31)? };
/// assert!(!deposit.is_expired(Date::from_ymd(2024, 6, 30)?));
/// assert!(deposit.is_expired(Date::from_ymd(2025, 1, 1)?));
/// # Ok(())
/// # }
/// ```
pub trait Instrument {
    /// Returns the date on which the instrument matures or expires.
    fn maturity(&self) -> Date;

    /// Returns `true` when `as_of` is on or after [`Instrument::maturity`].
    ///
    /// An instrument is considered expired from the maturity date onward, so an
    /// engine can use this method to reject valuation after the final cashflow.
    fn is_expired(&self, as_of: Date) -> bool {
        as_of >= self.maturity()
    }
}

/// Option sensitivities (the "Greeks").
///
/// All fields are derivatives of the option value with respect to a market
/// parameter. The units follow the usual pricing convention:
///
/// | Field | Derivative | Units |
/// | --- | --- | --- |
/// | `delta` | `dV / dS` | value per unit of underlying |
/// | `gamma` | `d2V / dS2` | delta per unit of underlying |
/// | `vega` | `dV / dsigma` | value per unit (100 percentage points) of volatility |
/// | `theta` | `dV / dt = -dV / dT` | value per year |
/// | `rho` | `dV / dr` | value per unit (100 percentage points) of rate |
///
/// The struct is plain data; [`Greeks::validate`] rejects non-finite values.
///
/// # Examples
///
/// ```
/// use kurumi_pricing::Greeks;
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let greeks = Greeks::new(0.5, 0.02, 10.0, -3.0, 4.0)?;
/// assert!((greeks.delta - 0.5).abs() < 1e-12);
/// assert!(Greeks::default().delta.abs() < 1e-15);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Greeks {
    /// First derivative of value with respect to the underlying price.
    pub delta: f64,
    /// Second derivative of value with respect to the underlying price.
    pub gamma: f64,
    /// First derivative of value with respect to volatility, per unit of
    /// volatility (a move from 20% to 21% is a change of `0.01`).
    pub vega: f64,
    /// First derivative of value with respect to calendar time, per year.
    pub theta: f64,
    /// First derivative of value with respect to the risk-free rate, per unit
    /// of rate (a move from 5% to 6% is a change of `0.01`).
    pub rho: f64,
}

impl Greeks {
    /// Creates a sensitivity set, rejecting any non-finite component.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] naming the first non-finite
    /// component.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::Greeks;
    ///
    /// assert!(Greeks::new(0.5, 0.02, 10.0, -3.0, 4.0).is_ok());
    /// assert!(Greeks::new(f64::NAN, 0.0, 0.0, 0.0, 0.0).is_err());
    /// ```
    pub fn new(
        delta: f64,
        gamma: f64,
        vega: f64,
        theta: f64,
        rho: f64,
    ) -> Result<Self, KurumiError> {
        let greeks = Self {
            delta,
            gamma,
            vega,
            theta,
            rho,
        };
        greeks.validate()?;
        Ok(greeks)
    }

    /// Validates that every component is finite.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] naming the first non-finite
    /// component.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::Greeks;
    ///
    /// assert!(Greeks::default().validate().is_ok());
    /// ```
    pub fn validate(&self) -> Result<(), KurumiError> {
        for (parameter, value) in [
            ("delta", self.delta),
            ("gamma", self.gamma),
            ("vega", self.vega),
            ("theta", self.theta),
            ("rho", self.rho),
        ] {
            if !value.is_finite() {
                return Err(KurumiError::InvalidParameter {
                    parameter,
                    reason: "must be finite".to_owned(),
                });
            }
        }
        Ok(())
    }
}

/// The result of valuing an instrument.
///
/// `npv` is the net present value of the instrument's cashflows in the pricing
/// currency. `greeks` is `Some` when the engine computes sensitivities and
/// `None` when it does not (for example, a Monte Carlo engine before a bump-and-
/// revalue pass).
///
/// # Examples
///
/// ```
/// use kurumi_pricing::{Greeks, ValuationResult};
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let plain = ValuationResult::new(12.5)?;
/// assert!((plain.npv - 12.5).abs() < 1e-12);
/// assert!(plain.greeks.is_none());
///
/// let with_greeks = ValuationResult::with_greeks(12.5, Greeks::default())?;
/// assert!(with_greeks.greeks.is_some());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValuationResult {
    /// Net present value of the instrument, in the pricing currency.
    pub npv: f64,
    /// Sensitivities, when the engine provides them.
    pub greeks: Option<Greeks>,
}

impl ValuationResult {
    /// Creates a valuation result without sensitivities.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `npv` is `NaN` or
    /// infinite.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::ValuationResult;
    ///
    /// assert!(ValuationResult::new(0.0).is_ok());
    /// assert!(ValuationResult::new(f64::INFINITY).is_err());
    /// ```
    pub fn new(npv: f64) -> Result<Self, KurumiError> {
        validate_npv(npv)?;
        Ok(Self { npv, greeks: None })
    }

    /// Creates a valuation result with a complete set of sensitivities.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `npv` or any Greek is not
    /// finite.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::{Greeks, ValuationResult};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let result = ValuationResult::with_greeks(10.0, Greeks::default())?;
    /// assert!(result.greeks.is_some());
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_greeks(npv: f64, greeks: Greeks) -> Result<Self, KurumiError> {
        validate_npv(npv)?;
        greeks.validate()?;
        Ok(Self {
            npv,
            greeks: Some(greeks),
        })
    }
}

/// A pricing engine for instruments of type `I`.
///
/// The engine owns whatever market data it needs (a yield curve, a spot and
/// volatility set, a tree size). Instruments supply only their contractual
/// metadata, so one engine instance can value many instruments as of different
/// dates.
///
/// # Examples
///
/// ```
/// use kurumi_core::{Date, KurumiError};
/// use kurumi_pricing::{Instrument, PricingEngine, ValuationResult};
///
/// struct FixedPayout {
///     maturity: Date,
///     amount: f64,
/// }
///
/// impl Instrument for FixedPayout {
///     fn maturity(&self) -> Date {
///         self.maturity
///     }
/// }
///
/// struct ZeroRateEngine {
///     rate: f64,
/// }
///
/// impl PricingEngine<FixedPayout> for ZeroRateEngine {
///     fn price(
///         &self,
///         instrument: &FixedPayout,
///         as_of: Date,
///     ) -> Result<ValuationResult, KurumiError> {
///         let years = (i64::from(instrument.maturity.days_since_epoch())
///             - i64::from(as_of.days_since_epoch())) as f64
///             / 365.0;
///         ValuationResult::new(instrument.amount * (-self.rate * years).exp())
///     }
/// }
///
/// # fn main() -> Result<(), KurumiError> {
/// let engine = ZeroRateEngine { rate: 0.05 };
/// let instrument = FixedPayout {
///     maturity: Date::from_ymd(2025, 1, 1)?,
///     amount: 100.0,
/// };
/// let value = engine.price(&instrument, Date::from_ymd(2024, 1, 1)?)?;
/// assert!(value.npv < 100.0);
/// # Ok(())
/// # }
/// ```
pub trait PricingEngine<I: Instrument> {
    /// Values `instrument` as of `as_of`.
    ///
    /// # Errors
    ///
    /// Returns a [`KurumiError`] when the instrument has expired, a required
    /// market input is invalid, or the numerical method fails to converge.
    fn price(&self, instrument: &I, as_of: Date) -> Result<ValuationResult, KurumiError>;
}

fn validate_npv(npv: f64) -> Result<(), KurumiError> {
    if npv.is_finite() {
        Ok(())
    } else {
        Err(KurumiError::InvalidParameter {
            parameter: "npv",
            reason: "must be finite".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct ZeroCouponBond {
        maturity: Date,
        face: f64,
    }

    impl Instrument for ZeroCouponBond {
        fn maturity(&self) -> Date {
            self.maturity
        }
    }

    struct FlatDiscountEngine {
        rate: f64,
    }

    impl PricingEngine<ZeroCouponBond> for FlatDiscountEngine {
        fn price(
            &self,
            instrument: &ZeroCouponBond,
            as_of: Date,
        ) -> Result<ValuationResult, KurumiError> {
            if instrument.is_expired(as_of) {
                return Err(KurumiError::InvalidParameter {
                    parameter: "as_of",
                    reason: "instrument has expired".to_owned(),
                });
            }
            let years = (i64::from(instrument.maturity.days_since_epoch())
                - i64::from(as_of.days_since_epoch())) as f64
                / 365.0;
            ValuationResult::new(instrument.face * (-self.rate * years).exp())
        }
    }

    #[test]
    fn default_expiry_check_includes_the_maturity_date() {
        let instrument = ZeroCouponBond {
            maturity: Date::from_ymd(2024, 6, 30).expect("valid date"),
            face: 100.0,
        };
        assert!(!instrument.is_expired(Date::from_ymd(2024, 6, 29).expect("valid date")));
        assert!(instrument.is_expired(Date::from_ymd(2024, 6, 30).expect("valid date")));
        assert!(instrument.is_expired(Date::from_ymd(2024, 7, 1).expect("valid date")));
    }

    #[test]
    fn engine_values_a_zero_coupon_bond_with_the_flat_rate() {
        let engine = FlatDiscountEngine { rate: 0.05 };
        let instrument = ZeroCouponBond {
            maturity: Date::from_ymd(2025, 1, 1).expect("valid date"),
            face: 100.0,
        };
        let result = engine
            .price(&instrument, Date::from_ymd(2024, 1, 1).expect("valid date"))
            .expect("valuation succeeds");
        // 2024 is a leap year, so 2023-01-01..2025-01-01 is 366 days.
        let expected = 100.0 * (-0.05_f64 * 366.0 / 365.0).exp();
        assert!((result.npv - expected).abs() < 1e-12);
        assert!(result.greeks.is_none());
    }

    #[test]
    fn valuation_result_rejects_non_finite_npv_and_greeks() {
        assert!(ValuationResult::new(f64::NAN).is_err());
        assert!(ValuationResult::new(f64::INFINITY).is_err());
        let invalid_greeks = Greeks {
            delta: f64::NAN,
            ..Greeks::default()
        };
        assert!(ValuationResult::with_greeks(1.0, invalid_greeks).is_err());
        assert!(ValuationResult::with_greeks(f64::NAN, Greeks::default()).is_err());
    }

    #[test]
    fn greeks_report_the_offending_component() {
        let error = Greeks::new(0.0, 0.0, f64::INFINITY, 0.0, 0.0).expect_err("infinite vega");
        assert_eq!(
            error,
            KurumiError::InvalidParameter {
                parameter: "vega",
                reason: "must be finite".to_owned(),
            }
        );
    }

    #[test]
    fn greeks_default_to_zero_and_validate() {
        let greeks = Greeks::default();
        assert!(greeks.delta.abs() < 1e-15);
        assert!(greeks.validate().is_ok());
    }
}
