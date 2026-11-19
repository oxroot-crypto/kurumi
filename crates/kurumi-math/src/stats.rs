//! Descriptive statistics and the standard normal distribution.
//!
//! Sample moments use `n - 1` in the denominator (the unbiased sample
//! estimator). Sums are accumulated with Neumaier compensated summation to limit
//! cancellation error. Every public function validates its inputs at the
//! boundary and returns [`KurumiError`] instead of propagating `NaN` or
//! infinity.

use kurumi_core::KurumiError;

/// `sqrt(2 * pi)`, the normalizing constant of the standard normal density.
const SQRT_TWO_PI: f64 = 2.506_628_274_631_000_2;

/// The probability density function of the standard normal distribution.
///
/// ```text
/// phi(x) = exp(-x^2 / 2) / sqrt(2 pi)
/// ```
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when `x` is `NaN` or infinite.
///
/// # Examples
///
/// ```
/// use kurumi_math::standard_normal_pdf;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let density = standard_normal_pdf(0.0)?;
/// assert!((density - 0.398_942_280_401_432_7).abs() < 1e-15);
/// # Ok(())
/// # }
/// ```
#[inline]
pub fn standard_normal_pdf(x: f64) -> Result<f64, KurumiError> {
    ensure_finite(x, "x")?;
    let value = (-0.5 * x * x).exp() / SQRT_TWO_PI;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(KurumiError::NumericalOverflow {
            operation: "standard_normal_pdf",
        })
    }
}

/// The cumulative distribution function of the standard normal distribution.
///
/// ```text
/// Phi(x) = 1/2 * erfc(-x / sqrt(2))
/// ```
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when `x` is `NaN` or infinite.
///
/// # Examples
///
/// ```
/// use kurumi_math::standard_normal_cdf;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// assert!((standard_normal_cdf(0.0)? - 0.5).abs() < 1e-15);
/// assert!((standard_normal_cdf(1.96)? - 0.975_002_104_851_779_5).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
#[inline]
pub fn standard_normal_cdf(x: f64) -> Result<f64, KurumiError> {
    ensure_finite(x, "x")?;
    let value = 0.5 * libm::erfc(-x / std::f64::consts::SQRT_2);
    if value.is_finite() {
        Ok(value)
    } else {
        Err(KurumiError::NumericalOverflow {
            operation: "standard_normal_cdf",
        })
    }
}

/// The arithmetic mean of a non-empty sample.
///
/// ```text
/// mean = (1 / n) * sum_i x_i
/// ```
///
/// The sum uses Neumaier compensated summation so that a few large values do
/// not erase smaller ones.
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for an empty sample,
/// [`KurumiError::InvalidParameter`] when a value is not finite, and
/// [`KurumiError::NumericalOverflow`] when the sum is not representable.
///
/// # Examples
///
/// ```
/// use kurumi_math::mean;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let average = mean(&[1.0, 2.0, 3.0, 4.0])?;
/// assert!((average - 2.5).abs() < 1e-15);
/// # Ok(())
/// # }
/// ```
pub fn mean(values: &[f64]) -> Result<f64, KurumiError> {
    if values.is_empty() {
        return Err(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        });
    }
    let sum = compensated_sum(values.iter().copied())?;
    let average = sum / values.len() as f64;
    if average.is_finite() {
        Ok(average)
    } else {
        Err(KurumiError::NumericalOverflow { operation: "mean" })
    }
}

/// The unbiased sample variance of a sample with at least two observations.
///
/// ```text
/// s^2 = sum_i (x_i - mean)^2 / (n - 1)
/// ```
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for fewer than two observations,
/// [`KurumiError::InvalidParameter`] when a value is not finite, and
/// [`KurumiError::NumericalOverflow`] when a squared deviation is not
/// representable.
///
/// # Examples
///
/// ```
/// use kurumi_math::sample_variance;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let variance = sample_variance(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0])?;
/// assert!((variance - 4.571_428_571_428_571).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn sample_variance(values: &[f64]) -> Result<f64, KurumiError> {
    if values.len() < 2 {
        return Err(KurumiError::InsufficientData {
            required: 2,
            available: values.len(),
        });
    }
    let average = mean(values)?;
    let mut squared_deviations = Vec::with_capacity(values.len());
    for &value in values {
        let deviation = value - average;
        let squared = deviation * deviation;
        if !squared.is_finite() {
            return Err(KurumiError::NumericalOverflow {
                operation: "sample_variance",
            });
        }
        squared_deviations.push(squared);
    }
    let sum = compensated_sum(squared_deviations.into_iter())?;
    let variance = sum / (values.len() - 1) as f64;
    if variance.is_finite() {
        Ok(variance)
    } else {
        Err(KurumiError::NumericalOverflow {
            operation: "sample_variance",
        })
    }
}

