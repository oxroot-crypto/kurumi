//! Zero-coupon yield curves and compounding conventions.
//!
//! A [`ZeroCurve`] maps a date to the continuously or discretely compounded
//! zero rate for the maturity date. Discount factors are derived from the
//! interpolated zero rate and the curve's day-count convention:
//!
//! ```text
//! continuous:            df(t) = exp(-r(t) * t)
//! discrete, m per year:  df(t) = (1 + r(t) / m)^(-m * t)
//! ```
//!
//! Curves hold one or more pillar dates. Zero rates are interpolated linearly in
//! the year-fraction measured from the curve reference date. The curve extends
//! the first and last pillar rates flat outside the pillar range, which is the
//! usual convention for a curve that is only observed at benchmark tenors;
//! callers who need a market-consistent forward outside the pillars should
//! supply more knots. Pillar year fractions must be strictly increasing under
//! the curve's day-count convention.

use kurumi_core::{Date, DayCountConvention, KurumiError};

/// A compounding convention for zero rates and discount factors.
///
/// `Continuous` uses `exp(-r t)`. `Discrete { frequency }` compounds `frequency`
/// times per year and uses `(1 + r / frequency)^(-frequency * t)`. A frequency
/// of one is annual compounding; two is semiannual compounding, and so on.
///
/// # Examples
///
/// ```
/// use kurumi_pricing::Compounding;
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let annual = Compounding::annual();
/// let semiannual = Compounding::semiannual();
/// assert_eq!(annual.frequency(), Some(1));
/// assert_eq!(semiannual.frequency(), Some(2));
/// assert!(Compounding::discrete(0).is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compounding {
    /// Continuous compounding, `(1 + r/m)^(m t) -> exp(r t)` as `m` grows.
    Continuous,
    /// Discrete compounding `frequency` times per year.
    Discrete {
        /// Number of compounding periods per year. Must be positive.
        frequency: u32,
    },
}

impl Compounding {
    /// Annual discrete compounding.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::Compounding;
    ///
    /// assert_eq!(Compounding::annual().frequency(), Some(1));
    /// ```
    #[must_use]
    pub const fn annual() -> Self {
        Self::Discrete { frequency: 1 }
    }

    /// Semiannual discrete compounding.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::Compounding;
    ///
    /// assert_eq!(Compounding::semiannual().frequency(), Some(2));
    /// ```
    #[must_use]
    pub const fn semiannual() -> Self {
        Self::Discrete { frequency: 2 }
    }

    /// Creates a discrete compounding convention.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `frequency` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::Compounding;
    ///
    /// assert!(Compounding::discrete(4).is_ok());
    /// assert!(Compounding::discrete(0).is_err());
    /// ```
    pub fn discrete(frequency: u32) -> Result<Self, KurumiError> {
        if frequency == 0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "frequency",
                reason: "must be greater than zero".to_owned(),
            });
        }
        Ok(Self::Discrete { frequency })
    }

    /// Returns the number of compounding periods per year, or `None` for
    /// continuous compounding.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::Compounding;
    ///
    /// assert_eq!(Compounding::Continuous.frequency(), None);
    /// assert_eq!(Compounding::annual().frequency(), Some(1));
    /// ```
    #[must_use]
    pub const fn frequency(self) -> Option<u32> {
        match self {
            Self::Continuous => None,
            Self::Discrete { frequency } => Some(frequency),
        }
    }

    /// Returns the discount factor for `rate` over `year_fraction` years.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `rate` or `year_fraction`
    /// is not finite, when a discrete frequency is zero, or when a discrete rate
    /// would make `1 + rate / frequency` non-positive. Returns
    /// [`KurumiError::NumericalOverflow`] when the discount factor is not
    /// representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::Compounding;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let continuous = Compounding::Continuous.discount_factor(0.05, 1.0)?;
    /// assert!((continuous - (-0.05_f64).exp()).abs() < 1e-15);
    ///
    /// let semiannual = Compounding::semiannual().discount_factor(0.05, 1.0)?;
    /// assert!((semiannual - 1.025_f64.powi(-2)).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn discount_factor(self, rate: f64, year_fraction: f64) -> Result<f64, KurumiError> {
        if !rate.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "rate",
                reason: "must be finite".to_owned(),
            });
        }
        if !year_fraction.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "year_fraction",
                reason: "must be finite".to_owned(),
            });
        }

        let factor = match self {
            Self::Continuous => (-rate * year_fraction).exp(),
            Self::Discrete { frequency } => {
                if frequency == 0 {
                    return Err(KurumiError::InvalidParameter {
                        parameter: "frequency",
                        reason: "must be greater than zero".to_owned(),
                    });
                }
                let periods = f64::from(frequency);
                let base = 1.0 + rate / periods;
                if base <= 0.0 {
                    return Err(KurumiError::InvalidParameter {
                        parameter: "rate",
                        reason: format!(
                            "rate {rate} is below the discrete-compounding lower bound of {}",
                            -periods
                        ),
                    });
                }
                base.powf(-periods * year_fraction)
            }
        };

        if factor.is_finite() {
            Ok(factor)
        } else {
            Err(KurumiError::NumericalOverflow {
                operation: "discount factor",
            })
        }
    }
}

