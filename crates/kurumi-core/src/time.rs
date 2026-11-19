//! Calendar dates, business-day conventions, and day-count conventions.
//!
//! The date type is a compact proleptic Gregorian calendar date stored as a
//! signed day count from the Unix epoch (1970-01-01). It supports the range
//! 0001-01-01 through 9999-12-31 without external dependencies, so date
//! arithmetic is exact and deterministic.
//!
//! Day-count conventions convert a date interval into a year fraction. They are
//! dimensionless and used to annualize interest accrual and discounting.

use crate::error::KurumiError;
use std::fmt;

const MIN_YEAR: i32 = 1;
const MAX_YEAR: i32 = 9999;

/// A day of the week.
///
/// # Examples
///
/// ```
/// use kurumi_core::{Date, Weekday};
///
/// # fn main() -> Result<(), kurumi_core::KurumiError> {
/// let monday = Date::from_ymd(2024, 1, 1)?;
/// assert_eq!(monday.weekday(), Weekday::Monday);
/// assert!(!monday.weekday().is_weekend());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Weekday {
    /// Monday.
    Monday,
    /// Tuesday.
    Tuesday,
    /// Wednesday.
    Wednesday,
    /// Thursday.
    Thursday,
    /// Friday.
    Friday,
    /// Saturday.
    Saturday,
    /// Sunday.
    Sunday,
}

impl Weekday {
    /// Returns the ISO 8601 weekday number, where Monday is 1 and Sunday is 7.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Weekday;
    ///
    /// assert_eq!(Weekday::Monday.number_from_monday(), 1);
    /// assert_eq!(Weekday::Sunday.number_from_monday(), 7);
    /// ```
    #[must_use]
    pub const fn number_from_monday(self) -> u8 {
        match self {
            Self::Monday => 1,
            Self::Tuesday => 2,
            Self::Wednesday => 3,
            Self::Thursday => 4,
            Self::Friday => 5,
            Self::Saturday => 6,
            Self::Sunday => 7,
        }
    }

    /// Returns `true` for Saturday and Sunday.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Weekday;
    ///
    /// assert!(Weekday::Saturday.is_weekend());
    /// assert!(!Weekday::Friday.is_weekend());
    /// ```
    #[must_use]
    pub const fn is_weekend(self) -> bool {
        matches!(self, Self::Saturday | Self::Sunday)
    }
}

impl fmt::Display for Weekday {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Monday => "Monday",
            Self::Tuesday => "Tuesday",
            Self::Wednesday => "Wednesday",
            Self::Thursday => "Thursday",
            Self::Friday => "Friday",
            Self::Saturday => "Saturday",
            Self::Sunday => "Sunday",
        })
    }
}

/// A proleptic Gregorian calendar date in the range 0001-01-01 to 9999-12-31.
///
/// Dates are stored as a signed number of days from 1970-01-01, which makes
/// ordering, differences, and weekday arithmetic cheap and exact. Invalid
/// calendar dates can never be constructed.
///
/// # Examples
///
/// ```
/// use kurumi_core::{Date, Weekday};
///
/// # fn main() -> Result<(), kurumi_core::KurumiError> {
/// let date = Date::from_ymd(2024, 2, 29)?;
/// assert_eq!((date.year(), date.month(), date.day()), (2024, 2, 29));
/// assert_eq!(date.weekday(), Weekday::Thursday);
/// assert_eq!(date.checked_add_days(1)?.to_string(), "2024-03-01");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Date {
    days: i32,
}

