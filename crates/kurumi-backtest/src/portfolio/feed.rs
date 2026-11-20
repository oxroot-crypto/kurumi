//! Synchronised multi-asset market data.
//!
//! [`MultiAssetFeed`] turns one OHLCV series per instrument into the union
//! timeline of [`MarketSnapshot`] observations. Bars are grouped by their
//! millisecond timestamp; a snapshot contains a bar only for the instruments
//! that actually traded at that timestamp, so market holidays, halts, and
//! staggered listing histories are represented without forward-filling.
//!
//! All containers are [`BTreeMap`]s, which makes both the symbol order and the
//! timeline order deterministic across runs.
//!
//! # Examples
//!
//! ```
//! use kurumi_backtest::portfolio::MultiAssetFeed;
//! use kurumi_ta::OhlcvBar;
//! use std::collections::BTreeMap;
//!
//! # fn main() -> Result<(), kurumi_backtest::KurumiError> {
//! let day = 86_400_000;
//! let a = vec![OhlcvBar::new(day, 10.0, 11.0, 9.0, 10.5, 1_000.0, 10_500.0)?];
//! let b = vec![OhlcvBar::new(day, 20.0, 21.0, 19.0, 20.5, 2_000.0, 41_000.0)?];
//! let mut series = BTreeMap::new();
//! series.insert("AAA".to_owned(), a);
//! series.insert("BBB".to_owned(), b);
//!
//! let feed = MultiAssetFeed::new(series)?;
//! assert_eq!(feed.len(), 1);
//! let snapshot = feed.iter().next().expect("one snapshot");
//! assert_eq!(snapshot.bars().len(), 2);
//! # Ok(())
//! # }
//! ```

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use kurumi_core::KurumiError;
use kurumi_ta::OhlcvBar;
use rust_decimal::Decimal;

/// One observation of a multi-asset universe at a single timestamp.
///
/// A snapshot holds only the instruments that traded at [`MarketSnapshot::timestamp`];
/// instruments without a bar keep their previously known price in the
/// portfolio account.
///
/// # Conventions
///
/// - `timestamp` is the UTC instant of the observation at millisecond
///   resolution, identical to the OHLCV bars' Unix-millisecond timestamps.
/// - `bars` is keyed by instrument symbol and sorted by [`BTreeMap`] order.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::{
///     chrono::{DateTime, Utc},
///     MarketSnapshot,
/// };
/// use kurumi_ta::OhlcvBar;
/// use std::collections::BTreeMap;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let timestamp = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
/// let mut bars = BTreeMap::new();
/// bars.insert(
///     "AAA".to_owned(),
///     OhlcvBar::new(0, 10.0, 11.0, 9.0, 10.5, 1_000.0, 10_500.0)?,
/// );
/// let snapshot = MarketSnapshot::new(timestamp, bars)?;
/// assert_eq!(snapshot.bars().len(), 1);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MarketSnapshot {
    timestamp: DateTime<Utc>,
    bars: BTreeMap<String, OhlcvBar>,
}

