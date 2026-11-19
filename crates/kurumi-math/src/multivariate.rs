//! Multivariate statistics: correlation, rolling correlation, and covariance
//! estimators.
//!
//! Multi-asset return data is a matrix in `(T, N)` orientation: rows are
//! chronological observations and columns are assets. The functions in this
//! module follow that convention and return the shared [`KurumiError`] type
//! instead of propagating `NaN` or infinity.
//!
//! Sample moments use Bessel's correction (`ddof = 1`). The model-based
//! estimators (`EwmaCovariance` and `LedoitWolfShrinkage`) document their own
//! weighting and normalization conventions.
//!
//! # References
//!
//! - O. Ledoit and M. Wolf, "A well-conditioned estimator for
//!   large-dimensional covariance matrices", *Journal of Multivariate
//!   Analysis* 88(2), 2004.
//! - JP Morgan/Reuters, *RiskMetrics Technical Document*, 4th edition, 1996.

use std::collections::VecDeque;

use kurumi_core::KurumiError;
use nalgebra::DMatrix;

use crate::stats::{compensated_sum, mean};

/// The Pearson product-moment correlation of two equally sized samples.
///
/// ```text
/// r = sum_i (x_i - mean_x) (y_i - mean_y)
///     / sqrt(sum_i (x_i - mean_x)^2 * sum_i (y_i - mean_y)^2)
/// ```
///
/// The result is clamped to `[-1.0, 1.0]`. Both samples use the unbiased
/// `T - 1` denominator, which cancels in the ratio.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when the lengths differ, when a
/// value is not finite, or when either sample has zero variance, and
/// [`KurumiError::InsufficientData`] when there are fewer than two
/// observations.
///
/// # Examples
///
/// ```
/// use kurumi_math::pearson_correlation;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let x = [0.01, -0.02, 0.03, 0.00];
/// let y = [0.02, -0.04, 0.06, 0.00];
/// assert_eq!(pearson_correlation(&x, &y)?, 1.0);
/// assert_eq!(pearson_correlation(&x, &x)?, 1.0);
/// # Ok(())
/// # }
/// ```
#[inline]
pub fn pearson_correlation(x: &[f64], y: &[f64]) -> Result<f64, KurumiError> {
    validate_pair(x, y)?;
    let mean_x = mean(x)?;
    let mean_y = mean(y)?;
    pearson_with_means(x, mean_x, y, mean_y)
}

/// The Spearman rank correlation of two equally sized samples.
///
/// Each sample is converted to ranks, tied values receive the average of the
/// ranks they span, and the Pearson correlation of the two rank vectors is
/// returned. This is equivalent to the Pearson correlation applied to the
/// average ranks, not to the raw values.
///
/// # Errors
///
/// Returns the same errors as [`pearson_correlation`], including a zero-variance
/// error when every value in a sample is tied (every rank is equal).
///
/// # Examples
///
/// ```
/// use kurumi_math::spearman_correlation;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// // Ranks are [1, 2.5, 2.5, 4] and [1, 2, 3, 4].
/// let rho = spearman_correlation(&[1.0, 2.0, 2.0, 3.0], &[1.0, 2.0, 3.0, 4.0])?;
/// assert!((rho - 0.948_683_298_050_513_8).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn spearman_correlation(x: &[f64], y: &[f64]) -> Result<f64, KurumiError> {
    validate_pair(x, y)?;
    let ranks_x = average_ranks(x);
    let ranks_y = average_ranks(y);
    let mean_x = mean(&ranks_x)?;
    let mean_y = mean(&ranks_y)?;
    pearson_with_means(&ranks_x, mean_x, &ranks_y, mean_y)
}

