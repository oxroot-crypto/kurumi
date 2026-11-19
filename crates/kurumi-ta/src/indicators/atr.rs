//! Average True Range with Wilder smoothing.

use kurumi_core::KurumiError;

use crate::bar::OhlcvBar;
use crate::traits::StreamingIndicator;

/// The Average True Range (ATR) with Wilder smoothing.
///
/// ```text
/// TR_t = max(high_t - low_t,
///            |high_t - close_{t-1}|,
///            |low_t  - close_{t-1}|)
///
/// first average: ATR = mean(TR_1..TR_n)
/// afterwards:    ATR = (ATR * (n - 1) + TR_t) / n
/// ```
///
/// The very first candle has no previous close, so its true range is simply
/// `high - low`. The indicator consumes validated [`OhlcvBar`] values; the
/// Wilder recursion is `O(1)` and allocates nothing.
///
/// # Warm-up
///
/// The first value is produced by the `n`-th candle.
///
/// # Examples
///
/// ```
/// use kurumi_ta::{Atr, OhlcvBar, StreamingIndicator};
///
/// # fn main() -> Result<(), kurumi_ta::KurumiError> {
/// let mut atr = Atr::new(2)?;
/// let first = OhlcvBar::new(0, 10.0, 10.0, 9.0, 9.5, 1.0, 9.5)?;
/// let second = OhlcvBar::new(1, 9.5, 11.0, 9.5, 10.5, 1.0, 10.5)?;
/// assert_eq!(atr.next(first)?, None);
/// // TR_1 = 1.0, TR_2 = 1.5, so ATR = 1.25.
/// assert_eq!(atr.next(second)?, Some(1.25));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Atr {
    period: usize,
    previous_close: Option<f64>,
    tr_count: usize,
    tr_sum: f64,
    atr: f64,
}

impl Atr {
    /// Creates an ATR with the given Wilder period, typically `14`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `period` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::Atr;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// assert!(Atr::new(0).is_err());
    /// assert_eq!(Atr::new(14)?.period(), 14);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(period: usize) -> Result<Self, KurumiError> {
        if period == 0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "period",
                reason: "must be strictly positive".to_owned(),
            });
        }
        Ok(Self {
            period,
            previous_close: None,
            tr_count: 0,
            tr_sum: 0.0,
            atr: 0.0,
        })
    }

    /// Returns the Wilder smoothing period.
    #[must_use]
    #[inline]
    pub const fn period(&self) -> usize {
        self.period
    }

    /// Returns the current ATR, or `None` before warm-up completes.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::{Atr, OhlcvBar, StreamingIndicator};
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut atr = Atr::new(1)?;
    /// let bar = OhlcvBar::new(0, 10.0, 11.0, 9.0, 10.0, 1.0, 10.0)?;
    /// assert_eq!(atr.value(), None);
    /// atr.next(bar)?;
    /// assert_eq!(atr.value(), Some(2.0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn value(&self) -> Option<f64> {
        if self.tr_count == self.period {
            Some(self.atr)
        } else {
            None
        }
    }
}

impl StreamingIndicator for Atr {
    type Input = OhlcvBar;
    type Output = f64;

    #[inline]
    fn next(&mut self, input: OhlcvBar) -> Result<Option<f64>, KurumiError> {
        let true_range = if let Some(previous_close) = self.previous_close {
            (input.high() - input.low())
                .max((input.high() - previous_close).abs())
                .max((input.low() - previous_close).abs())
        } else {
            input.high() - input.low()
        };
        if !true_range.is_finite() {
            self.reset();
            return Err(KurumiError::NumericalOverflow {
                operation: "Atr::next",
            });
        }
        self.previous_close = Some(input.close());

        if self.tr_count < self.period {
            self.tr_sum += true_range;
            self.tr_count += 1;
            if self.tr_count < self.period {
                return Ok(None);
            }
            self.atr = self.tr_sum / self.period as f64;
        } else {
            let period = self.period as f64;
            self.atr = (self.atr * (period - 1.0) + true_range) / period;
        }

        if !self.atr.is_finite() {
            self.reset();
            return Err(KurumiError::NumericalOverflow {
                operation: "Atr::next",
            });
        }
        Ok(Some(self.atr))
    }

    fn reset(&mut self) {
        self.previous_close = None;
        self.tr_count = 0;
        self.tr_sum = 0.0;
        self.atr = 0.0;
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
    fn atr_seeds_with_the_mean_true_range() {
        let mut atr = Atr::new(2).expect("positive period");
        let first = bar(0, 10.0, 10.0, 9.0, 9.5);
        let second = bar(1, 9.5, 11.0, 9.5, 10.5);
        assert_eq!(atr.next(first).expect("valid bar"), None);
        assert_relative_eq!(
            atr.next(second).expect("valid bar").expect("warmed up"),
            1.25,
            epsilon = 1e-15
        );
    }

    #[test]
    fn atr_uses_wilder_smoothing_after_the_seed() {
        let mut atr = Atr::new(2).expect("positive period");
        for value in [
            bar(0, 10.0, 10.0, 9.0, 9.5),
            bar(1, 9.5, 11.0, 9.5, 10.5),
            bar(2, 10.5, 14.0, 10.5, 13.0),
        ] {
            atr.next(value).expect("valid bar");
        }
        // Seed ATR = 1.25; TR_3 = 3.5; ATR_3 = (1.25 + 3.5) / 2 = 2.375.
        assert_relative_eq!(atr.value().expect("warmed up"), 2.375, epsilon = 1e-15);
    }

    #[test]
    fn atr_period_one_tracks_true_range() {
        let mut atr = Atr::new(1).expect("positive period");
        assert_eq!(
            atr.next(bar(0, 10.0, 11.0, 9.0, 10.0))
                .expect("valid bar")
                .expect("warmed up"),
            2.0
        );
        assert_relative_eq!(
            atr.next(bar(1, 10.0, 12.5, 10.0, 12.0))
                .expect("valid bar")
                .expect("warmed up"),
            2.5,
            epsilon = 1e-15
        );
    }

    #[test]
    fn atr_of_a_constant_bar_is_zero() {
        let mut atr = Atr::new(3).expect("positive period");
        for index in 0..10 {
            atr.next(bar(index, 10.0, 10.0, 10.0, 10.0))
                .expect("valid bar");
        }
        assert_eq!(atr.value(), Some(0.0));
    }

    #[test]
    fn atr_reset_restarts_warm_up() {
        let mut atr = Atr::new(2).expect("positive period");
        atr.next(bar(0, 10.0, 11.0, 9.0, 10.0)).expect("valid bar");
        atr.next(bar(1, 10.0, 11.0, 9.0, 10.0)).expect("valid bar");
        assert!(atr.value().is_some());
        atr.reset();
        assert_eq!(atr.value(), None);
        assert_eq!(
            atr.next(bar(0, 10.0, 11.0, 9.0, 10.0)).expect("valid bar"),
            None
        );
    }
}
