//! Cashflows, discounted cashflow valuation, internal rates of return, and
//! fixed-rate bonds.
//!
//! Cashflow amounts are stored as [`Decimal`] so discrete money values stay
//! exact. Discounting promotes an amount to `f64` only at the final
//! multiplication by a continuous discount factor; the resulting net present
//! value is an `f64`, matching the rest of the pricing layer.
//!
//! The module provides three levels of functionality:
//!
//! - [`discounted_cash_flow`] discounts a cashflow stream with a [`ZeroCurve`];
//! - [`net_present_value`] and [`internal_rate_of_return`] use a single flat
//!   rate and a compounding convention, which is the standard IRR / YTM
//!   formulation;
//! - [`FixedRateBond`] adds coupon schedules, accrued interest, clean and dirty
//!   prices, Macaulay duration, convexity, and yield to maturity.

use crate::traits::{Instrument, PricingEngine, ValuationResult};
use crate::yield_curve::{Compounding, ZeroCurve};
use kurumi_core::{Date, DayCountConvention, KurumiError};
use kurumi_math::{RootFinderConfig, brent};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

/// The maximum number of coupon periods generated for one bond schedule.
const MAX_COUPON_PERIODS: usize = 1_000_000;

/// A dated cashflow amount.
///
/// The amount is an exact decimal; it may be negative to represent an outflow
/// such as an initial investment or a bond purchase.
///
/// # Examples
///
/// ```
/// use kurumi_core::Date;
/// use kurumi_pricing::CashFlow;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let flow = CashFlow::new(Date::from_ymd(2025, 1, 1)?, Decimal::new(1050, 1));
/// assert_eq!(flow.amount, Decimal::new(1050, 1));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CashFlow {
    /// Date on which the amount is paid.
    pub date: Date,
    /// Exact amount paid. Positive for inflows, negative for outflows.
    pub amount: Decimal,
}

impl CashFlow {
    /// Creates a dated cashflow.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    /// use kurumi_pricing::CashFlow;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let flow = CashFlow::new(Date::from_ymd(2025, 1, 1)?, Decimal::ONE);
    /// assert_eq!(flow.amount, Decimal::ONE);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn new(date: Date, amount: Decimal) -> Self {
        Self { date, amount }
    }
}

/// Discounts `cashflows` using the discount factors of `curve`.
///
/// The curve reference date is the valuation date. For a cashflow at time `t_i`
/// with amount `C_i` and curve discount factor `df(t_i)`, the result is
///
/// ```text
/// DCF = sum_i C_i * df(t_i).
/// ```
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for an empty cashflow stream,
/// [`KurumiError::InvalidParameter`] when a cashflow predates the curve
/// reference date or an amount cannot be represented as `f64`, and
/// [`KurumiError::NumericalOverflow`] when the sum is not representable.
///
/// # Examples
///
/// ```
/// use kurumi_core::{Date, DayCountConvention};
/// use kurumi_pricing::{CashFlow, Compounding, ZeroCurve, discounted_cash_flow};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let reference = Date::from_ymd(2024, 1, 1)?;
/// let curve = ZeroCurve::flat(
///     reference,
///     0.05,
///     Compounding::Continuous,
///     DayCountConvention::Act365Fixed,
/// )?;
/// let flows = [
///     CashFlow::new(Date::from_ymd(2025, 1, 1)?, Decimal::from(100)),
/// ];
/// let value = discounted_cash_flow(&flows, &curve)?;
/// assert!(value > 90.0 && value < 100.0);
/// # Ok(())
/// # }
/// ```
pub fn discounted_cash_flow(cashflows: &[CashFlow], curve: &ZeroCurve) -> Result<f64, KurumiError> {
    if cashflows.is_empty() {
        return Err(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        });
    }
    let as_of = curve.reference_date();
    let mut total = 0.0;
    for cashflow in cashflows {
        if cashflow.date < as_of {
            return Err(KurumiError::InvalidParameter {
                parameter: "cashflows",
                reason: "a cashflow precedes the curve reference date".to_owned(),
            });
        }
        let factor = curve.discount_factor(cashflow.date)?;
        let amount = decimal_to_f64(cashflow.amount, "cashflow amount")?;
        let present_value = amount * factor;
        if !present_value.is_finite() {
            return Err(KurumiError::NumericalOverflow {
                operation: "discounted_cash_flow",
            });
        }
        total += present_value;
    }
    if total.is_finite() {
        Ok(total)
    } else {
        Err(KurumiError::NumericalOverflow {
            operation: "discounted_cash_flow",
        })
    }
}

/// Returns the net present value of `cashflows` at a single flat `rate`.
///
/// With time `t_i` measured from `as_of` under `day_count`, the net present
/// value is
///
/// ```text
/// NPV = sum_i C_i * df(rate, t_i),
/// ```
///
/// where `df` is the [`Compounding`] discount factor.
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for an empty cashflow stream,
/// [`KurumiError::InvalidParameter`] when a rate is not finite or a cashflow
/// predates `as_of`, and [`KurumiError::NumericalOverflow`] when a discount
/// factor or the sum is not representable.
///
/// # Examples
///
/// ```
/// use kurumi_core::{Date, DayCountConvention};
/// use kurumi_pricing::{CashFlow, Compounding, net_present_value};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let as_of = Date::from_ymd(2024, 1, 1)?;
/// let flows = [
///     CashFlow::new(as_of, Decimal::from(-100)),
///     CashFlow::new(Date::from_ymd(2025, 1, 1)?, Decimal::from(110)),
/// ];
/// let value = net_present_value(
///     &flows,
///     as_of,
///     0.10,
///     Compounding::annual(),
///     DayCountConvention::Act365Fixed,
/// )?;
/// assert!(value > -5.0 && value < 5.0);
/// # Ok(())
/// # }
/// ```
pub fn net_present_value(
    cashflows: &[CashFlow],
    as_of: Date,
    rate: f64,
    compounding: Compounding,
    day_count: DayCountConvention,
) -> Result<f64, KurumiError> {
    if cashflows.is_empty() {
        return Err(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        });
    }
    if !rate.is_finite() {
        return Err(KurumiError::InvalidParameter {
            parameter: "rate",
            reason: "must be finite".to_owned(),
        });
    }

    let mut total = 0.0;
    for cashflow in cashflows {
        if cashflow.date < as_of {
            return Err(KurumiError::InvalidParameter {
                parameter: "cashflows",
                reason: "a cashflow precedes the valuation date".to_owned(),
            });
        }
        let time = day_count.year_fraction(as_of, cashflow.date);
        let factor = compounding.discount_factor(rate, time)?;
        let amount = decimal_to_f64(cashflow.amount, "cashflow amount")?;
        let present_value = amount * factor;
        if !present_value.is_finite() {
            return Err(KurumiError::NumericalOverflow {
                operation: "net_present_value",
            });
        }
        total += present_value;
    }
    if total.is_finite() {
        Ok(total)
    } else {
        Err(KurumiError::NumericalOverflow {
            operation: "net_present_value",
        })
    }
}

