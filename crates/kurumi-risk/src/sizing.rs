//! Position sizing: the Kelly criterion and volatility targeting.
//!
//! Both sizing rules produce a **fraction of equity** to allocate to a single
//! risky asset. The fraction is dimensionless: `0.25` means 25% of equity.
//!
//! # Kelly criterion
//!
//! For a bet that wins `b` units per unit risked with probability `p`, the
//! growth-optimal fraction is
//!
//! ```text
//! f* = p - (1 - p) / b
//! ```
//!
//! A negative `f*` means the bet has negative expected log growth and no
//! position should be taken.
//!
//! # Volatility targeting
//!
//! The volatility-target rule scales exposure inversely with realized
//! volatility so the portfolio's risk stays near a target:
//!
//! ```text
//! exposure = min(target_volatility / asset_volatility, max_leverage)
//! ```
//!
//! # References
//!
//! - J. L. Kelly Jr., "A New Interpretation of Information Rate", *Bell System
//!   Technical Journal* 35(4), 1956.
//! - E. O. Thorp, "The Kelly Criterion in Blackjack, Sports Betting, and the
//!   Stock Market", in *Handbook of Asset and Liability Management*, 2006.

use kurumi_core::{KurumiError, Price, Quantity};
use rust_decimal::Decimal;

/// The classic Kelly fraction for a two-outcome bet.
///
/// `win_probability` is the probability of the favorable outcome and
/// `payoff_ratio` is the average win divided by the average loss (`b > 0`).
/// The result may be negative, meaning the bet should be avoided.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when `win_probability` is outside
/// `[0, 1]` or `payoff_ratio` is not finite and strictly positive.
///
/// # Examples
///
/// ```
/// use kurumi_risk::kelly_fraction;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// // p = 0.6 with even odds gives the textbook 20% allocation.
/// let fraction = kelly_fraction(0.6, 1.0)?;
/// assert!((fraction - 0.2).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn kelly_fraction(win_probability: f64, payoff_ratio: f64) -> Result<f64, KurumiError> {
    if !win_probability.is_finite() || !(0.0..=1.0).contains(&win_probability) {
        return Err(KurumiError::InvalidParameter {
            parameter: "win_probability",
            reason: "must be finite and in [0, 1]".to_owned(),
        });
    }
    if !payoff_ratio.is_finite() || payoff_ratio <= 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "payoff_ratio",
            reason: "must be finite and strictly positive".to_owned(),
        });
    }
    let fraction = win_probability - (1.0 - win_probability) / payoff_ratio;
    if fraction.is_finite() {
        Ok(fraction)
    } else {
        Err(KurumiError::NumericalOverflow {
            operation: "kelly_fraction",
        })
    }
}

/// The volatility-target exposure fraction.
///
/// The result is `target_volatility / asset_volatility`, capped at
/// `max_leverage`. A zero target produces a zero allocation.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when `asset_volatility` is not
/// finite and strictly positive, `target_volatility` is not finite and
/// non-negative, or `max_leverage` is not finite and strictly positive.
///
/// # Examples
///
/// ```
/// use kurumi_risk::volatility_target_fraction;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// // A 40% volatile asset scaled to a 10% target needs a 25% allocation.
/// let fraction = volatility_target_fraction(0.40, 0.10, 2.0)?;
/// assert!((fraction - 0.25).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn volatility_target_fraction(
    asset_volatility: f64,
    target_volatility: f64,
    max_leverage: f64,
) -> Result<f64, KurumiError> {
    validate_asset_volatility(asset_volatility)?;
    validate_target(target_volatility)?;
    validate_leverage(max_leverage)?;
    let exposure = target_volatility / asset_volatility;
    if !exposure.is_finite() {
        return Err(KurumiError::NumericalOverflow {
            operation: "volatility_target_fraction",
        });
    }
    Ok(exposure.min(max_leverage))
}

/// A reusable volatility-target position calculator.
///
/// The target volatility and leverage cap are validated once at construction;
/// [`VolatilityTargetSizer::fraction`] and the sizing helpers can then be
/// applied to any asset volatility.
///
/// # Examples
///
/// ```
/// use kurumi_risk::VolatilityTargetSizer;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let sizer = VolatilityTargetSizer::new(0.10, 2.0)?;
/// assert!((sizer.fraction(0.20)? - 0.5).abs() < 1e-12);
/// // Excess exposure is capped at the leverage limit.
/// assert!((sizer.fraction(0.01)? - 2.0).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolatilityTargetSizer {
    target_volatility: f64,
    max_leverage: f64,
}

