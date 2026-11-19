//! Bollinger Bands with an O(1) sliding Welford variance.

use kurumi_core::KurumiError;

use crate::ring_buffer::RollingVariance;
use crate::traits::StreamingIndicator;

/// The middle, upper, and lower bands at one observation.
///
/// ```text
/// middle = SMA(close, n)
/// upper  = middle + k * sigma
/// lower  = middle - k * sigma
/// ```
///
/// `sigma` is the population standard deviation (`sqrt(M2 / n)`), the
/// convention used by Bollinger's original definition. All three values are in
/// price units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BollingerBandsOutput {
    /// The `n`-period moving average.
    pub middle: f64,
    /// `middle + k * sigma`.
    pub upper: f64,
    /// `middle - k * sigma`.
    pub lower: f64,
}

/// Bollinger Bands: a moving average with volatility bands.
///
/// ```text
/// middle_t = mean(x_{t-n+1..t})
/// sigma_t  = sqrt(population variance of x_{t-n+1..t})
/// upper_t  = middle_t + k * sigma_t
/// lower_t  = middle_t - k * sigma_t
/// ```
///
/// The variance is maintained by [`RollingVariance`], which applies a Welford
/// update for the arriving observation and a reverse Welford update for the
/// evicted one. Both are `O(1)`, so the bands never scan the window. The usual
/// parameters are `n = 20` and `k = 2`.
///
/// # Warm-up
///
/// The first value is produced by the `n`-th observation.
///
/// # Examples
///
/// ```
/// use kurumi_ta::{BollingerBands, StreamingIndicator};
///
/// # fn main() -> Result<(), kurumi_ta::KurumiError> {
/// let mut bands = BollingerBands::new(3, 2.0)?;
/// assert_eq!(bands.next(2.0)?, None);
/// assert_eq!(bands.next(4.0)?, None);
/// let value = bands.next(4.0)?.expect("warmed up");
/// assert_eq!(value.middle, 10.0 / 3.0);
/// // Population variance is 8/9, so sigma = 0.9428...
/// assert!((value.upper - (10.0 / 3.0 + 2.0 * (8.0_f64 / 9.0).sqrt())).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct BollingerBands {
    period: usize,
    multiplier: f64,
    variance: RollingVariance,
}

impl BollingerBands {
    /// Creates Bollinger Bands with a moving-average period and a standard
    /// deviation multiplier.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `period` is zero, or when
    /// `multiplier` is negative or not finite.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::BollingerBands;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// assert!(BollingerBands::new(0, 2.0).is_err());
    /// assert!(BollingerBands::new(20, -1.0).is_err());
    /// assert!(BollingerBands::new(20, f64::NAN).is_err());
    /// let bands = BollingerBands::new(20, 2.0)?;
    /// assert_eq!(bands.period(), 20);
    /// assert_eq!(bands.multiplier(), 2.0);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(period: usize, multiplier: f64) -> Result<Self, KurumiError> {
        if period == 0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "period",
                reason: "must be strictly positive".to_owned(),
            });
        }
        if !multiplier.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "multiplier",
                reason: "must be finite".to_owned(),
            });
        }
        if multiplier < 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "multiplier",
                reason: "must be non-negative".to_owned(),
            });
        }
        Ok(Self {
            period,
            multiplier,
            variance: RollingVariance::new(period)?,
        })
    }

    /// Returns the moving-average window length.
    #[must_use]
    #[inline]
    pub const fn period(&self) -> usize {
        self.period
    }

    /// Returns the standard-deviation multiplier.
    #[must_use]
    #[inline]
    pub const fn multiplier(&self) -> f64 {
        self.multiplier
    }

    /// Returns the current bands, or `None` before warm-up completes.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::{BollingerBands, StreamingIndicator};
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut bands = BollingerBands::new(2, 2.0)?;
    /// bands.next(1.0)?;
    /// assert_eq!(bands.value(), None);
    /// bands.next(3.0)?;
    /// let value = bands.value().expect("warmed up");
    /// assert_eq!(value.middle, 2.0);
    /// assert_eq!(value.upper, 4.0);
    /// assert_eq!(value.lower, 0.0);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn value(&self) -> Option<BollingerBandsOutput> {
        if !self.variance.is_full() {
            return None;
        }
        let middle = self.variance.mean()?;
        let sigma = self.variance.std_dev()?;
        Some(BollingerBandsOutput {
            middle,
            upper: middle + self.multiplier * sigma,
            lower: middle - self.multiplier * sigma,
        })
    }
}

