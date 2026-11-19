//! Option contracts: payoff type, strike, expiry, and exercise style.

use crate::traits::Instrument;
use kurumi_core::{Date, DayCountConvention, KurumiError};

/// Whether an option gives the holder the right to buy or to sell.
///
/// # Examples
///
/// ```
/// use kurumi_pricing::OptionType;
///
/// assert_ne!(OptionType::Call, OptionType::Put);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptionType {
    /// The right to buy the underlying at the strike price.
    Call,
    /// The right to sell the underlying at the strike price.
    Put,
}

/// When an option may be exercised.
///
/// # Examples
///
/// ```
/// use kurumi_pricing::ExerciseStyle;
///
/// assert_ne!(ExerciseStyle::European, ExerciseStyle::American);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExerciseStyle {
    /// Exercise is permitted only at expiry.
    European,
    /// Exercise is permitted at any time up to and including expiry.
    American,
}

/// A vanilla call or put option.
///
/// The contract stores the contractual terms only: payoff direction, strike,
/// expiry, and exercise style. Market data (spot, rates, dividend yield, and
/// volatility) lives in [`MarketData`](crate::options::MarketData) and is owned
/// by the pricing engine.
///
/// # Examples
///
/// ```
/// use kurumi_core::Date;
/// use kurumi_pricing::{ExerciseStyle, Instrument, OptionContract, OptionType};
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let call = OptionContract::new(
///     OptionType::Call,
///     100.0,
///     Date::from_ymd(2025, 1, 1)?,
///     ExerciseStyle::European,
/// )?;
/// assert!(call.is_call());
/// assert!(!call.is_expired(Date::from_ymd(2024, 12, 31)?));
/// assert!(call.is_expired(Date::from_ymd(2025, 1, 1)?));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OptionContract {
    option_type: OptionType,
    strike: f64,
    expiry: Date,
    exercise: ExerciseStyle,
}

impl OptionContract {
    /// Creates a vanilla option contract.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `strike` is not finite and
    /// strictly positive.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::{ExerciseStyle, OptionContract, OptionType};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// assert!(OptionContract::new(
    ///     OptionType::Put,
    ///     50.0,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     ExerciseStyle::American,
    /// ).is_ok());
    /// assert!(OptionContract::new(
    ///     OptionType::Put,
    ///     0.0,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     ExerciseStyle::American,
    /// ).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        option_type: OptionType,
        strike: f64,
        expiry: Date,
        exercise: ExerciseStyle,
    ) -> Result<Self, KurumiError> {
        if !strike.is_finite() || strike <= 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "strike",
                reason: "must be finite and strictly positive".to_owned(),
            });
        }
        Ok(Self {
            option_type,
            strike,
            expiry,
            exercise,
        })
    }

    /// Returns the payoff direction.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::{ExerciseStyle, OptionContract, OptionType};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let option = OptionContract::new(
    ///     OptionType::Call,
    ///     100.0,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     ExerciseStyle::European,
    /// )?;
    /// assert_eq!(option.option_type(), OptionType::Call);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn option_type(&self) -> OptionType {
        self.option_type
    }

    /// Returns the strike price.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::{ExerciseStyle, OptionContract, OptionType};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let option = OptionContract::new(
    ///     OptionType::Call,
    ///     100.0,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     ExerciseStyle::European,
    /// )?;
    /// assert!((option.strike() - 100.0).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn strike(&self) -> f64 {
        self.strike
    }

    /// Returns the expiry date.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::{ExerciseStyle, OptionContract, OptionType};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let expiry = Date::from_ymd(2025, 1, 1)?;
    /// let option = OptionContract::new(
    ///     OptionType::Call,
    ///     100.0,
    ///     expiry,
    ///     ExerciseStyle::European,
    /// )?;
    /// assert_eq!(option.expiry(), expiry);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn expiry(&self) -> Date {
        self.expiry
    }

    /// Returns the exercise style.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::{ExerciseStyle, OptionContract, OptionType};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let option = OptionContract::new(
    ///     OptionType::Call,
    ///     100.0,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     ExerciseStyle::American,
    /// )?;
    /// assert!(option.is_american());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn exercise_style(&self) -> ExerciseStyle {
        self.exercise
    }

    /// Returns `true` for a call.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::{ExerciseStyle, OptionContract, OptionType};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let option = OptionContract::new(
    ///     OptionType::Call,
    ///     100.0,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     ExerciseStyle::European,
    /// )?;
    /// assert!(option.is_call());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn is_call(&self) -> bool {
        matches!(self.option_type, OptionType::Call)
    }

    /// Returns `true` for a put.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::{ExerciseStyle, OptionContract, OptionType};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let option = OptionContract::new(
    ///     OptionType::Put,
    ///     100.0,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     ExerciseStyle::European,
    /// )?;
    /// assert!(option.is_put());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn is_put(&self) -> bool {
        matches!(self.option_type, OptionType::Put)
    }

    /// Returns `true` for a European option.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::{ExerciseStyle, OptionContract, OptionType};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let option = OptionContract::new(
    ///     OptionType::Call,
    ///     100.0,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     ExerciseStyle::European,
    /// )?;
    /// assert!(option.is_european());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn is_european(&self) -> bool {
        matches!(self.exercise, ExerciseStyle::European)
    }

    /// Returns `true` for an American option.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::{ExerciseStyle, OptionContract, OptionType};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let option = OptionContract::new(
    ///     OptionType::Call,
    ///     100.0,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     ExerciseStyle::American,
    /// )?;
    /// assert!(option.is_american());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn is_american(&self) -> bool {
        matches!(self.exercise, ExerciseStyle::American)
    }

    /// Returns the option's payoff at an expiry spot price `spot`.
    ///
    /// A call pays `max(spot - strike, 0)`; a put pays
    /// `max(strike - spot, 0)`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `spot` is not finite and
    /// non-negative.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::{ExerciseStyle, OptionContract, OptionType};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let call = OptionContract::new(
    ///     OptionType::Call,
    ///     100.0,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     ExerciseStyle::European,
    /// )?;
    /// assert!((call.payoff(120.0)? - 20.0).abs() < 1e-15);
    /// assert!((call.payoff(90.0)? - 0.0).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    pub fn payoff(&self, spot: f64) -> Result<f64, KurumiError> {
        if !spot.is_finite() || spot < 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "spot",
                reason: "must be finite and non-negative".to_owned(),
            });
        }
        Ok(match self.option_type {
            OptionType::Call => (spot - self.strike).max(0.0),
            OptionType::Put => (self.strike - spot).max(0.0),
        })
    }

    /// Returns the time to expiry in years as of `as_of`.
    ///
    /// The year fraction uses the Actual/365 Fixed convention, which is the
    /// standard calendar-day convention for exchange-traded equity options:
    /// `t = days(as_of, expiry) / 365`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `as_of` is on or after the
    /// expiry date.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::{ExerciseStyle, OptionContract, OptionType};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let option = OptionContract::new(
    ///     OptionType::Call,
    ///     100.0,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     ExerciseStyle::European,
    /// )?;
    /// let time = option.time_to_expiry(Date::from_ymd(2024, 1, 1)?)?;
    /// assert!((time - 366.0 / 365.0).abs() < 1e-12);
    /// # Ok(())
    /// # }
    /// ```
    pub fn time_to_expiry(&self, as_of: Date) -> Result<f64, KurumiError> {
        if as_of >= self.expiry {
            return Err(KurumiError::InvalidParameter {
                parameter: "as_of",
                reason: "option has expired".to_owned(),
            });
        }
        let time = DayCountConvention::Act365Fixed.year_fraction(as_of, self.expiry);
        if time.is_finite() && time > 0.0 {
            Ok(time)
        } else {
            Err(KurumiError::InvalidParameter {
                parameter: "as_of",
                reason: "time to expiry must be finite and positive".to_owned(),
            })
        }
    }
}