impl VolatilityTargetSizer {
    /// Creates a volatility-target sizer.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `target_volatility` is not
    /// finite and non-negative, or `max_leverage` is not finite and strictly
    /// positive.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_risk::VolatilityTargetSizer;
    ///
    /// assert!(VolatilityTargetSizer::new(0.10, 1.0).is_ok());
    /// assert!(VolatilityTargetSizer::new(-0.10, 1.0).is_err());
    /// assert!(VolatilityTargetSizer::new(0.10, 0.0).is_err());
    /// ```
    pub fn new(target_volatility: f64, max_leverage: f64) -> Result<Self, KurumiError> {
        validate_target(target_volatility)?;
        validate_leverage(max_leverage)?;
        Ok(Self {
            target_volatility,
            max_leverage,
        })
    }

    /// Returns the annualized target volatility.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_risk::VolatilityTargetSizer;
    ///
    /// # fn main() -> Result<(), kurumi_risk::KurumiError> {
    /// let sizer = VolatilityTargetSizer::new(0.10, 1.0)?;
    /// assert!((sizer.target_volatility() - 0.10).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn target_volatility(self) -> f64 {
        self.target_volatility
    }

    /// Returns the maximum gross exposure as a multiple of equity.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_risk::VolatilityTargetSizer;
    ///
    /// # fn main() -> Result<(), kurumi_risk::KurumiError> {
    /// assert!((VolatilityTargetSizer::new(0.10, 1.5)?.max_leverage() - 1.5).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn max_leverage(self) -> f64 {
        self.max_leverage
    }

    /// Returns the capped exposure fraction for an asset volatility.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `asset_volatility` is not
    /// finite and strictly positive, and [`KurumiError::NumericalOverflow`] when
    /// the ratio is not finite.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_risk::VolatilityTargetSizer;
    ///
    /// # fn main() -> Result<(), kurumi_risk::KurumiError> {
    /// let sizer = VolatilityTargetSizer::new(0.10, 1.0)?;
    /// assert!((sizer.fraction(0.40)? - 0.25).abs() < 1e-12);
    /// assert!(sizer.fraction(0.0).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn fraction(self, asset_volatility: f64) -> Result<f64, KurumiError> {
        volatility_target_fraction(asset_volatility, self.target_volatility, self.max_leverage)
    }

    /// Returns the quote-currency notional implied by the exposure fraction.
    ///
    /// The notional is `equity * fraction`, rounded to eight decimal places to
    /// keep the decimal ledger exact and free of binary floating-point noise.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `equity` is negative, and
    /// propagates the errors of [`VolatilityTargetSizer::fraction`].
    /// [`KurumiError::NumericalOverflow`] is returned when the product is not
    /// representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_risk::VolatilityTargetSizer;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_risk::KurumiError> {
    /// let sizer = VolatilityTargetSizer::new(0.10, 1.0)?;
    /// let notional = sizer.notional(Decimal::from(100_000), 0.40)?;
    /// assert_eq!(notional, Decimal::from(25_000));
    /// # Ok(())
    /// # }
    /// ```
    pub fn notional(self, equity: Decimal, asset_volatility: f64) -> Result<Decimal, KurumiError> {
        if equity.is_sign_negative() {
            return Err(KurumiError::InvalidParameter {
                parameter: "equity",
                reason: "must be non-negative".to_owned(),
            });
        }
        let fraction = self.fraction(asset_volatility)?;
        let fraction_decimal = Decimal::from_f64_retain(fraction)
            .map(|value| value.round_dp(8))
            .ok_or(KurumiError::NumericalOverflow {
                operation: "volatility_target notional",
            })?;
        equity
            .checked_mul(fraction_decimal)
            .map(|value| value.round_dp(8))
            .ok_or(KurumiError::NumericalOverflow {
                operation: "volatility_target notional",
            })
    }

    /// Returns the quantity implied by the exposure fraction.
    ///
    /// The quantity is `notional / price`, rounded to eight decimal places.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `equity` is negative, and
    /// propagates the errors of [`VolatilityTargetSizer::notional`].
    /// [`KurumiError::NumericalOverflow`] is returned when the division is not
    /// representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Price;
    /// use kurumi_risk::VolatilityTargetSizer;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_risk::KurumiError> {
    /// let sizer = VolatilityTargetSizer::new(0.10, 1.0)?;
    /// let price = Price::new(Decimal::from(100))?;
    /// let units = sizer.quantity(Decimal::from(100_000), 0.40, price)?;
    /// assert_eq!(units.value(), Decimal::from(250));
    /// # Ok(())
    /// # }
    /// ```
    pub fn quantity(
        self,
        equity: Decimal,
        asset_volatility: f64,
        price: Price,
    ) -> Result<Quantity, KurumiError> {
        let notional = self.notional(equity, asset_volatility)?;
        let quantity = notional
            .checked_div(price.value())
            .ok_or(KurumiError::NumericalOverflow {
                operation: "volatility_target quantity",
            })?
            .round_dp(8);
        Quantity::new(quantity)
    }
}

