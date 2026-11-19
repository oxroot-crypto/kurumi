//! Validated open-high-low-close-volume (OHLCV) candles.
//!
//! [`OhlcvBar`] is the common input type of the bar-based indicators. Its
//! constructor enforces the invariants every technical indicator assumes:
//!
//! - the timestamp is an arbitrary signed Unix time in milliseconds;
//! - open, high, low, and close are finite and strictly positive;
//! - high is the maximum and low is the minimum of the four price fields;
//! - volume and turnover are finite and non-negative.
//!
//! Fields are private and exposed through accessors so an invalid candle can
//! never be handed to an indicator.

use kurumi_core::KurumiError;

/// A validated OHLCV candle.
///
/// # Conventions
///
/// - `timestamp` is Unix time in milliseconds (UTC). It is stored as-is and is
///   never rescaled; intraday and daily bars can share the same type.
/// - `open`, `high`, `low`, and `close` are prices in the quote currency and
///   must be finite and strictly positive.
/// - `volume` is traded base-unit volume and `turnover` is traded quote-currency
///   value (成交额). Both must be finite and non-negative.
/// - The candle is valid when `high >= max(open, close, low)` and
///   `low <= min(open, close, high)`.
///
/// # Examples
///
/// ```
/// use kurumi_ta::OhlcvBar;
///
/// # fn main() -> Result<(), kurumi_ta::KurumiError> {
/// let bar = OhlcvBar::new(1_700_000_000_000, 100.0, 103.0, 99.0, 102.0, 1_500.0, 152_500.0)?;
/// assert_eq!(bar.close(), 102.0);
/// assert_eq!(bar.range(), 4.0);
/// assert!(bar.is_bullish());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OhlcvBar {
    timestamp: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    turnover: f64,
}