impl Instrument for OptionContract {
    fn maturity(&self) -> Date {
        self.expiry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn date(year: i32, month: u32, day: u32) -> Date {
        Date::from_ymd(year, month, day).expect("valid date")
    }

    #[test]
    fn call_and_put_payoffs_match_the_textbook_definitions() {
        let call = OptionContract::new(
            OptionType::Call,
            100.0,
            date(2025, 1, 1),
            ExerciseStyle::European,
        )
        .expect("valid contract");
        let put = OptionContract::new(
            OptionType::Put,
            100.0,
            date(2025, 1, 1),
            ExerciseStyle::European,
        )
        .expect("valid contract");
        assert_relative_eq!(call.payoff(130.0).expect("payoff"), 30.0, epsilon = 1e-15);
        assert_relative_eq!(call.payoff(70.0).expect("payoff"), 0.0, epsilon = 1e-15);
        assert_relative_eq!(put.payoff(70.0).expect("payoff"), 30.0, epsilon = 1e-15);
        assert_relative_eq!(put.payoff(130.0).expect("payoff"), 0.0, epsilon = 1e-15);
        assert!(call.payoff(-1.0).is_err());
        assert!(call.payoff(f64::NAN).is_err());
    }

    #[test]
    fn time_to_expiry_uses_actual_over_365() {
        let option = OptionContract::new(
            OptionType::Call,
            100.0,
            date(2025, 1, 1),
            ExerciseStyle::European,
        )
        .expect("valid contract");
        assert_relative_eq!(
            option.time_to_expiry(date(2024, 1, 1)).expect("time"),
            366.0 / 365.0,
            epsilon = 1e-15
        );
        assert!(option.time_to_expiry(date(2025, 1, 1)).is_err());
        assert!(option.time_to_expiry(date(2025, 2, 1)).is_err());
    }

    #[test]
    fn construction_rejects_invalid_strikes() {
        for strike in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(
                OptionContract::new(
                    OptionType::Call,
                    strike,
                    date(2025, 1, 1),
                    ExerciseStyle::European
                )
                .is_err()
            );
        }
    }

    #[test]
    fn instrument_maturity_is_the_expiry_date() {
        let option = OptionContract::new(
            OptionType::Call,
            100.0,
            date(2025, 1, 1),
            ExerciseStyle::American,
        )
        .expect("valid contract");
        assert_eq!(Instrument::maturity(&option), date(2025, 1, 1));
        assert!(option.is_american());
        assert!(option.is_call());
        assert!(!option.is_put());
        assert!(!option.is_european());
    }
}