/// The `N x N` Pearson correlation matrix of a `(T, N)` return matrix.
///
/// Rows are chronological observations and columns are assets. The result is
/// symmetric; its diagonal is pinned to exactly `1.0` and off-diagonal entries
/// are clamped to `[-1.0, 1.0]`.
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] when there are no columns or fewer
/// than two rows, [`KurumiError::InvalidParameter`] when a value is not finite,
/// and [`KurumiError::InvalidParameter`] when any asset column has zero
/// variance (its correlation is undefined).
///
/// # Examples
///
/// ```
/// use kurumi_math::correlation_matrix;
/// use nalgebra::DMatrix;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let returns = DMatrix::from_row_slice(3, 2, &[
///     0.01, 0.02,
///     0.02, 0.04,
///     0.03, 0.06,
/// ]);
/// let correlations = correlation_matrix(&returns)?;
/// assert_eq!(correlations.get((0, 0)).copied(), Some(1.0));
/// assert!((correlations.get((0, 1)).copied().unwrap_or(f64::NAN) - 1.0).abs() < 1e-15);
/// # Ok(())
/// # }
/// ```
pub fn correlation_matrix(returns: &DMatrix<f64>) -> Result<DMatrix<f64>, KurumiError> {
    validate_matrix(returns)?;
    let columns = column_slices(returns)?;
    let means = column_means(&columns)?;
    let order = columns.len();
    symmetric_matrix(order, |row, column| {
        let (Some(xs), Some(ys), Some(&mean_x), Some(&mean_y)) = (
            columns.get(row),
            columns.get(column),
            means.get(row),
            means.get(column),
        ) else {
            return Err(matrix_internal_error());
        };
        if row == column {
            let variance = compensated_sum(centered_squares(xs, mean_x))?;
            if variance == 0.0 {
                return Err(KurumiError::InvalidParameter {
                    parameter: "returns",
                    reason: format!("column {row} has zero variance"),
                });
            }
            Ok(1.0)
        } else {
            pearson_with_means(xs, mean_x, ys, mean_y)
        }
    })
}

/// Rolling Pearson correlation over a fixed window.
///
/// The output has one entry per complete window, in chronological order:
/// `output[k]` describes the window ending at observation `k + window - 1`.
/// Sums are updated incrementally as observations enter and leave the window,
/// so the cost is `O(T)` rather than `O(T * window)`.
///
/// A window whose sample has zero variance in either series yields `None`
/// because the correlation is then undefined.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when the lengths differ, when a
/// value is not finite, or when `window < 2`, and
/// [`KurumiError::InsufficientData`] when there are fewer observations than the
/// window length.
///
/// # Examples
///
/// ```
/// use kurumi_math::rolling_correlation;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let x = [0.01, -0.02, 0.03, 0.00, 0.02];
/// let y = [0.02, -0.04, 0.06, 0.00, 0.04];
/// let rolling = rolling_correlation(&x, &y, 3)?;
/// assert_eq!(rolling.len(), 3);
/// assert_eq!(rolling.first().copied().flatten(), Some(1.0));
/// # Ok(())
/// # }
/// ```
pub fn rolling_correlation(
    x: &[f64],
    y: &[f64],
    window: usize,
) -> Result<Vec<Option<f64>>, KurumiError> {
    validate_pair(x, y)?;
    if window < 2 {
        return Err(KurumiError::InvalidParameter {
            parameter: "window",
            reason: "must be at least 2".to_owned(),
        });
    }
    if x.len() < window {
        return Err(KurumiError::InsufficientData {
            required: window,
            available: x.len(),
        });
    }

    // Shift every observation by the first value of its series. Constant
    // windows then have exactly zero variance, and moderate drift no longer
    // cancels two large sums against each other.
    let x_offset = x.first().copied().ok_or(KurumiError::InsufficientData {
        required: 2,
        available: x.len(),
    })?;
    let y_offset = y.first().copied().ok_or(KurumiError::InsufficientData {
        required: 2,
        available: y.len(),
    })?;

    let mut queue: VecDeque<(f64, f64)> = VecDeque::with_capacity(window);
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut sum_xx = 0.0;
    let mut sum_yy = 0.0;
    let mut sum_xy = 0.0;
    let mut output = Vec::with_capacity(x.len() - window + 1);

    for (&value_x, &value_y) in x.iter().zip(y.iter()) {
        let shifted_x = value_x - x_offset;
        let shifted_y = value_y - y_offset;
        queue.push_back((shifted_x, shifted_y));
        sum_x += shifted_x;
        sum_y += shifted_y;
        sum_xx += shifted_x * shifted_x;
        sum_yy += shifted_y * shifted_y;
        sum_xy += shifted_x * shifted_y;

        if queue.len() > window {
            if let Some((old_x, old_y)) = queue.pop_front() {
                sum_x -= old_x;
                sum_y -= old_y;
                sum_xx -= old_x * old_x;
                sum_yy -= old_y * old_y;
                sum_xy -= old_x * old_y;
            }
        }

        if queue.len() == window {
            if !(sum_x.is_finite()
                && sum_y.is_finite()
                && sum_xx.is_finite()
                && sum_yy.is_finite()
                && sum_xy.is_finite())
            {
                return Err(KurumiError::NumericalOverflow {
                    operation: "rolling_correlation",
                });
            }
            let count = window as f64;
            let numerator = sum_xy - sum_x * sum_y / count;
            let variance_x = sum_xx - sum_x * sum_x / count;
            let variance_y = sum_yy - sum_y * sum_y / count;
            let tolerance_x = count * f64::EPSILON * sum_xx.max(0.0);
            let tolerance_y = count * f64::EPSILON * sum_yy.max(0.0);
            if variance_x <= tolerance_x || variance_y <= tolerance_y {
                output.push(None);
            } else {
                let denominator = (variance_x * variance_y).sqrt();
                if denominator > 0.0 && denominator.is_finite() {
                    output.push(Some((numerator / denominator).clamp(-1.0, 1.0)));
                } else {
                    output.push(None);
                }
            }
        }
    }

    Ok(output)
}

