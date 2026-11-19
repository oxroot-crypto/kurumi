//! Moving average convergence/divergence.

use kurumi_core::KurumiError;

use crate::indicators::Ema;
use crate::traits::StreamingIndicator;

/// The MACD line, signal line, and histogram at one observation.
///
/// ```text
/// MACD_t   = EMA_fast(x)_t - EMA_slow(x)_t
/// signal_t = EMA_signal(MACD)_t
/// hist_t   = MACD_t - signal_t
/// ```
///
/// The values are dimensionless price differences and are reported in the same
/// price units as the input closes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MacdOutput {
    /// The fast-minus-slow EMA difference.
    pub macd: f64,
    /// The signal EMA of the MACD line.
    pub signal: f64,
    /// `macd - signal`.
    pub histogram: f64,
}

/// Moving average convergence/divergence.
///
/// ```text
/// MACD_t   = EMA(x, fast) - EMA(x, slow)
/// signal_t = EMA(MACD, signal)
/// hist_t   = MACD_t - signal_t
/// ```
///
/// All three exponential averages use the [`Ema`] seed convention: an EMA is
/// seeded with the simple mean of its first `period` inputs. Because every EMA
/// is an `O(1)` recursive update, one MACD observation is `O(1)` overall and
/// allocates nothing after construction.
///
/// # Warm-up
///
/// The MACD line starts when both EMAs are seeded, at observation
/// `slow_period`. The signal line starts `signal_period` MACD values later, so
/// the first complete output is observation
/// `slow_period + signal_period - 1`.
///
/// # Examples
///
/// ```
/// use kurumi_ta::{Macd, StreamingIndicator};
///
/// # fn main() -> Result<(), kurumi_ta::KurumiError> {
/// let mut macd = Macd::new(2, 3, 2)?;
/// // A constant series has zero MACD, zero signal, and zero histogram.
/// for _ in 0..5 {
///     macd.next(10.0)?;
/// }
/// let output = macd.next(10.0)?.expect("warmed up");
/// assert_eq!(output.macd, 0.0);
/// assert_eq!(output.signal, 0.0);
/// assert_eq!(output.histogram, 0.0);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Macd {
    fast_period: usize,
    slow_period: usize,
    signal_period: usize,
    fast: Ema,
    slow: Ema,
    signal: Ema,
}

impl Macd {
    /// Creates a MACD with the given fast, slow, and signal periods.
    ///
    /// The usual parameters are `(12, 26, 9)`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when any period is zero or
    /// when `fast_period` is not strictly smaller than `slow_period`.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::Macd;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// assert!(Macd::new(26, 12, 9).is_err());
    /// assert!(Macd::new(0, 26, 9).is_err());
    /// let macd = Macd::new(12, 26, 9)?;
    /// assert_eq!(macd.fast_period(), 12);
    /// assert_eq!(macd.slow_period(), 26);
    /// assert_eq!(macd.signal_period(), 9);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        fast_period: usize,
        slow_period: usize,
        signal_period: usize,
    ) -> Result<Self, KurumiError> {
        for (parameter, period) in [
            ("fast_period", fast_period),
            ("slow_period", slow_period),
            ("signal_period", signal_period),
        ] {
            if period == 0 {
                return Err(KurumiError::InvalidParameter {
                    parameter,
                    reason: "must be strictly positive".to_owned(),
                });
            }
        }
        if fast_period >= slow_period {
            return Err(KurumiError::InvalidParameter {
                parameter: "fast_period",
                reason: "must be strictly smaller than slow_period".to_owned(),
            });
        }
        Ok(Self {
            fast_period,
            slow_period,
            signal_period,
            fast: Ema::new(fast_period)?,
            slow: Ema::new(slow_period)?,
            signal: Ema::new(signal_period)?,
        })
    }

    /// Returns the fast EMA period.
    #[must_use]
    #[inline]
    pub const fn fast_period(&self) -> usize {
        self.fast_period
    }

    /// Returns the slow EMA period.
    #[must_use]
    #[inline]
    pub const fn slow_period(&self) -> usize {
        self.slow_period
    }

    /// Returns the signal EMA period.
    #[must_use]
    #[inline]
    pub const fn signal_period(&self) -> usize {
        self.signal_period
    }

    /// Returns the current MACD output, or `None` before warm-up completes.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::{Macd, StreamingIndicator};
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut macd = Macd::new(2, 3, 1)?;
    /// for _ in 0..3 {
    ///     macd.next(10.0)?;
    /// }
    /// assert!(macd.value().is_some());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn value(&self) -> Option<MacdOutput> {
        let macd = self.fast.value()? - self.slow.value()?;
        let signal = self.signal.value()?;
        Some(MacdOutput {
            macd,
            signal,
            histogram: macd - signal,
        })
    }
}

