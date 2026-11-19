//! Stochastic oscillator (KD line).

use kurumi_core::KurumiError;

use crate::bar::OhlcvBar;
use crate::indicators::Sma;
use crate::traits::StreamingIndicator;
use kurumi_core::ring_buffer::RollingWindow;

/// The `%K` and `%D` values of the stochastic oscillator at one candle.
///
/// ```text
/// %K_t = 100 * (close_t - lowest_low(n)) / (highest_high(n) - lowest_low(n))
/// %D_t = SMA(%K, d)
/// ```
///
/// The values are percentages in the closed interval `[0, 100]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StochasticOutput {
    /// The raw stochastic value `%K`.
    pub k: f64,
    /// The `d`-period simple moving average of `%K`.
    pub d: f64,
}

/// The stochastic oscillator (KD indicator).
///
/// ```text
/// %K_t = 100 * (close_t - min(low_{t-n+1..t})) / (max(high_{t-n+1..t}) - min(low_{t-n+1..t}))
/// %D_t = SMA(%K, d)
/// ```
///
/// The highest high and lowest low are maintained by two [`RollingWindow`]
/// instances, so the raw `%K` update is `O(1)` with no window scan. `%D` is an
/// [`Sma`] over the `%K` stream. When the `n`-candle range is exactly zero the
/// oscillator is undefined; this implementation returns the neutral value `50`
/// for `%K`, which keeps `%D` continuous for flat markets.
///
/// # Warm-up
///
/// `%K` is available after `k_period` candles. The combined output is available
/// after `k_period + d_period - 1` candles, when `%D` has `d_period` `%K`
/// values.
///
/// # Examples
///
/// ```
/// use kurumi_ta::{OhlcvBar, Stochastic, StreamingIndicator};
///
/// # fn main() -> Result<(), kurumi_ta::KurumiError> {
/// let mut stochastic = Stochastic::new(3, 1)?;
/// let bars = [
///     OhlcvBar::new(0, 10.0, 11.0, 9.0, 10.0, 1.0, 10.0)?,
///     OhlcvBar::new(1, 10.0, 12.0, 9.0, 11.0, 1.0, 11.0)?,
///     OhlcvBar::new(2, 11.0, 13.0, 10.0, 12.0, 1.0, 12.0)?,
/// ];
/// assert_eq!(stochastic.next(bars[0])?, None);
/// assert_eq!(stochastic.next(bars[1])?, None);
/// let output = stochastic.next(bars[2])?.expect("warmed up");
/// // Range 9..13, close 12: %K = 100 * 3 / 4 = 75.
/// assert_eq!(output.k, 75.0);
/// assert_eq!(output.d, 75.0);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Stochastic {
    k_period: usize,
    d_period: usize,
    highs: RollingWindow,
    lows: RollingWindow,
    d: Sma,
    latest_close: Option<f64>,
}

impl Stochastic {
    /// Creates a stochastic oscillator with the given `%K` and `%D` periods.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when either period is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::Stochastic;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// assert!(Stochastic::new(0, 3).is_err());
    /// assert!(Stochastic::new(14, 0).is_err());
    /// let stochastic = Stochastic::new(14, 3)?;
    /// assert_eq!(stochastic.k_period(), 14);
    /// assert_eq!(stochastic.d_period(), 3);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(k_period: usize, d_period: usize) -> Result<Self, KurumiError> {
        Ok(Self {
            k_period,
            d_period,
            highs: RollingWindow::new(k_period)?,
            lows: RollingWindow::new(k_period)?,
            d: Sma::new(d_period)?,
            latest_close: None,
        })
    }

    /// Returns the `%K` lookback period.
    #[must_use]
    #[inline]
    pub const fn k_period(&self) -> usize {
        self.k_period
    }

    /// Returns the `%D` smoothing period.
    #[must_use]
    #[inline]
    pub const fn d_period(&self) -> usize {
        self.d_period
    }

    /// Returns the current output, or `None` before warm-up completes.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::{OhlcvBar, Stochastic, StreamingIndicator};
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut stochastic = Stochastic::new(2, 1)?;
    /// let flat = OhlcvBar::new(0, 10.0, 10.0, 10.0, 10.0, 1.0, 10.0)?;
    /// stochastic.next(flat)?;
    /// assert_eq!(stochastic.value(), None);
    /// stochastic.next(flat)?;
    /// assert_eq!(stochastic.value().expect("warmed up").k, 50.0);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn value(&self) -> Option<StochasticOutput> {
        if !self.highs.is_full() {
            return None;
        }
        let highest = self.highs.max()?;
        let lowest = self.lows.min()?;
        let close = self.latest_close?;
        let range = highest - lowest;
        let k = if range > 0.0 {
            100.0 * (close - lowest) / range
        } else {
            50.0
        };
        let d = self.d.value()?;
        Some(StochasticOutput { k, d })
    }
}

