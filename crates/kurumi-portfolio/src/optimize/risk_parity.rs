//! Risk-parity allocation via the Spinu convex formulation.
//!
//! The equal-risk-contribution (ERC) portfolio chooses weights so that every
//! asset contributes the same fraction of total portfolio risk. With
//! `Sigma` the covariance matrix, the percentage contribution to risk of asset
//! `i` is
//!
//! ```text
//! PCTR_i = w_i * (Sigma w)_i / (w^T Sigma w),   sum_i PCTR_i = 1.
//! ```
//!
//! For custom risk budgets `b_i > 0`, `sum_i b_i = 1`, the target is
//! `PCTR_i = b_i`. The allocation is computed with Daniel Spinu's convex
//! reformulation
//!
//! ```text
//! minimize (1/2) x^T Sigma x - sum_i b_i ln(x_i),   x > 0,
//! w = x / sum(x),
//! ```
//!
//! which is solved by cyclical coordinate descent. The first-order condition for
//! coordinate `i` is `(Sigma x)_i = b_i / x_i`, so with
//! `a = sum_{j != i} Sigma_ij x_j` the update is the positive root of
//! `Sigma_ii x_i^2 + a x_i - b_i = 0`:
//!
//! ```text
//! x_i <- (-a + sqrt(a^2 + 4 Sigma_ii b_i)) / (2 Sigma_ii).
//! ```
//!
//! # References
//!
//! - D. Spinu, "An Algorithm for Computing Risk Parity Weights", SSRN, 2013.
//! - T. Roncalli, *Introduction to Risk Parity and Budgeting*, Chapman &
//!   Hall/CRC, 2013.

use kurumi_core::KurumiError;
use nalgebra::{DMatrix, DVector};

use super::{
    OptimizationConstraints, OptimizationResult, PortfolioOptimizer, build_result,
    project_box_simplex, validate_problem,
};

/// The default convergence tolerance on the relative coordinate update.
pub const DEFAULT_ERC_TOLERANCE: f64 = 1e-12;
/// The default cyclical-coordinate-descent iteration budget.
pub const DEFAULT_ERC_ITERATIONS: usize = 100_000;

/// The equal-risk-contribution (ERC) portfolio, optionally with custom risk
/// budgets.
///
/// With an empty [`risk_budgets`](Self::risk_budgets) vector every asset gets
/// an equal share `1 / N` of total risk. Supplied budgets are normalized to sum
/// to one; each must be finite and strictly positive. The final weights honor
/// the caller's [`OptimizationConstraints`], which may make the exact risk
/// budgets unattainable when a bound is active.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{EqualRiskContribution, PortfolioOptimizer};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let means = DVector::from_vec(vec![0.01, 0.02, 0.03]);
/// let covariance = DMatrix::from_diagonal(&DVector::from_vec(vec![0.04, 0.09, 0.01]));
/// let result = EqualRiskContribution::new().optimize(&means, &covariance)?;
/// // Risk contributions are equal even though the weights are not.
/// assert!((result.weights[0] - result.weights[1]).abs() > 0.01);
/// assert!((result.weights.sum() - 1.0).abs() < 1e-9);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct EqualRiskContribution {
    /// Per-asset bounds and the target net weight.
    pub constraints: OptimizationConstraints,
    /// Risk budgets `b_i`; empty means equal budgets `1 / N`.
    pub risk_budgets: Vec<f64>,
    /// Relative coordinate-update tolerance.
    pub tolerance: f64,
    /// Maximum number of coordinate-descent sweeps.
    pub max_iterations: usize,
}

