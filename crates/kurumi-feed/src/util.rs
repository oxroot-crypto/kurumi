//! Small internal helpers shared by the feed modules.

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};

/// Returns the current wall-clock time as a UTC timestamp.
///
/// The workspace's `chrono` dependency deliberately leaves the `clock` feature
/// disabled, so this helper reads [`SystemTime`] and converts without pulling
/// the time-zone database into the crate.
pub(crate) fn now_utc() -> DateTime<Utc> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let seconds = i64::try_from(millis / 1_000).unwrap_or(i64::MAX);
    let nanos = u32::try_from((millis % 1_000) * 1_000_000).unwrap_or(0);
    DateTime::<Utc>::from_timestamp(seconds, nanos).unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_after_the_unix_epoch() {
        let now = now_utc();
        assert!(now.timestamp() > 0);
    }
}