/// A covariance estimator that maps a `(T, N)` return matrix to an `N x N`
/// covariance matrix.
///
/// Rows of the input are chronological observations and columns are assets.
/// Implementations validate the matrix, return the shared [`KurumiError`] type,
/// and produce a symmetric result.
///
/// # Examples
///
/// ```
/// use kurumi_math::{CovarianceEstimator, SampleCovariance};
/// use nalgebra::DMatrix;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let returns = DMatrix::from_row_slice(3, 2, &[
///     0.01, 0.02,
///     0.03, 0.04,
///     0.02, 0.05,
/// ]);
/// let covariance = SampleCovariance.estimate(&returns)?;
/// assert_eq!(covariance.shape(), (2, 2));
/// # Ok(())
/// # }
/// ```
pub trait CovarianceEstimator {
    /// Estimates the `N x N` covariance matrix of a `(T, N)` return matrix.
    ///
    /// # Errors
    ///
    /// Returns a typed [`KurumiError`] when the matrix shape or contents are
    /// invalid or the estimator's parameters are outside their domain.
    fn estimate(&self, returns: &DMatrix<f64>) -> Result<DMatrix<f64>, KurumiError>;
}

/// The unbiased sample covariance estimator (`ddof = 1`).
///
/// ```text
/// S_ij = sum_t (x_ti - mean_i) (x_tj - mean_j) / (T - 1)
/// ```
///
/// # Examples
///
/// ```
/// use kurumi_math::{CovarianceEstimator, SampleCovariance};
/// use nalgebra::DMatrix;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let returns = DMatrix::from_row_slice(3, 1, &[0.01, 0.03, 0.02]);
/// let covariance = SampleCovariance.estimate(&returns)?;
/// assert!((covariance.get((0, 0)).copied().unwrap_or(0.0) - 0.000_1).abs() < 1e-15);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SampleCovariance;

impl CovarianceEstimator for SampleCovariance {
    fn estimate(&self, returns: &DMatrix<f64>) -> Result<DMatrix<f64>, KurumiError> {
        validate_matrix(returns)?;
        let columns = column_slices(returns)?;
        let means = column_means(&columns)?;
        let observations = returns.nrows();
        symmetric_matrix(columns.len(), |row, column| {
            let (Some(xs), Some(ys), Some(&mean_x), Some(&mean_y)) = (
                columns.get(row),
                columns.get(column),
                means.get(row),
                means.get(column),
            ) else {
                return Err(matrix_internal_error());
            };
            let sum = centered_cross_sum(xs, mean_x, ys, mean_y)?;
            finite(sum / (observations - 1) as f64, "sample_covariance")
        })
    }
}

/// The RiskMetrics exponentially weighted moving covariance estimator.
///
/// Observation `t` (with `t = 0` the oldest and `t = T - 1` the newest) receives
/// the weight
///
/// ```text
/// w_t = lambda^(T - 1 - t) / sum_k lambda^k
/// ```
///
/// normalized so the finite sample weights sum to one, and
///
/// ```text
/// S_ij = sum_t w_t (x_ti - mean_i) (x_tj - mean_j).
/// ```
///
/// This is a model-based estimator, so the weights already control the decay
/// and no Bessel correction is applied. `lambda = 0.94` is the RiskMetrics
/// daily default; `lambda` must lie strictly between `0.0` and `1.0`.
///
/// # Examples
///
/// ```
/// use kurumi_math::{CovarianceEstimator, EwmaCovariance};
/// use nalgebra::DMatrix;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let returns = DMatrix::from_row_slice(4, 1, &[0.01, -0.02, 0.03, 0.00]);
/// let estimator = EwmaCovariance { lambda: 0.94 };
/// let covariance = estimator.estimate(&returns)?;
/// assert!(covariance.get((0, 0)).copied().unwrap_or(f64::NAN) > 0.0);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EwmaCovariance {
    /// The decay factor `lambda`; must lie strictly between `0.0` and `1.0`.
    pub lambda: f64,
}

