//! Relative Strength Index with Wilder smoothing.

use kurumi_core::KurumiError;

use crate::traits::StreamingIndicator;
use crate::util::ensure_finite;

/// The Relative Strength Index (RSI) with Wilder smoothing.
///
/// ```text
/// gain_t = max(x_t - x_{t-1}, 0)
/// loss_t = max(x_{t-1} - x_t, 0)
///
/// first average: avg_gain = mean(gain_1..gain_n), avg_loss = mean(loss_1..loss_n)
/// afterwards:    avg_gain = (avg_gain * (n - 1) + gain_t) / n
///                avg_loss = (avg_loss * (n - 1) + loss_t) / n
///
/// RS  = avg_gain / avg_loss
/// RSI = 100 - 100 / (1 + RS)
/// ```
///
/// The indicator consumes close prices. Gaps between observations are treated
/// as one period each; this is the standard closing-price RSI. The recursive
/// Wilder averages are `O(1)` per update.
///
/// # Degenerate windows
///
/// When `avg_loss` is zero the ratio is undefined; this implementation returns
/// `100` for a window with only gains and `50` for a perfectly flat window
/// (both averages zero). Those are the conventional neutral/bullish limits and
/// are documented so callers can special-case them if their study requires a
/// different convention.
///
/// # Warm-up
///
/// The first value requires `period + 1` prices: `period` changes are needed to
/// seed the averages.
///
/// # Examples
///
/// ```
/// use kurumi_ta::{Rsi, StreamingIndicator};
///
/// # fn main() -> Result<(), kurumi_ta::KurumiError> {
/// let mut rsi = Rsi::new(2)?;
/// assert_eq!(rsi.next(1.0)?, None);
/// assert_eq!(rsi.next(2.0)?, None);
/// assert_eq!(rsi.next(3.0)?, Some(100.0));
/// // One gain and one loss of equal size leave RS = 1 and RSI = 50.
/// assert_eq!(rsi.next(2.0)?, Some(50.0));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Rsi {
    period: usize,
    previous_close: Option<f64>,
    changes: usize,
    seed_gain: f64,
    seed_loss: f64,
    avg_gain: f64,
    avg_loss: f64,
}

impl Rsi {
    /// Creates an RSI with the given Wilder period, typically `14`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `period` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::Rsi;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// assert!(Rsi::new(0).is_err());
    /// assert_eq!(Rsi::new(14)?.period(), 14);
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
            changes: 0,
            seed_gain: 0.0,
            seed_loss: 0.0,
            avg_gain: 0.0,
            avg_loss: 0.0,
        })
    }

    /// Returns the Wilder smoothing period.
    #[must_use]
    #[inline]
    pub const fn period(&self) -> usize {
        self.period
    }

    /// Returns the current average gain, or `None` before warm-up completes.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::{Rsi, StreamingIndicator};
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut rsi = Rsi::new(2)?;
    /// rsi.next(1.0)?;
    /// rsi.next(2.0)?;
    /// assert_eq!(rsi.average_gain(), None);
    /// rsi.next(3.0)?;
    /// assert_eq!(rsi.average_gain(), Some(1.0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn average_gain(&self) -> Option<f64> {
        if self.changes == self.period && self.period > 0 {
            Some(self.avg_gain)
        } else {
            None
        }
    }

    /// Returns the current average loss, or `None` before warm-up completes.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::{Rsi, StreamingIndicator};
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut rsi = Rsi::new(2)?;
    /// rsi.next(3.0)?;
    /// rsi.next(2.0)?;
    /// rsi.next(1.0)?;
    /// assert_eq!(rsi.average_loss(), Some(1.0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn average_loss(&self) -> Option<f64> {
        if self.changes == self.period && self.period > 0 {
            Some(self.avg_loss)
        } else {
            None
        }
    }

    /// Returns the current RSI value, or `None` before warm-up completes.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::{Rsi, StreamingIndicator};
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut rsi = Rsi::new(2)?;
    /// for price in [1.0, 2.0, 3.0] {
    ///     rsi.next(price)?;
    /// }
    /// assert_eq!(rsi.value(), Some(100.0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn value(&self) -> Option<f64> {
        if self.changes == self.period && self.period > 0 {
            Some(rsi_from_averages(self.avg_gain, self.avg_loss))
        } else {
            None
        }
    }
}