impl MarketSnapshot {
    /// Creates a snapshot from its timestamp and the bars observed at it.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when a bar's Unix-millisecond
    /// timestamp does not match `timestamp` at millisecond resolution.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::{
    ///     chrono::{DateTime, Utc},
    ///     MarketSnapshot,
    /// };
    /// use kurumi_ta::OhlcvBar;
    /// use std::collections::BTreeMap;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let timestamp = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
    /// let mut bars = BTreeMap::new();
    /// bars.insert(
    ///     "AAA".to_owned(),
    ///     OhlcvBar::new(0, 10.0, 11.0, 9.0, 10.5, 1_000.0, 10_500.0)?,
    /// );
    /// assert!(MarketSnapshot::new(timestamp, bars).is_ok());
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        timestamp: DateTime<Utc>,
        bars: BTreeMap<String, OhlcvBar>,
    ) -> Result<Self, KurumiError> {
        let millis = timestamp.timestamp_millis();
        for (symbol, bar) in &bars {
            if bar.timestamp() != millis {
                return Err(KurumiError::InvalidParameter {
                    parameter: "bars",
                    reason: format!(
                        "bar of `{symbol}` has timestamp {} but the snapshot is {millis}",
                        bar.timestamp()
                    ),
                });
            }
        }
        Ok(Self { timestamp, bars })
    }

    /// Returns the observation timestamp in UTC.
    #[must_use]
    #[inline]
    pub fn timestamp(&self) -> DateTime<Utc> {
        self.timestamp
    }

    /// Returns the bars observed at this timestamp, keyed by symbol.
    #[must_use]
    #[inline]
    pub fn bars(&self) -> &BTreeMap<String, OhlcvBar> {
        &self.bars
    }

    /// Returns the bar of `symbol`, or `None` when it did not trade.
    #[must_use]
    #[inline]
    pub fn bar(&self, symbol: &str) -> Option<&OhlcvBar> {
        self.bars.get(symbol)
    }

    /// Returns the close prices as an exact decimal map.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when a close price cannot be
    /// represented as a strictly positive [`Decimal`] at 12-decimal precision.
    pub fn close_prices(&self) -> Result<BTreeMap<String, Decimal>, KurumiError> {
        let mut prices = BTreeMap::new();
        for (symbol, bar) in &self.bars {
            prices.insert(symbol.clone(), decimal_from_f64(bar.close(), symbol)?);
        }
        Ok(prices)
    }
}

/// The synchronised timeline of a multi-asset universe.
///
/// The feed validates every per-instrument series and merges their timestamps
/// into one ascending union. Each step is a [`MarketSnapshot`] that may contain
/// a subset of the universe.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::MultiAssetFeed;
/// use kurumi_ta::OhlcvBar;
/// use std::collections::BTreeMap;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let mut series = BTreeMap::new();
/// series.insert(
///     "AAA".to_owned(),
///     vec![
///         OhlcvBar::new(0, 10.0, 11.0, 9.0, 10.5, 1_000.0, 10_500.0)?,
///         OhlcvBar::new(2 * 86_400_000, 10.5, 12.0, 10.0, 11.5, 900.0, 10_350.0)?,
///     ],
/// );
/// series.insert(
///     "BBB".to_owned(),
///     vec![OhlcvBar::new(86_400_000, 20.0, 21.0, 19.0, 20.5, 500.0, 10_250.0)?],
/// );
///
/// let feed = MultiAssetFeed::new(series)?;
/// assert_eq!(feed.len(), 3);
/// let timestamps: Vec<_> = feed.iter().map(|snapshot| snapshot.bars().len()).collect();
/// assert_eq!(timestamps, vec![1, 1, 1]);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MultiAssetFeed {
    snapshots: Vec<MarketSnapshot>,
}

impl MultiAssetFeed {
    /// Builds the union timeline from one OHLCV series per instrument.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when
    ///
    /// - an instrument symbol is empty;
    /// - a series is not strictly increasing in time; or
    /// - a timestamp cannot be represented as a [`DateTime<Utc>`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::MultiAssetFeed;
    /// use kurumi_ta::OhlcvBar;
    /// use std::collections::BTreeMap;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let mut series = BTreeMap::new();
    /// series.insert(
    ///     "AAA".to_owned(),
    ///     vec![OhlcvBar::new(0, 10.0, 11.0, 9.0, 10.5, 1_000.0, 10_500.0)?],
    /// );
    /// let feed = MultiAssetFeed::new(series)?;
    /// assert_eq!(feed.len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(series: BTreeMap<String, Vec<OhlcvBar>>) -> Result<Self, KurumiError> {
        for (symbol, bars) in &series {
            if symbol.trim().is_empty() {
                return Err(KurumiError::InvalidParameter {
                    parameter: "series",
                    reason: "instrument symbols must not be empty".to_owned(),
                });
            }
            let mut previous: Option<i64> = None;
            for bar in bars {
                if let Some(previous) = previous {
                    if bar.timestamp() <= previous {
                        return Err(KurumiError::InvalidParameter {
                            parameter: "series",
                            reason: format!("timestamps of `{symbol}` must be strictly increasing"),
                        });
                    }
                }
                previous = Some(bar.timestamp());
            }
        }

