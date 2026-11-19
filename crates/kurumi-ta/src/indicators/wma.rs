//! Linearly weighted moving average.

use kurumi_core::KurumiError;

use crate::ring_buffer::RingBuffer;
use crate::traits::StreamingIndicator;
use crate::util::ensure_finite;

/// The linearly weighted moving average.
///
/// ```text
/// WMA_t(n) = sum_{i=0}^{n-1} (n - i) * x_{t-i} / (n * (n + 1) / 2)
/// ```
///
/// The newest observation has weight `n` and the oldest has weight `1`. The
/// weighted numerator and the plain window sum are both updated incrementally:
/// when `x` arrives, the numerator becomes
/// `n * x + numerator - previous_window_sum`, which is `O(1)` and never
/// traverses the window. The window itself is a fixed-capacity
/// [`RingBuffer`] allocated once in the constructor.
///
/// # Warm-up
///
/// The first value is produced by the `n`-th observation.
///
/// # Examples
///
/// ```
/// use kurumi_ta::{Wma, StreamingIndicator};
///
/// # fn main() -> Result<(), kurumi_ta::KurumiError> {
/// let mut wma = Wma::new(3)?;
/// assert_eq!(wma.next(1.0)?, None);
/// assert_eq!(wma.next(2.0)?, None);
/// // (1*1 + 2*2 + 3*3) / 6
/// assert_eq!(wma.next(3.0)?, Some(14.0 / 6.0));
/// // (2*1 + 3*2 + 4*3) / 6
/// assert_eq!(wma.next(4.0)?, Some(20.0 / 6.0));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Wma {
    period: usize,
    window: RingBuffer<f64>,
    window_sum: f64,
    weighted_sum: f64,
    denominator: f64,
}

impl Wma {
    /// Creates a WMA over `period` observations.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `period` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::Wma;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// assert!(Wma::new(0).is_err());
    /// assert_eq!(Wma::new(5)?.period(), 5);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(period: usize) -> Result<Self, KurumiError> {
        let denominator = period as f64 * (period as f64 + 1.0) / 2.0;
        Ok(Self {
            period,
            window: RingBuffer::new(period)?,
            window_sum: 0.0,
            weighted_sum: 0.0,
            denominator,
        })
    }

    /// Returns the window length.
    #[must_use]
    #[inline]
    pub const fn period(&self) -> usize {
        self.period
    }

    /// Returns the current WMA, or `None` before warm-up completes.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::{Wma, StreamingIndicator};
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut wma = Wma::new(2)?;
    /// wma.next(1.0)?;
    /// assert_eq!(wma.value(), None);
    /// wma.next(3.0)?;
    /// assert_eq!(wma.value(), Some(7.0 / 3.0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn value(&self) -> Option<f64> {
        if self.window.is_full() {
            Some(self.weighted_sum / self.denominator)
        } else {
            None
        }
    }
}

impl StreamingIndicator for Wma {
    type Input = f64;
    type Output = f64;

    #[inline]
    fn next(&mut self, input: f64) -> Result<Option<f64>, KurumiError> {
        ensure_finite(input, "price")?;

        let period = self.period as f64;
        self.weighted_sum = period.mul_add(input, self.weighted_sum - self.window_sum);
        self.window_sum += input;
        if let Some(evicted) = self.window.push(input) {
            self.window_sum -= evicted;
        }

        if !self.weighted_sum.is_finite() || !self.window_sum.is_finite() {
            self.reset();
            return Err(KurumiError::NumericalOverflow {
                operation: "Wma::next",
            });
        }
        Ok(self.value())
    }

    fn reset(&mut self) {
        self.window.clear();
        self.window_sum = 0.0;
        self.weighted_sum = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn wma_weights_the_newest_value_most() {
        let mut wma = Wma::new(3).expect("positive period");
        assert_eq!(wma.next(1.0).expect("finite value"), None);
        assert_eq!(wma.next(2.0).expect("finite value"), None);
        assert_relative_eq!(
            wma.next(3.0).expect("finite value").expect("warmed up"),
            14.0 / 6.0,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            wma.next(4.0).expect("finite value").expect("warmed up"),
            20.0 / 6.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn wma_of_a_constant_series_equals_the_constant() {
        let mut wma = Wma::new(4).expect("positive period");
        for _ in 0..16 {
            wma.next(2.5).expect("finite value");
        }
        assert_relative_eq!(wma.value().expect("warmed up"), 2.5, epsilon = 1e-15);
    }

    #[test]
    fn wma_period_one_reduces_to_the_identity() {
        let mut wma = Wma::new(1).expect("positive period");
        for value in [3.0, -1.5, 8.25] {
            assert_eq!(
                wma.next(value).expect("finite value"),
                Some(value),
                "period-one WMA is the input"
            );
        }
    }

    #[test]
    fn wma_rejects_non_finite_values_and_reset_restarts_warm_up() {
        let mut wma = Wma::new(2).expect("positive period");
        assert!(wma.next(f64::INFINITY).is_err());
        wma.next(1.0).expect("finite value");
        wma.next(2.0).expect("finite value");
        assert!(wma.value().is_some());
        wma.reset();
        assert_eq!(wma.next(2.0).expect("finite value"), None);
        assert_relative_eq!(
            wma.next(4.0).expect("finite value").expect("warmed up"),
            10.0 / 3.0,
            epsilon = 1e-15
        );
    }
}