/// The unbiased sample standard deviation.
///
/// This is the square root of [`sample_variance`].
///
/// # Errors
///
/// Returns the same errors as [`sample_variance`].
///
/// # Examples
///
/// ```
/// use kurumi_math::sample_std_dev;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let deviation = sample_std_dev(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0])?;
/// assert!((deviation - 2.138_089_935_299_395).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn sample_std_dev(values: &[f64]) -> Result<f64, KurumiError> {
    Ok(sample_variance(values)?.sqrt())
}

/// The unbiased sample covariance of two equally sized samples.
///
/// ```text
/// cov(x, y) = sum_i (x_i - mean_x) (y_i - mean_y) / (n - 1)
/// ```
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for fewer than two observations,
/// [`KurumiError::InvalidParameter`] when the lengths differ or a value is not
/// finite, and [`KurumiError::NumericalOverflow`] when a product is not
/// representable.
///
/// # Examples
///
/// ```
/// use kurumi_math::covariance;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let value = covariance(&[1.0, 2.0, 3.0], &[2.0, 4.0, 7.0])?;
/// assert!((value - 2.5).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn covariance(xs: &[f64], ys: &[f64]) -> Result<f64, KurumiError> {
    if xs.len() != ys.len() {
        return Err(KurumiError::InvalidParameter {
            parameter: "ys",
            reason: "must have the same length as xs".to_owned(),
        });
    }
    if xs.len() < 2 {
        return Err(KurumiError::InsufficientData {
            required: 2,
            available: xs.len(),
        });
    }
    let mean_x = mean(xs)?;
    let mean_y = mean(ys)?;
    covariance_with_means(xs, mean_x, ys, mean_y)
}

/// The unbiased sample covariance matrix of a set of observation columns.
///
/// `columns[i]` holds the observations of variable `i`. The result is a
/// symmetric `k x k` matrix whose entry `(i, j)` is
/// [`covariance`]`(columns[i], columns[j])`; the diagonal entries are the
/// sample variances.
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] when there are no columns or fewer
/// than two observations per column, [`KurumiError::InvalidParameter`] when the
/// columns have different lengths or contain a non-finite value, and
/// [`KurumiError::NumericalOverflow`] when a product is not representable.
///
/// # Examples
///
/// ```
/// use kurumi_math::covariance_matrix;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let matrix = covariance_matrix(&[&[1.0, 2.0, 3.0], &[2.0, 4.0, 7.0]])?;
/// let first_row = matrix.first().expect("two rows");
/// assert!((first_row.first().expect("two columns") - 1.0).abs() < 1e-12);
/// assert!((first_row.get(1).expect("two columns") - 2.5).abs() < 1e-12);
/// let second_row = matrix.get(1).expect("two rows");
/// assert!((second_row.get(1).expect("two columns") - 6.333_333_333_333_333).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn covariance_matrix(columns: &[&[f64]]) -> Result<Vec<Vec<f64>>, KurumiError> {
    let first = columns.first().ok_or(KurumiError::InsufficientData {
        required: 1,
        available: 0,
    })?;
    let observations = first.len();
    if observations < 2 {
        return Err(KurumiError::InsufficientData {
            required: 2,
            available: observations,
        });
    }
    for (index, column) in columns.iter().enumerate() {
        if column.len() != observations {
            return Err(KurumiError::InvalidParameter {
                parameter: "columns",
                reason: format!(
                    "column {index} has {} observations but expected {observations}",
                    column.len()
                ),
            });
        }
    }

    let means = columns
        .iter()
        .map(|column| mean(column))
        .collect::<Result<Vec<_>, _>>()?;

    let mut matrix = Vec::with_capacity(columns.len());
    for (row_column, row_mean) in columns.iter().zip(means.iter()) {
        let mut row = Vec::with_capacity(columns.len());
        for (column, column_mean) in columns.iter().zip(means.iter()) {
            row.push(covariance_with_means(
                row_column,
                *row_mean,
                column,
                *column_mean,
            )?);
        }
        matrix.push(row);
    }
    Ok(matrix)
}