/// A zero-coupon yield curve defined by benchmark pillar dates and zero rates.
///
/// The curve stores zero rates in its own compounding convention and interpolates
/// them linearly in year-fraction space. Pillar dates must not precede the
/// reference date, and their year fractions must be strictly increasing.
///
/// # Examples
///
/// ```
/// use kurumi_core::{Date, DayCountConvention};
/// use kurumi_pricing::{Compounding, ZeroCurve};
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let reference = Date::from_ymd(2024, 1, 1)?;
/// let one_year = Date::from_ymd(2025, 1, 1)?;
/// let two_years = Date::from_ymd(2026, 1, 1)?;
/// let curve = ZeroCurve::new(
///     reference,
///     vec![one_year, two_years],
///     vec![0.04, 0.05],
///     Compounding::Continuous,
///     DayCountConvention::Act365Fixed,
/// )?;
///
/// let df = curve.discount_factor(two_years)?;
/// assert!(df > 0.0 && df < 1.0);
/// assert!((curve.zero_rate(one_year)? - 0.04).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct ZeroCurve {
    reference_date: Date,
    pillars: Vec<Date>,
    rates: Vec<f64>,
    times: Vec<f64>,
    compounding: Compounding,
    day_count: DayCountConvention,
}

impl ZeroCurve {
    /// Creates a zero curve from pillar dates and zero rates.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InsufficientData`] when no pillar is supplied,
    /// [`KurumiError::InvalidParameter`] when the lengths differ, a rate is not
    /// finite, a pillar precedes the reference date, dates or year fractions are
    /// not strictly increasing, or a discrete frequency is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, ZeroCurve};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let reference = Date::from_ymd(2024, 1, 1)?;
    /// let curve = ZeroCurve::new(
    ///     reference,
    ///     vec![Date::from_ymd(2025, 1, 1)?],
    ///     vec![0.03],
    ///     Compounding::semiannual(),
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// assert!((curve.discount_factor(reference)? - 1.0).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        reference_date: Date,
        pillars: Vec<Date>,
        rates: Vec<f64>,
        compounding: Compounding,
        day_count: DayCountConvention,
    ) -> Result<Self, KurumiError> {
        if pillars.is_empty() {
            return Err(KurumiError::InsufficientData {
                required: 1,
                available: 0,
            });
        }
        if pillars.len() != rates.len() {
            return Err(KurumiError::InvalidParameter {
                parameter: "rates",
                reason: "must have the same length as pillars".to_owned(),
            });
        }
        if let Compounding::Discrete { frequency } = compounding {
            if frequency == 0 {
                return Err(KurumiError::InvalidParameter {
                    parameter: "frequency",
                    reason: "must be greater than zero".to_owned(),
                });
            }
        }

        let mut times = Vec::with_capacity(pillars.len());
        let mut previous_date: Option<Date> = None;
        let mut previous_time: Option<f64> = None;
        for (index, pillar) in pillars.iter().enumerate() {
            if *pillar < reference_date {
                return Err(KurumiError::InvalidParameter {
                    parameter: "pillars",
                    reason: format!("pillar {index} precedes the reference date"),
                });
            }
            if let Some(previous) = previous_date {
                if *pillar <= previous {
                    return Err(KurumiError::InvalidParameter {
                        parameter: "pillars",
                        reason: "must be strictly increasing".to_owned(),
                    });
                }
            }
            let time = day_count.year_fraction(reference_date, *pillar);
            if let Some(previous) = previous_time {
                if time <= previous {
                    return Err(KurumiError::InvalidParameter {
                        parameter: "pillars",
                        reason: format!(
                            "year fractions must be strictly increasing; pillar {index} repeats or precedes the previous fraction"
                        ),
                    });
                }
            }
            let rate = rates
                .get(index)
                .copied()
                .ok_or_else(|| KurumiError::InvalidParameter {
                    parameter: "rates",
                    reason: format!("missing rate at index {index}"),
                })?;
            if !rate.is_finite() {
                return Err(KurumiError::InvalidParameter {
                    parameter: "rates",
                    reason: format!("rate at index {index} must be finite"),
                });
            }
            times.push(time);
            previous_date = Some(*pillar);
            previous_time = Some(time);
        }

        Ok(Self {
            reference_date,
            pillars,
            rates,
            times,
            compounding,
            day_count,
        })
    }

    /// Creates a flat curve whose zero rate is the same for every maturity.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `rate` is not finite or a
    /// discrete frequency is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, ZeroCurve};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let reference = Date::from_ymd(2024, 1, 1)?;
    /// let curve = ZeroCurve::flat(
    ///     reference,
    ///     0.05,
    ///     Compounding::Continuous,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// assert!((curve.zero_rate(Date::from_ymd(2034, 1, 1)?)? - 0.05).abs() < 1e-12);
    /// # Ok(())
    /// # }
    /// ```
    pub fn flat(
        reference_date: Date,
        rate: f64,
        compounding: Compounding,
        day_count: DayCountConvention,
    ) -> Result<Self, KurumiError> {
        Self::new(
            reference_date,
            vec![reference_date],
            vec![rate],
            compounding,
            day_count,
        )
    }

    /// Returns the curve's reference (valuation) date.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, ZeroCurve};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let reference = Date::from_ymd(2024, 1, 1)?;
    /// let curve = ZeroCurve::flat(
    ///     reference,
    ///     0.05,
    ///     Compounding::Continuous,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// assert_eq!(curve.reference_date(), reference);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn reference_date(&self) -> Date {
        self.reference_date
    }

    /// Returns the curve's compounding convention.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, ZeroCurve};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let curve = ZeroCurve::flat(
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     0.05,
    ///     Compounding::semiannual(),
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// assert_eq!(curve.compounding(), Compounding::semiannual());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn compounding(&self) -> Compounding {
        self.compounding
    }

    /// Returns the curve's day-count convention.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, ZeroCurve};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let curve = ZeroCurve::flat(
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     0.05,
    ///     Compounding::Continuous,
    ///     DayCountConvention::Thirty360Us,
    /// )?;
    /// assert_eq!(curve.day_count(), DayCountConvention::Thirty360Us);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn day_count(&self) -> DayCountConvention {
        self.day_count
    }

    /// Returns the pillar dates.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, ZeroCurve};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let pillar = Date::from_ymd(2025, 1, 1)?;
    /// let curve = ZeroCurve::new(
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     vec![pillar],
    ///     vec![0.05],
    ///     Compounding::Continuous,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// assert_eq!(curve.pillars(), &[pillar]);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn pillars(&self) -> &[Date] {
        &self.pillars
    }

    /// Returns the zero rates quoted at the pillar dates.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, ZeroCurve};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let curve = ZeroCurve::new(
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     vec![Date::from_ymd(2025, 1, 1)?],
    ///     vec![0.05],
    ///     Compounding::Continuous,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// assert!((curve.rates().first().copied().unwrap_or(0.0) - 0.05).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn rates(&self) -> &[f64] {
        &self.rates
    }

    /// Returns the year fraction from the reference date to `date`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `date` precedes the
    /// reference date.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, ZeroCurve};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let curve = ZeroCurve::flat(
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     0.05,
    ///     Compounding::Continuous,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// assert!((curve.year_fraction(Date::from_ymd(2025, 1, 1)?)? - 366.0 / 365.0).abs() < 1e-12);
    /// # Ok(())
    /// # }
    /// ```
    pub fn year_fraction(&self, date: Date) -> Result<f64, KurumiError> {
        if date < self.reference_date {
            return Err(KurumiError::InvalidParameter {
                parameter: "date",
                reason: "must not precede the curve reference date".to_owned(),
            });
        }
        Ok(self.day_count.year_fraction(self.reference_date, date))
    }

    /// Returns the interpolated zero rate for `date`.
    ///
    /// Rates are interpolated linearly in the year fraction measured from the
    /// reference date. Dates before the first pillar and after the last pillar
    /// use the nearest pillar rate.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `date` precedes the
    /// reference date.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, ZeroCurve};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let reference = Date::from_ymd(2024, 1, 1)?;
    /// let curve = ZeroCurve::new(
    ///     reference,
    ///     vec![Date::from_ymd(2025, 1, 1)?, Date::from_ymd(2026, 1, 1)?],
    ///     vec![0.04, 0.06],
    ///     Compounding::Continuous,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// let rate = curve.zero_rate(Date::from_ymd(2025, 7, 1)?)?;
    /// assert!(rate > 0.04 && rate < 0.06);
    /// # Ok(())
    /// # }
    /// ```
    pub fn zero_rate(&self, date: Date) -> Result<f64, KurumiError> {
        let time = self.year_fraction(date)?;
        Ok(self.rate_at_time(time))
    }

    /// Returns the discount factor from the reference date to `date`.
    ///
    /// The discount factor is `1` at the reference date and positive thereafter.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `date` precedes the
    /// reference date, and [`KurumiError::NumericalOverflow`] when the discount
    /// factor is not representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, ZeroCurve};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let reference = Date::from_ymd(2024, 1, 1)?;
    /// let curve = ZeroCurve::flat(
    ///     reference,
    ///     0.05,
    ///     Compounding::Continuous,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// let df = curve.discount_factor(Date::from_ymd(2025, 1, 1)?)?;
    /// assert!((df - (-0.05_f64 * 366.0 / 365.0).exp()).abs() < 1e-12);
    /// # Ok(())
    /// # }
    /// ```
    pub fn discount_factor(&self, date: Date) -> Result<f64, KurumiError> {
        let time = self.year_fraction(date)?;
        if time <= 0.0 {
            return Ok(1.0);
        }
        let rate = self.rate_at_time(time);
        self.compounding.discount_factor(rate, time)
    }

    /// Returns the forward discount factor from `start` to `end`.
    ///
    /// This is `df(end) / df(start)`, so it can discount a cashflow at `end`
    /// back to `start` even when `start` is later than the curve reference date.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `start` precedes the
    /// reference date or `end` precedes `start`, and
    /// [`KurumiError::NumericalOverflow`] when the ratio is not representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, ZeroCurve};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let reference = Date::from_ymd(2024, 1, 1)?;
    /// let curve = ZeroCurve::flat(
    ///     reference,
    ///     0.05,
    ///     Compounding::Continuous,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// let forward = curve.forward_discount_factor(
    ///     Date::from_ymd(2024, 7, 1)?,
    ///     Date::from_ymd(2025, 1, 1)?,
    /// )?;
    /// assert!(forward > 0.0 && forward < 1.0);
    /// # Ok(())
    /// # }
    /// ```
    pub fn forward_discount_factor(&self, start: Date, end: Date) -> Result<f64, KurumiError> {
        if start < self.reference_date {
            return Err(KurumiError::InvalidParameter {
                parameter: "start",
                reason: "must not precede the curve reference date".to_owned(),
            });
        }
        if end < start {
            return Err(KurumiError::InvalidParameter {
                parameter: "end",
                reason: "must not precede start".to_owned(),
            });
        }
        if end == start {
            return Ok(1.0);
        }
        let start_factor = self.discount_factor(start)?;
        let end_factor = self.discount_factor(end)?;
        if start_factor <= 0.0 {
            return Err(KurumiError::NumericalOverflow {
                operation: "forward discount factor",
            });
        }
        let ratio = end_factor / start_factor;
        if ratio.is_finite() {
            Ok(ratio)
        } else {
            Err(KurumiError::NumericalOverflow {
                operation: "forward discount factor",
            })
        }
    }

    fn rate_at_time(&self, time: f64) -> f64 {
        let first_time = self.times.first().copied().unwrap_or(0.0);
        let first_rate = self.rates.first().copied().unwrap_or(0.0);
        if time <= first_time {
            return first_rate;
        }
        let last_time = self.times.last().copied().unwrap_or(first_time);
        let last_rate = self.rates.last().copied().unwrap_or(first_rate);
        if time >= last_time {
            return last_rate;
        }

        let upper = self.times.partition_point(|probe| *probe <= time);
        let lower = upper.saturating_sub(1);
        let left_time = self.times.get(lower).copied().unwrap_or(first_time);
        let right_time = self.times.get(upper).copied().unwrap_or(last_time);
        let left_rate = self.rates.get(lower).copied().unwrap_or(first_rate);
        let right_rate = self.rates.get(upper).copied().unwrap_or(last_rate);
        let width = right_time - left_time;
        if width <= 0.0 {
            return left_rate;
        }
        let weight = (time - left_time) / width;
        left_rate + weight * (right_rate - left_rate)
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
    fn continuous_discount_factor_matches_the_exponential() {
        let factor = Compounding::Continuous
            .discount_factor(0.05, 2.0)
            .expect("valid inputs");
        assert_relative_eq!(factor, (-0.10_f64).exp(), epsilon = 1e-15);
    }

    #[test]
    fn discrete_discount_factor_compounds_at_the_requested_frequency() {
        let annual = Compounding::annual()
            .discount_factor(0.05, 2.0)
            .expect("valid inputs");
        assert_relative_eq!(annual, 1.05_f64.powi(-2), epsilon = 1e-15);

        let semiannual = Compounding::semiannual()
            .discount_factor(0.05, 2.0)
            .expect("valid inputs");
        assert_relative_eq!(semiannual, 1.025_f64.powi(-4), epsilon = 1e-15);
    }

    #[test]
    fn discrete_compounding_rejects_rates_below_the_lower_bound() {
        let error = Compounding::semiannual()
            .discount_factor(-2.5, 1.0)
            .expect_err("negative base");
        assert!(matches!(error, KurumiError::InvalidParameter { .. }));
        assert!(
            Compounding::Discrete { frequency: 0 }
                .discount_factor(0.05, 1.0)
                .is_err()
        );
    }

    #[test]
    fn discount_factor_rejects_non_finite_inputs() {
        assert!(
            Compounding::Continuous
                .discount_factor(f64::NAN, 1.0)
                .is_err()
        );
        assert!(
            Compounding::Continuous
                .discount_factor(0.05, f64::INFINITY)
                .is_err()
        );
    }

    #[test]
    fn zero_curve_is_normalized_at_the_reference_date() {
        let reference = date(2024, 1, 1);
        let curve = ZeroCurve::flat(
            reference,
            0.05,
            Compounding::Continuous,
            DayCountConvention::Act365Fixed,
        )
        .expect("valid curve");
        assert_relative_eq!(
            curve.discount_factor(reference).expect("reference date"),
            1.0,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            curve.year_fraction(reference).expect("reference date"),
            0.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn flat_curve_discounts_with_the_flat_zero_rate() {
        let reference = date(2024, 1, 1);
        let curve = ZeroCurve::flat(
            reference,
            0.05,
            Compounding::Continuous,
            DayCountConvention::Act365Fixed,
        )
        .expect("valid curve");
        let target = date(2025, 1, 1);
        let time = 366.0 / 365.0;
        assert_relative_eq!(
            curve.discount_factor(target).expect("discount factor"),
            (-0.05_f64 * time).exp(),
            epsilon = 1e-15
        );
    }

    #[test]
    fn discrete_curve_reproduces_the_semiannual_discount_factors() {
        let reference = date(2021, 1, 1);
        let curve = ZeroCurve::flat(
            reference,
            0.10,
            Compounding::semiannual(),
            DayCountConvention::Thirty360Us,
        )
        .expect("valid curve");
        for periods in 1..=6 {
            let target = date(2021 + periods / 2, if periods % 2 == 1 { 7 } else { 1 }, 1);
            let expected = 1.05_f64.powi(-periods);
            assert_relative_eq!(
                curve.discount_factor(target).expect("discount factor"),
                expected,
                epsilon = 1e-15
            );
        }
    }

    #[test]
    fn zero_rates_interpolate_linearly_between_pillars() {
        let reference = date(2024, 1, 1);
        let curve = ZeroCurve::new(
            reference,
            vec![date(2025, 1, 1), date(2026, 1, 1)],
            vec![0.04, 0.06],
            Compounding::Continuous,
            DayCountConvention::Act365Fixed,
        )
        .expect("valid curve");
        let midpoint = date(2025, 7, 2);
        let time = DayCountConvention::Act365Fixed.year_fraction(reference, midpoint);
        let weight = (time - 366.0 / 365.0) / (731.0 / 365.0 - 366.0 / 365.0);
        let expected = 0.04 + weight * 0.02;
        assert_relative_eq!(
            curve.zero_rate(midpoint).expect("interpolated rate"),
            expected,
            epsilon = 1e-12
        );
    }

    #[test]
    fn curve_extends_the_end_pillar_rates_flat() {
        let reference = date(2024, 1, 1);
        let curve = ZeroCurve::new(
            reference,
            vec![date(2025, 1, 1), date(2026, 1, 1)],
            vec![0.04, 0.06],
            Compounding::Continuous,
            DayCountConvention::Act365Fixed,
        )
        .expect("valid curve");
        assert!(curve.zero_rate(date(2023, 1, 1)).is_err());
        assert_relative_eq!(
            curve
                .zero_rate(date(2030, 1, 1))
                .expect("after last pillar"),
            0.06,
            epsilon = 1e-15
        );
    }

    #[test]
    fn forward_discount_factor_divides_the_two_factors() {
        let reference = date(2024, 1, 1);
        let curve = ZeroCurve::flat(
            reference,
            0.05,
            Compounding::Continuous,
            DayCountConvention::Act365Fixed,
        )
        .expect("valid curve");
        let start = date(2024, 7, 1);
        let end = date(2025, 1, 1);
        let expected =
            curve.discount_factor(end).expect("end") / curve.discount_factor(start).expect("start");
        assert_relative_eq!(
            curve.forward_discount_factor(start, end).expect("forward"),
            expected,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            curve
                .forward_discount_factor(start, start)
                .expect("zero interval"),
            1.0,
            epsilon = 1e-15
        );
        assert!(curve.forward_discount_factor(end, start).is_err());
    }

    #[test]
    fn construction_rejects_invalid_pillars_and_rates() {
        let reference = date(2024, 1, 1);
        assert!(matches!(
            ZeroCurve::new(
                reference,
                vec![],
                vec![],
                Compounding::Continuous,
                DayCountConvention::Act365Fixed
            ),
            Err(KurumiError::InsufficientData { .. })
        ));
        assert!(
            ZeroCurve::new(
                reference,
                vec![date(2025, 1, 1)],
                vec![0.05, 0.06],
                Compounding::Continuous,
                DayCountConvention::Act365Fixed
            )
            .is_err()
        );
        assert!(
            ZeroCurve::new(
                reference,
                vec![date(2025, 1, 1)],
                vec![f64::NAN],
                Compounding::Continuous,
                DayCountConvention::Act365Fixed
            )
            .is_err()
        );
        assert!(
            ZeroCurve::new(
                reference,
                vec![date(2023, 1, 1)],
                vec![0.05],
                Compounding::Continuous,
                DayCountConvention::Act365Fixed
            )
            .is_err()
        );
        assert!(
            ZeroCurve::new(
                reference,
                vec![date(2025, 1, 1), date(2025, 1, 1)],
                vec![0.05, 0.06],
                Compounding::Continuous,
                DayCountConvention::Act365Fixed
            )
            .is_err()
        );
        assert!(
            ZeroCurve::new(
                reference,
                vec![date(2025, 1, 1)],
                vec![0.05],
                Compounding::Discrete { frequency: 0 },
                DayCountConvention::Act365Fixed
            )
            .is_err()
        );
    }

    #[test]
    fn dates_before_the_reference_date_are_rejected() {
        let curve = ZeroCurve::flat(
            date(2024, 1, 1),
            0.05,
            Compounding::Continuous,
            DayCountConvention::Act365Fixed,
        )
        .expect("valid curve");
        assert!(curve.discount_factor(date(2023, 12, 31)).is_err());
        assert!(curve.zero_rate(date(2023, 12, 31)).is_err());
        assert!(curve.year_fraction(date(2023, 12, 31)).is_err());
    }
}