        let mut timestamps = BTreeSet::new();
        for bars in series.values() {
            for bar in bars {
                timestamps.insert(bar.timestamp());
            }
        }

        let mut snapshots = Vec::with_capacity(timestamps.len());
        for timestamp in timestamps {
            let instant = DateTime::<Utc>::from_timestamp_millis(timestamp).ok_or_else(|| {
                KurumiError::InvalidParameter {
                    parameter: "timestamp",
                    reason: format!(
                        "timestamp {timestamp} is outside the representable datetime range"
                    ),
                }
            })?;
            let mut bars = BTreeMap::new();
            for (symbol, instrument_bars) in &series {
                if let Ok(index) =
                    instrument_bars.binary_search_by_key(&timestamp, |bar| bar.timestamp())
                {
                    if let Some(bar) = instrument_bars.get(index) {
                        bars.insert(symbol.clone(), *bar);
                    }
                }
            }
            snapshots.push(MarketSnapshot {
                timestamp: instant,
                bars,
            });
        }

        Ok(Self { snapshots })
    }

    /// Returns the synchronised snapshots in ascending time order.
    #[must_use]
    #[inline]
    pub fn snapshots(&self) -> &[MarketSnapshot] {
        &self.snapshots
    }

    /// Returns the number of snapshots on the union timeline.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    /// Returns `true` when the feed has no observations.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    /// Iterates over the snapshots in ascending time order.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::MultiAssetFeed;
    /// use kurumi_ta::OhlcvBar;
    /// use std::collections::BTreeMap;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let mut series = BTreeMap::new();
    /// series.insert(
    ///     "AAA".to_owned(),
    ///     vec![OhlcvBar::new(0, 10.0, 11.0, 9.0, 10.5, 1_000.0, 10_500.0)?],
    /// );
    /// let feed = MultiAssetFeed::new(series)?;
    /// assert_eq!(feed.iter().count(), 1);
    /// # Ok(())
    /// # }
    /// ```
    pub fn iter(&self) -> std::slice::Iter<'_, MarketSnapshot> {
        self.snapshots.iter()
    }
}

impl<'a> IntoIterator for &'a MultiAssetFeed {
    type Item = &'a MarketSnapshot;
    type IntoIter = std::slice::Iter<'a, MarketSnapshot>;

    fn into_iter(self) -> Self::IntoIter {
        self.snapshots.iter()
    }
}

impl IntoIterator for MultiAssetFeed {
    type Item = MarketSnapshot;
    type IntoIter = std::vec::IntoIter<MarketSnapshot>;

    fn into_iter(self) -> Self::IntoIter {
        self.snapshots.into_iter()
    }
}

