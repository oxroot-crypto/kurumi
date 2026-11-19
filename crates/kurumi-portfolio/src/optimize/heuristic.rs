//! Closed-form heuristic allocators: equal weight and inverse volatility.
//!
//! Both heuristics produce a base allocation and then enforce the caller's
//! [`OptimizationConstraints`] with the same box-simplex projection used by the
//! iterative solvers. With the default long-only, fully invested constraints the
//! projection is the identity, so the textbook allocations are recovered
//! exactly.
//!
//! # References
//!
//! - H. M. Markowitz, "Portfolio Selection", *Journal of Finance* 7(1), 1952.
//! - E. Qian, "Risk Parity Portfolios: Efficient Portfolios Through True
//!   Diversification", PanAgora Asset Management, 2005.

use kurumi_core::KurumiError;
use nalgebra::{DMatrix, DVector};

use super::{
    OptimizationConstraints, OptimizationResult, PortfolioOptimizer, build_result,
    project_box_simplex, validate_problem,
};

/// The equally weighted portfolio `w_i = target_sum / N`.
///
/// This is the simplest diversification rule and the standard naive benchmark
/// against which optimized allocations are compared. The allocation is
/// independent of the expected returns and covariance; the moments are used
/// only to report the resulting portfolio statistics.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{EqualWeight, PortfolioOptimizer};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let means = DVector::from_vec(vec![0.01, 0.02, 0.03]);
/// let covariance = DMatrix::from_diagonal(&DVector::from_vec(vec![0.04, 0.09, 0.01]));
/// let result = EqualWeight::new().optimize(&means, &covariance)?;
/// assert!((result.weights.sum() - 1.0).abs() < 1e-15);
/// assert!((result.expected_return - 0.02).abs() < 1e-15);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct EqualWeight {
    /// Per-asset bounds and the target net weight.
    pub constraints: OptimizationConstraints,
}

impl EqualWeight {
    /// Creates an equal-weight optimizer with the default long-only,
    /// fully invested constraints.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::EqualWeight;
    /// assert_eq!(EqualWeight::new().constraints.target_sum, 1.0);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            constraints: OptimizationConstraints::default(),
        }
    }

    /// Creates an equal-weight optimizer with explicit constraints.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::{EqualWeight, OptimizationConstraints};
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let constraints = OptimizationConstraints::new(Vec::new(), vec![0.4, 0.4], 1.0)?;
    /// let optimizer = EqualWeight::with_constraints(constraints);
    /// assert_eq!(optimizer.constraints.max_weights, [0.4, 0.4]);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_constraints(constraints: OptimizationConstraints) -> Self {
        Self { constraints }
    }
}

impl Default for EqualWeight {
    fn default() -> Self {
        Self::new()
    }
}

impl PortfolioOptimizer for EqualWeight {
    fn optimize(
        &self,
        expected_returns: &DVector<f64>,
        covariance: &DMatrix<f64>,
    ) -> Result<OptimizationResult, KurumiError> {
        let assets = expected_returns.len();
        let _ = validate_problem(expected_returns, covariance, &self.constraints)?;
        let (minimum, maximum) = self.constraints.bounds(assets)?;
        let base = self.constraints.target_sum / assets as f64;
        let raw = DVector::from_element(assets, base);
        let weights = project_box_simplex(&raw, &minimum, &maximum, self.constraints.target_sum)?;
        build_result(&weights, expected_returns, covariance, 0)
    }
}

/// The inverse-volatility portfolio, also called the naive risk-parity
/// portfolio.
///
/// With `sigma_i = sqrt(Sigma_ii)` the raw allocation is
///
/// ```text
/// w_i proportional to 1 / sigma_i,
/// ```
///
/// rescaled so that `sum(w) = target_sum` and then projected onto the
/// constrained simplex. Assets with lower volatility receive a larger weight.
///
/// A zero-variance asset makes the raw weight infinite and is rejected with
/// [`KurumiError::InvalidParameter`].
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{InverseVolatility, PortfolioOptimizer};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let means = DVector::from_vec(vec![0.01, 0.02]);
/// let covariance = DMatrix::from_diagonal(&DVector::from_vec(vec![0.01, 0.04]));
/// let result = InverseVolatility::new().optimize(&means, &covariance)?;
/// // Volatilities are 0.1 and 0.2, so the weights are 2/3 and 1/3.
/// assert!((result.weights[0] - 2.0 / 3.0).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct InverseVolatility {
    /// Per-asset bounds and the target net weight.
    pub constraints: OptimizationConstraints,
}

