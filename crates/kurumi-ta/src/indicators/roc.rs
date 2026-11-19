//! Rate of change.

use kurumi_core::KurumiError;

use crate::traits::StreamingIndicator;
use crate::util::ensure_finite;
use kurumi_core::ring_buffer::RingBuffer;

/// The rate of change over a lookback of `period` observations.
///
/// ```text
/// ROC_t(n) = 100 * (x_t - x_{t-n}) / x_{t-n}
/// ```
///
/// The result is a percentage. The reference observation `x_{t-n}` is read in
/// `O(1)` from the oldest slot of a fixed-capacity [`RingBuffer`]; no window
/// scan or allocation happens per update.
///
/// # Warm-up
///
/// The first value requires `period + 1` observations: the current price and
/// its `period`-observation-old reference.
///
/// # Examples
///
/// ```
/// use kurumi_ta::{Roc, StreamingIndicator};
///
/// # fn main() -> Result<(), kurumi_ta::KurumiError> {
/// let mut roc = Roc::new(1)?;
/// assert_eq!(roc.next(100.0)?, None);
/// assert_eq!(roc.next(110.0)?, Some(10.0));
/// assert_eq!(roc.next(99.0)?, Some(-10.0));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Roc {
    period: usize,
    window: RingBuffer<f64>,
}

impl Roc {
    /// Creates a rate-of-change indicator with the given lookback.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `period` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::Roc;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// assert!(Roc::new(0).is_err());
    /// assert_eq!(Roc::new(12)?.period(), 12);
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
        let capacity = period
            .checked_add(1)
            .ok_or(KurumiError::NumericalOverflow {
                operation: "Roc::new",
            })?;
        Ok(Self {
            period,
            window: RingBuffer::new(capacity)?,
        })
    }

    /// Returns the lookback length.
    #[must_use]
    #[inline]
    pub const fn period(&self) -> usize {
        self.period
    }

    /// Returns the current rate of change, or `None` before warm-up completes.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::{Roc, StreamingIndicator};
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut roc = Roc::new(2)?;
    /// roc.next(100.0)?;
    /// assert_eq!(roc.value(), None);
    /// roc.next(105.0)?;
    /// roc.next(100.0)?;
    /// assert_eq!(roc.value(), Some(0.0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn value(&self) -> Option<f64> {
        if !self.window.is_full() {
            return None;
        }
        let reference = *self.window.front()?;
        let current = *self.window.back()?;
        if reference == 0.0 {
            return None;
        }
        Some(100.0 * (current - reference) / reference)
    }
}

impl StreamingIndicator for Roc {
    type Input = f64;
    type Output = f64;

    #[inline]
    fn next(&mut self, input: f64) -> Result<Option<f64>, KurumiError> {
        ensure_finite(input, "price")?;
        self.window.push(input);

        let Some(reference) = self.window.front().copied() else {
            return Ok(None);
        };
        if !self.window.is_full() {
            return Ok(None);
        }
        if reference == 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "price",
                reason: "reference price from `period` observations ago must be non-zero"
                    .to_owned(),
            });
        }
        let current = match self.window.back() {
            Some(value) => *value,
            None => return Ok(None),
        };
        let value = 100.0 * (current - reference) / reference;
        if !value.is_finite() {
            self.reset();
            return Err(KurumiError::NumericalOverflow {
                operation: "Roc::next",
            });
        }
        Ok(Some(value))
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
    fn roc_matches_the_percentage_change() {
        let mut roc = Roc::new(1).expect("positive period");
        assert_eq!(roc.next(100.0).expect("finite value"), None);
        assert_relative_eq!(
            roc.next(110.0).expect("finite value").expect("warmed up"),
            10.0,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            roc.next(99.0).expect("finite value").expect("warmed up"),
            -10.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn roc_compares_against_the_observation_period_steps_back() {
        let mut roc = Roc::new(2).expect("positive period");
        assert_eq!(roc.next(100.0).expect("finite value"), None);
        assert_eq!(roc.next(105.0).expect("finite value"), None);
        assert_relative_eq!(
            roc.next(121.0).expect("finite value").expect("warmed up"),
            21.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn roc_of_a_constant_series_is_zero() {
        let mut roc = Roc::new(3).expect("positive period");
        for _ in 0..10 {
            roc.next(7.0).expect("finite value");
        }
        assert_relative_eq!(roc.value().expect("warmed up"), 0.0, epsilon = 1e-15);
    }

    #[test]
    fn roc_rejects_a_zero_reference_and_non_finite_prices() {
        let mut roc = Roc::new(1).expect("positive period");
        roc.next(0.0).expect("finite value");
        assert!(roc.next(1.0).is_err());

        let mut roc = Roc::new(1).expect("positive period");
        assert!(roc.next(f64::NAN).is_err());
    }

    #[test]
    fn roc_reset_restarts_warm_up() {
        let mut roc = Roc::new(2).expect("positive period");
        for value in [1.0, 2.0, 4.0] {
            roc.next(value).expect("finite value");
        }
        assert!(roc.value().is_some());
        roc.reset();
        assert_eq!(roc.next(1.0).expect("finite value"), None);
        assert_eq!(roc.next(2.0).expect("finite value"), None);
        assert_eq!(roc.next(3.0).expect("finite value"), Some(200.0));
    }
}
