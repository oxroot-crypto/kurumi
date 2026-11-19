//! Value at Risk, Expected Shortfall, and their parametric normal counterparts.
//!
//! Two estimation approaches are provided:
//!
//! - **Historical simulation** uses the empirical distribution of a return
//!   series. No distributional assumption is made; the tail is read directly
//!   from the observed returns with the Hyndman-Fan type-7 quantile.
//! - **Parametric (variance-covariance)** assumes normally distributed returns
//!   and uses the closed-form normal quantile.
//!
//! Risk measures are reported as **non-negative loss magnitudes** in the same
//! units as the input returns. A VaR of `0.03` means that the loss at the chosen
//! confidence level is 3% of the portfolio value; a negative loss (a gain at
//! that quantile) is clamped to zero.
//!
//! # References
//!
//! - P. Jorion, *Value at Risk: The New Benchmark for Managing Financial Risk*,
//!   3rd edition, McGraw-Hill, 2007.
//! - R. T. Rockafellar and S. Uryasev, "Optimization of Conditional Value-at-
//!   Risk", *Journal of Risk* 2(3), 2000.
//! - R. J. Hyndman and Y. Fan, "Sample Quantiles in Statistical Packages",
//!   *The American Statistician* 50(4), 1996.

use kurumi_core::KurumiError;
use kurumi_math::{RootFinderConfig, brent, standard_normal_cdf, standard_normal_pdf};

/// Historical-simulation Value at Risk.
///
/// The `confidence_level` is the probability that losses stay at or below the
/// returned value (`0.95` is the usual 95% VaR). The loss quantile is the
/// `1 - confidence_level` empirical quantile estimated with linear
/// interpolation between order statistics (Hyndman-Fan type 7):
///
/// ```text
/// h = (n - 1) * (1 - confidence_level)
/// VaR = -((1 - w) * r_(floor(h)) + w * r_(floor(h) + 1))
/// ```
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for an empty series and
/// [`KurumiError::InvalidParameter`] when a return is not finite or the
/// confidence level is not strictly between zero and one.
///
/// # Examples
///
/// ```
/// use kurumi_risk::historical_var;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let returns = [-0.10, -0.05, 0.0, 0.02, 0.04];
/// let var = historical_var(&returns, 0.90)?;
/// assert!((var - 0.08).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn historical_var(returns: &[f64], confidence_level: f64) -> Result<f64, KurumiError> {
    validate_returns(returns)?;
    validate_confidence(confidence_level)?;
    let mut sorted = returns.to_vec();
    sorted.sort_by(f64::total_cmp);
    let quantile = empirical_quantile(&sorted, 1.0 - confidence_level)?;
    finite((-quantile).max(0.0), "historical_var")
}

/// Historical-simulation Expected Shortfall (CVaR).
///
/// Expected Shortfall is the average loss in the worst `1 - confidence_level`
/// fraction of observations. The tail contains the `ceil(n * (1 - c))` worst
/// returns (at least one):
///
/// ```text
/// ES = -(1 / k) * sum of the k smallest returns,  k = max(1, ceil(n * (1 - c)))
/// ```
///
/// # Errors
///
/// Returns the same errors as [`historical_var`].
///
/// # Examples
///
/// ```
/// use kurumi_risk::historical_expected_shortfall;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// // At 90% confidence the tail is exactly the single worst observation.
/// let es = historical_expected_shortfall(&[-0.10, -0.05, 0.0, 0.02, 0.04], 0.90)?;
/// assert!((es - 0.10).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn historical_expected_shortfall(
    returns: &[f64],
    confidence_level: f64,
) -> Result<f64, KurumiError> {
    validate_returns(returns)?;
    validate_confidence(confidence_level)?;
    let mut sorted = returns.to_vec();
    sorted.sort_by(f64::total_cmp);
    let count = tail_count(sorted.len(), confidence_level)?;
    let mut sum = 0.0;
    for &value in sorted.iter().take(count) {
        sum += value;
    }
    let expected = sum / count as f64;
    finite((-expected).max(0.0), "historical_expected_shortfall")
}

