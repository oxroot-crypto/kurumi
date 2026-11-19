//! Ordinary least squares regression.
//!
//! The single-variable solver is a closed-form covariance/variance fit. The
//! multiple-variable solver builds an optional intercept column and solves the
//! least-squares problem with a singular value decomposition (SVD) of the design
//! matrix, which stays numerically stable when regressors are collinear.
//!
//! Residual standard errors use the residual degrees of freedom, and `R^2` is
//! clamped to `[0, 1]` to absorb floating-point round-off.
//!
//! # References
//!
//! - G. Strang, *Linear Algebra and Its Applications*, 5th edition, 2016.

use kurumi_core::KurumiError;
use nalgebra::{DMatrix, DVector};

use crate::stats::{compensated_sum, covariance, mean, sample_variance};

/// The fitted coefficients of a simple ordinary least squares regression.
///
/// The model is
///
/// ```text
/// y_i = alpha + beta * x_i + epsilon_i
/// ```
///
/// where [`beta`](Self::beta) is the slope, [`alpha`](Self::alpha) the
/// intercept, [`r_squared`](Self::r_squared) the coefficient of determination,
/// and [`residual_std_error`](Self::residual_std_error) the square root of the
/// residual sum of squares divided by `T - 2`.
///
/// # Examples
///
/// ```
/// use kurumi_math::ols_simple;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let x = [0.0, 1.0, 2.0, 3.0];
/// let y = [0.5, 2.5, 4.5, 6.5];
/// let fit = ols_simple(&x, &y)?;
/// assert!((fit.beta - 2.0).abs() < 1e-12);
/// assert!((fit.alpha - 0.5).abs() < 1e-12);
/// assert!((fit.r_squared - 1.0).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimpleRegressionResult {
    /// The estimated intercept `alpha`.
    pub alpha: f64,
    /// The estimated slope `beta`.
    pub beta: f64,
    /// The coefficient of determination `R^2`, clamped to `[0, 1]`.
    pub r_squared: f64,
    /// The residual standard error, `sqrt(SSR / (T - 2))`.
    pub residual_std_error: f64,
}