impl StreamingIndicator for Macd {
    type Input = f64;
    type Output = MacdOutput;

    #[inline]
    fn next(&mut self, input: f64) -> Result<Option<MacdOutput>, KurumiError> {
        let fast = self.fast.next(input)?;
        let slow = self.slow.next(input)?;

        let (Some(fast), Some(slow)) = (fast, slow) else {
            return Ok(None);
        };
        let macd = fast - slow;
        if !macd.is_finite() {
            self.reset();
            return Err(KurumiError::NumericalOverflow {
                operation: "Macd::next",
            });
        }
        let Some(signal) = self.signal.next(macd)? else {
            return Ok(None);
        };
        let histogram = macd - signal;
        if !histogram.is_finite() {
            self.reset();
            return Err(KurumiError::NumericalOverflow {
                operation: "Macd::next",
            });
        }
        Ok(Some(MacdOutput {
            macd,
            signal,
            histogram,
        }))
    }

    fn reset(&mut self) {
        self.fast.reset();
        self.slow.reset();
        self.signal.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn macd_warms_up_after_slow_plus_signal_minus_one_observations() {
        let mut macd = Macd::new(2, 3, 2).expect("valid periods");
        let outputs: Vec<_> = (0..6)
            .map(|_| macd.next(10.0).expect("finite value"))
            .collect();
        assert_eq!(outputs[0], None);
        assert_eq!(outputs[1], None);
        assert_eq!(outputs[2], None);
        assert!(outputs[3].is_some());
        assert!(outputs[4].is_some());
        assert!(outputs[5].is_some());
    }

    #[test]
    fn macd_of_a_constant_series_is_zero() {
        let mut macd = Macd::new(2, 3, 2).expect("valid periods");
        for _ in 0..10 {
            macd.next(5.0).expect("finite value");
        }
        let output = macd.value().expect("warmed up");
        assert_relative_eq!(output.macd, 0.0, epsilon = 1e-15);
        assert_relative_eq!(output.signal, 0.0, epsilon = 1e-15);
        assert_relative_eq!(output.histogram, 0.0, epsilon = 1e-15);
    }

    #[test]
    fn macd_line_and_histogram_are_consistent() {
        let mut macd = Macd::new(2, 3, 2).expect("valid periods");
        let mut last = None;
        for value in [1.0, 2.0, 3.0, 5.0, 8.0, 13.0, 21.0, 34.0] {
            last = macd.next(value).expect("finite value").or(last);
        }
        let output = last.expect("warmed up");
        assert_relative_eq!(
            output.histogram,
            output.macd - output.signal,
            epsilon = 1e-15
        );
    }

    #[test]
    fn macd_rejects_invalid_parameters() {
        assert!(Macd::new(0, 26, 9).is_err());
        assert!(Macd::new(12, 0, 9).is_err());
        assert!(Macd::new(12, 26, 0).is_err());
        assert!(Macd::new(12, 12, 9).is_err());
        assert!(Macd::new(26, 12, 9).is_err());
    }

    #[test]
    fn macd_reset_restarts_warm_up() {
        let mut macd = Macd::new(2, 3, 2).expect("valid periods");
        for _ in 0..6 {
            macd.next(10.0).expect("finite value");
        }
        assert!(macd.value().is_some());
        macd.reset();
        assert_eq!(macd.value(), None);
        assert_eq!(macd.next(10.0).expect("finite value"), None);
    }
}