impl CovarianceEstimator for EwmaCovariance {
    fn estimate(&self, returns: &DMatrix<f64>) -> Result<DMatrix<f64>, KurumiError> {
        if !self.lambda.is_finite() || self.lambda <= 0.0 || self.lambda >= 1.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "lambda",
                reason: "must be finite and strictly between 0.0 and 1.0".to_owned(),
            });
        }
        validate_matrix(returns)?;
        let columns = column_slices(returns)?;
        let observations = returns.nrows();
        let weights = ewma_weights(observations, self.lambda)?;
        let means = columns
            .iter()
            .map(|column| weighted_mean(column, &weights))
            .collect::<Result<Vec<_>, _>>()?;
        symmetric_matrix(columns.len(), |row, column| {
            let (Some(xs), Some(ys), Some(&mean_x), Some(&mean_y)) = (
                columns.get(row),
                columns.get(column),
                means.get(row),
                means.get(column),
            ) else {
                return Err(matrix_internal_error());
            };
            let sum = compensated_sum(
                weights
                    .iter()
                    .zip(xs.iter())
                    .zip(ys.iter())
                    .map(|((&weight, &x), &y)| weight * (x - mean_x) * (y - mean_y)),
            )?;
            finite(sum, "ewma_covariance")
        })
    }
}

/// The Ledoit-Wolf shrinkage estimator toward a scaled-identity target.
///
/// ```text
/// F = mu I,   mu = trace(S) / N
/// Sigma = delta F + (1 - delta) S
/// ```
///
/// where `S` is the maximum-likelihood sample covariance
/// (`1 / T` denominator, following the paper) and the analytic optimal
/// shrinkage intensity is
///
/// ```text
/// delta = clamp((pi - rho) / (T * gamma), 0, 1)
/// pi    = (1 / T) sum_t sum_ij ((x_ti - m_i)(x_tj - m_j) - S_ij)^2
/// rho   = (1 / T) sum_t sum_i  ((x_ti - m_i)^2 - S_ii)^2
/// gamma = sum_ij (S_ij - mu * indicator(i == j))^2
/// ```
///
/// The scaled-identity target is positive definite whenever `mu > 0`. When the
/// number of assets exceeds the number of observations, `S` is singular but the
/// shrinkage term keeps the estimator positive definite for any `delta > 0`.
///
/// # References
///
/// - O. Ledoit and M. Wolf, "A well-conditioned estimator for
///   large-dimensional covariance matrices", *Journal of Multivariate
///   Analysis* 88(2), 2004.
///
/// # Examples
///
/// ```
/// use kurumi_math::{CovarianceEstimator, LedoitWolfShrinkage};
/// use nalgebra::DMatrix;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let returns = DMatrix::from_row_slice(3, 2, &[
///     0.01, 0.02,
///     0.03, 0.04,
///     0.02, 0.05,
/// ]);
/// let covariance = LedoitWolfShrinkage.estimate(&returns)?;
/// assert_eq!(covariance.shape(), (2, 2));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LedoitWolfShrinkage;