/// The fitted coefficients of a multiple ordinary least squares regression.
///
/// The model is
///
/// ```text
/// y = intercept + X * coefficients + epsilon
/// ```
///
/// when the model is fitted with an intercept, and
/// `y = X * coefficients + epsilon` otherwise. Coefficients are reported in the
/// column order of the design matrix.
///
/// # Examples
///
/// ```
/// use kurumi_math::ols_multiple;
/// use nalgebra::{DMatrix, DVector};
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let x = DMatrix::from_row_slice(4, 2, &[
///     1.0, 0.5,
///     2.0, 1.0,
///     3.0, 1.5,
///     4.0, 2.0,
/// ]);
/// let y = DVector::from_vec(vec![2.0, 4.0, 6.0, 8.0]);
/// let fit = ols_multiple(&x, &y, true)?;
/// assert!((fit.intercept.unwrap_or(f64::NAN) - 0.0).abs() < 1e-12);
/// assert_eq!(fit.coefficients.len(), 2);
/// assert!((fit.r_squared - 1.0).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MultipleRegressionResult {
    /// The fitted intercept, or `None` when the model was fitted without one.
    pub intercept: Option<f64>,
    /// The fitted factor coefficients in the column order of the design matrix.
    pub coefficients: Vec<f64>,
    /// The coefficient of determination `R^2`, clamped to `[0, 1]`.
    pub r_squared: f64,
    /// The degrees-of-freedom adjusted coefficient of determination.
    pub adjusted_r_squared: f64,
}

/// Fits a simple ordinary least squares regression with an intercept.
///
/// ```text
/// beta  = cov(x, y) / var(x)
/// alpha = mean(y) - beta * mean(x)
/// R^2   = 1 - SSR / SST
/// ```
///
/// `SST` is the centered sum of squares because the model includes an
/// intercept. The residual standard error is `sqrt(SSR / (T - 2))`.
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for fewer than three observations
/// (the residual standard error needs at least one residual degree of freedom),
/// [`KurumiError::InvalidParameter`] when the lengths differ, a value is not
/// finite, `x` has zero variance, or `y` is constant, and
/// [`KurumiError::NumericalOverflow`] when a sum is not representable.
///
/// # Examples
///
/// ```
/// use kurumi_math::ols_simple;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let x = [-2.0, -1.0, 0.0, 1.0, 2.0];
/// let y = [-3.99, -1.99, 0.01, 2.01, 4.01];
/// let fit = ols_simple(&x, &y)?;
/// assert_eq!(fit.beta, 2.0);
/// assert!((fit.alpha - 0.01).abs() < 1e-12);
/// assert_eq!(fit.r_squared, 1.0);
/// # Ok(())
/// # }
/// ```
pub fn ols_simple(x: &[f64], y: &[f64]) -> Result<SimpleRegressionResult, KurumiError> {
    validate_pair(x, y)?;
    if x.len() < 3 {
        return Err(KurumiError::InsufficientData {
            required: 3,
            available: x.len(),
        });
    }

    let variance_x = sample_variance(x)?;
    if variance_x == 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "x",
            reason: "must have non-zero variance".to_owned(),
        });
    }
    let beta = finite(covariance(x, y)? / variance_x, "ols_simple")?;
    let mean_x = mean(x)?;
    let mean_y = mean(y)?;
    let alpha = finite(mean_y - beta * mean_x, "ols_simple")?;

    let residual_sum = compensated_sum(x.iter().zip(y.iter()).map(|(&value_x, &value_y)| {
        let residual = value_y - (alpha + beta * value_x);
        residual * residual
    }))?;
    let total_sum = compensated_sum(y.iter().map(|&value| {
        let deviation = value - mean_y;
        deviation * deviation
    }))?;
    if total_sum == 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "y",
            reason: "must not be constant".to_owned(),
        });
    }
    let r_squared = finite(1.0 - residual_sum / total_sum, "ols_simple")?.clamp(0.0, 1.0);
    let residual_std_error = finite((residual_sum / (x.len() - 2) as f64).sqrt(), "ols_simple")?;

    Ok(SimpleRegressionResult {
        alpha,
        beta,
        r_squared,
        residual_std_error,
    })
}