impl StreamingIndicator for BollingerBands {
    type Input = f64;
    type Output = BollingerBandsOutput;

    #[inline]
    fn next(&mut self, input: f64) -> Result<Option<BollingerBandsOutput>, KurumiError> {
        self.variance.push(input)?;
        Ok(self.value())
    }

    fn reset(&mut self) {
        self.variance.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn bands_match_the_population_standard_deviation() {
        let mut bands = BollingerBands::new(3, 2.0).expect("valid parameters");
        assert_eq!(bands.next(2.0).expect("finite value"), None);
        assert_eq!(bands.next(4.0).expect("finite value"), None);
        let output = bands.next(4.0).expect("finite value").expect("warmed up");
        let sigma = (8.0_f64 / 9.0).sqrt();
        assert_relative_eq!(output.middle, 10.0 / 3.0, epsilon = 1e-15);
        assert_relative_eq!(output.upper, 10.0 / 3.0 + 2.0 * sigma, epsilon = 1e-12);
        assert_relative_eq!(output.lower, 10.0 / 3.0 - 2.0 * sigma, epsilon = 1e-12);
    }

    #[test]
    fn bands_update_incrementally_after_eviction() {
        let mut bands = BollingerBands::new(3, 1.0).expect("valid parameters");
        for value in [1.0, 2.0, 3.0] {
            bands.next(value).expect("finite value");
        }
        let output = bands.next(7.0).expect("finite value").expect("warmed up");
        // Window [2, 3, 7]: mean 4, population variance 14/3.
        assert_relative_eq!(output.middle, 4.0, epsilon = 1e-12);
        let sigma = (14.0_f64 / 3.0).sqrt();
        assert_relative_eq!(output.upper, 4.0 + sigma, epsilon = 1e-12);
        assert_relative_eq!(output.lower, 4.0 - sigma, epsilon = 1e-12);
    }

    #[test]
    fn flat_window_has_coincident_bands() {
        let mut bands = BollingerBands::new(4, 2.0).expect("valid parameters");
        for _ in 0..10 {
            bands.next(10.0).expect("finite value");
        }
        let output = bands.value().expect("warmed up");
        assert_eq!(output.middle, 10.0);
        assert_eq!(output.upper, 10.0);
        assert_eq!(output.lower, 10.0);
    }

    #[test]
    fn zero_multiplier_collapses_the_bands_onto_the_middle() {
        let mut bands = BollingerBands::new(3, 0.0).expect("valid parameters");
        for value in [1.0, 5.0, 3.0] {
            bands.next(value).expect("finite value");
        }
        let output = bands.value().expect("warmed up");
        assert_eq!(output.upper, output.middle);
        assert_eq!(output.lower, output.middle);
    }

    #[test]
    fn bollinger_rejects_invalid_parameters_and_non_finite_prices() {
        assert!(BollingerBands::new(0, 2.0).is_err());
        assert!(BollingerBands::new(20, -2.0).is_err());
        assert!(BollingerBands::new(20, f64::INFINITY).is_err());

        let mut bands = BollingerBands::new(2, 2.0).expect("valid parameters");
        assert!(bands.next(f64::NAN).is_err());
    }

    #[test]
    fn reset_restarts_warm_up() {
        let mut bands = BollingerBands::new(2, 2.0).expect("valid parameters");
        bands.next(1.0).expect("finite value");
        bands.next(3.0).expect("finite value");
        assert!(bands.value().is_some());
        bands.reset();
        assert_eq!(bands.value(), None);
        assert_eq!(bands.next(2.0).expect("finite value"), None);
    }
}