/// Returns the internal rate of return of `cashflows`.
///
/// The IRR is the rate `y` for which
/// [`net_present_value`]`(cashflows, as_of, y, compounding, day_count)` is zero.
/// The search first checks the wide bracket `[-0.9, 10]` (a rate of `10` means
/// 1000% per year); if the endpoints do not straddle zero it scans the bracket
/// for an interior sign change and then expands the upper bound geometrically.
/// The first bracketed root is returned, which is the conventional IRR for a
/// stream with one sign change. The root is refined with
/// [`kurumi_math::brent`] to a bracket width of `1e-12`.
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for an empty cashflow stream,
/// [`KurumiError::InvalidParameter`] for invalid rates or dates, and
/// [`KurumiError::ConvergenceFailed`] when no sign change can be bracketed.
///
/// # Examples
///
/// ```
/// use kurumi_core::{Date, DayCountConvention};
/// use kurumi_pricing::{CashFlow, Compounding, internal_rate_of_return};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let as_of = Date::from_ymd(2023, 1, 1)?;
/// let flows = [
///     CashFlow::new(as_of, Decimal::from(-100)),
///     CashFlow::new(Date::from_ymd(2024, 1, 1)?, Decimal::from(110)),
/// ];
/// let rate = internal_rate_of_return(
///     &flows,
///     as_of,
///     Compounding::annual(),
///     DayCountConvention::Act365Fixed,
/// )?;
/// assert!((rate - 0.10).abs() < 1e-6);
/// # Ok(())
/// # }
/// ```
pub fn internal_rate_of_return(
    cashflows: &[CashFlow],
    as_of: Date,
    compounding: Compounding,
    day_count: DayCountConvention,
) -> Result<f64, KurumiError> {
    if cashflows.is_empty() {
        return Err(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        });
    }
    let config = RootFinderConfig::new(200, 1e-12)?;
    let objective = |rate: f64| -> f64 {
        net_present_value(cashflows, as_of, rate, compounding, day_count).unwrap_or(f64::NAN)
    };

    let lower = -0.9;
    let upper = 10.0;
    let lower_value = net_present_value(cashflows, as_of, lower, compounding, day_count)?;
    let upper_value = net_present_value(cashflows, as_of, upper, compounding, day_count)?;
    if lower_value.abs() <= f64::EPSILON {
        return Ok(lower);
    }
    if upper_value.abs() <= f64::EPSILON {
        return Ok(upper);
    }
    if lower_value.is_sign_positive() != upper_value.is_sign_positive() {
        return brent(objective, lower, upper, config);
    }

    // The endpoints have the same sign, so look for an interior sign change.
    let steps = 2_000_usize;
    let mut previous_rate = lower;
    let mut previous_value = lower_value;
    for step in 1..=steps {
        let fraction = step as f64 / steps as f64;
        let rate = lower + (upper - lower) * fraction;
        let value = net_present_value(cashflows, as_of, rate, compounding, day_count)?;
        if value.abs() <= f64::EPSILON {
            return Ok(rate);
        }
        if value.is_sign_positive() != previous_value.is_sign_positive() {
            return brent(objective, previous_rate, rate, config);
        }
        previous_rate = rate;
        previous_value = value;
    }

    // A very short-dated or high-return stream can have a root above 1000%.
    let mut high = upper;
    let mut high_value = upper_value;
    for _ in 0..20 {
        let next = high * 10.0;
        let next_value = net_present_value(cashflows, as_of, next, compounding, day_count)?;
        if next_value.abs() <= f64::EPSILON {
            return Ok(next);
        }
        if next_value.is_sign_positive() != high_value.is_sign_positive() {
            return brent(objective, high, next, config);
        }
        high = next;
        high_value = next_value;
    }

    Err(KurumiError::ConvergenceFailed {
        method: "internal_rate_of_return",
        iterations: config.max_iterations,
    })
}

/// Returns the yield to maturity of `cashflows`.
///
/// This is the standard fixed-income name for
/// [`internal_rate_of_return`]: the single flat yield that equates the present
/// value of the future cashflows to the current price.
///
/// # Errors
///
/// Returns the same errors as [`internal_rate_of_return`].
///
/// # Examples
///
/// ```
/// use kurumi_core::{Date, DayCountConvention};
/// use kurumi_pricing::{CashFlow, Compounding, yield_to_maturity};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let as_of = Date::from_ymd(2023, 1, 1)?;
/// let flows = [
///     CashFlow::new(as_of, Decimal::from(-100)),
///     CashFlow::new(Date::from_ymd(2024, 1, 1)?, Decimal::from(105)),
/// ];
/// let ytm = yield_to_maturity(
///     &flows,
///     as_of,
///     Compounding::annual(),
///     DayCountConvention::Act365Fixed,
/// )?;
/// assert!((ytm - 0.05).abs() < 1e-6);
/// # Ok(())
/// # }
/// ```
pub fn yield_to_maturity(
    cashflows: &[CashFlow],
    as_of: Date,
    compounding: Compounding,
    day_count: DayCountConvention,
) -> Result<f64, KurumiError> {
    internal_rate_of_return(cashflows, as_of, compounding, day_count)
}