/// Fits a multiple ordinary least squares regression.
///
/// `x` has shape `(T, p)`: rows are observations and columns are regressors.
/// When `add_intercept` is `true`, a leading column of ones is prepended before
/// the SVD solve. Collinear or rank-deficient regressors are handled by a
/// truncated pseudo-inverse and produce a minimum-norm coefficient vector.
///
/// ```text
/// R^2 = 1 - SSR / SST
/// adjusted R^2 = 1 - (1 - R^2) * dft / dfr
/// ```
///
/// `SST` is centered around the response mean when an intercept is present and
/// uncentered otherwise; `dft` is `T - 1` with an intercept and `T` without one,
/// and `dfr` is the residual degrees of freedom (`T - p - 1` with an intercept,
/// `T - p` without one).
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] when the design matrix has no
/// columns or does not leave at least one residual degree of freedom,
/// [`KurumiError::InvalidParameter`] when the shapes disagree, a value is not
/// finite, the design has no non-zero column, or the response is constant (or
/// identically zero without an intercept), and
/// [`KurumiError::NumericalOverflow`] when a sum is not representable.
///
/// # Examples
///
/// ```
/// use kurumi_math::ols_multiple;
/// use nalgebra::{DMatrix, DVector};
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let x = DMatrix::from_row_slice(4, 2, &[
///     1.0, 0.5,
///     2.0, 1.0,
///     3.0, 1.5,
///     4.0, 2.0,
/// ]);
/// let y = DVector::from_vec(vec![3.0, 5.0, 7.0, 9.0]);
/// let fit = ols_multiple(&x, &y, true)?;
/// // The second regressor is collinear with the first; the fit is still exact.
/// assert!((fit.r_squared - 1.0).abs() < 1e-12);
/// assert!(fit.coefficients.iter().all(|value| value.is_finite()));
/// # Ok(())
/// # }
/// ```
pub fn ols_multiple(
    x: &DMatrix<f64>,
    y: &DVector<f64>,
    add_intercept: bool,
) -> Result<MultipleRegressionResult, KurumiError> {
    validate_multiple_input(x, y)?;
    let observations = x.nrows();
    let predictors = x.ncols();
    let design_columns = predictors + usize::from(add_intercept);
    let required = design_columns + 1;
    if observations < required {
        return Err(KurumiError::InsufficientData {
            required,
            available: observations,
        });
    }

    let mut row_major = Vec::with_capacity(observations * design_columns);
    for (_, predictor_row) in y.iter().zip(x.row_iter()) {
        if add_intercept {
            row_major.push(1.0);
        }
        row_major.extend(predictor_row.iter().copied());
    }
    let design = DMatrix::from_row_slice(observations, design_columns, &row_major);

    let svd = design.clone().svd(true, true);
    let largest_singular_value = svd.singular_values.iter().copied().fold(0.0, f64::max);
    if largest_singular_value <= 0.0 || !largest_singular_value.is_finite() {
        return Err(KurumiError::InvalidParameter {
            parameter: "x",
            reason: "must contain at least one non-zero column".to_owned(),
        });
    }
    let tolerance =
        largest_singular_value * (observations.max(design_columns) as f64) * f64::EPSILON;
    let solution = svd
        .solve(y, tolerance)
        .map_err(|_| KurumiError::InvalidParameter {
            parameter: "x",
            reason: "SVD solve failed to produce a least-squares solution".to_owned(),
        })?;

    let (intercept, coefficients) = if add_intercept {
        let intercept = solution.get(0).copied().ok_or_else(matrix_internal_error)?;
        (
            Some(intercept),
            solution.iter().skip(1).copied().collect::<Vec<f64>>(),
        )
    } else {
        (None, solution.iter().copied().collect::<Vec<f64>>())
    };
    if !coefficients.iter().all(|value| value.is_finite())
        || intercept.is_some_and(|value| !value.is_finite())
    {
        return Err(KurumiError::NumericalOverflow {
            operation: "ols_multiple",
        });
    }

    let fitted = &design * &solution;
    let residual_sum =
        compensated_sum(fitted.iter().zip(y.iter()).map(|(&prediction, &target)| {
            let residual = target - prediction;
            residual * residual
        }))?;
    let total_sum = if add_intercept {
        let mean_y = mean(y.as_slice())?;
        compensated_sum(y.iter().map(|&value| {
            let deviation = value - mean_y;
            deviation * deviation
        }))?
    } else {
        compensated_sum(y.iter().map(|&value| value * value))?
    };
    if total_sum == 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "y",
            reason: "must not be constant".to_owned(),
        });
    }

    let r_squared = finite(1.0 - residual_sum / total_sum, "ols_multiple")?.clamp(0.0, 1.0);
    let residual_degrees = observations - design_columns;
    let total_degrees = if add_intercept {
        observations - 1
    } else {
        observations
    };
    let adjusted_r_squared = finite(
        1.0 - (1.0 - r_squared) * total_degrees as f64 / residual_degrees as f64,
        "ols_multiple",
    )?;

    Ok(MultipleRegressionResult {
        intercept,
        coefficients,
        r_squared,
        adjusted_r_squared,
    })
}

fn validate_pair(x: &[f64], y: &[f64]) -> Result<(), KurumiError> {
    if x.len() != y.len() {
        return Err(KurumiError::InvalidParameter {
            parameter: "y",
            reason: "must have the same length as x".to_owned(),
        });
    }
    ensure_finite(x, "x")?;
    ensure_finite(y, "y")
}