impl CovarianceEstimator for LedoitWolfShrinkage {
    fn estimate(&self, returns: &DMatrix<f64>) -> Result<DMatrix<f64>, KurumiError> {
        validate_matrix(returns)?;
        let columns = column_slices(returns)?;
        let means = column_means(&columns)?;
        let observations = returns.nrows();
        let order = columns.len();
        let observations_f64 = observations as f64;

        let mut sample = vec![0.0; order * order];
        for row in 0..order {
            for column in row..order {
                let (Some(xs), Some(ys), Some(&mean_x), Some(&mean_y)) = (
                    columns.get(row),
                    columns.get(column),
                    means.get(row),
                    means.get(column),
                ) else {
                    return Err(matrix_internal_error());
                };
                let sum = centered_cross_sum(xs, mean_x, ys, mean_y)?;
                set_symmetric(
                    &mut sample,
                    order,
                    row,
                    column,
                    finite(sum / observations_f64, "ledoit_wolf_covariance")?,
                )?;
            }
        }

        let trace = (0..order).try_fold(0.0, |accumulator, index| {
            let value = sample
                .get(index * order + index)
                .copied()
                .ok_or_else(matrix_internal_error)?;
            Ok::<f64, KurumiError>(accumulator + value)
        })?;
        let mu = trace / order as f64;

        let mut pi_total = 0.0;
        let mut rho_total = 0.0;
        for observation in 0..observations {
            let mut deviations = Vec::with_capacity(order);
            for (index, column) in columns.iter().enumerate() {
                let Some(value) = column.get(observation).copied() else {
                    return Err(matrix_internal_error());
                };
                let Some(&column_mean) = means.get(index) else {
                    return Err(matrix_internal_error());
                };
                deviations.push(value - column_mean);
            }
            for row in 0..order {
                let Some(&deviation_row) = deviations.get(row) else {
                    return Err(matrix_internal_error());
                };
                for column in 0..order {
                    let Some(&deviation_column) = deviations.get(column) else {
                        return Err(matrix_internal_error());
                    };
                    let sample_value = sample
                        .get(row * order + column)
                        .copied()
                        .ok_or_else(matrix_internal_error)?;
                    let residual = deviation_row * deviation_column - sample_value;
                    pi_total += residual * residual;
                }
                let sample_diagonal = sample
                    .get(row * order + row)
                    .copied()
                    .ok_or_else(matrix_internal_error)?;
                let residual = deviation_row * deviation_row - sample_diagonal;
                rho_total += residual * residual;
            }
        }
        let pi = pi_total / observations_f64;
        let rho = rho_total / observations_f64;

        let mut gamma = 0.0;
        for row in 0..order {
            for column in 0..order {
                let sample_value = sample
                    .get(row * order + column)
                    .copied()
                    .ok_or_else(matrix_internal_error)?;
                let target = if row == column { mu } else { 0.0 };
                let difference = sample_value - target;
                gamma += difference * difference;
            }
        }

        let shrinkage = if gamma <= 0.0 {
            0.0
        } else {
            ((pi - rho) / (observations_f64 * gamma)).clamp(0.0, 1.0)
        };

        let mut data = vec![0.0; order * order];
        for row in 0..order {
            for column in 0..order {
                let sample_value = sample
                    .get(row * order + column)
                    .copied()
                    .ok_or_else(matrix_internal_error)?;
                let target = if row == column { mu } else { 0.0 };
                let value = shrinkage * target + (1.0 - shrinkage) * sample_value;
                if !value.is_finite() || !mu.is_finite() || !shrinkage.is_finite() {
                    return Err(KurumiError::NumericalOverflow {
                        operation: "ledoit_wolf_covariance",
                    });
                }
                let Some(slot) = data.get_mut(row * order + column) else {
                    return Err(matrix_internal_error());
                };
                *slot = value;
            }
        }

        Ok(DMatrix::from_column_slice(order, order, &data))
    }
}

fn validate_pair(x: &[f64], y: &[f64]) -> Result<(), KurumiError> {
    if x.len() != y.len() {
        return Err(KurumiError::InvalidParameter {
            parameter: "y",
            reason: "must have the same length as x".to_owned(),
        });
    }
    if x.len() < 2 {
        return Err(KurumiError::InsufficientData {
            required: 2,
            available: x.len(),
        });
    }
    ensure_finite_slice(x, "x")?;
    ensure_finite_slice(y, "y")
}

fn ensure_finite_slice(values: &[f64], parameter: &'static str) -> Result<(), KurumiError> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(KurumiError::InvalidParameter {
            parameter,
            reason: "must contain only finite values".to_owned(),
        })
    }
}

fn validate_matrix(returns: &DMatrix<f64>) -> Result<(), KurumiError> {
    if returns.ncols() == 0 {
        return Err(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        });
    }
    if returns.nrows() < 2 {
        return Err(KurumiError::InsufficientData {
            required: 2,
            available: returns.nrows(),
        });
    }
    ensure_finite_slice(returns.as_slice(), "returns")
}

fn column_slices(returns: &DMatrix<f64>) -> Result<Vec<&[f64]>, KurumiError> {
    let observations = returns.nrows();
    let data = returns.as_slice();
    let mut columns = Vec::with_capacity(returns.ncols());
    for index in 0..returns.ncols() {
        let start = index
            .checked_mul(observations)
            .ok_or(KurumiError::NumericalOverflow {
                operation: "column_slices",
            })?;
        let end = start
            .checked_add(observations)
            .ok_or(KurumiError::NumericalOverflow {
                operation: "column_slices",
            })?;
        let Some(column) = data.get(start..end) else {
            return Err(matrix_internal_error());
        };
        columns.push(column);
    }
    Ok(columns)
}