impl OhlcvBar {
    /// Creates a candle after validating every OHLCV invariant.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when
    ///
    /// - any price, volume, or turnover is `NaN` or infinite;
    /// - open, high, low, or close is zero or negative;
    /// - `high` is below the open, close, or low;
    /// - `low` is above the open, close, or high;
    /// - volume or turnover is negative.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::OhlcvBar;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let bar = OhlcvBar::new(0, 10.0, 12.0, 9.5, 11.0, 100.0, 1_100.0)?;
    /// assert_eq!(bar.typical_price(), (12.0 + 9.5 + 11.0) / 3.0);
    ///
    /// assert!(OhlcvBar::new(0, 10.0, 9.0, 9.5, 11.0, 100.0, 1_100.0).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        timestamp: i64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: f64,
        turnover: f64,
    ) -> Result<Self, KurumiError> {
        for (parameter, price) in [
            ("open", open),
            ("high", high),
            ("low", low),
            ("close", close),
        ] {
            if !price.is_finite() {
                return Err(KurumiError::InvalidParameter {
                    parameter,
                    reason: "must be finite".to_owned(),
                });
            }
            if price <= 0.0 {
                return Err(KurumiError::InvalidParameter {
                    parameter,
                    reason: "must be strictly positive".to_owned(),
                });
            }
        }

        if high < open.max(close).max(low) {
            return Err(KurumiError::InvalidParameter {
                parameter: "high",
                reason: "must be greater than or equal to open, close, and low".to_owned(),
            });
        }
        if low > open.min(close).min(high) {
            return Err(KurumiError::InvalidParameter {
                parameter: "low",
                reason: "must be less than or equal to open, close, and high".to_owned(),
            });
        }

        for (parameter, value) in [("volume", volume), ("turnover", turnover)] {
            if !value.is_finite() {
                return Err(KurumiError::InvalidParameter {
                    parameter,
                    reason: "must be finite".to_owned(),
                });
            }
            if value < 0.0 {
                return Err(KurumiError::InvalidParameter {
                    parameter,
                    reason: "must be non-negative".to_owned(),
                });
            }
        }

        Ok(Self {
            timestamp,
            open,
            high,
            low,
            close,
            volume,
            turnover,
        })
    }

    /// Returns the bar timestamp in Unix milliseconds (UTC).
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::OhlcvBar;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let bar = OhlcvBar::new(1_700_000_000_000, 10.0, 11.0, 9.0, 10.5, 5.0, 52.5)?;
    /// assert_eq!(bar.timestamp(), 1_700_000_000_000);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn timestamp(self) -> i64 {
        self.timestamp
    }

    /// Returns the open price.
    #[must_use]
    #[inline]
    pub const fn open(self) -> f64 {
        self.open
    }

    /// Returns the high price.
    #[must_use]
    #[inline]
    pub const fn high(self) -> f64 {
        self.high
    }

    /// Returns the low price.
    #[must_use]
    #[inline]
    pub const fn low(self) -> f64 {
        self.low
    }

    /// Returns the close price.
    #[must_use]
    #[inline]
    pub const fn close(self) -> f64 {
        self.close
    }

    /// Returns the traded base-unit volume.
    #[must_use]
    #[inline]
    pub const fn volume(self) -> f64 {
        self.volume
    }

    /// Returns the traded quote-currency turnover (成交额).
    #[must_use]
    #[inline]
    pub const fn turnover(self) -> f64 {
        self.turnover
    }

    /// Returns `high - low`, the full range of the candle.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::OhlcvBar;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let bar = OhlcvBar::new(0, 10.0, 12.5, 9.0, 11.0, 100.0, 1_100.0)?;
    /// assert_eq!(bar.range(), 3.5);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn range(self) -> f64 {
        self.high - self.low
    }

    /// Returns `(high + low + close) / 3`, the typical price used by several
    /// volume-weighted studies.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::OhlcvBar;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let bar = OhlcvBar::new(0, 10.0, 12.0, 9.0, 11.0, 100.0, 1_100.0)?;
    /// assert_eq!(bar.typical_price(), 32.0 / 3.0);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn typical_price(self) -> f64 {
        (self.high + self.low + self.close) / 3.0
    }

    /// Returns `true` when the candle closed at or above its open.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::OhlcvBar;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let up = OhlcvBar::new(0, 10.0, 11.0, 9.0, 10.5, 1.0, 10.5)?;
    /// let down = OhlcvBar::new(1, 10.5, 11.0, 9.0, 10.0, 1.0, 10.0)?;
    /// assert!(up.is_bullish());
    /// assert!(!down.is_bullish());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn is_bullish(self) -> bool {
        self.close >= self.open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar() -> OhlcvBar {
        OhlcvBar::new(
            1_700_000_000_000,
            100.0,
            103.0,
            99.0,
            102.0,
            1_500.0,
            152_500.0,
        )
        .expect("valid bar")
    }

    #[test]
    fn accessors_round_trip_the_constructor_arguments() {
        let bar = bar();
        assert_eq!(bar.timestamp(), 1_700_000_000_000);
        assert_eq!(bar.open(), 100.0);
        assert_eq!(bar.high(), 103.0);
        assert_eq!(bar.low(), 99.0);
        assert_eq!(bar.close(), 102.0);
        assert_eq!(bar.volume(), 1_500.0);
        assert_eq!(bar.turnover(), 152_500.0);
    }

    #[test]
    fn derived_quantities_follow_their_definitions() {
        let bar = bar();
        assert_eq!(bar.range(), 4.0);
        assert_eq!(bar.typical_price(), (103.0 + 99.0 + 102.0) / 3.0);
        assert!(bar.is_bullish());
    }

    #[test]
    fn high_must_dominate_every_other_price() {
        assert!(OhlcvBar::new(0, 10.0, 9.5, 9.0, 10.0, 1.0, 10.0).is_err());
        assert!(OhlcvBar::new(0, 10.0, 10.0, 9.0, 10.5, 1.0, 10.5).is_err());
        assert!(OhlcvBar::new(0, 10.0, 10.0, 10.5, 10.0, 1.0, 10.0).is_err());
    }

    #[test]
    fn low_must_be_dominated_by_every_other_price() {
        assert!(OhlcvBar::new(0, 10.0, 11.0, 9.5, 9.0, 1.0, 9.0).is_err());
        assert!(OhlcvBar::new(0, 10.0, 11.0, 10.5, 10.0, 1.0, 10.0).is_err());
        assert!(OhlcvBar::new(0, 10.0, 9.0, 9.5, 10.0, 1.0, 10.0).is_err());
    }

    #[test]
    fn prices_must_be_strictly_positive_and_finite() {
        assert!(OhlcvBar::new(0, 0.0, 1.0, 0.0, 0.5, 1.0, 0.5).is_err());
        assert!(OhlcvBar::new(0, -1.0, 1.0, -2.0, 0.5, 1.0, 0.5).is_err());
        assert!(OhlcvBar::new(0, f64::NAN, 1.0, 0.5, 0.7, 1.0, 0.7).is_err());
        assert!(OhlcvBar::new(0, 1.0, f64::INFINITY, 0.5, 0.7, 1.0, 0.7).is_err());
    }

    #[test]
    fn volume_and_turnover_must_be_finite_and_non_negative() {
        assert!(OhlcvBar::new(0, 1.0, 2.0, 0.5, 1.5, -1.0, 1.5).is_err());
        assert!(OhlcvBar::new(0, 1.0, 2.0, 0.5, 1.5, 1.0, -1.5).is_err());
        assert!(OhlcvBar::new(0, 1.0, 2.0, 0.5, 1.5, f64::NAN, 1.5).is_err());
        assert!(OhlcvBar::new(0, 1.0, 2.0, 0.5, 1.5, 1.0, f64::NAN).is_err());
    }

    #[test]
    fn zero_volume_flat_candles_are_valid() {
        let flat =
            OhlcvBar::new(0, 10.0, 10.0, 10.0, 10.0, 0.0, 0.0).expect("flat but valid candle");
        assert_eq!(flat.range(), 0.0);
        assert!(flat.is_bullish());
    }
}