/// A fixed-rate, fixed-maturity bond with a regular coupon schedule.
///
/// The coupon schedule is generated backwards from `maturity` in steps of
/// `12 / frequency` months, so the maturity date is always a coupon date. The
/// coupon paid on each coupon date is
///
/// ```text
/// C = notional * coupon_rate / frequency.
/// ```
///
/// Accrued interest between coupon dates is the fraction of the current coupon
/// period that has elapsed under the bond's day-count convention:
///
/// ```text
/// AI = C * (yf(previous, settlement) / yf(previous, next)).
/// ```
///
/// # Examples
///
/// ```
/// use kurumi_core::{Date, DayCountConvention};
/// use kurumi_pricing::FixedRateBond;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let bond = FixedRateBond::new(
///     Decimal::from(100),
///     0.08,
///     Date::from_ymd(2021, 1, 1)?,
///     Date::from_ymd(2024, 1, 1)?,
///     2,
///     DayCountConvention::Thirty360Us,
/// )?;
/// assert_eq!(bond.coupon_dates().len(), 6);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct FixedRateBond {
    notional: Decimal,
    coupon_rate: f64,
    issue_date: Date,
    maturity: Date,
    frequency: u32,
    day_count: DayCountConvention,
    coupon_dates: Vec<Date>,
}