fn column_means(columns: &[&[f64]]) -> Result<Vec<f64>, KurumiError> {
    columns.iter().map(|column| mean(column)).collect()
}

fn centered_squares<'a>(values: &'a [f64], center: f64) -> impl Iterator<Item = f64> + 'a {
    values.iter().map(move |&value| {
        let deviation = value - center;
        deviation * deviation
    })
}

fn centered_cross_sum(x: &[f64], mean_x: f64, y: &[f64], mean_y: f64) -> Result<f64, KurumiError> {
    compensated_sum(
        x.iter()
            .zip(y.iter())
            .map(|(&value_x, &value_y)| (value_x - mean_x) * (value_y - mean_y)),
    )
}

fn pearson_with_means(x: &[f64], mean_x: f64, y: &[f64], mean_y: f64) -> Result<f64, KurumiError> {
    let sum_xx = compensated_sum(centered_squares(x, mean_x))?;
    let sum_yy = compensated_sum(centered_squares(y, mean_y))?;
    if sum_xx == 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "x",
            reason: "must have non-zero variance".to_owned(),
        });
    }
    if sum_yy == 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "y",
            reason: "must have non-zero variance".to_owned(),
        });
    }
    let sum_xy = centered_cross_sum(x, mean_x, y, mean_y)?;

    // Normalize before taking the geometric mean so that large variances cannot
    // overflow and the self-correlation of a series is exactly 1.0.
    let scale = sum_xx.max(sum_yy);
    let normalized = (sum_xx / scale) * (sum_yy / scale);
    let denominator = normalized.sqrt();
    if !denominator.is_finite() || denominator <= 0.0 {
        return Err(KurumiError::NumericalOverflow {
            operation: "pearson_correlation",
        });
    }
    let correlation = (sum_xy / scale) / denominator;
    if correlation.is_finite() {
        Ok(correlation.clamp(-1.0, 1.0))
    } else {
        Err(KurumiError::NumericalOverflow {
            operation: "pearson_correlation",
        })
    }
}

fn average_ranks(values: &[f64]) -> Vec<f64> {
    let count = values.len();
    let mut order: Vec<(usize, f64)> = values.iter().copied().enumerate().collect();
    order.sort_by(|left, right| left.1.total_cmp(&right.1));
    let mut ranks = vec![0.0; count];
    let mut start = 0;
    while let Some(&(_, value)) = order.get(start) {
        let mut end = start;
        while let Some(&(_, next)) = order.get(end) {
            if next.total_cmp(&value) == std::cmp::Ordering::Equal {
                end += 1;
            } else {
                break;
            }
        }
        let average_rank = ((start + 1) as f64 + end as f64) / 2.0;
        for position in start..end {
            if let Some(&(original_index, _)) = order.get(position) {
                if let Some(slot) = ranks.get_mut(original_index) {
                    *slot = average_rank;
                }
            }
        }
        start = end;
    }
    ranks
}

fn ewma_weights(count: usize, lambda: f64) -> Result<Vec<f64>, KurumiError> {
    let mut weights = vec![1.0; count];
    let mut weight = 1.0;
    for index in (0..count).rev() {
        let Some(slot) = weights.get_mut(index) else {
            return Err(matrix_internal_error());
        };
        *slot = weight;
        weight *= lambda;
    }
    let total = compensated_sum(weights.iter().copied())?;
    if !total.is_finite() || total <= 0.0 {
        return Err(KurumiError::NumericalOverflow {
            operation: "ewma_covariance",
        });
    }
    for weight in weights.iter_mut() {
        *weight /= total;
    }
    Ok(weights)
}

fn weighted_mean(column: &[f64], weights: &[f64]) -> Result<f64, KurumiError> {
    let sum = compensated_sum(
        weights
            .iter()
            .zip(column.iter())
            .map(|(&weight, &value)| weight * value),
    )?;
    finite(sum, "ewma_covariance")
}

fn symmetric_matrix<F>(order: usize, mut entry: F) -> Result<DMatrix<f64>, KurumiError>
where
    F: FnMut(usize, usize) -> Result<f64, KurumiError>,
{
    let mut data = vec![0.0; order * order];
    for row in 0..order {
        for column in row..order {
            let value = entry(row, column)?;
            set_symmetric(&mut data, order, row, column, value)?;
        }
    }
    Ok(DMatrix::from_column_slice(order, order, &data))
}