/// Parametric (variance-covariance) Value at Risk for normal returns.
///
/// ```text
/// z = Phi^-1(1 - confidence_level)
/// VaR = -(mean + z * standard_deviation)
/// ```
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when `mean` or `standard_deviation`
/// is not finite, when the standard deviation is negative, or when the
/// confidence level is not strictly between zero and one. Returns
/// [`KurumiError::NumericalOverflow`] when an intermediate value is not finite.
///
/// # Examples
///
/// ```
/// use kurumi_risk::parametric_var;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// // The one-sided 95% normal quantile is 1.6448536269514722.
/// let var = parametric_var(0.0, 0.01, 0.95)?;
/// assert!((var - 0.016_448_536_269_514_722).abs() < 1e-9);
/// # Ok(())
/// # }
/// ```
pub fn parametric_var(
    mean: f64,
    standard_deviation: f64,
    confidence_level: f64,
) -> Result<f64, KurumiError> {
    validate_normal_inputs(mean, standard_deviation)?;
    validate_confidence(confidence_level)?;
    let z = inverse_standard_normal(1.0 - confidence_level)?;
    let value = -(mean + z * standard_deviation);
    finite(value.max(0.0), "parametric_var")
}

/// Parametric Expected Shortfall for normal returns.
///
/// With `alpha = 1 - confidence_level` and `z = Phi^-1(alpha)`:
///
/// ```text
/// ES = -(mean - standard_deviation * phi(z) / alpha)
/// ```
///
/// which is the mean of the normal distribution restricted to its lower
/// `alpha` tail, negated.
///
/// # Errors
///
/// Returns the same errors as [`parametric_var`].
///
/// # Examples
///
/// ```
/// use kurumi_risk::parametric_expected_shortfall;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// // The 95% normal ES of a unit-variance zero-mean variable is about 2.0627.
/// let es = parametric_expected_shortfall(0.0, 1.0, 0.95)?;
/// assert!((es - 2.062_712_681_777_283).abs() < 1e-6);
/// # Ok(())
/// # }
/// ```
pub fn parametric_expected_shortfall(
    mean: f64,
    standard_deviation: f64,
    confidence_level: f64,
) -> Result<f64, KurumiError> {
    validate_normal_inputs(mean, standard_deviation)?;
    validate_confidence(confidence_level)?;
    let tail_probability = 1.0 - confidence_level;
    let z = inverse_standard_normal(tail_probability)?;
    let density = standard_normal_pdf(z)?;
    let value = -(mean - standard_deviation * density / tail_probability);
    finite(value.max(0.0), "parametric_expected_shortfall")
}

fn validate_returns(returns: &[f64]) -> Result<(), KurumiError> {
    if returns.is_empty() {
        return Err(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        });
    }
    for (index, &value) in returns.iter().enumerate() {
        if !value.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "returns",
                reason: format!("value at index {index} must be finite"),
            });
        }
    }
    Ok(())
}

fn validate_confidence(confidence_level: f64) -> Result<(), KurumiError> {
    if !confidence_level.is_finite() || confidence_level <= 0.0 || confidence_level >= 1.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "confidence_level",
            reason: "must be finite and strictly between zero and one".to_owned(),
        });
    }
    Ok(())
}

fn validate_normal_inputs(mean: f64, standard_deviation: f64) -> Result<(), KurumiError> {
    if !mean.is_finite() {
        return Err(KurumiError::InvalidParameter {
            parameter: "mean",
            reason: "must be finite".to_owned(),
        });
    }
    if !standard_deviation.is_finite() || standard_deviation < 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "standard_deviation",
            reason: "must be finite and non-negative".to_owned(),
        });
    }
    Ok(())
}