impl FixedRateBond {
    /// Creates a fixed-rate bond.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the notional is not
    /// positive, the coupon rate is not finite, `frequency` is not in
    /// `1..=12`, `frequency` does not divide 12, the issue date is not before
    /// maturity, or the coupon schedule cannot be generated.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::FixedRateBond;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let bond = FixedRateBond::new(
    ///     Decimal::from(1000),
    ///     0.05,
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     Date::from_ymd(2029, 1, 1)?,
    ///     2,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// assert_eq!(bond.frequency(), 2);
    /// assert!(FixedRateBond::new(
    ///     Decimal::from(1000),
    ///     0.05,
    ///     Date::from_ymd(2029, 1, 1)?,
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     2,
    ///     DayCountConvention::Act365Fixed,
    /// ).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        notional: Decimal,
        coupon_rate: f64,
        issue_date: Date,
        maturity: Date,
        frequency: u32,
        day_count: DayCountConvention,
    ) -> Result<Self, KurumiError> {
        if notional <= Decimal::ZERO {
            return Err(KurumiError::InvalidParameter {
                parameter: "notional",
                reason: "must be strictly positive".to_owned(),
            });
        }
        if !coupon_rate.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "coupon_rate",
                reason: "must be finite".to_owned(),
            });
        }
        if !(1..=12).contains(&frequency) || 12 % frequency != 0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "frequency",
                reason: "must be in 1..=12 and divide 12".to_owned(),
            });
        }
        if issue_date >= maturity {
            return Err(KurumiError::InvalidParameter {
                parameter: "issue_date",
                reason: "must be strictly before the maturity date".to_owned(),
            });
        }

        let coupon_dates = coupon_schedule(issue_date, maturity, frequency)?;
        Ok(Self {
            notional,
            coupon_rate,
            issue_date,
            maturity,
            frequency,
            day_count,
            coupon_dates,
        })
    }

    /// Returns the bond's face value.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::FixedRateBond;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let bond = FixedRateBond::new(
    ///     Decimal::from(100),
    ///     0.05,
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     1,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// assert_eq!(bond.notional(), Decimal::from(100));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn notional(&self) -> Decimal {
        self.notional
    }

    /// Returns the annual coupon rate.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::FixedRateBond;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let bond = FixedRateBond::new(
    ///     Decimal::from(100),
    ///     0.05,
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     1,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// assert!((bond.coupon_rate() - 0.05).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn coupon_rate(&self) -> f64 {
        self.coupon_rate
    }

    /// Returns the issue date.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::FixedRateBond;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let issue = Date::from_ymd(2024, 1, 1)?;
    /// let bond = FixedRateBond::new(
    ///     Decimal::from(100),
    ///     0.05,
    ///     issue,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     1,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// assert_eq!(bond.issue_date(), issue);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn issue_date(&self) -> Date {
        self.issue_date
    }

    /// Returns the maturity date.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::FixedRateBond;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let maturity = Date::from_ymd(2025, 1, 1)?;
    /// let bond = FixedRateBond::new(
    ///     Decimal::from(100),
    ///     0.05,
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     maturity,
    ///     1,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// assert_eq!(bond.maturity(), maturity);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn maturity(&self) -> Date {
        self.maturity
    }

    /// Returns the number of coupon payments per year.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::FixedRateBond;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let bond = FixedRateBond::new(
    ///     Decimal::from(100),
    ///     0.05,
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     2,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// assert_eq!(bond.frequency(), 2);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn frequency(&self) -> u32 {
        self.frequency
    }

    /// Returns the bond's day-count convention.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::FixedRateBond;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let bond = FixedRateBond::new(
    ///     Decimal::from(100),
    ///     0.05,
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     Date::from_ymd(2025, 1, 1)?,
    ///     1,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// assert_eq!(bond.day_count(), DayCountConvention::Act365Fixed);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn day_count(&self) -> DayCountConvention {
        self.day_count
    }

    /// Returns the coupon dates, in ascending order. The last date is the
    /// maturity date.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::FixedRateBond;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let bond = FixedRateBond::new(
    ///     Decimal::from(100),
    ///     0.05,
    ///     Date::from_ymd(2021, 1, 1)?,
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     2,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// assert_eq!(bond.coupon_dates().len(), 6);
    /// assert_eq!(bond.coupon_dates().last(), Some(&Date::from_ymd(2024, 1, 1)?));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn coupon_dates(&self) -> &[Date] {
        &self.coupon_dates
    }

    /// Returns the remaining cashflows as of `settlement`.
    ///
    /// The coupon at maturity is combined with the principal repayment. Coupons
    /// paid on or before `settlement` are excluded.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `settlement` is before the
    /// issue date or on/after maturity, and [`KurumiError::NumericalOverflow`]
    /// when the coupon amount is not representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::FixedRateBond;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let bond = FixedRateBond::new(
    ///     Decimal::from(100),
    ///     0.10,
    ///     Date::from_ymd(2021, 1, 1)?,
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     1,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// let flows = bond.cashflows(Date::from_ymd(2021, 1, 1)?)?;
    /// assert_eq!(flows.len(), 3);
    /// # Ok(())
    /// # }
    /// ```
    pub fn cashflows(&self, settlement: Date) -> Result<Vec<CashFlow>, KurumiError> {
        self.validate_settlement(settlement)?;
        let coupon = self.coupon_amount()?;
        let mut flows = Vec::with_capacity(self.coupon_dates.len());
        for date in &self.coupon_dates {
            if *date <= settlement {
                continue;
            }
            let amount = if *date == self.maturity {
                coupon
                    .checked_add(self.notional)
                    .ok_or(KurumiError::NumericalOverflow {
                        operation: "FixedRateBond final cashflow",
                    })?
            } else {
                coupon
            };
            flows.push(CashFlow::new(*date, amount));
        }
        Ok(flows)
    }

    /// Returns the accrued interest as of `settlement`.
    ///
    /// The accrued amount is the fraction of the coupon period elapsed, scaled
    /// by the coupon:
    ///
    /// ```text
    /// AI = C * (yf(previous_coupon, settlement) / yf(previous_coupon, next_coupon)).
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `settlement` is outside
    /// `[issue_date, maturity)` or the day-count formula produces a non-positive
    /// coupon period, and [`KurumiError::NumericalOverflow`] when the accrued
    /// amount is not representable as `f64`.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::FixedRateBond;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let bond = FixedRateBond::new(
    ///     Decimal::from(100),
    ///     0.08,
    ///     Date::from_ymd(2021, 1, 1)?,
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     2,
    ///     DayCountConvention::Thirty360Us,
    /// )?;
    /// let accrued = bond.accrued_interest(Date::from_ymd(2021, 4, 1)?)?;
    /// assert!((accrued - 2.0).abs() < 1e-12);
    /// # Ok(())
    /// # }
    /// ```
    pub fn accrued_interest(&self, settlement: Date) -> Result<f64, KurumiError> {
        self.validate_settlement(settlement)?;
        let coupon = decimal_to_f64(self.coupon_amount()?, "coupon amount")?;
        let previous = self
            .coupon_dates
            .iter()
            .rev()
            .find(|date| **date <= settlement)
            .copied()
            .unwrap_or(self.issue_date);
        let next = self
            .coupon_dates
            .iter()
            .find(|date| **date > settlement)
            .copied()
            .ok_or_else(|| KurumiError::InvalidParameter {
                parameter: "settlement",
                reason: "no coupon date follows the settlement date".to_owned(),
            })?;
        let accrued_fraction = self.day_count.year_fraction(previous, settlement);
        let period_fraction = self.day_count.year_fraction(previous, next);
        if !period_fraction.is_finite() || period_fraction <= 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "day_count",
                reason: "the coupon period has a non-positive year fraction".to_owned(),
            });
        }
        let accrued = coupon * (accrued_fraction / period_fraction);
        if accrued.is_finite() {
            Ok(accrued)
        } else {
            Err(KurumiError::NumericalOverflow {
                operation: "FixedRateBond accrued interest",
            })
        }
    }

    /// Returns the dirty (full) price as of `settlement`.
    ///
    /// The dirty price is the present value of the remaining cashflows, including
    /// the coupon accrued to the settlement date. Forward discount factors are
    /// rebased to `settlement`, so the curve reference date may precede it.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `settlement` is invalid,
    /// the curve reference date is after `settlement`, or a cashflow amount is
    /// not representable, and [`KurumiError::NumericalOverflow`] when the price
    /// is not representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, FixedRateBond, ZeroCurve};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let issue = Date::from_ymd(2021, 1, 1)?;
    /// let bond = FixedRateBond::new(
    ///     Decimal::from(100),
    ///     0.10,
    ///     issue,
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     1,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// let curve = ZeroCurve::flat(
    ///     issue,
    ///     0.10,
    ///     Compounding::Continuous,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// let dirty = bond.dirty_price(&curve, issue)?;
    /// assert!((dirty - 98.725_685_986_128_38).abs() < 1e-6);
    /// # Ok(())
    /// # }
    /// ```
    pub fn dirty_price(&self, curve: &ZeroCurve, settlement: Date) -> Result<f64, KurumiError> {
        let cashflows = self.cashflows(settlement)?;
        let mut price = 0.0;
        for cashflow in &cashflows {
            let factor = curve.forward_discount_factor(settlement, cashflow.date)?;
            let amount = decimal_to_f64(cashflow.amount, "cashflow amount")?;
            let present_value = amount * factor;
            if !present_value.is_finite() {
                return Err(KurumiError::NumericalOverflow {
                    operation: "FixedRateBond dirty price",
                });
            }
            price += present_value;
        }
        if price.is_finite() {
            Ok(price)
        } else {
            Err(KurumiError::NumericalOverflow {
                operation: "FixedRateBond dirty price",
            })
        }
    }

    /// Returns the clean price as of `settlement`.
    ///
    /// The clean price is the dirty price minus the accrued interest:
    /// `clean = dirty - AI`.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`FixedRateBond::dirty_price`] and
    /// [`FixedRateBond::accrued_interest`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, FixedRateBond, ZeroCurve};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let issue = Date::from_ymd(2021, 1, 1)?;
    /// let bond = FixedRateBond::new(
    ///     Decimal::from(100),
    ///     0.10,
    ///     issue,
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     1,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// let curve = ZeroCurve::flat(
    ///     issue,
    ///     0.10,
    ///     Compounding::Continuous,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// let settlement = Date::from_ymd(2021, 7, 1)?;
    /// let clean = bond.clean_price(&curve, settlement)?;
    /// let dirty = bond.dirty_price(&curve, settlement)?;
    /// let accrued = bond.accrued_interest(settlement)?;
    /// assert!((dirty - accrued - clean).abs() < 1e-12);
    /// # Ok(())
    /// # }
    /// ```
    pub fn clean_price(&self, curve: &ZeroCurve, settlement: Date) -> Result<f64, KurumiError> {
        let dirty = self.dirty_price(curve, settlement)?;
        let accrued = self.accrued_interest(settlement)?;
        let clean = dirty - accrued;
        if clean.is_finite() {
            Ok(clean)
        } else {
            Err(KurumiError::NumericalOverflow {
                operation: "FixedRateBond clean price",
            })
        }
    }

    /// Returns the Macaulay duration as of `settlement`.
    ///
    /// With `PV_i = C_i * df(settlement, t_i)` and dirty price `P`,
    ///
    /// ```text
    /// D = sum_i t_i * PV_i / P,
    /// ```
    ///
    /// where `t_i` is measured in years from `settlement` under the bond's
    /// day-count convention. A parallel shift of continuously compounded zero
    /// rates changes the price approximately by `-D * P * dy`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `settlement` is invalid or
    /// the dirty price is not positive, and [`KurumiError::NumericalOverflow`]
    /// when a weighted sum is not representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, FixedRateBond, ZeroCurve};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let issue = Date::from_ymd(2021, 1, 1)?;
    /// let bond = FixedRateBond::new(
    ///     Decimal::from(100),
    ///     0.10,
    ///     issue,
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     2,
    ///     DayCountConvention::Thirty360Us,
    /// )?;
    /// let curve = ZeroCurve::flat(
    ///     issue,
    ///     0.12,
    ///     Compounding::Continuous,
    ///     DayCountConvention::Thirty360Us,
    /// )?;
    /// let duration = bond.macaulay_duration(&curve, issue)?;
    /// // Hull's textbook example: 2.653 years.
    /// assert!((duration - 2.653).abs() < 1e-2);
    /// # Ok(())
    /// # }
    /// ```
    pub fn macaulay_duration(
        &self,
        curve: &ZeroCurve,
        settlement: Date,
    ) -> Result<f64, KurumiError> {
        let (price, time_weighted) = self.price_and_time_weights(curve, settlement, false)?;
        if price <= 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "curve",
                reason: "the dirty price must be positive to compute duration".to_owned(),
            });
        }
        let duration = time_weighted / price;
        if duration.is_finite() {
            Ok(duration)
        } else {
            Err(KurumiError::NumericalOverflow {
                operation: "FixedRateBond Macaulay duration",
            })
        }
    }

    /// Returns the convexity as of `settlement`.
    ///
    /// With `PV_i = C_i * df(settlement, t_i)` and dirty price `P`,
    ///
    /// ```text
    /// C = sum_i t_i^2 * PV_i / P.
    /// ```
    ///
    /// This is the second derivative of the price with respect to a parallel
    /// shift of continuously compounded zero rates, divided by the price:
    /// `(1 / P) * d2P / dy2`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `settlement` is invalid or
    /// the dirty price is not positive, and [`KurumiError::NumericalOverflow`]
    /// when a weighted sum is not representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, FixedRateBond, ZeroCurve};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let issue = Date::from_ymd(2021, 1, 1)?;
    /// let bond = FixedRateBond::new(
    ///     Decimal::from(100),
    ///     0.10,
    ///     issue,
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     2,
    ///     DayCountConvention::Thirty360Us,
    /// )?;
    /// let curve = ZeroCurve::flat(
    ///     issue,
    ///     0.12,
    ///     Compounding::Continuous,
    ///     DayCountConvention::Thirty360Us,
    /// )?;
    /// let convexity = bond.convexity(&curve, issue)?;
    /// assert!(convexity > 0.0);
    /// # Ok(())
    /// # }
    /// ```
    pub fn convexity(&self, curve: &ZeroCurve, settlement: Date) -> Result<f64, KurumiError> {
        let (price, squared_weighted) = self.price_and_time_weights(curve, settlement, true)?;
        if price <= 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "curve",
                reason: "the dirty price must be positive to compute convexity".to_owned(),
            });
        }
        let convexity = squared_weighted / price;
        if convexity.is_finite() {
            Ok(convexity)
        } else {
            Err(KurumiError::NumericalOverflow {
                operation: "FixedRateBond convexity",
            })
        }
    }

    /// Returns the yield to maturity implied by a dirty price.
    ///
    /// The yield is the single rate `y`, compounded `frequency` times per year,
    /// that satisfies
    ///
    /// ```text
    /// dirty_price = sum_i C_i / (1 + y / frequency)^(frequency * t_i).
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `settlement` is invalid or
    /// `dirty_price` is not finite and positive, and propagates the errors of
    /// [`internal_rate_of_return`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, FixedRateBond, ZeroCurve};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let issue = Date::from_ymd(2021, 1, 1)?;
    /// let bond = FixedRateBond::new(
    ///     Decimal::from(100),
    ///     0.10,
    ///     issue,
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     2,
    ///     DayCountConvention::Thirty360Us,
    /// )?;
    /// let curve = ZeroCurve::flat(
    ///     issue,
    ///     0.10,
    ///     Compounding::semiannual(),
    ///     DayCountConvention::Thirty360Us,
    /// )?;
    /// let dirty = bond.dirty_price(&curve, issue)?;
    /// let ytm = bond.yield_to_maturity(dirty, issue)?;
    /// assert!((ytm - 0.10).abs() < 1e-6);
    /// # Ok(())
    /// # }
    /// ```
    pub fn yield_to_maturity(
        &self,
        dirty_price: f64,
        settlement: Date,
    ) -> Result<f64, KurumiError> {
        if !dirty_price.is_finite() || dirty_price <= 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "dirty_price",
                reason: "must be finite and strictly positive".to_owned(),
            });
        }
        self.validate_settlement(settlement)?;
        let price = Decimal::try_from(dirty_price).map_err(|_| KurumiError::InvalidParameter {
            parameter: "dirty_price",
            reason: "must be representable as a decimal".to_owned(),
        })?;
        let outflow = Decimal::ZERO
            .checked_sub(price)
            .ok_or(KurumiError::NumericalOverflow {
                operation: "FixedRateBond yield_to_maturity outflow",
            })?;
        let mut cashflows = Vec::with_capacity(self.coupon_dates.len() + 1);
        cashflows.push(CashFlow::new(settlement, outflow));
        cashflows.extend(self.cashflows(settlement)?);
        internal_rate_of_return(
            &cashflows,
            settlement,
            Compounding::Discrete {
                frequency: self.frequency,
            },
            self.day_count,
        )
    }

    fn validate_settlement(&self, settlement: Date) -> Result<(), KurumiError> {
        if settlement < self.issue_date {
            return Err(KurumiError::InvalidParameter {
                parameter: "settlement",
                reason: "must not precede the issue date".to_owned(),
            });
        }
        if settlement >= self.maturity {
            return Err(KurumiError::InvalidParameter {
                parameter: "settlement",
                reason: "must precede the maturity date".to_owned(),
            });
        }
        Ok(())
    }

    fn coupon_amount(&self) -> Result<Decimal, KurumiError> {
        let rate =
            Decimal::try_from(self.coupon_rate).map_err(|_| KurumiError::InvalidParameter {
                parameter: "coupon_rate",
                reason: "must be representable as a decimal".to_owned(),
            })?;
        let periods = Decimal::from(self.frequency);
        let product = self
            .notional
            .checked_mul(rate)
            .ok_or(KurumiError::NumericalOverflow {
                operation: "FixedRateBond coupon amount",
            })?;
        product
            .checked_div(periods)
            .ok_or(KurumiError::NumericalOverflow {
                operation: "FixedRateBond coupon amount",
            })
    }

    fn price_and_time_weights(
        &self,
        curve: &ZeroCurve,
        settlement: Date,
        squared: bool,
    ) -> Result<(f64, f64), KurumiError> {
        let cashflows = self.cashflows(settlement)?;
        let mut price = 0.0;
        let mut weighted = 0.0;
        for cashflow in &cashflows {
            let factor = curve.forward_discount_factor(settlement, cashflow.date)?;
            let amount = decimal_to_f64(cashflow.amount, "cashflow amount")?;
            let present_value = amount * factor;
            let time = self.day_count.year_fraction(settlement, cashflow.date);
            if !present_value.is_finite() || !time.is_finite() {
                return Err(KurumiError::NumericalOverflow {
                    operation: "FixedRateBond price weights",
                });
            }
            price += present_value;
            let weight = if squared { time * time } else { time };
            weighted += weight * present_value;
        }
        if !price.is_finite() || !weighted.is_finite() {
            return Err(KurumiError::NumericalOverflow {
                operation: "FixedRateBond price weights",
            });
        }
        Ok((price, weighted))
    }
}