/// Converts an OHLCV `f64` price into the decimal ledger.
///
/// OHLCV bars are already validated as finite and strictly positive, so the
/// only remaining failures are values outside [`Decimal`]'s range and values
/// that round to zero at 12-decimal precision.
pub(crate) fn decimal_from_f64(value: f64, symbol: &str) -> Result<Decimal, KurumiError> {
    let decimal = Decimal::from_f64_retain(value).ok_or_else(|| KurumiError::InvalidParameter {
        parameter: "close",
        reason: format!("close price of `{symbol}` is not representable as a Decimal"),
    })?;
    let rounded = decimal.round_dp(12);
    if rounded <= Decimal::ZERO {
        return Err(KurumiError::InvalidParameter {
            parameter: "close",
            reason: format!("close price of `{symbol}` is not positive at 12-decimal precision"),
        });
    }
    Ok(rounded)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400_000;

    fn bar(timestamp: i64, close: f64) -> OhlcvBar {
        OhlcvBar::new(
            timestamp,
            close,
            close + 1.0,
            close - 1.0,
            close,
            1_000.0,
            close * 1_000.0,
        )
        .expect("valid test bar")
    }

    fn series(entries: &[(i64, f64)]) -> Vec<OhlcvBar> {
        entries
            .iter()
            .map(|(day, close)| bar(*day, *close))
            .collect()
    }

    #[test]
    fn feed_aligns_misaligned_series_into_an_ascending_union() {
        let mut input = BTreeMap::new();
        input.insert(
            "AAA".to_owned(),
            series(&[(0, 10.0), (2 * DAY, 12.0), (4 * DAY, 14.0)]),
        );
        input.insert(
            "BBB".to_owned(),
            series(&[(DAY, 20.0), (2 * DAY, 22.0), (3 * DAY, 24.0)]),
        );

        let feed = MultiAssetFeed::new(input).expect("valid feed");
        let timestamps: Vec<i64> = feed
            .snapshots()
            .iter()
            .map(|snapshot| snapshot.timestamp().timestamp_millis())
            .collect();
        assert_eq!(timestamps, vec![0, DAY, 2 * DAY, 3 * DAY, 4 * DAY]);

        let sizes: Vec<usize> = feed.iter().map(|snapshot| snapshot.bars().len()).collect();
        assert_eq!(sizes, vec![1, 1, 2, 1, 1]);

        let shared = feed.snapshots().get(2).expect("shared snapshot");
        assert!(shared.bar("AAA").is_some());
        assert!(shared.bar("BBB").is_some());
        assert_eq!(shared.bar("AAA").map(|bar| bar.close()), Some(12.0));
        assert!(
            feed.snapshots()
                .get(1)
                .expect("BBB only")
                .bar("AAA")
                .is_none()
        );
    }

    #[test]
    fn feed_rejects_non_increasing_timestamps() {
        let mut duplicated = BTreeMap::new();
        duplicated.insert("AAA".to_owned(), series(&[(0, 10.0), (0, 11.0)]));
        assert!(MultiAssetFeed::new(duplicated).is_err());

        let mut decreasing = BTreeMap::new();
        decreasing.insert("AAA".to_owned(), series(&[(DAY, 10.0), (0, 11.0)]));
        assert!(MultiAssetFeed::new(decreasing).is_err());
    }

    #[test]
    fn feed_rejects_empty_symbols() {
        let mut input = BTreeMap::new();
        input.insert("   ".to_owned(), series(&[(0, 10.0)]));
        assert!(MultiAssetFeed::new(input).is_err());
    }

    #[test]
    fn empty_series_produce_an_empty_feed() {
        let feed = MultiAssetFeed::new(BTreeMap::new()).expect("empty feed");
        assert!(feed.is_empty());
        assert_eq!(feed.len(), 0);
        assert_eq!(feed.iter().count(), 0);
    }

    #[test]
    fn consuming_iterator_yields_owned_snapshots_in_order() {
        let mut input = BTreeMap::new();
        input.insert("AAA".to_owned(), series(&[(0, 10.0), (DAY, 11.0)]));
        let feed = MultiAssetFeed::new(input).expect("valid feed");
        let timestamps: Vec<i64> = feed
            .into_iter()
            .map(|snapshot| snapshot.timestamp().timestamp_millis())
            .collect();
        assert_eq!(timestamps, vec![0, DAY]);
    }

    #[test]
    fn market_snapshot_rejects_bars_from_another_timestamp() {
        let timestamp = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
        let mut bars = BTreeMap::new();
        bars.insert("AAA".to_owned(), bar(1, 10.0));
        assert!(MarketSnapshot::new(timestamp, bars).is_err());
    }

    #[test]
    fn close_prices_convert_exactly_at_twelve_decimals() {
        let timestamp = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
        let mut bars = BTreeMap::new();
        bars.insert("AAA".to_owned(), bar(0, 100.5));
        let snapshot = MarketSnapshot::new(timestamp, bars).expect("valid snapshot");
        let prices = snapshot.close_prices().expect("convertible prices");
        assert_eq!(prices.get("AAA"), Some(&Decimal::new(1_005, 1)));
    }
}
