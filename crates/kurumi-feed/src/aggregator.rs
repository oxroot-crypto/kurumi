//! Streaming aggregation of trades into aligned OHLCV bars.
//!
//! [`BarAggregator`] turns a stream of [`TradeTick`]s into
//! [`kurumi_ta::OhlcvBar`] candles whose timestamps are aligned to a fixed
//! bucket width. It is intentionally synchronous: the feed runtime receives the
//! trades, and the aggregator performs the bucketing without allocating on the
//! hot path beyond the per-instrument map entry.
//!
//! # Ordering and late ticks
//!
//! Trades are ordered by their exchange timestamp, not by arrival order:
//!
//! - the open is the trade with the smallest exchange timestamp in the bucket;
//! - the close is the trade with the largest exchange timestamp, with a later
//!   arrival winning an exact timestamp tie;
//! - high and low include every accepted trade regardless of arrival order;
//! - a trade whose exchange timestamp falls in an already-finalized bucket is
//!   dropped as late and never mutates the current candle.
//!
//! A bucket is finalized when a trade for the same instrument arrives in a
//! later bucket, or when [`BarAggregator::flush`] is called. Only final bars are
//! emitted; consumers that need an in-progress candle can build one from the
//! next trade or keep their own provisional state.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use kurumi_ta::OhlcvBar;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::error::FeedError;
use crate::model::{BarEvent, InstrumentId, TradeTick};

/// The provider name attached to errors raised by the aggregator.
const PROVIDER: &str = "bar-aggregator";

/// Aggregates trades into fixed-interval OHLCV bars.
///
/// The aggregator keeps one open bucket per instrument in a
/// [`BTreeMap`], so [`BarAggregator::flush`] emits
/// bars in deterministic instrument order. Volume is the sum of base-unit trade
/// sizes and turnover is the sum of `price * size`, both accumulated with
/// checked [`Decimal`] arithmetic.
///
/// # Examples
///
/// ```
/// use chrono::{DateTime, Utc};
/// use kurumi_feed::{BarAggregator, InstrumentId, TradeTick, Venue};
/// use rust_decimal::Decimal;
///
/// let epoch = DateTime::<Utc>::UNIX_EPOCH;
/// let mut aggregator = BarAggregator::new(60)?;
/// let tick = TradeTick {
///     instrument: InstrumentId::new("AAPL", Venue::Nasdaq),
///     price: Decimal::new(100_00, 2),
///     size: Decimal::new(2, 0),
///     trade_id: None,
///     conditions: Vec::new(),
///     exchange_ts: epoch,
///     received_ts: epoch,
/// };
/// assert!(aggregator.on_trade(&tick)?.is_none());
/// let bars = aggregator.flush()?;
/// assert_eq!(bars.len(), 1);
/// assert!(bars.first().is_some_and(|bar| bar.is_final));
/// # Ok::<(), kurumi_feed::FeedError>(())
/// ```
#[derive(Debug, Clone)]
pub struct BarAggregator {
    interval_secs: u64,
    interval_ms: i64,
    bars: BTreeMap<InstrumentId, OpenBar>,
}

impl BarAggregator {
    /// Creates an aggregator with the given bucket width.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] when `interval_secs` is zero or so
    /// large that its millisecond representation overflows `i64`.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_feed::BarAggregator;
    ///
    /// assert!(BarAggregator::new(0).is_err());
    /// assert!(BarAggregator::new(60).is_ok());
    /// ```
    pub fn new(interval_secs: u64) -> Result<Self, FeedError> {
        if interval_secs == 0 {
            return Err(FeedError::InvalidConfig {
                reason: "bar interval must be strictly positive".to_owned(),
            });
        }
        let interval_ms = interval_secs
            .checked_mul(1_000)
            .and_then(|millis| i64::try_from(millis).ok())
            .ok_or_else(|| FeedError::InvalidConfig {
                reason: "bar interval is too large to represent in milliseconds".to_owned(),
            })?;
        Ok(Self {
            interval_secs,
            interval_ms,
            bars: BTreeMap::new(),
        })
    }