impl Date {
    /// Creates a date from a year, month, and day.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::DateError`] when the year is outside
    /// `1..=9999`, the month is outside `1..=12`, or the day does not exist in
    /// that month.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert!(Date::from_ymd(2024, 2, 29).is_ok());
    /// assert!(Date::from_ymd(2023, 2, 29).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Result<Self, KurumiError> {
        if !(MIN_YEAR..=MAX_YEAR).contains(&year) {
            return Err(KurumiError::DateError {
                reason: format!("year {year} is outside {MIN_YEAR}..={MAX_YEAR}"),
            });
        }
        if !(1..=12).contains(&month) {
            return Err(KurumiError::DateError {
                reason: format!("month {month} is outside 1..=12"),
            });
        }
        let length = month_length(year, month);
        if day == 0 || day > length {
            return Err(KurumiError::DateError {
                reason: format!("day {day} is invalid for {year}-{month:02} (1..={length})"),
            });
        }
        let days = days_from_civil(year, month, day);
        let days = i32::try_from(days).map_err(|_| KurumiError::DateError {
            reason: format!("date {year}-{month:02}-{day:02} is outside the supported range"),
        })?;
        Self::from_days_since_epoch(days)
    }

    /// Creates a date from a signed day count relative to 1970-01-01.
    ///
    /// Day `0` is 1970-01-01 and day `-1` is 1969-12-31.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::DateError`] when the day count falls outside
    /// 0001-01-01 to 9999-12-31.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert_eq!(Date::from_days_since_epoch(0)?.to_string(), "1970-01-01");
    /// assert_eq!(Date::from_days_since_epoch(-1)?.to_string(), "1969-12-31");
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_days_since_epoch(days: i32) -> Result<Self, KurumiError> {
        let (year, _, _) = civil_from_days(days);
        if !(MIN_YEAR..=MAX_YEAR).contains(&year) {
            return Err(KurumiError::DateError {
                reason: format!("day count {days} is outside {MIN_YEAR}-01-01..={MAX_YEAR}-12-31"),
            });
        }
        Ok(Self { days })
    }

    /// Returns the signed number of days since 1970-01-01.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert_eq!(Date::from_ymd(1970, 1, 1)?.days_since_epoch(), 0);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn days_since_epoch(self) -> i32 {
        self.days
    }

    /// Returns the calendar year.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert_eq!(Date::from_ymd(2024, 7, 1)?.year(), 2024);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn year(self) -> i32 {
        civil_from_days(self.days).0
    }

    /// Returns the calendar month (`1..=12`).
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert_eq!(Date::from_ymd(2024, 7, 1)?.month(), 7);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn month(self) -> u32 {
        civil_from_days(self.days).1
    }

    /// Returns the day of the month (`1..=31`).
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert_eq!(Date::from_ymd(2024, 7, 15)?.day(), 15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn day(self) -> u32 {
        civil_from_days(self.days).2
    }

    /// Returns the day of the week.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, Weekday};
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert_eq!(Date::from_ymd(1970, 1, 1)?.weekday(), Weekday::Thursday);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn weekday(self) -> Weekday {
        match self.days.rem_euclid(7) {
            0 => Weekday::Thursday,
            1 => Weekday::Friday,
            2 => Weekday::Saturday,
            3 => Weekday::Sunday,
            4 => Weekday::Monday,
            5 => Weekday::Tuesday,
            _ => Weekday::Wednesday,
        }
    }

    /// Returns `true` when `year` is a leap year in the Gregorian calendar.
    ///
    /// A year is a leap year when it is divisible by 4, except for century
    /// years that are not divisible by 400.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert!(Date::from_ymd(2024, 1, 1)?.is_leap_year());
    /// assert!(!Date::from_ymd(1900, 1, 1)?.is_leap_year());
    /// assert!(Date::from_ymd(2000, 1, 1)?.is_leap_year());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn is_leap_year(self) -> bool {
        is_leap_year(self.year())
    }

    /// Returns the number of days in this date's month.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert_eq!(Date::from_ymd(2024, 2, 1)?.days_in_month(), 29);
    /// assert_eq!(Date::from_ymd(2023, 2, 1)?.days_in_month(), 28);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn days_in_month(self) -> u32 {
        month_length(self.year(), self.month())
    }

    /// Returns the ordinal day within the year (`1..=366`).
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert_eq!(Date::from_ymd(2024, 12, 31)?.ordinal_day(), 366);
    /// assert_eq!(Date::from_ymd(2023, 12, 31)?.ordinal_day(), 365);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn ordinal_day(self) -> u32 {
        let start = days_from_civil(self.year(), 1, 1);
        (i64::from(self.days) - start + 1) as u32
    }

    /// Returns `true` when this date is the last day of its month.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert!(Date::from_ymd(2024, 2, 29)?.is_last_day_of_month());
    /// assert!(!Date::from_ymd(2024, 2, 28)?.is_last_day_of_month());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn is_last_day_of_month(self) -> bool {
        self.day() == self.days_in_month()
    }

    /// Adds a signed number of calendar days.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::DateError`] when the result leaves the supported
    /// range or the day count overflows.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Date;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// let month_end = Date::from_ymd(2024, 2, 29)?;
    /// assert_eq!(month_end.checked_add_days(1)?.to_string(), "2024-03-01");
    /// assert_eq!(month_end.checked_add_days(-1)?.to_string(), "2024-02-28");
    /// # Ok(())
    /// # }
    /// ```
    pub fn checked_add_days(self, days: i64) -> Result<Self, KurumiError> {
        let total =
            i64::from(self.days)
                .checked_add(days)
                .ok_or_else(|| KurumiError::DateError {
                    reason: "day arithmetic overflowed".to_owned(),
                })?;
        let total = i32::try_from(total).map_err(|_| KurumiError::DateError {
            reason: "date is outside the supported range".to_owned(),
        })?;
        Self::from_days_since_epoch(total)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02}",
            self.year(),
            self.month(),
            self.day()
        )
    }
}

