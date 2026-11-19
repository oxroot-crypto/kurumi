//! Exponential moving average.

use kurumi_core::KurumiError;

use crate::traits::StreamingIndicator;
use crate::util::ensure_finite;

/// The exponential moving average.
///
/// ```text
/// alpha = 2 / (n + 1)
/// EMA_0 = mean(x_1, ..., x_n)                  (seed)
/// EMA_t = alpha * x_t + (1 - alpha) * EMA_{t-1}
/// ```
///
/// The seed is the arithmetic mean of the first `n` observations, which is the
/// convention used by most charting platforms and by the MACD definition. After
/// the seed every update is one fused multiply-add, so the cost is `O(1)` and
/// independent of the period.
///
/// # Warm-up
///
/// The first value is produced by the `n`-th observation.
///
/// # Examples
///
/// ```
/// use kurumi_ta::{Ema, StreamingIndicator};
///
/// # fn main() -> Result<(), kurumi_ta::KurumiError> {
/// let mut ema = Ema::new(3)?;
/// assert_eq!(ema.next(1.0)?, None);
/// assert_eq!(ema.next(2.0)?, None);
/// // Seed is (1 + 2 + 3) / 3 = 2, alpha = 0.5.
/// assert_eq!(ema.next(3.0)?, Some(2.0));
/// assert_eq!(ema.next(4.0)?, Some(3.0));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Ema {
    period: usize,
    alpha: f64,
    seed_count: usize,
    seed_sum: f64,
    value: Option<f64>,
}

impl Ema {
    /// Creates an EMA with the given smoothing period.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `period` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::Ema;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// assert!(Ema::new(0).is_err());
    /// let ema = Ema::new(10)?;
    /// assert_eq!(ema.period(), 10);
    /// assert!((ema.alpha() - 2.0 / 11.0).abs() < 1e-15);
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
            alpha: 2.0 / (period as f64 + 1.0),
            seed_count: 0,
            seed_sum: 0.0,
            value: None,
        })
    }

    /// Returns the smoothing period.
    #[must_use]
    #[inline]
    pub const fn period(&self) -> usize {
        self.period
    }

    /// Returns the smoothing factor `2 / (period + 1)`.
    #[must_use]
    #[inline]
    pub const fn alpha(&self) -> f64 {
        self.alpha
    }

    /// Returns the current EMA, or `None` before warm-up completes.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::{Ema, StreamingIndicator};
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut ema = Ema::new(2)?;
    /// ema.next(1.0)?;
    /// assert_eq!(ema.value(), None);
    /// ema.next(3.0)?;
    /// assert_eq!(ema.value(), Some(2.0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn value(&self) -> Option<f64> {
        self.value
    }
}

impl StreamingIndicator for Ema {
    type Input = f64;
    type Output = f64;

    #[inline]
    fn next(&mut self, input: f64) -> Result<Option<f64>, KurumiError> {
        ensure_finite(input, "price")?;

        if let Some(previous) = self.value {
            let next = self.alpha.mul_add(input, (1.0 - self.alpha) * previous);
            if !next.is_finite() {
                self.reset();
                return Err(KurumiError::NumericalOverflow {
                    operation: "Ema::next",
                });
            }
            self.value = Some(next);
            return Ok(Some(next));
        }

        self.seed_sum += input;
        self.seed_count += 1;
        if self.seed_count < self.period {
            return Ok(None);
        }

        let seed = self.seed_sum / self.period as f64;
        if !seed.is_finite() {
            self.reset();
            return Err(KurumiError::NumericalOverflow {
                operation: "Ema::next",
            });
        }
        self.value = Some(seed);
        Ok(Some(seed))
    }

    fn reset(&mut self) {
        self.seed_count = 0;
        self.seed_sum = 0.0;
        self.value = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn ema_is_seeded_with_the_sma_of_the_first_window() {
        let mut ema = Ema::new(3).expect("positive period");
        assert_eq!(ema.next(1.0).expect("finite value"), None);
        assert_eq!(ema.next(2.0).expect("finite value"), None);
        assert_relative_eq!(
            ema.next(3.0).expect("finite value").expect("warmed up"),
            2.0,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            ema.next(4.0).expect("finite value").expect("warmed up"),
            3.0,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            ema.next(5.0).expect("finite value").expect("warmed up"),
            4.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn ema_recursion_matches_the_closed_form_update() {
        let mut ema = Ema::new(2).expect("positive period");
        ema.next(2.0).expect("finite value");
        ema.next(4.0).expect("finite value");
        // alpha = 2 / 3: EMA = 2/3 * 10 + 1/3 * 3 = 23/3.
        assert_relative_eq!(
            ema.next(10.0).expect("finite value").expect("warmed up"),
            23.0 / 3.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn ema_of_a_constant_series_is_the_constant() {
        let mut ema = Ema::new(5).expect("positive period");
        for _ in 0..30 {
            ema.next(3.25).expect("finite value");
        }
        assert_relative_eq!(ema.value().expect("warmed up"), 3.25, epsilon = 1e-12);
    }

    #[test]
    fn ema_rejects_non_finite_values_and_reset_restarts_warm_up() {
        let mut ema = Ema::new(2).expect("positive period");
        assert!(ema.next(f64::NAN).is_err());
        ema.next(1.0).expect("finite value");
        ema.next(3.0).expect("finite value");
        assert!(ema.value().is_some());
        ema.reset();
        assert_eq!(ema.value(), None);
        assert_eq!(ema.next(5.0).expect("finite value"), None);
        assert_eq!(ema.next(7.0).expect("finite value"), Some(6.0));
    }
}