fn covariance_with_means(
    xs: &[f64],
    mean_x: f64,
    ys: &[f64],
    mean_y: f64,
) -> Result<f64, KurumiError> {
    let products = xs
        .iter()
        .zip(ys.iter())
        .map(|(&x, &y)| (x - mean_x) * (y - mean_y));
    let sum = compensated_sum(products)?;
    let covariance = sum / (xs.len() - 1) as f64;
    if covariance.is_finite() {
        Ok(covariance)
    } else {
        Err(KurumiError::NumericalOverflow {
            operation: "covariance",
        })
    }
}

pub(crate) fn compensated_sum<I>(values: I) -> Result<f64, KurumiError>
where
    I: Iterator<Item = f64>,
{
    let mut sum = 0.0;
    let mut compensation = 0.0;
    for value in values {
        if !value.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "value",
                reason: "must be finite".to_owned(),
            });
        }
        let next = sum + value;
        if sum.abs() >= value.abs() {
            compensation += (sum - next) + value;
        } else {
            compensation += (value - next) + sum;
        }
        sum = next;
    }
    let total = sum + compensation;
    if total.is_finite() {
        Ok(total)
    } else {
        Err(KurumiError::NumericalOverflow {
            operation: "compensated_sum",
        })
    }
}

fn ensure_finite(value: f64, parameter: &'static str) -> Result<(), KurumiError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(KurumiError::InvalidParameter {
            parameter,
            reason: "must be finite".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn close(left: f64, right: f64) -> bool {
        (left - right).abs() < 1e-12
    }

    #[test]
    fn standard_normal_pdf_matches_reference_values() {
        assert_relative_eq!(
            standard_normal_pdf(0.0).expect("finite input"),
            0.398_942_280_401_432_7,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            standard_normal_pdf(1.0).expect("finite input"),
            0.241_970_724_519_143_37,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            standard_normal_pdf(-1.0).expect("finite input"),
            0.241_970_724_519_143_37,
            epsilon = 1e-15
        );
    }

    #[test]
    fn standard_normal_cdf_matches_reference_values() {
        assert_relative_eq!(
            standard_normal_cdf(0.0).expect("finite input"),
            0.5,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            standard_normal_cdf(1.0).expect("finite input"),
            0.841_344_746_068_542_9,
            epsilon = 1e-12
        );
        assert_relative_eq!(
            standard_normal_cdf(-1.0).expect("finite input"),
            0.158_655_253_931_457_07,
            epsilon = 1e-12
        );
        assert_relative_eq!(
            standard_normal_cdf(1.96).expect("finite input"),
            0.975_002_104_851_779_5,
            epsilon = 1e-12
        );
        assert_relative_eq!(
            standard_normal_cdf(-1.96).expect("finite input"),
            0.024_997_895_148_220_435,
            epsilon = 1e-12
        );
    }

    #[test]
    fn standard_normal_cdf_is_symmetric_and_bounded() {
        for x in [0.1, 0.5, 1.5, 2.5, 3.5] {
            let upper = standard_normal_cdf(x).expect("finite input");
            let lower = standard_normal_cdf(-x).expect("finite input");
            assert_relative_eq!(upper + lower, 1.0, epsilon = 1e-12);
            assert!((0.0..=1.0).contains(&upper));
        }
    }

    #[test]
    fn normal_distribution_rejects_non_finite_inputs() {
        assert!(standard_normal_pdf(f64::NAN).is_err());
        assert!(standard_normal_pdf(f64::INFINITY).is_err());
        assert!(standard_normal_cdf(f64::NEG_INFINITY).is_err());
    }

    #[test]
    fn mean_matches_the_arithmetic_average() {
        assert_relative_eq!(
            mean(&[1.0, 2.0, 3.0, 4.0]).expect("non-empty"),
            2.5,
            epsilon = 1e-15
        );
        assert_relative_eq!(mean(&[5.0]).expect("one value"), 5.0, epsilon = 1e-15);
    }

    #[test]
    fn mean_uses_compensated_summation() {
        // Naive left-to-right summation returns 0 here; the compensated sum
        // keeps the small addend.
        let average = mean(&[1.0e16, 1.0, -1.0e16]).expect("finite values");
        assert_relative_eq!(average, 1.0 / 3.0, epsilon = 1e-15);
    }

    #[test]
    fn mean_rejects_empty_and_non_finite_inputs() {
        assert!(matches!(
            mean(&[]),
            Err(KurumiError::InsufficientData {
                required: 1,
                available: 0
            })
        ));
        assert!(mean(&[1.0, f64::NAN]).is_err());
        assert!(mean(&[1.0, f64::INFINITY]).is_err());
    }

    #[test]
    fn sample_variance_matches_the_textbook_example() {
        let values = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        assert_relative_eq!(
            sample_variance(&values).expect("two or more values"),
            32.0 / 7.0,
            epsilon = 1e-12
        );
        assert_relative_eq!(
            sample_std_dev(&values).expect("two or more values"),
            (32.0_f64 / 7.0).sqrt(),
            epsilon = 1e-12
        );
    }

    #[test]
    fn sample_variance_requires_at_least_two_observations() {
        assert!(matches!(
            sample_variance(&[1.0]),
            Err(KurumiError::InsufficientData {
                required: 2,
                available: 1
            })
        ));
        assert!(sample_std_dev(&[]).is_err());
    }

    #[test]
    fn covariance_matches_the_direct_formula() {
        assert_relative_eq!(
            covariance(&[1.0, 2.0, 3.0], &[2.0, 4.0, 7.0]).expect("valid samples"),
            2.5,
            epsilon = 1e-12
        );
        assert!(close(
            covariance(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]).expect("valid samples"),
            sample_variance(&[1.0, 2.0, 3.0]).expect("valid sample")
        ));
    }

    #[test]
    fn covariance_rejects_mismatched_or_short_samples() {
        assert!(covariance(&[1.0, 2.0], &[1.0]).is_err());
        assert!(matches!(
            covariance(&[1.0], &[1.0]),
            Err(KurumiError::InsufficientData { .. })
        ));
        assert!(covariance(&[1.0, f64::NAN], &[1.0, 2.0]).is_err());
    }

    #[test]
    fn covariance_matrix_is_symmetric_with_variances_on_the_diagonal() {
        let matrix =
            covariance_matrix(&[&[1.0, 2.0, 3.0], &[2.0, 4.0, 7.0]]).expect("valid columns");
        assert_eq!(matrix.len(), 2);
        assert_relative_eq!(matrix[0][0], 1.0, epsilon = 1e-12);
        assert_relative_eq!(matrix[0][1], 2.5, epsilon = 1e-12);
        assert_relative_eq!(matrix[1][0], 2.5, epsilon = 1e-12);
        assert_relative_eq!(matrix[1][1], 19.0 / 3.0, epsilon = 1e-12);
    }

    #[test]
    fn covariance_matrix_rejects_invalid_inputs() {
        assert!(matches!(
            covariance_matrix(&[]),
            Err(KurumiError::InsufficientData { .. })
        ));
        assert!(covariance_matrix(&[&[1.0]]).is_err());
        assert!(covariance_matrix(&[&[1.0, 2.0], &[1.0]]).is_err());
        assert!(covariance_matrix(&[&[1.0, 2.0], &[1.0, f64::NAN]]).is_err());
    }

    #[test]
    fn covariance_matrix_with_one_column_is_its_sample_variance() {
        let matrix =
            covariance_matrix(&[&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]]).expect("valid column");
        assert_eq!(matrix.len(), 1);
        assert_relative_eq!(matrix[0][0], 32.0 / 7.0, epsilon = 1e-12);
    }
}