fn empirical_quantile(sorted: &[f64], probability: f64) -> Result<f64, KurumiError> {
    let length = sorted.len();
    let first = sorted
        .first()
        .copied()
        .ok_or(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        })?;
    if length == 1 {
        return Ok(first);
    }
    let position = probability * (length - 1) as f64;
    let lower_index = position.floor() as usize;
    let upper_index = (lower_index + 1).min(length - 1);
    let weight = position - lower_index as f64;
    let lower = sorted
        .get(lower_index)
        .copied()
        .ok_or(KurumiError::NumericalOverflow {
            operation: "empirical_quantile",
        })?;
    let upper = sorted
        .get(upper_index)
        .copied()
        .ok_or(KurumiError::NumericalOverflow {
            operation: "empirical_quantile",
        })?;
    Ok(lower + weight * (upper - lower))
}

fn tail_count(length: usize, confidence_level: f64) -> Result<usize, KurumiError> {
    let tail_probability = 1.0 - confidence_level;
    let raw = (tail_probability * length as f64).ceil();
    if !raw.is_finite() {
        return Err(KurumiError::NumericalOverflow {
            operation: "expected_shortfall tail count",
        });
    }
    let count = if raw < 1.0 { 1 } else { raw as usize };
    Ok(count.min(length))
}

/// The inverse of the standard normal CDF.
///
/// The function is monotone, so the quantile is found by bracketing it on
/// `[-40, 40]` and solving `Phi(z) = probability` with Brent's method. The
/// bracket covers every probability representable as a positive `f64`: at
/// `-40` the CDF has underflowed to zero and at `40` it rounds to one.
fn inverse_standard_normal(probability: f64) -> Result<f64, KurumiError> {
    if !probability.is_finite() || probability <= 0.0 || probability >= 1.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "probability",
            reason: "must be finite and strictly between zero and one".to_owned(),
        });
    }
    let config = RootFinderConfig::new(200, 1e-12)?;
    brent(
        |value| match standard_normal_cdf(value) {
            Ok(cumulative) => cumulative - probability,
            // The solver only evaluates finite points inside the bracket, so
            // this branch is unreachable; returning NaN lets Brent report the
            // failure instead of silently accepting a bad value.
            Err(_) => f64::NAN,
        },
        -40.0,
        40.0,
        config,
    )
}