fn validate_multiple_input(x: &DMatrix<f64>, y: &DVector<f64>) -> Result<(), KurumiError> {
    if x.nrows() != y.len() {
        return Err(KurumiError::InvalidParameter {
            parameter: "y",
            reason: "must have one entry per row of x".to_owned(),
        });
    }
    if x.ncols() == 0 {
        return Err(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        });
    }
    ensure_finite(x.as_slice(), "x")?;
    ensure_finite(y.as_slice(), "y")
}

fn ensure_finite(values: &[f64], parameter: &'static str) -> Result<(), KurumiError> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(KurumiError::InvalidParameter {
            parameter,
            reason: "must contain only finite values".to_owned(),
        })
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

    #[test]
    fn simple_regression_recovers_two_x_plus_a_hundredth() {
        let x = [-2.0, -1.0, 0.0, 1.0, 2.0];
        let y = [-3.99, -1.99, 0.01, 2.01, 4.01];
        let fit = ols_simple(&x, &y).expect("valid regression");
        assert_eq!(fit.beta, 2.0);
        assert_relative_eq!(fit.alpha, 0.01, epsilon = 1e-12);
        assert_eq!(fit.r_squared, 1.0);
        assert!(fit.residual_std_error < 1e-12);
    }

    #[test]
    fn simple_regression_rejects_degenerate_inputs() {
        assert!(matches!(
            ols_simple(&[1.0, 2.0], &[1.0, 2.0]),
            Err(KurumiError::InsufficientData { .. })
        ));
        assert!(matches!(
            ols_simple(&[1.0, 1.0, 1.0], &[1.0, 2.0, 3.0]),
            Err(KurumiError::InvalidParameter { .. })
        ));
        assert!(matches!(
            ols_simple(&[1.0, 2.0, 3.0], &[2.0, 2.0, 2.0]),
            Err(KurumiError::InvalidParameter { .. })
        ));
    }

    #[test]
    fn multiple_regression_recovers_exact_coefficients() {
        let x = DMatrix::from_row_slice(5, 2, &[1.0, 0.0, 2.0, 1.0, 3.0, 0.0, 4.0, 1.0, 5.0, 0.0]);
        let y = DVector::from_vec(vec![1.5, 4.5, 3.5, 6.5, 5.5]);
        let fit = ols_multiple(&x, &y, true).expect("valid regression");
        assert_relative_eq!(fit.intercept.expect("intercept"), 0.5, epsilon = 1e-12);
        assert_eq!(fit.coefficients.len(), 2);
        assert_relative_eq!(fit.coefficients[0], 1.0, epsilon = 1e-12);
        assert_relative_eq!(fit.coefficients[1], 2.0, epsilon = 1e-12);
        assert_relative_eq!(fit.r_squared, 1.0, epsilon = 1e-15);
    }

    #[test]
    fn multiple_regression_handles_collinear_regressors() {
        let x = DMatrix::from_row_slice(5, 2, &[1.0, 2.0, 2.0, 4.0, 3.0, 6.0, 4.0, 8.0, 5.0, 10.0]);
        let y = DVector::from_vec(vec![6.0, 11.0, 16.0, 21.0, 26.0]);
        let fit = ols_multiple(&x, &y, true).expect("valid regression");
        assert!(fit.coefficients.iter().all(|value| value.is_finite()));
        assert_relative_eq!(fit.r_squared, 1.0, epsilon = 1e-15);
        assert!(fit.adjusted_r_squared.is_finite());
    }

    #[test]
    fn multiple_regression_without_intercept_uses_uncentered_tss() {
        let x = DMatrix::from_row_slice(4, 1, &[1.0, 2.0, 3.0, 4.0]);
        let y = DVector::from_vec(vec![2.0, 4.0, 6.0, 8.0]);
        let fit = ols_multiple(&x, &y, false).expect("valid regression");
        assert!(fit.intercept.is_none());
        assert_relative_eq!(fit.coefficients[0], 2.0, epsilon = 1e-12);
        assert_relative_eq!(fit.r_squared, 1.0, epsilon = 1e-15);
    }
}