fn validate_asset_volatility(asset_volatility: f64) -> Result<(), KurumiError> {
    if !asset_volatility.is_finite() || asset_volatility <= 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "asset_volatility",
            reason: "must be finite and strictly positive".to_owned(),
        });
    }
    Ok(())
}

fn validate_target(target_volatility: f64) -> Result<(), KurumiError> {
    if !target_volatility.is_finite() || target_volatility < 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "target_volatility",
            reason: "must be finite and non-negative".to_owned(),
        });
    }
    Ok(())
}

fn validate_leverage(max_leverage: f64) -> Result<(), KurumiError> {
    if !max_leverage.is_finite() || max_leverage <= 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "max_leverage",
            reason: "must be finite and strictly positive".to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn kelly_matches_the_textbook_even_odds_case() {
        // p = 0.6, b = 1 -> f* = 0.6 - 0.4 = 0.2.
        assert_relative_eq!(
            kelly_fraction(0.6, 1.0).expect("valid inputs"),
            0.2,
            epsilon = 1e-12
        );
        // p = 0.55, b = 2 -> f* = 0.55 - 0.45 / 2 = 0.325.
        assert_relative_eq!(
            kelly_fraction(0.55, 2.0).expect("valid inputs"),
            0.325,
            epsilon = 1e-12
        );
    }

    #[test]
    fn kelly_is_negative_for_an_unfavourable_bet() {
        let fraction = kelly_fraction(0.4, 1.0).expect("valid inputs");
        assert_relative_eq!(fraction, -0.2, epsilon = 1e-12);
        assert!(fraction < 0.0);
    }

    #[test]
    fn kelly_boundaries_are_one_and_no_bet() {
        assert_relative_eq!(
            kelly_fraction(1.0, 1.0).expect("valid inputs"),
            1.0,
            epsilon = 1e-12
        );
        assert!(kelly_fraction(0.0, 1.0).expect("valid inputs") < 0.0);
    }

    #[test]
    fn kelly_rejects_invalid_probabilities_and_payoffs() {
        assert!(kelly_fraction(-0.1, 1.0).is_err());
        assert!(kelly_fraction(1.1, 1.0).is_err());
        assert!(kelly_fraction(f64::NAN, 1.0).is_err());
        assert!(kelly_fraction(0.5, 0.0).is_err());
        assert!(kelly_fraction(0.5, f64::INFINITY).is_err());
    }

    #[test]
    fn volatility_target_scales_inverse_to_volatility() {
        assert_relative_eq!(
            volatility_target_fraction(0.40, 0.10, 2.0).expect("valid inputs"),
            0.25,
            epsilon = 1e-12
        );
        assert_relative_eq!(
            volatility_target_fraction(0.20, 0.10, 2.0).expect("valid inputs"),
            0.5,
            epsilon = 1e-12
        );
    }

    #[test]
    fn volatility_target_caps_exposure_and_allows_zero_target() {
        assert_relative_eq!(
            volatility_target_fraction(0.01, 0.10, 2.0).expect("valid inputs"),
            2.0,
            epsilon = 1e-12
        );
        assert_relative_eq!(
            volatility_target_fraction(0.20, 0.0, 2.0).expect("valid inputs"),
            0.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn volatility_target_rejects_invalid_inputs() {
        assert!(volatility_target_fraction(0.0, 0.1, 1.0).is_err());
        assert!(volatility_target_fraction(-0.2, 0.1, 1.0).is_err());
        assert!(volatility_target_fraction(0.2, -0.1, 1.0).is_err());
        assert!(volatility_target_fraction(0.2, 0.1, 0.0).is_err());
        assert!(volatility_target_fraction(f64::NAN, 0.1, 1.0).is_err());
    }

    #[test]
    fn sizer_notional_and_quantity_are_consistent() {
        let sizer = VolatilityTargetSizer::new(0.10, 1.0).expect("valid sizer");
        let equity = Decimal::from(100_000);
        let notional = sizer.notional(equity, 0.40).expect("valid inputs");
        assert_eq!(notional, Decimal::from(25_000));

        let price = Price::new(Decimal::from(100)).expect("valid price");
        let quantity = sizer.quantity(equity, 0.40, price).expect("valid inputs");
        assert_eq!(quantity.value(), Decimal::from(250));
    }

    #[test]
    fn sizer_rejects_invalid_equity_and_leverage() {
        let sizer = VolatilityTargetSizer::new(0.10, 1.0).expect("valid sizer");
        assert!(sizer.notional(Decimal::from(-1), 0.2).is_err());
        assert!(VolatilityTargetSizer::new(0.10, -1.0).is_err());
        assert!(VolatilityTargetSizer::new(f64::NAN, 1.0).is_err());
    }
}