impl EqualRiskContribution {
    /// Creates an ERC optimizer with equal risk budgets and default
    /// constraints.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::EqualRiskContribution;
    /// let optimizer = EqualRiskContribution::new();
    /// assert!(optimizer.risk_budgets.is_empty());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            constraints: OptimizationConstraints::default(),
            risk_budgets: Vec::new(),
            tolerance: DEFAULT_ERC_TOLERANCE,
            max_iterations: DEFAULT_ERC_ITERATIONS,
        }
    }

    /// Creates an ERC optimizer with explicit risk budgets.
    ///
    /// The budgets are validated and normalized when
    /// [`PortfolioOptimizer::optimize`] runs, because their length must match
    /// the asset count.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::EqualRiskContribution;
    /// let optimizer = EqualRiskContribution::with_risk_budgets(vec![0.5, 0.3, 0.2]);
    /// assert_eq!(optimizer.risk_budgets, [0.5, 0.3, 0.2]);
    /// ```
    #[must_use]
    pub fn with_risk_budgets(risk_budgets: Vec<f64>) -> Self {
        Self {
            risk_budgets,
            ..Self::new()
        }
    }

    /// Overrides the solver tolerance and iteration budget.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `tolerance` is not finite
    /// and strictly positive or when `max_iterations` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::EqualRiskContribution;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let optimizer = EqualRiskContribution::new().with_solver_settings(1e-10, 50_000)?;
    /// assert_eq!(optimizer.max_iterations, 50_000);
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_solver_settings(
        mut self,
        tolerance: f64,
        max_iterations: usize,
    ) -> Result<Self, KurumiError> {
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "tolerance",
                reason: "must be finite and strictly positive".to_owned(),
            });
        }
        if max_iterations == 0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "max_iterations",
                reason: "must be strictly positive".to_owned(),
            });
        }
        self.tolerance = tolerance;
        self.max_iterations = max_iterations;
        Ok(self)
    }

    /// Normalizes the configured budgets for `assets` assets.
    fn normalized_budgets(&self, assets: usize) -> Result<Vec<f64>, KurumiError> {
        if self.risk_budgets.is_empty() {
            return Ok(vec![1.0 / assets as f64; assets]);
        }
        if self.risk_budgets.len() != assets {
            return Err(KurumiError::InvalidParameter {
                parameter: "risk_budgets",
                reason: format!(
                    "length {} does not match the {assets} assets",
                    self.risk_budgets.len()
                ),
            });
        }
        let mut total = 0.0;
        for (index, budget) in self.risk_budgets.iter().enumerate() {
            if !budget.is_finite() || *budget <= 0.0 {
                return Err(KurumiError::InvalidParameter {
                    parameter: "risk_budgets",
                    reason: format!("budget at index {index} must be finite and strictly positive"),
                });
            }
            total += budget;
        }
        if !total.is_finite() || total <= 0.0 {
            return Err(KurumiError::NumericalOverflow {
                operation: "risk_budget_normalization",
            });
        }
        Ok(self
            .risk_budgets
            .iter()
            .map(|budget| budget / total)
            .collect())
    }
}

impl Default for EqualRiskContribution {
    fn default() -> Self {
        Self::new()
    }
}

impl PortfolioOptimizer for EqualRiskContribution {
    fn optimize(
        &self,
        expected_returns: &DVector<f64>,
        covariance: &DMatrix<f64>,
    ) -> Result<OptimizationResult, KurumiError> {
        let assets = expected_returns.len();
        let _ = validate_problem(expected_returns, covariance, &self.constraints)?;
        let (minimum, maximum) = self.constraints.bounds(assets)?;
        let budgets = self.normalized_budgets(assets)?;
        if !self.tolerance.is_finite() || self.tolerance <= 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "tolerance",
                reason: "must be finite and strictly positive".to_owned(),
            });
        }
        if self.max_iterations == 0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "max_iterations",
                reason: "must be strictly positive".to_owned(),
            });
        }

        let mut x = Vec::with_capacity(assets);
        for index in 0..assets {
            let variance = covariance
                .get((index, index))
                .copied()
                .ok_or_else(|| index_error(index))?;
            if variance <= 0.0 {
                return Err(KurumiError::InvalidParameter {
                    parameter: "covariance",
                    reason: format!("asset {index} has zero variance, so risk parity is undefined"),
                });
            }
            x.push(1.0 / variance.sqrt());
        }

        let mut iterations = 0;
        let mut converged = false;
        for sweep in 0..self.max_iterations {
            iterations = sweep + 1;
            let mut largest_change = 0.0_f64;
            for index in 0..assets {
                let diagonal = covariance
                    .get((index, index))
                    .copied()
                    .ok_or_else(|| index_error(index))?;
                let Some(&budget) = budgets.get(index) else {
                    return Err(index_error(index));
                };
                let mut cross = 0.0;
                for other in 0..assets {
                    if other == index {
                        continue;
                    }
                    let value = covariance
                        .get((index, other))
                        .copied()
                        .ok_or_else(|| index_error(other))?;
                    let Some(&other_weight) = x.get(other) else {
                        return Err(index_error(other));
                    };
                    cross += value * other_weight;
                }
                let discriminant = cross * cross + 4.0 * diagonal * budget;
                if !discriminant.is_finite() || discriminant < 0.0 {
                    return Err(KurumiError::NumericalOverflow {
                        operation: "risk_parity_coordinate_descent",
                    });
                }
                let updated = (-cross + discriminant.sqrt()) / (2.0 * diagonal);
                if !updated.is_finite() || updated <= 0.0 {
                    return Err(KurumiError::ConvergenceFailed {
                        method: "risk_parity_coordinate_descent",
                        iterations,
                    });
                }
                let Some(previous) = x.get(index).copied() else {
                    return Err(index_error(index));
                };
                let relative = (updated - previous).abs() / previous.abs().max(1.0);
                largest_change = largest_change.max(relative);
                let Some(slot) = x.get_mut(index) else {
                    return Err(index_error(index));
                };
                *slot = updated;
            }
            if largest_change <= self.tolerance {
                converged = true;
                break;
            }
        }
        if !converged {
            return Err(KurumiError::ConvergenceFailed {
                method: "risk_parity_coordinate_descent",
                iterations,
            });
        }

        let total = x.iter().sum::<f64>();
        if !total.is_finite() || total <= 0.0 {
            return Err(KurumiError::NumericalOverflow {
                operation: "risk_parity_normalization",
            });
        }
        let scale = self.constraints.target_sum / total;
        let raw = DVector::from_vec(x.iter().map(|value| value * scale).collect());
        let weights = project_box_simplex(&raw, &minimum, &maximum, self.constraints.target_sum)?;
        build_result(&weights, expected_returns, covariance, iterations)
    }
}