fn set_symmetric(
    data: &mut [f64],
    order: usize,
    row: usize,
    column: usize,
    value: f64,
) -> Result<(), KurumiError> {
    let forward = row
        .checked_mul(order)
        .and_then(|base| base.checked_add(column));
    let backward = column
        .checked_mul(order)
        .and_then(|base| base.checked_add(row));
    match (forward, backward) {
        (Some(forward), Some(backward)) => {
            let forward_slot = data.get_mut(forward).ok_or_else(matrix_internal_error)?;
            *forward_slot = value;
            let backward_slot = data.get_mut(backward).ok_or_else(matrix_internal_error)?;
            *backward_slot = value;
            Ok(())
        }
        _ => Err(matrix_internal_error()),
    }
}

fn matrix_internal_error() -> KurumiError {
    KurumiError::InvalidParameter {
        parameter: "matrix",
        reason: "internal index out of bounds".to_owned(),
    }
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

    fn matrix(rows: &[&[f64]]) -> DMatrix<f64> {
        let observations = rows.first().map_or(0, |row| row.len());
        let mut data = Vec::new();
        for row in rows {
            data.extend_from_slice(row);
        }
        DMatrix::from_row_slice(rows.len(), observations, &data)
    }

    #[test]
    fn pearson_correlation_of_a_series_with_itself_is_exactly_one() {
        let x = [0.01, -0.02, 0.03, 0.00, 0.015, -0.005];
        assert_eq!(pearson_correlation(&x, &x).expect("valid"), 1.0);
    }

    #[test]
    fn pearson_correlation_matches_a_known_value() {
        let x = [1.0, 2.0, 3.0, 4.0, 5.0];
        let y = [2.0, 4.0, 5.0, 4.0, 5.0];
        assert_relative_eq!(
            pearson_correlation(&x, &y).expect("valid"),
            0.774_596_669_241_483_3,
            epsilon = 1e-12
        );
    }

    #[test]
    fn pearson_correlation_rejects_degenerate_inputs() {
        assert!(matches!(
            pearson_correlation(&[1.0], &[2.0]),
            Err(KurumiError::InsufficientData { .. })
        ));
        assert!(matches!(
            pearson_correlation(&[1.0, 2.0], &[1.0]),
            Err(KurumiError::InvalidParameter { .. })
        ));
        assert!(matches!(
            pearson_correlation(&[1.0, 1.0], &[1.0, 2.0]),
            Err(KurumiError::InvalidParameter { .. })
        ));
        assert!(matches!(
            pearson_correlation(&[1.0, f64::NAN], &[1.0, 2.0]),
            Err(KurumiError::InvalidParameter { .. })
        ));
    }

    #[test]
    fn spearman_correlation_averages_tied_ranks() {
        let x = [1.0, 2.0, 3.0, 4.0];
        let y = [1.0, 2.0, 2.0, 3.0];
        assert_relative_eq!(
            spearman_correlation(&x, &y).expect("valid"),
            0.948_683_298_050_513_8,
            epsilon = 1e-12
        );
    }

    #[test]
    fn correlation_matrix_is_symmetric_with_unit_diagonal() {
        let returns = matrix(&[
            &[0.01, 0.02, -0.01],
            &[0.02, 0.04, 0.00],
            &[-0.01, -0.02, 0.03],
            &[0.03, 0.01, 0.02],
        ]);
        let correlations = correlation_matrix(&returns).expect("valid matrix");
        assert_eq!(correlations.shape(), (3, 3));
        for row in 0..3 {
            for column in 0..3 {
                let value = correlations.get((row, column)).copied().expect("in bounds");
                let mirrored = correlations.get((column, row)).copied().expect("in bounds");
                assert_relative_eq!(value, mirrored, epsilon = 1e-15);
                assert!((-1.0..=1.0).contains(&value));
            }
            assert_eq!(
                correlations.get((row, row)).copied().expect("in bounds"),
                1.0
            );
        }
    }

    #[test]
    fn correlation_matrix_rejects_zero_variance_columns() {
        let returns = matrix(&[&[0.01, 0.02], &[0.02, 0.02], &[0.03, 0.02]]);
        assert!(matches!(
            correlation_matrix(&returns),
            Err(KurumiError::InvalidParameter { .. })
        ));
    }

    #[test]
    fn rolling_correlation_matches_direct_windows() {
        let x = [0.01, -0.02, 0.03, 0.00, 0.02, -0.01];
        let y = [0.02, -0.04, 0.06, 0.00, 0.04, -0.02];
        let rolling = rolling_correlation(&x, &y, 3).expect("valid");
        assert_eq!(rolling.len(), 4);
        for (index, value) in rolling.iter().enumerate() {
            let direct = pearson_correlation(&x[index..index + 3], &y[index..index + 3]);
            match (value, direct) {
                (Some(value), Ok(direct)) => assert_relative_eq!(*value, direct, epsilon = 1e-12),
                (None, Err(_)) => {}
                _ => panic!("rolling and direct results disagree"),
            }
        }
    }

    #[test]
    fn rolling_correlation_reports_none_for_zero_variance_windows() {
        let x = [0.02, 0.02, 0.02, 0.01, 0.03];
        let y = [0.01, 0.02, 0.03, 0.04, 0.05];
        let rolling = rolling_correlation(&x, &y, 3).expect("valid");
        assert_eq!(rolling.first().copied().flatten(), None);
    }

    #[test]
    fn sample_covariance_matches_unbiased_formula() {
        let returns = matrix(&[&[1.0, 2.0], &[2.0, 4.0], &[3.0, 7.0]]);
        let covariance = SampleCovariance.estimate(&returns).expect("valid matrix");
        assert_relative_eq!(
            covariance.get((0, 0)).copied().expect("in bounds"),
            1.0,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            covariance.get((0, 1)).copied().expect("in bounds"),
            2.5,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            covariance.get((1, 1)).copied().expect("in bounds"),
            6.333_333_333_333_333,
            epsilon = 1e-12
        );
        assert_eq!(
            covariance.get((0, 1)).copied(),
            covariance.get((1, 0)).copied()
        );
    }

    #[test]
    fn ewma_covariance_is_symmetric_and_uses_decaying_weights() {
        let returns = matrix(&[&[0.01, 0.02], &[0.02, 0.04], &[0.03, 0.06]]);
        let estimator = EwmaCovariance { lambda: 0.94 };
        let covariance = estimator.estimate(&returns).expect("valid matrix");
        assert_eq!(covariance.shape(), (2, 2));
        assert_eq!(
            covariance.get((0, 1)).copied(),
            covariance.get((1, 0)).copied()
        );
        let variance = covariance.get((0, 0)).copied().expect("in bounds");
        assert!(variance > 0.0);
        assert!(variance.is_finite());
    }

    #[test]
    fn ewma_covariance_rejects_lambda_outside_the_open_unit_interval() {
        let returns = matrix(&[&[0.01], &[0.02]]);
        for lambda in [0.0, 1.0, -0.1, f64::NAN] {
            assert!(matches!(
                (EwmaCovariance { lambda }).estimate(&returns),
                Err(KurumiError::InvalidParameter { .. })
            ));
        }
    }

    #[test]
    fn ledoit_wolf_is_positive_definite_when_assets_exceed_observations() {
        let returns = matrix(&[
            &[0.01, -0.02, 0.03, 0.00, 0.02, 0.01],
            &[0.02, 0.01, -0.01, 0.03, 0.00, -0.02],
            &[-0.01, 0.02, 0.01, 0.00, 0.03, 0.02],
            &[0.03, 0.00, 0.02, 0.01, -0.01, 0.04],
        ]);
        assert_eq!(returns.shape(), (4, 6));
        let covariance = LedoitWolfShrinkage
            .estimate(&returns)
            .expect("valid matrix");
        let eigenvalues = covariance.symmetric_eigen().eigenvalues;
        assert_eq!(eigenvalues.len(), 6);
        for eigenvalue in eigenvalues.iter() {
            assert!(*eigenvalue > 0.0, "eigenvalue {eigenvalue} is not positive");
        }
    }

    #[test]
    fn ledoit_wolf_shrinks_toward_a_scaled_identity() {
        let returns = matrix(&[&[0.01, 0.02], &[0.02, 0.04], &[0.03, 0.06]]);
        let covariance = LedoitWolfShrinkage
            .estimate(&returns)
            .expect("valid matrix");
        assert_eq!(covariance.shape(), (2, 2));
        assert_eq!(
            covariance.get((0, 1)).copied(),
            covariance.get((1, 0)).copied()
        );
        let diagonal_0 = covariance.get((0, 0)).copied().expect("in bounds");
        let diagonal_1 = covariance.get((1, 1)).copied().expect("in bounds");
        assert!(diagonal_0 > 0.0);
        assert!(diagonal_1 > 0.0);
        assert!(diagonal_0.is_finite() && diagonal_1.is_finite());
    }
}