impl Instrument for FixedRateBond {
    fn maturity(&self) -> Date {
        self.maturity
    }
}

/// A [`PricingEngine`] that discounts a bond's cashflows with a [`ZeroCurve`].
///
/// The engine owns the curve; the bond supplies its cashflow schedule. The
/// valuation date passed to [`PricingEngine::price`] is the settlement date.
///
/// # Examples
///
/// ```
/// use kurumi_core::{Date, DayCountConvention};
/// use kurumi_pricing::{Compounding, DiscountedCashFlowEngine, FixedRateBond, PricingEngine, ZeroCurve};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let issue = Date::from_ymd(2021, 1, 1)?;
/// let bond = FixedRateBond::new(
///     Decimal::from(100),
///     0.10,
///     issue,
///     Date::from_ymd(2024, 1, 1)?,
///     2,
///     DayCountConvention::Thirty360Us,
/// )?;
/// let curve = ZeroCurve::flat(
///     issue,
///     0.12,
///     Compounding::Continuous,
///     DayCountConvention::Thirty360Us,
/// )?;
/// let engine = DiscountedCashFlowEngine::new(curve);
/// let result = engine.price(&bond, issue)?;
/// assert!((result.npv - 94.213).abs() < 1e-2);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct DiscountedCashFlowEngine {
    curve: ZeroCurve,
}