impl InverseVolatility {
    /// Creates an inverse-volatility optimizer with the default long-only,
    /// fully invested constraints.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::InverseVolatility;
    /// assert_eq!(InverseVolatility::new().constraints.target_sum, 1.0);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            constraints: OptimizationConstraints::default(),
        }
    }

    /// Creates an inverse-volatility optimizer with explicit constraints.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::{InverseVolatility, OptimizationConstraints};
    ///
    /// let optimizer =
    ///     InverseVolatility::with_constraints(OptimizationConstraints::default());
    /// assert!(optimizer.constraints.min_weights.is_empty());
    /// ```
    #[must_use]
    pub fn with_constraints(constraints: OptimizationConstraints) -> Self {
        Self { constraints }
    }
}

impl Default for InverseVolatility {
    fn default() -> Self {
        Self::new()
    }
}

impl PortfolioOptimizer for InverseVolatility {
    fn optimize(
        &self,
        expected_returns: &DVector<f64>,
        covariance: &DMatrix<f64>,
    ) -> Result<OptimizationResult, KurumiError> {
        let assets = expected_returns.len();
        let _ = validate_problem(expected_returns, covariance, &self.constraints)?;
        let (minimum, maximum) = self.constraints.bounds(assets)?;
        let mut raw = Vec::with_capacity(assets);
        let mut total = 0.0;
        for index in 0..assets {
            let variance = covariance
                .get((index, index))
                .copied()
                .ok_or_else(|| index_error(index))?;
            if variance < 0.0 {
                return Err(KurumiError::InvalidParameter {
                    parameter: "covariance",
                    reason: format!("diagonal entry {index} must be non-negative"),
                });
            }
            if variance == 0.0 {
                return Err(KurumiError::InvalidParameter {
                    parameter: "covariance",
                    reason: format!(
                        "asset {index} has zero variance, so inverse volatility is undefined"
                    ),
                });
            }
            let inverse = 1.0 / variance.sqrt();
            if !inverse.is_finite() {
                return Err(KurumiError::NumericalOverflow {
                    operation: "inverse_volatility",
                });
            }
            total += inverse;
            raw.push(inverse);
        }
        if !total.is_finite() || total <= 0.0 {
            return Err(KurumiError::NumericalOverflow {
                operation: "inverse_volatility",
            });
        }
        let scale = self.constraints.target_sum / total;
        let scaled = DVector::from_vec(raw.iter().map(|value| value * scale).collect::<Vec<_>>());
        let weights =
            project_box_simplex(&scaled, &minimum, &maximum, self.constraints.target_sum)?;
        build_result(&weights, expected_returns, covariance, 0)
    }
}

fn index_error(index: usize) -> KurumiError {
    KurumiError::InvalidParameter {
        parameter: "covariance",
        reason: format!("diagonal entry {index} is missing"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn equal_weight_is_uniform_and_fully_invested() {
        let means = DVector::from_vec(vec![0.01, 0.02, 0.03, 0.04]);
        let covariance = DMatrix::from_diagonal(&DVector::from_vec(vec![0.04, 0.09, 0.01, 0.16]));
        let result = EqualWeight::new()
            .optimize(&means, &covariance)
            .expect("equal weight succeeds");
        assert_relative_eq!(result.weights.sum(), 1.0, epsilon = 1e-15);
        for weight in result.weights.iter() {
            assert_relative_eq!(*weight, 0.25, epsilon = 1e-15);
        }
        assert_relative_eq!(result.expected_return, 0.025, epsilon = 1e-15);
        assert_eq!(result.iterations, 0);
    }

    #[test]
    fn inverse_volatility_is_proportional_to_one_over_sigma() {
        let means = DVector::from_vec(vec![0.01, 0.02]);
        let covariance = DMatrix::from_diagonal(&DVector::from_vec(vec![0.01, 0.04]));
        let result = InverseVolatility::new()
            .optimize(&means, &covariance)
            .expect("inverse volatility succeeds");
        assert_relative_eq!(result.weights[0], 2.0 / 3.0, epsilon = 1e-12);
        assert_relative_eq!(result.weights[1], 1.0 / 3.0, epsilon = 1e-12);
    }

    #[test]
    fn heuristics_respect_upper_bounds() {
        let means = DVector::from_vec(vec![0.01, 0.02]);
        let covariance = DMatrix::from_diagonal(&DVector::from_vec(vec![0.01, 0.04]));
        let constraints =
            OptimizationConstraints::new(Vec::new(), vec![0.5, 0.5], 1.0).expect("valid bounds");
        let result = InverseVolatility::with_constraints(constraints)
            .optimize(&means, &covariance)
            .expect("constrained inverse volatility succeeds");
        assert_relative_eq!(result.weights[0], 0.5, epsilon = 1e-12);
        assert_relative_eq!(result.weights[1], 0.5, epsilon = 1e-12);
    }

    #[test]
    fn inverse_volatility_rejects_zero_variance() {
        let means = DVector::from_vec(vec![0.01, 0.02]);
        let covariance = DMatrix::from_diagonal(&DVector::from_vec(vec![0.0, 0.04]));
        assert!(
            InverseVolatility::new()
                .optimize(&means, &covariance)
                .is_err()
        );
    }
}
