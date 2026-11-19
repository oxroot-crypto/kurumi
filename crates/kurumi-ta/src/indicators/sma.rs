//! Simple moving average.

use kurumi_core::KurumiError;

use crate::traits::StreamingIndicator;
use kurumi_core::ring_buffer::RollingWindow;

/// The simple moving average over a fixed window.
///
/// ```text
/// SMA_t(n) = (1 / n) * sum_{i=0}^{n-1} x_{t-i}
/// ```
///
/// The implementation keeps a compensated running sum in a fixed-capacity
/// window: pushing one observation adds it and subtracts the evicted one, so
/// each update is `O(1)` and no window traversal or allocation occurs after
/// construction.
///
/// # Warm-up
///
/// The first value is produced by the `n`-th observation.
///
/// # Examples
///
/// ```
/// use kurumi_ta::{Sma, StreamingIndicator};
///
/// # fn main() -> Result<(), kurumi_ta::KurumiError> {
/// let mut sma = Sma::new(3)?;
/// assert_eq!(sma.next(1.0)?, None);
/// assert_eq!(sma.next(2.0)?, None);
/// assert_eq!(sma.next(3.0)?, Some(2.0));
/// assert_eq!(sma.next(4.0)?, Some(3.0));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Sma {
    period: usize,
    window: RollingWindow,
}

impl Sma {
    /// Creates an SMA over `period` observations.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `period` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::Sma;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// assert!(Sma::new(0).is_err());
    /// assert_eq!(Sma::new(20)?.period(), 20);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(period: usize) -> Result<Self, KurumiError> {
        Ok(Self {
            period,
            window: RollingWindow::new(period)?,
        })
    }

    /// Returns the window length.
    #[must_use]
    #[inline]
    pub const fn period(&self) -> usize {
        self.period
    }

    /// Returns the current average, or `None` before warm-up completes.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::{Sma, StreamingIndicator};
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut sma = Sma::new(2)?;
    /// sma.next(2.0)?;
    /// assert_eq!(sma.value(), None);
    /// sma.next(4.0)?;
    /// assert_eq!(sma.value(), Some(3.0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn value(&self) -> Option<f64> {
        if self.window.is_full() {
            self.window.mean()
        } else {
            None
        }
    }
}

impl StreamingIndicator for Sma {
    type Input = f64;
    type Output = f64;

    #[inline]
    fn next(&mut self, input: f64) -> Result<Option<f64>, KurumiError> {
        self.window.push(input)?;
        Ok(self.value())
    }

    fn reset(&mut self) {
        self.window.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn sma_matches_the_arithmetic_mean_of_the_window() {
        let mut sma = Sma::new(3).expect("positive period");
        let outputs: Vec<_> = [1.0, 2.0, 3.0, 4.0, 5.0]
            .into_iter()
            .map(|value| sma.next(value).expect("finite value"))
            .collect();
        assert_eq!(outputs[0], None);
        assert_eq!(outputs[1], None);
        assert_relative_eq!(outputs[2].expect("warmed up"), 2.0, epsilon = 1e-15);
        assert_relative_eq!(outputs[3].expect("warmed up"), 3.0, epsilon = 1e-15);
        assert_relative_eq!(outputs[4].expect("warmed up"), 4.0, epsilon = 1e-15);
    }

    #[test]
    fn sma_of_a_constant_series_equals_the_constant() {
        let mut sma = Sma::new(4).expect("positive period");
        for _ in 0..20 {
            sma.next(7.5).expect("finite value");
        }
        assert_eq!(sma.value(), Some(7.5));
    }

    #[test]
    fn sma_rejects_non_finite_values_without_corrupting_state() {
        let mut sma = Sma::new(2).expect("positive period");
        sma.next(1.0).expect("finite value");
        assert!(sma.next(f64::NAN).is_err());
        assert_eq!(sma.value(), None);
        sma.next(3.0).expect("finite value");
        assert_eq!(sma.value(), Some(2.0));
    }

    #[test]
    fn sma_reset_restarts_the_warm_up() {
        let mut sma = Sma::new(2).expect("positive period");
        sma.next(1.0).expect("finite value");
        sma.next(2.0).expect("finite value");
        assert!(sma.value().is_some());
        sma.reset();
        assert_eq!(sma.next(10.0).expect("finite value"), None);
        assert_eq!(sma.next(20.0).expect("finite value"), Some(15.0));
    }
}