impl DiscountedCashFlowEngine {
    /// Creates an engine backed by `curve`.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, DiscountedCashFlowEngine, ZeroCurve};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let reference = Date::from_ymd(2024, 1, 1)?;
    /// let curve = ZeroCurve::flat(
    ///     reference,
    ///     0.05,
    ///     Compounding::Continuous,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// let engine = DiscountedCashFlowEngine::new(curve);
    /// assert_eq!(engine.curve().reference_date(), reference);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn new(curve: ZeroCurve) -> Self {
        Self { curve }
    }

    /// Returns the curve used by the engine.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    /// use kurumi_pricing::{Compounding, DiscountedCashFlowEngine, ZeroCurve};
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let curve = ZeroCurve::flat(
    ///     Date::from_ymd(2024, 1, 1)?,
    ///     0.05,
    ///     Compounding::Continuous,
    ///     DayCountConvention::Act365Fixed,
    /// )?;
    /// let engine = DiscountedCashFlowEngine::new(curve);
    /// assert_eq!(engine.curve().compounding(), Compounding::Continuous);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn curve(&self) -> &ZeroCurve {
        &self.curve
    }
}

impl PricingEngine<FixedRateBond> for DiscountedCashFlowEngine {
    fn price(
        &self,
        instrument: &FixedRateBond,
        as_of: Date,
    ) -> Result<ValuationResult, KurumiError> {
        let npv = instrument.dirty_price(&self.curve, as_of)?;
        ValuationResult::new(npv)
    }
}

fn decimal_to_f64(value: Decimal, parameter: &'static str) -> Result<f64, KurumiError> {
    value.to_f64().ok_or_else(|| KurumiError::InvalidParameter {
        parameter,
        reason: "must be representable as an f64".to_owned(),
    })
}

fn coupon_schedule(
    issue_date: Date,
    maturity: Date,
    frequency: u32,
) -> Result<Vec<Date>, KurumiError> {
    let step_months = i64::from(12 / frequency);
    let mut dates = Vec::new();
    let mut reached_issue = false;
    for period in 0..MAX_COUPON_PERIODS {
        let offset = i64::try_from(period)
            .ok()
            .and_then(|value| value.checked_mul(step_months))
            .and_then(|value| value.checked_neg())
            .ok_or(KurumiError::NumericalOverflow {
                operation: "FixedRateBond coupon schedule",
            })?;
        let coupon_date = add_months(maturity, offset)?;
        if coupon_date <= issue_date {
            reached_issue = true;
            break;
        }
        dates.push(coupon_date);
    }
    if !reached_issue {
        return Err(KurumiError::InvalidParameter {
            parameter: "maturity",
            reason: format!("coupon schedule exceeds {MAX_COUPON_PERIODS} periods"),
        });
    }
    dates.reverse();
    if dates.is_empty() {
        return Err(KurumiError::InvalidParameter {
            parameter: "maturity",
            reason: "must be after the issue date".to_owned(),
        });
    }
    Ok(dates)
}