/// A convention for converting a date interval into a year fraction.
///
/// Year fractions are dimensionless and follow the market-standard definitions
/// below. `start` and `end` may be reversed; the result is then the negative of
/// the forward fraction.
///
/// # Examples
///
/// ```
/// use kurumi_core::{Date, DayCountConvention};
///
/// # fn main() -> Result<(), kurumi_core::KurumiError> {
/// let start = Date::from_ymd(2024, 1, 1)?;
/// let end = Date::from_ymd(2024, 7, 1)?;
/// let fraction = DayCountConvention::Act360.year_fraction(start, end);
/// assert!((fraction - 182.0 / 360.0).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DayCountConvention {
    /// Actual/360: actual days divided by 360.
    ///
    /// Common for money-market instruments quoted on an actual/360 basis.
    Act360,
    /// Actual/365 Fixed: actual days divided by 365, even in leap years.
    ///
    /// Common for sterling money-market instruments and some bonds.
    Act365Fixed,
    /// Actual/Actual (ISDA): actual days split across calendar years, each
    /// portion divided by the number of days in its year.
    ///
    /// Common for fixed-income swaps and government bonds.
    ActActIsda,
    /// 30/360 (US, bond basis): each month counted as 30 days under the US
    /// end-of-month rules.
    ///
    /// Common for US corporate and municipal bonds.
    Thirty360Us,
}

impl DayCountConvention {
    /// Returns the year fraction between `start` and `end` under this
    /// convention.
    ///
    /// The result is dimensionless. For actual-day conventions it equals
    /// `actual_days / denominator`; for [`DayCountConvention::ActActIsda`] the
    /// interval is split at calendar-year boundaries; and for
    /// [`DayCountConvention::Thirty360Us`] the US end-of-month adjustments are
    /// applied before dividing by 360.
    ///
    /// The calculation is antisymmetric: reversing the dates negates the
    /// result.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Date, DayCountConvention};
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// let start = Date::from_ymd(2024, 2, 29)?;
    /// let end = Date::from_ymd(2025, 2, 28)?;
    /// // 30/360 US treats both February month-ends as day 30.
    /// assert!((DayCountConvention::Thirty360Us.year_fraction(start, end) - 1.0).abs() < 1e-12);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn year_fraction(self, start: Date, end: Date) -> f64 {
        if start > end {
            return -self.year_fraction(end, start);
        }
        let actual_days = (i64::from(end.days) - i64::from(start.days)) as f64;
        match self {
            Self::Act360 => actual_days / 360.0,
            Self::Act365Fixed => actual_days / 365.0,
            Self::ActActIsda => act_act_isda(start, end),
            Self::Thirty360Us => thirty_360_us(start, end) / 360.0,
        }
    }
}