    /// Returns the bucket width in seconds.
    #[must_use]
    pub fn interval_secs(&self) -> u64 {
        self.interval_secs
    }

    /// Applies one trade and returns the bar that was finalized, if any.
    ///
    /// The returned bar has `is_final == true` and its timestamp is the start
    /// of the bucket it covers. A trade that belongs to the current bucket only
    /// updates the open candle; a trade from an already-finalized bucket is
    /// dropped as late.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::ProtocolError`] when the trade price is not
    /// strictly positive, the size is negative, checked accumulation
    /// overflows, or the aggregated values cannot be represented as a
    /// [`kurumi_ta::OhlcvBar`].
    pub fn on_trade(&mut self, tick: &TradeTick) -> Result<Option<BarEvent>, FeedError> {
        validate_trade(tick)?;
        let bucket_start_ms =
            bucket_start_ms(tick.exchange_ts.timestamp_millis(), self.interval_ms)?;
        let instrument = &tick.instrument;

        if let Some(open) = self.bars.get(instrument) {
            if bucket_start_ms < open.bucket_start_ms {
                return Ok(None);
            }
            if bucket_start_ms == open.bucket_start_ms {
                if let Some(open) = self.bars.get_mut(instrument) {
                    open.apply(tick)?;
                }
                return Ok(None);
            }
        }

        let finalized = match self.bars.get(instrument) {
            Some(open) => Some(build_bar_event(
                instrument.clone(),
                self.interval_secs,
                open,
            )?),
            None => None,
        };
        if finalized.is_some() {
            self.bars.remove(instrument);
        }
        self.bars
            .insert(instrument.clone(), OpenBar::new(bucket_start_ms, tick)?);
        Ok(finalized)
    }

    /// Finalizes every open bucket and returns the bars in instrument order.
    ///
    /// The aggregator is left empty and can be reused for a new session.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::ProtocolError`] when an open bucket cannot be
    /// represented as a [`kurumi_ta::OhlcvBar`]. On error the open buckets are
    /// preserved.
    pub fn flush(&mut self) -> Result<Vec<BarEvent>, FeedError> {
        let bars = self
            .bars
            .iter()
            .map(|(instrument, open)| build_bar_event(instrument.clone(), self.interval_secs, open))
            .collect::<Result<Vec<_>, _>>()?;
        self.bars.clear();
        Ok(bars)
    }
}

/// The mutable state of one in-progress bucket.
#[derive(Debug, Clone)]
struct OpenBar {
    bucket_start_ms: i64,
    open: Decimal,
    high: Decimal,
    low: Decimal,
    close: Decimal,
    volume: Decimal,
    turnover: Decimal,
    open_ts: DateTime<Utc>,
    close_ts: DateTime<Utc>,
}

impl OpenBar {
    fn new(bucket_start_ms: i64, tick: &TradeTick) -> Result<Self, FeedError> {
        let turnover = checked_turnover(tick)?;
        Ok(Self {
            bucket_start_ms,
            open: tick.price,
            high: tick.price,
            low: tick.price,
            close: tick.price,
            volume: tick.size,
            turnover,
            open_ts: tick.exchange_ts,
            close_ts: tick.exchange_ts,
        })
    }

    fn apply(&mut self, tick: &TradeTick) -> Result<(), FeedError> {
        let trade_turnover = checked_turnover(tick)?;
        let volume = self
            .volume
            .checked_add(tick.size)
            .ok_or_else(|| data_error("accumulated volume overflows Decimal"))?;
        let turnover = self
            .turnover
            .checked_add(trade_turnover)
            .ok_or_else(|| data_error("accumulated turnover overflows Decimal"))?;

        if tick.exchange_ts < self.open_ts {
            self.open = tick.price;
            self.open_ts = tick.exchange_ts;
        }
        if tick.exchange_ts >= self.close_ts {
            self.close = tick.price;
            self.close_ts = tick.exchange_ts;
        }
        self.high = self.high.max(tick.price);
        self.low = self.low.min(tick.price);
        self.volume = volume;
        self.turnover = turnover;
        Ok(())
    }
}