fn add_months(date: Date, months: i64) -> Result<Date, KurumiError> {
    let total_months = i64::from(date.year())
        .checked_mul(12)
        .and_then(|value| value.checked_add(i64::from(date.month()) - 1))
        .and_then(|value| value.checked_add(months))
        .ok_or(KurumiError::DateError {
            reason: "month arithmetic overflowed".to_owned(),
        })?;
    let year = total_months.div_euclid(12);
    let month = total_months.rem_euclid(12) + 1;
    if !(1..=9999).contains(&year) {
        return Err(KurumiError::DateError {
            reason: format!("resulting year {year} is outside 1..=9999"),
        });
    }
    let year = i32::try_from(year).map_err(|_| KurumiError::DateError {
        reason: "resulting year is not representable".to_owned(),
    })?;
    let month = u32::try_from(month).map_err(|_| KurumiError::DateError {
        reason: "resulting month is not representable".to_owned(),
    })?;
    let last_day = Date::from_ymd(year, month, 1)?.days_in_month();
    let day = date.day().min(last_day);
    Date::from_ymd(year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn date(year: i32, month: u32, day: u32) -> Date {
        Date::from_ymd(year, month, day).expect("valid date")
    }

    fn decimal(value: i64, scale: u32) -> Decimal {
        Decimal::new(value, scale)
    }

    fn hull_bond() -> FixedRateBond {
        // Hull, Options, Futures, and Other Derivatives, Chapter 4: a 3-year
        // 10% coupon bond with semiannual coupons and a 12% continuously
        // compounded yield.
        FixedRateBond::new(
            decimal(100, 0),
            0.10,
            date(2021, 1, 1),
            date(2024, 1, 1),
            2,
            DayCountConvention::Thirty360Us,
        )
        .expect("valid bond")
    }

    fn hull_curve() -> ZeroCurve {
        ZeroCurve::flat(
            date(2021, 1, 1),
            0.12,
            Compounding::Continuous,
            DayCountConvention::Thirty360Us,
        )
        .expect("valid curve")
    }

    #[test]
    fn hull_bond_price_duration_and_convexity_match_the_textbook() {
        let bond = hull_bond();
        let curve = hull_curve();
        let settlement = date(2021, 1, 1);

        // Hull (2018) gives a price of 94.213 and a duration of 2.653 years.
        assert_relative_eq!(
            bond.dirty_price(&curve, settlement).expect("dirty price"),
            94.213_020_554_763,
            epsilon = 1e-9
        );
        assert_relative_eq!(
            bond.macaulay_duration(&curve, settlement)
                .expect("duration"),
            2.653_010_037_390_808,
            epsilon = 1e-12
        );
        assert_relative_eq!(
            bond.convexity(&curve, settlement).expect("convexity"),
            7.570_034_887_790_739,
            epsilon = 1e-12
        );
    }

    #[test]
    fn coupon_dates_are_anchored_on_maturity() {
        let bond = hull_bond();
        assert_eq!(
            bond.coupon_dates(),
            &[
                date(2021, 7, 1),
                date(2022, 1, 1),
                date(2022, 7, 1),
                date(2023, 1, 1),
                date(2023, 7, 1),
                date(2024, 1, 1),
            ]
        );
    }

    #[test]
    fn accrued_interest_is_the_elapsed_coupon_fraction() {
        let bond = FixedRateBond::new(
            decimal(100, 0),
            0.08,
            date(2021, 1, 1),
            date(2024, 1, 1),
            2,
            DayCountConvention::Thirty360Us,
        )
        .expect("valid bond");
        assert_relative_eq!(
            bond.accrued_interest(date(2021, 4, 1))
                .expect("accrued interest"),
            2.0,
            epsilon = 1e-12
        );
        assert_relative_eq!(
            bond.accrued_interest(date(2021, 7, 1))
                .expect("coupon date"),
            0.0,
            epsilon = 1e-12
        );
    }

    #[test]
    fn clean_price_is_dirty_price_minus_accrued_interest() {
        let bond = hull_bond();
        let curve = hull_curve();
        let settlement = date(2021, 4, 1);
        let dirty = bond.dirty_price(&curve, settlement).expect("dirty");
        let clean = bond.clean_price(&curve, settlement).expect("clean");
        let accrued = bond.accrued_interest(settlement).expect("accrued");
        assert_relative_eq!(dirty - accrued, clean, epsilon = 1e-12);
        assert!(accrued > 0.0);
    }

    #[test]
    fn discrete_yield_curve_reproduces_the_semiannual_bond_price() {
        let bond = FixedRateBond::new(
            decimal(100, 0),
            0.08,
            date(2021, 1, 1),
            date(2024, 1, 1),
            2,
            DayCountConvention::Thirty360Us,
        )
        .expect("valid bond");
        let curve = ZeroCurve::flat(
            date(2021, 1, 1),
            0.10,
            Compounding::semiannual(),
            DayCountConvention::Thirty360Us,
        )
        .expect("valid curve");
        let settlement = date(2021, 1, 1);
        assert_relative_eq!(
            bond.dirty_price(&curve, settlement).expect("dirty price"),
            94.924_307_932_732_52,
            epsilon = 1e-12
        );
        assert_relative_eq!(
            bond.macaulay_duration(&curve, settlement)
                .expect("duration"),
            2.717_449_262_903_515_6,
            epsilon = 1e-12
        );
    }

    #[test]
    fn bond_yield_to_maturity_recovers_the_curve_yield() {
        let bond = hull_bond();
        let curve = ZeroCurve::flat(
            date(2021, 1, 1),
            0.12,
            Compounding::semiannual(),
            DayCountConvention::Thirty360Us,
        )
        .expect("valid curve");
        let settlement = date(2021, 1, 1);
        let dirty = bond.dirty_price(&curve, settlement).expect("dirty");
        let ytm = bond.yield_to_maturity(dirty, settlement).expect("ytm");
        assert_relative_eq!(ytm, 0.12, epsilon = 1e-6);
    }

    #[test]
    fn generic_irr_recovers_a_single_period_rate() {
        let as_of = date(2024, 1, 1);
        let flows = [
            CashFlow::new(as_of, decimal(-100, 0)),
            CashFlow::new(date(2025, 1, 1), decimal(110, 0)),
        ];
        let irr = internal_rate_of_return(
            &flows,
            as_of,
            Compounding::annual(),
            DayCountConvention::Act365Fixed,
        )
        .expect("irr");
        // 2024 is a leap year, so the fraction is 366/365 and the annual rate
        // is (1.10)^(365/366) - 1.
        let expected = 1.10_f64.powf(365.0 / 366.0) - 1.0;
        assert_relative_eq!(irr, expected, epsilon = 1e-9);
    }

    #[test]
    fn generic_irr_matches_the_npv_root() {
        let as_of = date(2024, 1, 1);
        let flows = [
            CashFlow::new(as_of, decimal(-1000, 0)),
            CashFlow::new(date(2025, 1, 1), decimal(400, 0)),
            CashFlow::new(date(2026, 1, 1), decimal(400, 0)),
            CashFlow::new(date(2027, 1, 1), decimal(400, 0)),
        ];
        let irr = internal_rate_of_return(
            &flows,
            as_of,
            Compounding::annual(),
            DayCountConvention::Act365Fixed,
        )
        .expect("irr");
        let residual = net_present_value(
            &flows,
            as_of,
            irr,
            Compounding::annual(),
            DayCountConvention::Act365Fixed,
        )
        .expect("npv");
        assert!(residual.abs() < 1e-8);
    }

    #[test]
    fn npv_uses_the_requested_compounding() {
        let as_of = date(2021, 1, 1);
        let flows = [CashFlow::new(date(2022, 1, 1), decimal(105, 0))];
        let npv = net_present_value(
            &flows,
            as_of,
            0.05,
            Compounding::annual(),
            DayCountConvention::Act365Fixed,
        )
        .expect("npv");
        assert_relative_eq!(npv, 100.0, epsilon = 1e-12);
    }

    #[test]
    fn discounted_cash_flow_matches_the_curve_product() {
        let reference = date(2024, 1, 1);
        let curve = ZeroCurve::flat(
            reference,
            0.05,
            Compounding::Continuous,
            DayCountConvention::Act365Fixed,
        )
        .expect("valid curve");
        let target = date(2025, 1, 1);
        let flows = [CashFlow::new(target, decimal(100, 0))];
        let value = discounted_cash_flow(&flows, &curve).expect("dcf");
        let expected = 100.0 * curve.discount_factor(target).expect("discount factor");
        assert_relative_eq!(value, expected, epsilon = 1e-12);
    }

    #[test]
    fn coupon_schedule_clamps_month_ends() {
        let bond = FixedRateBond::new(
            decimal(100, 0),
            0.05,
            date(2024, 2, 29),
            date(2025, 8, 31),
            2,
            DayCountConvention::Act365Fixed,
        )
        .expect("valid bond");
        assert_eq!(
            bond.coupon_dates(),
            &[date(2024, 8, 31), date(2025, 2, 28), date(2025, 8, 31)]
        );
    }

    #[test]
    fn construction_rejects_invalid_bonds() {
        assert!(
            FixedRateBond::new(
                Decimal::ZERO,
                0.05,
                date(2024, 1, 1),
                date(2025, 1, 1),
                2,
                DayCountConvention::Act365Fixed
            )
            .is_err()
        );
        assert!(
            FixedRateBond::new(
                decimal(100, 0),
                f64::NAN,
                date(2024, 1, 1),
                date(2025, 1, 1),
                2,
                DayCountConvention::Act365Fixed
            )
            .is_err()
        );
        assert!(
            FixedRateBond::new(
                decimal(100, 0),
                0.05,
                date(2024, 1, 1),
                date(2025, 1, 1),
                5,
                DayCountConvention::Act365Fixed
            )
            .is_err()
        );
        assert!(
            FixedRateBond::new(
                decimal(100, 0),
                0.05,
                date(2025, 1, 1),
                date(2024, 1, 1),
                2,
                DayCountConvention::Act365Fixed
            )
            .is_err()
        );
    }

    #[test]
    fn settlement_outside_the_bond_life_is_rejected() {
        let bond = hull_bond();
        let curve = hull_curve();
        assert!(bond.dirty_price(&curve, date(2020, 12, 31)).is_err());
        assert!(bond.dirty_price(&curve, date(2024, 1, 1)).is_err());
        assert!(bond.accrued_interest(date(2020, 12, 31)).is_err());
        assert!(bond.accrued_interest(date(2024, 1, 1)).is_err());
    }

    #[test]
    fn empty_or_past_cashflows_are_rejected() {
        let as_of = date(2024, 1, 1);
        assert!(matches!(
            net_present_value(
                &[],
                as_of,
                0.05,
                Compounding::annual(),
                DayCountConvention::Act365Fixed
            ),
            Err(KurumiError::InsufficientData { .. })
        ));
        let past = [CashFlow::new(date(2023, 1, 1), decimal(100, 0))];
        assert!(
            net_present_value(
                &past,
                as_of,
                0.05,
                Compounding::annual(),
                DayCountConvention::Act365Fixed
            )
            .is_err()
        );
    }

    #[test]
    fn engine_price_matches_the_direct_dirty_price() {
        let bond = hull_bond();
        let curve = hull_curve();
        let settlement = date(2021, 4, 1);
        let engine = DiscountedCashFlowEngine::new(curve);
        let result = engine.price(&bond, settlement).expect("engine price");
        assert_relative_eq!(
            result.npv,
            bond.dirty_price(engine.curve(), settlement).expect("dirty"),
            epsilon = 1e-12
        );
        assert!(result.greeks.is_none());
    }

    #[test]
    fn instrument_maturity_is_the_bond_maturity() {
        let bond = hull_bond();
        assert_eq!(Instrument::maturity(&bond), date(2024, 1, 1));
        assert!(bond.is_expired(date(2024, 1, 1)));
    }
}