impl fmt::Display for DayCountConvention {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Act360 => "Act/360",
            Self::Act365Fixed => "Act/365F",
            Self::ActActIsda => "Act/Act ISDA",
            Self::Thirty360Us => "30/360 US",
        })
    }
}

/// A business-day adjustment rule.
///
/// # Examples
///
/// ```
/// use kurumi_core::{BusinessDayConvention, Date, WeekendCalendar};
///
/// # fn main() -> Result<(), kurumi_core::KurumiError> {
/// let saturday = Date::from_ymd(2024, 3, 9)?;
/// let adjusted = BusinessDayConvention::Following.adjust(saturday, &WeekendCalendar)?;
/// assert_eq!(adjusted.to_string(), "2024-03-11");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BusinessDayConvention {
    /// Move forward to the next business day.
    Following,
    /// Move forward to the next business day, except when that crosses a month
    /// boundary; then move backward to the previous business day.
    ModifiedFollowing,
    /// Move backward to the previous business day.
    Preceding,
}

impl BusinessDayConvention {
    /// Adjusts `date` to a business day according to this convention.
    ///
    /// The search is bounded to ten Gregorian years. A calendar with no
    /// business days in that window yields [`KurumiError::DateError`] rather
    /// than looping forever.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::DateError`] when no qualifying business day is
    /// found within the bounded search window.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{BusinessDayConvention, Date, WeekendCalendar};
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// let sunday = Date::from_ymd(2024, 3, 10)?;
    /// let preceding = BusinessDayConvention::Preceding.adjust(sunday, &WeekendCalendar)?;
    /// assert_eq!(preceding.to_string(), "2024-03-08");
    /// # Ok(())
    /// # }
    /// ```
    pub fn adjust(self, date: Date, calendar: &dyn Calendar) -> Result<Date, KurumiError> {
        adjust_impl(date, self, calendar)
    }
}

impl fmt::Display for BusinessDayConvention {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Following => "Following",
            Self::ModifiedFollowing => "Modified Following",
            Self::Preceding => "Preceding",
        })
    }
}

/// A source of business-day information.
///
/// Implement this trait to describe a market calendar. The required method
/// reports whether a date is a business day; [`Calendar::adjust`] provides the
/// standard roll conventions. Weekend handling is available through
/// [`WeekendCalendar`], which ignores holidays.
///
/// # Examples
///
/// ```
/// use kurumi_core::{BusinessDayConvention, Calendar, Date, KurumiError};
///
/// struct EveryDay;
///
/// impl Calendar for EveryDay {
///     fn is_business_day(&self, _date: Date) -> bool {
///         true
///     }
/// }
///
/// # fn main() -> Result<(), KurumiError> {
/// let date = Date::from_ymd(2024, 1, 1)?;
/// assert_eq!(EveryDay.adjust(date, BusinessDayConvention::Following)?, date);
/// # Ok(())
/// # }
/// ```
pub trait Calendar {
    /// Returns `true` when `date` is a business day in this calendar.
    fn is_business_day(&self, date: Date) -> bool;