fn index_error(index: usize) -> KurumiError {
    KurumiError::InvalidParameter {
        parameter: "covariance",
        reason: format!("entry for asset {index} is missing"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    use crate::analytics::{PortfolioWeights, risk_decomposition};

    fn three_asset_covariance() -> DMatrix<f64> {
        DMatrix::from_row_slice(
            3,
            3,
            &[
                0.040, 0.010, 0.006, 0.010, 0.090, 0.012, 0.006, 0.012, 0.025,
            ],
        )
    }

    #[test]
    fn equal_risk_contributions_have_equal_pctr() {
        let means = DVector::from_vec(vec![0.01, 0.02, 0.015]);
        let covariance = three_asset_covariance();
        let result = EqualRiskContribution::new()
            .optimize(&means, &covariance)
            .expect("ERC succeeds");
        let weights = PortfolioWeights::new(
            vec!["A".to_owned(), "B".to_owned(), "C".to_owned()],
            result.weights.clone(),
        )
        .expect("valid weights");
        let decomposition = risk_decomposition(&weights, &covariance).expect("decomposition");
        for contribution in decomposition.pctr.iter() {
            assert_relative_eq!(*contribution, 1.0 / 3.0, epsilon = 1e-9);
        }
    }

    #[test]
    fn custom_risk_budgets_are_matched() {
        let means = DVector::from_vec(vec![0.01, 0.02, 0.015]);
        let covariance = three_asset_covariance();
        let budgets = vec![0.5, 0.3, 0.2];
        let result = EqualRiskContribution::with_risk_budgets(budgets.clone())
            .optimize(&means, &covariance)
            .expect("budgeted ERC succeeds");
        let weights = PortfolioWeights::new(
            vec!["A".to_owned(), "B".to_owned(), "C".to_owned()],
            result.weights.clone(),
        )
        .expect("valid weights");
        let decomposition = risk_decomposition(&weights, &covariance).expect("decomposition");
        for (contribution, budget) in decomposition.pctr.iter().zip(budgets.iter()) {
            assert_relative_eq!(*contribution, *budget, epsilon = 1e-9);
        }
    }

    #[test]
    fn risk_parity_rejects_zero_variance_assets() {
        let means = DVector::from_vec(vec![0.01, 0.02]);
        let covariance = DMatrix::from_diagonal(&DVector::from_vec(vec![0.0, 0.09]));
        assert!(
            EqualRiskContribution::new()
                .optimize(&means, &covariance)
                .is_err()
        );
    }
}
