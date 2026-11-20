//! US Eastern time conversion for the Nasdaq feeds.
//!
//! Nasdaq timestamps are expressed in US Eastern local time: TotalView-ITCH
//! sends nanoseconds since midnight, while the NCDS REST endpoints send an
//! ISO-8601 local timestamp without an offset. The workspace's `chrono`
//! dependency deliberately omits the time-zone database, so this module applies
//! the United States Eastern daylight-saving rule directly:
//!
//! - daylight-saving time starts on the second Sunday of March;
//! - standard time resumes on the first Sunday of November.
//!
//! Both transitions happen at 02:00 local time on a Sunday, when the US equity
//! markets are closed, so a trading date falls entirely in one offset. The
//! conversion is exact for every trading date and documented as approximate
//! only for the transition Sundays themselves.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, Utc, Weekday};

use crate::error::FeedError;

/// Provider name attached to conversion errors.
const PROVIDER: &str = "nasdaq-time";

/// Nanoseconds in one second.
const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// Seconds in one US Eastern trading day.
const SECONDS_PER_DAY: u64 = 86_400;

/// Converts a TotalView-ITCH timestamp to UTC.
///
/// ITCH encodes the timestamp as an unsigned 48-bit count of nanoseconds since
/// midnight US Eastern time on the message's trading date. The 48-bit payload
/// is passed in as a `u64`; values at or beyond one full day are rejected.
pub(crate) fn timestamp_from_nanos_since_midnight(
    trading_date: NaiveDate,
    nanos: u64,
) -> Result<DateTime<Utc>, FeedError> {
    let day_nanos = SECONDS_PER_DAY
        .checked_mul(NANOS_PER_SECOND)
        .ok_or_else(|| conversion_error("nanoseconds per day overflow u64"))?;
    if nanos >= day_nanos {
        return Err(conversion_error(format!(
            "timestamp {nanos} ns is outside the {trading_date} trading day"
        )));
    }
    let seconds = nanos / NANOS_PER_SECOND;
    let subsecond_nanos = nanos % NANOS_PER_SECOND;
    let subsecond_nanos = u32::try_from(subsecond_nanos)
        .map_err(|_| conversion_error("sub-second nanoseconds overflow u32"))?;
    let seconds = u32::try_from(seconds)
        .map_err(|_| conversion_error("seconds since midnight overflow u32"))?;
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;
    let naive = trading_date
        .and_hms_nano_opt(hour, minute, second, subsecond_nanos)
        .ok_or_else(|| conversion_error("timestamp does not form a valid local time"))?;
    eastern_naive_to_utc(naive)
}

/// Converts a US Eastern wall-clock time to UTC.
///
/// The daylight-saving rule is evaluated for the local calendar date. See the
/// module documentation for the exact rule.
pub(crate) fn eastern_naive_to_utc(naive: NaiveDateTime) -> Result<DateTime<Utc>, FeedError> {
    let offset_hours = if is_daylight_saving(naive.date())? {
        4
    } else {
        5
    };
    let utc_naive = naive
        .checked_add_signed(Duration::hours(offset_hours))
        .ok_or_else(|| conversion_error("US Eastern timestamp is outside the supported range"))?;
    Ok(utc_naive.and_utc())
}

/// Returns `true` when the US Eastern offset is UTC-4 on `date`.
fn is_daylight_saving(date: NaiveDate) -> Result<bool, FeedError> {
    let year = date.year();
    let start = nth_weekday_of_month(year, 3, Weekday::Sun, 2)?;
    let end = nth_weekday_of_month(year, 11, Weekday::Sun, 1)?;
    Ok(date >= start && date < end)
}

/// Returns the `nth` (one-based) `weekday` of `month` in `year`.
fn nth_weekday_of_month(
    year: i32,
    month: u32,
    weekday: Weekday,
    nth: u32,
) -> Result<NaiveDate, FeedError> {
    if nth == 0 {
        return Err(conversion_error("weekday occurrence is one-based"));
    }
    let first = NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(|| conversion_error("invalid month while resolving the DST rule"))?;
    let days_to_first =
        (weekday.num_days_from_sunday() + 7 - first.weekday().num_days_from_sunday()) % 7;
    let day = 1 + days_to_first + (nth - 1) * 7;
    NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| conversion_error("invalid day while resolving the DST rule"))
}

/// Builds a protocol error attributed to the time conversion helper.
fn conversion_error(reason: impl Into<String>) -> FeedError {
    FeedError::ProtocolError {
        provider: PROVIDER.to_owned(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
    }

    #[test]
    fn standard_time_is_utc_minus_five() {
        let converted = timestamp_from_nanos_since_midnight(date(2024, 11, 20), 34_200_123_456_789)
            .expect("valid timestamp");
        assert_eq!(converted.to_string(), "2024-11-20 14:30:00.123456789 UTC");
    }

    #[test]
    fn daylight_saving_time_is_utc_minus_four() {
        let converted = timestamp_from_nanos_since_midnight(date(2024, 7, 10), 34_200_000_000_000)
            .expect("valid timestamp");
        assert_eq!(converted.to_string(), "2024-07-10 13:30:00 UTC");
    }

    #[test]
    fn daylight_saving_boundaries_follow_the_us_rule() {
        assert!(!is_daylight_saving(date(2024, 3, 9)).expect("valid date"));
        assert!(is_daylight_saving(date(2024, 3, 10)).expect("valid date"));
        assert!(is_daylight_saving(date(2024, 11, 2)).expect("valid date"));
        assert!(!is_daylight_saving(date(2024, 11, 3)).expect("valid date"));
    }

    #[test]
    fn out_of_range_nanoseconds_are_rejected() {
        let day_nanos = 86_400 * 1_000_000_000_u64;
        assert!(timestamp_from_nanos_since_midnight(date(2024, 11, 20), day_nanos).is_err());
    }
}