impl StreamingIndicator for Rsi {
    type Input = f64;
    type Output = f64;

    #[inline]
    fn next(&mut self, input: f64) -> Result<Option<f64>, KurumiError> {
        ensure_finite(input, "price")?;

        let Some(previous) = self.previous_close else {
            self.previous_close = Some(input);
            return Ok(None);
        };
        self.previous_close = Some(input);

        let change = input - previous;
        let gain = change.max(0.0);
        let loss = (-change).max(0.0);

        if self.changes < self.period {
            self.seed_gain += gain;
            self.seed_loss += loss;
            self.changes += 1;
            if self.changes < self.period {
                return Ok(None);
            }
            self.avg_gain = self.seed_gain / self.period as f64;
            self.avg_loss = self.seed_loss / self.period as f64;
        } else {
            let period = self.period as f64;
            self.avg_gain = (self.avg_gain * (period - 1.0) + gain) / period;
            self.avg_loss = (self.avg_loss * (period - 1.0) + loss) / period;
        }

        if !self.avg_gain.is_finite() || !self.avg_loss.is_finite() {
            self.reset();
            return Err(KurumiError::NumericalOverflow {
                operation: "Rsi::next",
            });
        }
        Ok(Some(rsi_from_averages(self.avg_gain, self.avg_loss)))
    }

    fn reset(&mut self) {
        self.previous_close = None;
        self.changes = 0;
        self.seed_gain = 0.0;
        self.seed_loss = 0.0;
        self.avg_gain = 0.0;
        self.avg_loss = 0.0;
    }
}

/// Converts Wilder's average gain and loss into the RSI value.
#[inline]
fn rsi_from_averages(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss > 0.0 {
        100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
    } else if avg_gain > 0.0 {
        100.0
    } else {
        50.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn rsi_reaches_its_limits_for_one_sided_moves() {
        let mut rising = Rsi::new(3).expect("positive period");
        for price in [1.0, 2.0, 3.0, 4.0] {
            rising.next(price).expect("finite value");
        }
        assert_eq!(rising.value(), Some(100.0));

        let mut falling = Rsi::new(3).expect("positive period");
        for price in [4.0, 3.0, 2.0, 1.0] {
            falling.next(price).expect("finite value");
        }
        assert_eq!(falling.value(), Some(0.0));
    }

    #[test]
    fn flat_series_has_a_neutral_rsi() {
        let mut rsi = Rsi::new(3).expect("positive period");
        for _ in 0..10 {
            rsi.next(10.0).expect("finite value");
        }
        assert_eq!(rsi.value(), Some(50.0));
    }

    #[test]
    fn rsi_matches_the_wilder_recursion_on_a_small_example() {
        let mut rsi = Rsi::new(2).expect("positive period");
        assert_eq!(rsi.next(1.0).expect("finite value"), None);
        assert_eq!(rsi.next(2.0).expect("finite value"), None);
        assert_eq!(rsi.next(3.0).expect("finite value"), Some(100.0));
        // Seed averages become gain 1, loss 0. Next change is -1:
        // avg_gain = 1/2, avg_loss = 1/2, so RS = 1 and RSI = 50.
        let value = rsi.next(2.0).expect("finite value").expect("warmed up");
        assert_relative_eq!(value, 50.0, epsilon = 1e-15);
        assert_eq!(rsi.average_gain(), Some(0.5));
        assert_eq!(rsi.average_loss(), Some(0.5));
    }

    #[test]
    fn rsi_warm_up_needs_period_plus_one_prices() {
        let mut rsi = Rsi::new(3).expect("positive period");
        for (index, price) in [1.0, 2.0, 3.0].into_iter().enumerate() {
            assert_eq!(
                rsi.next(price).expect("finite value"),
                None,
                "price {index}"
            );
        }
        assert!(rsi.next(4.0).expect("finite value").is_some());
    }

    #[test]
    fn rsi_rejects_non_finite_prices_and_reset_restarts() {
        let mut rsi = Rsi::new(2).expect("positive period");
        assert!(rsi.next(f64::NAN).is_err());
        rsi.next(1.0).expect("finite value");
        rsi.next(2.0).expect("finite value");
        rsi.next(3.0).expect("finite value");
        assert!(rsi.value().is_some());
        rsi.reset();
        assert_eq!(rsi.value(), None);
        assert_eq!(rsi.next(1.0).expect("finite value"), None);
    }
}