impl StreamingIndicator for Stochastic {
    type Input = OhlcvBar;
    type Output = StochasticOutput;

    #[inline]
    fn next(&mut self, input: OhlcvBar) -> Result<Option<StochasticOutput>, KurumiError> {
        self.highs.push(input.high())?;
        self.lows.push(input.low())?;
        self.latest_close = Some(input.close());

        if !self.highs.is_full() {
            return Ok(None);
        }

        let (Some(highest), Some(lowest)) = (self.highs.max(), self.lows.min()) else {
            return Ok(None);
        };
        let range = highest - lowest;
        let k = if range > 0.0 {
            100.0 * (input.close() - lowest) / range
        } else {
            50.0
        };

        let Some(d) = self.d.next(k)? else {
            return Ok(None);
        };
        Ok(Some(StochasticOutput { k, d }))
    }

    fn reset(&mut self) {
        self.highs.clear();
        self.lows.clear();
        self.d.reset();
        self.latest_close = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn bar(timestamp: i64, open: f64, high: f64, low: f64, close: f64) -> OhlcvBar {
        OhlcvBar::new(timestamp, open, high, low, close, 1.0, close).expect("valid bar")
    }

    #[test]
    fn stochastic_k_matches_the_range_position() {
        let mut stochastic = Stochastic::new(3, 1).expect("positive periods");
        let bars = [
            bar(0, 10.0, 11.0, 9.0, 10.0),
            bar(1, 10.0, 12.0, 9.0, 11.0),
            bar(2, 11.0, 13.0, 10.0, 12.0),
        ];
        assert_eq!(stochastic.next(bars[0]).expect("valid bar"), None);
        assert_eq!(stochastic.next(bars[1]).expect("valid bar"), None);
        let output = stochastic
            .next(bars[2])
            .expect("valid bar")
            .expect("warmed up");
        assert_relative_eq!(output.k, 75.0, epsilon = 1e-15);
        assert_relative_eq!(output.d, 75.0, epsilon = 1e-15);
    }

    #[test]
    fn stochastic_d_is_the_sma_of_k() {
        let mut stochastic = Stochastic::new(2, 2).expect("positive periods");
        let bars = [
            bar(0, 10.0, 11.0, 9.0, 10.0),
            bar(1, 10.0, 12.0, 10.0, 11.0),
            bar(2, 11.0, 13.0, 11.0, 12.0),
            bar(3, 12.0, 14.0, 12.0, 13.0),
        ];
        assert_eq!(stochastic.next(bars[0]).expect("valid bar"), None);
        assert_eq!(stochastic.next(bars[1]).expect("valid bar"), None);
        let first = stochastic
            .next(bars[2])
            .expect("valid bar")
            .expect("d available");
        assert_relative_eq!(first.k, 200.0 / 3.0, epsilon = 1e-15);
        assert_relative_eq!(first.d, 200.0 / 3.0, epsilon = 1e-15);
        let second = stochastic
            .next(bars[3])
            .expect("valid bar")
            .expect("d available");
        assert_relative_eq!(second.k, 200.0 / 3.0, epsilon = 1e-15);
        assert_relative_eq!(second.d, 200.0 / 3.0, epsilon = 1e-15);
    }

    #[test]
    fn flat_range_uses_the_neutral_value() {
        let mut stochastic = Stochastic::new(2, 1).expect("positive periods");
        let flat = bar(0, 10.0, 10.0, 10.0, 10.0);
        stochastic.next(flat).expect("valid bar");
        let output = stochastic
            .next(flat)
            .expect("valid bar")
            .expect("warmed up");
        assert_eq!(output.k, 50.0);
        assert_eq!(output.d, 50.0);
    }

    #[test]
    fn stochastic_rejects_zero_periods_and_reset_restarts() {
        assert!(Stochastic::new(0, 3).is_err());
        assert!(Stochastic::new(3, 0).is_err());

        let mut stochastic = Stochastic::new(2, 1).expect("positive periods");
        let bar = bar(0, 10.0, 11.0, 9.0, 10.0);
        stochastic.next(bar).expect("valid bar");
        stochastic.next(bar).expect("valid bar");
        assert!(stochastic.value().is_some());
        stochastic.reset();
        assert_eq!(stochastic.value(), None);
        assert_eq!(stochastic.next(bar).expect("valid bar"), None);
    }
}