/// Validates the fields the aggregator relies on.
fn validate_trade(tick: &TradeTick) -> Result<(), FeedError> {
    if tick.price <= Decimal::ZERO {
        return Err(data_error("trade price must be strictly positive"));
    }
    if tick.size < Decimal::ZERO {
        return Err(data_error("trade size must be non-negative"));
    }
    Ok(())
}

/// Computes `price * size` with checked decimal arithmetic.
fn checked_turnover(tick: &TradeTick) -> Result<Decimal, FeedError> {
    tick.price
        .checked_mul(tick.size)
        .ok_or_else(|| data_error("trade turnover overflows Decimal"))
}

/// Aligns a Unix-millisecond timestamp down to its bucket start.
fn bucket_start_ms(timestamp_ms: i64, interval_ms: i64) -> Result<i64, FeedError> {
    timestamp_ms
        .div_euclid(interval_ms)
        .checked_mul(interval_ms)
        .ok_or_else(|| data_error("bar bucket timestamp overflows i64"))
}

/// Converts an open bucket into a finalized [`BarEvent`].
fn build_bar_event(
    instrument: InstrumentId,
    interval_secs: u64,
    open: &OpenBar,
) -> Result<BarEvent, FeedError> {
    let bar = OhlcvBar::new(
        open.bucket_start_ms,
        decimal_to_f64(open.open, "open")?,
        decimal_to_f64(open.high, "high")?,
        decimal_to_f64(open.low, "low")?,
        decimal_to_f64(open.close, "close")?,
        decimal_to_f64(open.volume, "volume")?,
        decimal_to_f64(open.turnover, "turnover")?,
    )
    .map_err(|error| data_error(format!("invalid aggregated bar: {error}")))?;
    Ok(BarEvent {
        instrument,
        interval_secs,
        bar,
        is_final: true,
    })
}

/// Converts a decimal to a finite `f64` for `kurumi-ta`.
fn decimal_to_f64(value: Decimal, field: &'static str) -> Result<f64, FeedError> {
    value
        .to_f64()
        .filter(|converted| converted.is_finite())
        .ok_or_else(|| data_error(format!("{field} cannot be represented as f64")))
}

/// Builds a protocol error attributed to the aggregator.
fn data_error(reason: impl Into<String>) -> FeedError {
    FeedError::ProtocolError {
        provider: PROVIDER.to_owned(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{InstrumentId, Venue};

    #[test]
    fn zero_interval_is_rejected() {
        assert!(matches!(
            BarAggregator::new(0),
            Err(FeedError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn bucket_alignment_floors_negative_timestamps() {
        assert_eq!(bucket_start_ms(0, 60_000), Ok(0));
        assert_eq!(bucket_start_ms(60_001, 60_000), Ok(60_000));
        assert_eq!(bucket_start_ms(-1, 60_000), Ok(-60_000));
    }

    #[test]
    fn non_positive_prices_are_rejected() {
        let tick = TradeTick {
            instrument: InstrumentId::new("AAPL", Venue::Nasdaq),
            price: Decimal::ZERO,
            size: Decimal::new(1, 0),
            trade_id: None,
            conditions: Vec::new(),
            exchange_ts: DateTime::<Utc>::UNIX_EPOCH,
            received_ts: DateTime::<Utc>::UNIX_EPOCH,
        };
        let mut aggregator = BarAggregator::new(60).expect("valid interval");
        assert!(aggregator.on_trade(&tick).is_err());
    }
}