fn finite(value: f64, operation: &'static str) -> Result<f64, KurumiError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(KurumiError::NumericalOverflow { operation })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    const SAMPLE: [f64; 10] = [-0.10, -0.05, -0.02, 0.0, 0.01, 0.02, 0.03, 0.04, 0.05, 0.06];

    #[test]
    fn historical_var_uses_the_type_seven_quantile() {
        // h = (10 - 1) * 0.1 = 0.9 -> -0.10 + 0.9 * 0.05 = -0.055.
        let var = historical_var(&SAMPLE, 0.90).expect("valid sample");
        assert_relative_eq!(var, 0.055, epsilon = 1e-12);
    }

    #[test]
    fn historical_var_is_sorted_order_independent() {
        let mut reversed = SAMPLE;
        reversed.reverse();
        let original = historical_var(&SAMPLE, 0.95).expect("valid sample");
        let flipped = historical_var(&reversed, 0.95).expect("valid sample");
        assert_relative_eq!(original, flipped, epsilon = 1e-12);
    }

    #[test]
    fn historical_var_clamps_a_profitable_quantile_to_zero() {
        let var = historical_var(&[0.01, 0.02, 0.03], 0.50).expect("valid sample");
        assert_relative_eq!(var, 0.0, epsilon = 1e-15);
    }

    #[test]
    fn historical_expected_shortfall_averages_the_worst_tail() {
        // Tail count is ceil(10 * 0.1) = 1 -> the single worst return.
        let es = historical_expected_shortfall(&SAMPLE, 0.90).expect("valid sample");
        assert_relative_eq!(es, 0.10, epsilon = 1e-12);

        // Tail count is ceil(10 * 0.2) = 2 -> (-0.10 - 0.05) / 2.
        let es = historical_expected_shortfall(&SAMPLE, 0.80).expect("valid sample");
        assert_relative_eq!(es, 0.075, epsilon = 1e-12);
    }

    #[test]
    fn expected_shortfall_is_never_below_the_var_at_the_same_confidence() {
        for confidence in [0.75, 0.90, 0.95, 0.99] {
            let var = historical_var(&SAMPLE, confidence).expect("valid sample");
            let es = historical_expected_shortfall(&SAMPLE, confidence).expect("valid sample");
            assert!(es >= var - 1e-12);
        }
    }

    #[test]
    fn parametric_var_matches_the_normal_quantile() {
        let var = parametric_var(0.0, 0.01, 0.95).expect("valid inputs");
        assert_relative_eq!(var, 0.016_448_536_269_514_722, epsilon = 1e-9);

        let shifted = parametric_var(0.001, 0.01, 0.95).expect("valid inputs");
        assert_relative_eq!(shifted, 0.016_448_536_269_514_722 - 0.001, epsilon = 1e-9);
    }

    #[test]
    fn parametric_var_clamps_a_negative_loss_to_zero() {
        let var = parametric_var(0.10, 0.01, 0.95).expect("valid inputs");
        assert_relative_eq!(var, 0.0, epsilon = 1e-15);
    }

    #[test]
    fn parametric_expected_shortfall_matches_the_normal_tail_mean() {
        // E[Z | Z <= -1.6448536] = -phi(z) / 0.05 ~= -2.06271268.
        let es = parametric_expected_shortfall(0.0, 1.0, 0.95).expect("valid inputs");
        assert_relative_eq!(es, 2.062_712_681_777_283, epsilon = 1e-6);
        assert!(es > parametric_var(0.0, 1.0, 0.95).expect("valid inputs"));
    }

    #[test]
    fn inverse_standard_normal_matches_published_quantiles() {
        for (probability, expected) in [
            (0.5, 0.0),
            (0.95, 1.644_853_626_951_472_2),
            (0.975, 1.959_963_984_540_054),
            (0.99, 2.326_347_874_040_840_8),
            (0.005, -2.575_829_303_548_901),
        ] {
            let actual = inverse_standard_normal(probability).expect("valid probability");
            assert_relative_eq!(actual, expected, epsilon = 1e-9, max_relative = 1e-9);
        }
    }

    #[test]
    fn inverse_standard_normal_is_antisymmetric() {
        for probability in [0.01, 0.05, 0.25, 0.4] {
            let lower = inverse_standard_normal(probability).expect("valid probability");
            let upper = inverse_standard_normal(1.0 - probability).expect("valid probability");
            assert_relative_eq!(lower, -upper, epsilon = 1e-9);
        }
    }

    #[test]
    fn risk_measures_reject_invalid_inputs() {
        assert!(historical_var(&[], 0.95).is_err());
        assert!(historical_var(&[f64::NAN], 0.95).is_err());
        assert!(historical_var(&SAMPLE, 0.0).is_err());
        assert!(historical_var(&SAMPLE, 1.0).is_err());
        assert!(historical_var(&SAMPLE, f64::NAN).is_err());
        assert!(historical_expected_shortfall(&SAMPLE, -0.1).is_err());
        assert!(parametric_var(f64::NAN, 0.01, 0.95).is_err());
        assert!(parametric_var(0.0, -0.01, 0.95).is_err());
        assert!(parametric_var(0.0, f64::INFINITY, 0.95).is_err());
        assert!(parametric_expected_shortfall(0.0, 0.01, 1.0).is_err());
        assert!(inverse_standard_normal(0.0).is_err());
        assert!(inverse_standard_normal(1.0).is_err());
    }
}