    /// Adjusts `date` to a business day using `convention`.
    ///
    /// The default implementation applies the standard roll rules using
    /// [`Calendar::is_business_day`].
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::DateError`] when no business day is found within
    /// the bounded search window.
    fn adjust(&self, date: Date, convention: BusinessDayConvention) -> Result<Date, KurumiError> {
        adjust_impl(date, convention, self)
    }
}

/// A calendar with Monday-to-Friday business days and no holidays.
///
/// This is the minimal useful calendar: it handles weekends correctly and can
/// be extended by implementing [`Calendar`] with holiday data.
///
/// # Examples
///
/// ```
/// use kurumi_core::{Calendar, Date, WeekendCalendar};
///
/// # fn main() -> Result<(), kurumi_core::KurumiError> {
/// assert!(WeekendCalendar.is_business_day(Date::from_ymd(2024, 3, 8)?));
/// assert!(!WeekendCalendar.is_business_day(Date::from_ymd(2024, 3, 9)?));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct WeekendCalendar;

impl WeekendCalendar {
    /// Creates a weekend-only calendar.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::WeekendCalendar;
    ///
    /// let calendar = WeekendCalendar::new();
    /// assert_eq!(calendar, WeekendCalendar);
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Calendar for WeekendCalendar {
    fn is_business_day(&self, date: Date) -> bool {
        !date.weekday().is_weekend()
    }
}

const MAX_ADJUSTMENT_STEPS: usize = 366 * 10;

fn adjust_impl<C: Calendar + ?Sized>(
    date: Date,
    convention: BusinessDayConvention,
    calendar: &C,
) -> Result<Date, KurumiError> {
    match convention {
        BusinessDayConvention::Following => search_business_day(date, true, calendar),
        BusinessDayConvention::Preceding => search_business_day(date, false, calendar),
        BusinessDayConvention::ModifiedFollowing => {
            let following = search_business_day(date, true, calendar)?;
            if following.year() == date.year() && following.month() == date.month() {
                Ok(following)
            } else {
                search_business_day(date, false, calendar)
            }
        }
    }
}

fn search_business_day<C: Calendar + ?Sized>(
    date: Date,
    forward: bool,
    calendar: &C,
) -> Result<Date, KurumiError> {
    let mut current = date;
    for _ in 0..MAX_ADJUSTMENT_STEPS {
        if calendar.is_business_day(current) {
            return Ok(current);
        }
        current = current.checked_add_days(if forward { 1 } else { -1 })?;
    }
    Err(KurumiError::DateError {
        reason: format!("no business day found within {MAX_ADJUSTMENT_STEPS} calendar days"),
    })
}

fn act_act_isda(start: Date, end: Date) -> f64 {
    let start_year = start.year();
    let end_year = end.year();
    if start_year == end_year {
        let days = (i64::from(end.days) - i64::from(start.days)) as f64;
        return days / f64::from(days_in_year(start_year));
    }
    let start_portion = (days_from_civil(start_year + 1, 1, 1) - i64::from(start.days)) as f64;
    let end_portion = (i64::from(end.days) - days_from_civil(end_year, 1, 1)) as f64;
    start_portion / f64::from(days_in_year(start_year))
        + f64::from(end_year - start_year - 1)
        + end_portion / f64::from(days_in_year(end_year))
}

fn thirty_360_us(start: Date, end: Date) -> f64 {
    let mut start_day = start.day();
    let mut end_day = end.day();
    let start_is_february_end = start.month() == 2 && start.is_last_day_of_month();
    let end_is_february_end = end.month() == 2 && end.is_last_day_of_month();

    if start_is_february_end && end_is_february_end {
        end_day = 30;
    }
    if start_is_february_end {
        start_day = 30;
    }
    if end_day == 31 && start_day >= 30 {
        end_day = 30;
    }
    if start_day == 31 {
        start_day = 30;
    }

    let numerator = 360 * (i64::from(end.year()) - i64::from(start.year()))
        + 30 * (i64::from(end.month()) - i64::from(start.month()))
        + (i64::from(end_day) - i64::from(start_day));
    numerator as f64
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_year(year: i32) -> u32 {
    if is_leap_year(year) { 366 } else { 365 }
}

fn month_length(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Converts a signed day count from 1970-01-01 into a Gregorian date.
///
/// This is Howard Hinnant's `civil_from_days` algorithm, valid for the full
/// proleptic Gregorian range.
fn civil_from_days(days: i32) -> (i32, u32, u32) {
    let z = i64::from(days) + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year as i32, month as u32, day as u32)
}

/// Converts a Gregorian date into a signed day count from 1970-01-01.
///
/// This is Howard Hinnant's `days_from_civil` algorithm, the exact inverse of
/// [`civil_from_days`].
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = i64::from(year) - if month <= 2 { 1 } else { 0 };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_prime = i64::from(if month > 2 { month - 3 } else { month + 9 });
    let day_of_year = (153 * month_prime + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn date(year: i32, month: u32, day: u32) -> Date {
        Date::from_ymd(year, month, day).expect("test date must be valid")
    }

    #[test]
    fn date_constructor_rejects_invalid_calendar_dates() {
        assert!(Date::from_ymd(2023, 2, 29).is_err());
        assert!(Date::from_ymd(2024, 4, 31).is_err());
        assert!(Date::from_ymd(2024, 13, 1).is_err());
        assert!(Date::from_ymd(2024, 0, 1).is_err());
        assert!(Date::from_ymd(2024, 1, 0).is_err());
        assert!(Date::from_ymd(0, 1, 1).is_err());
        assert!(Date::from_ymd(10_000, 1, 1).is_err());
    }

    #[test]
    fn leap_years_follow_the_gregorian_rule() {
        assert!(date(2024, 1, 1).is_leap_year());
        assert!(date(2000, 1, 1).is_leap_year());
        assert!(!date(1900, 1, 1).is_leap_year());
        assert!(!date(2023, 1, 1).is_leap_year());
    }

    #[test]
    fn weekday_matches_known_reference_dates() {
        assert_eq!(date(1970, 1, 1).weekday(), Weekday::Thursday);
        assert_eq!(date(2024, 1, 1).weekday(), Weekday::Monday);
        assert_eq!(date(2024, 2, 29).weekday(), Weekday::Thursday);
        assert_eq!(date(2024, 3, 9).weekday(), Weekday::Saturday);
        assert_eq!(date(2024, 3, 10).weekday(), Weekday::Sunday);
    }

    #[test]
    fn day_arithmetic_crosses_month_and_year_boundaries() {
        assert_eq!(date(2024, 2, 28).checked_add_days(1), Ok(date(2024, 2, 29)));
        assert_eq!(date(2024, 2, 29).checked_add_days(1), Ok(date(2024, 3, 1)));
        assert_eq!(date(2023, 12, 31).checked_add_days(1), Ok(date(2024, 1, 1)));
        assert_eq!(
            date(2024, 1, 1).checked_add_days(-1),
            Ok(date(2023, 12, 31))
        );
    }

    #[test]
    fn day_counts_round_trip_through_the_civil_algorithms() {
        for day in [-719_162, -1, 0, 1, 10_958, 19_234, 2_932_896] {
            let parsed = Date::from_days_since_epoch(day).expect("day count in range");
            assert_eq!(parsed.days_since_epoch(), day);
            let rebuilt = Date::from_ymd(parsed.year(), parsed.month(), parsed.day())
                .expect("round-tripped date");
            assert_eq!(rebuilt, parsed);
        }
    }

    #[test]
    fn ordinal_day_handles_leap_and_common_years() {
        assert_eq!(date(2024, 1, 1).ordinal_day(), 1);
        assert_eq!(date(2024, 2, 29).ordinal_day(), 60);
        assert_eq!(date(2024, 12, 31).ordinal_day(), 366);
        assert_eq!(date(2023, 12, 31).ordinal_day(), 365);
    }

    #[test]
    fn last_day_of_month_detects_february_variants() {
        assert!(date(2024, 2, 29).is_last_day_of_month());
        assert!(date(2023, 2, 28).is_last_day_of_month());
        assert!(date(2024, 4, 30).is_last_day_of_month());
        assert!(!date(2024, 4, 29).is_last_day_of_month());
    }

    #[test]
    fn display_uses_iso_8601() {
        assert_eq!(date(2024, 2, 29).to_string(), "2024-02-29");
        assert_eq!(date(1, 1, 1).to_string(), "0001-01-01");
    }

    #[test]
    fn actual_360_matches_the_money_market_definition() {
        let fraction = DayCountConvention::Act360.year_fraction(date(2024, 1, 1), date(2024, 7, 1));
        assert_relative_eq!(fraction, 182.0 / 360.0, epsilon = 1e-15);
    }

    #[test]
    fn actual_365_fixed_ignores_leap_years() {
        let fraction =
            DayCountConvention::Act365Fixed.year_fraction(date(2024, 1, 1), date(2025, 1, 1));
        assert_relative_eq!(fraction, 366.0 / 365.0, epsilon = 1e-15);
    }

    #[test]
    fn actual_actual_isda_splits_at_year_boundaries() {
        let same_year =
            DayCountConvention::ActActIsda.year_fraction(date(2024, 1, 1), date(2024, 7, 1));
        assert_relative_eq!(same_year, 182.0 / 366.0, epsilon = 1e-15);

        // 2023-07-01..2024-01-01 is 184 days in a 365-day year; the remaining
        // 182 days fall in the 366-day year 2024.
        let cross_year =
            DayCountConvention::ActActIsda.year_fraction(date(2023, 7, 1), date(2024, 7, 1));
        assert_relative_eq!(cross_year, 184.0 / 365.0 + 182.0 / 366.0, epsilon = 1e-15);
    }

    #[test]
    fn actual_actual_isda_returns_exactly_one_for_full_years() {
        let common =
            DayCountConvention::ActActIsda.year_fraction(date(2023, 1, 1), date(2024, 1, 1));
        let leap = DayCountConvention::ActActIsda.year_fraction(date(2024, 1, 1), date(2025, 1, 1));
        assert_relative_eq!(common, 1.0, epsilon = 1e-15);
        assert_relative_eq!(leap, 1.0, epsilon = 1e-15);
    }

    #[test]
    fn thirty_360_us_matches_the_bond_basis_formula() {
        let fraction =
            DayCountConvention::Thirty360Us.year_fraction(date(2024, 1, 1), date(2024, 7, 1));
        assert_relative_eq!(fraction, 0.5, epsilon = 1e-15);

        let end_of_month =
            DayCountConvention::Thirty360Us.year_fraction(date(2024, 8, 31), date(2024, 9, 30));
        assert_relative_eq!(end_of_month, 30.0 / 360.0, epsilon = 1e-15);
    }

    #[test]
    fn thirty_360_us_handles_february_across_a_leap_year() {
        // Both endpoints are the last day of February, so both roll to day 30.
        let leap_to_common =
            DayCountConvention::Thirty360Us.year_fraction(date(2024, 2, 29), date(2025, 2, 28));
        assert_relative_eq!(leap_to_common, 1.0, epsilon = 1e-15);

        let common_to_leap =
            DayCountConvention::Thirty360Us.year_fraction(date(2023, 2, 28), date(2024, 2, 29));
        assert_relative_eq!(common_to_leap, 1.0, epsilon = 1e-15);

        // A single leap day still accrues exactly one 30/360 day.
        let leap_day =
            DayCountConvention::Thirty360Us.year_fraction(date(2024, 2, 29), date(2024, 3, 1));
        assert_relative_eq!(leap_day, 1.0 / 360.0, epsilon = 1e-15);
    }

    #[test]
    fn thirty_360_us_applies_the_us_end_of_month_rules() {
        // 2023-08-31 -> 2024-02-29: start day 31 rolls to 30, end day stays 29.
        let fraction =
            DayCountConvention::Thirty360Us.year_fraction(date(2023, 8, 31), date(2024, 2, 29));
        assert_relative_eq!(fraction, 179.0 / 360.0, epsilon = 1e-15);
    }

    #[test]
    fn year_fraction_is_antisymmetric() {
        let start = date(2024, 1, 15);
        let end = date(2024, 9, 30);
        for convention in [
            DayCountConvention::Act360,
            DayCountConvention::Act365Fixed,
            DayCountConvention::ActActIsda,
            DayCountConvention::Thirty360Us,
        ] {
            assert_relative_eq!(
                convention.year_fraction(start, end),
                -convention.year_fraction(end, start),
                epsilon = 1e-15
            );
        }
    }

    #[test]
    fn following_rolls_weekends_forward() {
        let saturday = date(2024, 3, 9);
        let adjusted = BusinessDayConvention::Following
            .adjust(saturday, &WeekendCalendar)
            .expect("following adjustment");
        assert_eq!(adjusted, date(2024, 3, 11));
    }

    #[test]
    fn preceding_rolls_weekends_backward() {
        let sunday = date(2024, 3, 10);
        let adjusted = BusinessDayConvention::Preceding
            .adjust(sunday, &WeekendCalendar)
            .expect("preceding adjustment");
        assert_eq!(adjusted, date(2024, 3, 8));
    }

    #[test]
    fn modified_following_stays_in_the_original_month() {
        // 2024-03-31 is a Sunday; plain Following would enter April, so the
        // modified rule rolls back to Friday 2024-03-29.
        let sunday = date(2024, 3, 31);
        let adjusted = BusinessDayConvention::ModifiedFollowing
            .adjust(sunday, &WeekendCalendar)
            .expect("modified following adjustment");
        assert_eq!(adjusted, date(2024, 3, 29));
    }

    #[test]
    fn modified_following_is_plain_following_inside_the_month() {
        let saturday = date(2024, 3, 9);
        let adjusted = BusinessDayConvention::ModifiedFollowing
            .adjust(saturday, &WeekendCalendar)
            .expect("modified following adjustment");
        assert_eq!(adjusted, date(2024, 3, 11));
    }

    #[test]
    fn business_day_calendars_can_be_implemented_by_callers() {
        struct OnlyMarchTwelfth;
        impl Calendar for OnlyMarchTwelfth {
            fn is_business_day(&self, value: Date) -> bool {
                value == date(2024, 3, 12)
            }
        }

        let start = date(2024, 3, 9);
        let adjusted = Calendar::adjust(&OnlyMarchTwelfth, start, BusinessDayConvention::Following)
            .expect("custom calendar adjustment");
        assert_eq!(adjusted, date(2024, 3, 12));
    }

    #[test]
    fn a_calendar_without_business_days_fails_instead_of_looping() {
        struct ClosedAlways;
        impl Calendar for ClosedAlways {
            fn is_business_day(&self, _value: Date) -> bool {
                false
            }
        }

        assert!(matches!(
            BusinessDayConvention::Following.adjust(date(2024, 1, 1), &ClosedAlways),
            Err(KurumiError::DateError { .. })
        ));
    }

    #[test]
    fn weekend_calendar_only_considers_monday_to_friday() {
        assert!(WeekendCalendar.is_business_day(date(2024, 3, 8)));
        assert!(!WeekendCalendar.is_business_day(date(2024, 3, 9)));
        assert!(!WeekendCalendar.is_business_day(date(2024, 3, 10)));
        assert!(WeekendCalendar.is_business_day(date(2024, 3, 11)));
    }

    #[test]
    fn display_formats_day_count_and_roll_conventions() {
        assert_eq!(DayCountConvention::ActActIsda.to_string(), "Act/Act ISDA");
        assert_eq!(BusinessDayConvention::Following.to_string(), "Following");
        assert_eq!(
            BusinessDayConvention::ModifiedFollowing.to_string(),
            "Modified Following"
        );
    }
}
