//! Portfolio weight optimization: constraints, the common optimizer interface,
//! and the classic allocation strategies.
//!
//! Every optimizer consumes a vector of expected returns `mu` and an `(N, N)`
//! covariance matrix `Sigma`, both in the same asset order, and returns an
//! [`OptimizationResult`] with the weight vector `w` and the portfolio
//! statistics
//!
//! ```text
//! mu_p    = w^T mu
//! sigma_p = sqrt(w^T Sigma w)
//! Sharpe  = mu_p / sigma_p
//! ```
//!
//! where the Sharpe ratio is expressed in the return units supplied to the
//! optimizer and assumes a zero risk-free rate. If `mu` and `Sigma` are
//! per-period quantities the ratio is per-period; if they are annualized it is
//! annualized. The ratio is reported as `0.0` when `sigma_p` is zero because it
//! is undefined in that case.
//!
//! All allocations satisfy the caller-supplied [`OptimizationConstraints`]:
//! per-asset lower and upper bounds plus a target net weight. The default is a
//! long-only, fully invested portfolio (`0 <= w_i <= 1`, `sum(w) = 1`). The
//! feasible set is convex, so the projections used by the iterative solvers are
//! well defined.
//!
//! | Strategy | Module | Objective |
//! | --- | --- | --- |
//! | [`EqualWeight`] | [`heuristic`] | `w_i = 1 / N` |
//! | [`InverseVolatility`] | [`heuristic`] | `w_i proportional to 1 / sigma_i` |
//! | [`GlobalMinVariance`] | [`mean_variance`] | minimize `w^T Sigma w` |
//! | [`TargetReturn`] | [`mean_variance`] | minimize `w^T Sigma w` subject to `w^T mu = target` |
//! | [`MaxSharpe`] | [`mean_variance`] | maximize `w^T mu / sqrt(w^T Sigma w)` |
//! | [`EqualRiskContribution`] | [`risk_parity`] | equalize `PCTR_i` (or custom risk budgets) |
//! | [`HierarchicalRiskParity`] | [`hrp`] | Lopez de Prado's hierarchical risk parity |
//!
//! The solvers are pure Rust and use `nalgebra` for linear algebra only; no
//! external optimization or C/Fortran library is linked. Covariance inputs are
//! validated for symmetry, finite entries, non-negative diagonal, and positive
//! semidefiniteness before any optimization runs.
//!
//! # Examples
//!
//! ```
//! use kurumi_portfolio::nalgebra::{DMatrix, DVector};
//! use kurumi_portfolio::{GlobalMinVariance, OptimizationConstraints, PortfolioOptimizer};
//!
//! # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
//! let means = DVector::from_vec(vec![0.10, 0.12, 0.08]);
//! let covariance = DMatrix::from_row_slice(
//!     3,
//!     3,
//!     &[
//!         0.040, 0.006, 0.002,
//!         0.006, 0.090, 0.004,
//!         0.002, 0.004, 0.025,
//!     ],
//! );
//! let optimizer = GlobalMinVariance::new();
//! let result = optimizer.optimize(&means, &covariance)?;
//! assert!((result.weights.sum() - 1.0).abs() < 1e-10);
//! assert!(result.volatility > 0.0);
//! # Ok(())
//! # }
//! ```

use kurumi_core::KurumiError;
use nalgebra::{DMatrix, DVector};

use crate::analytics::PortfolioWeights;

pub mod heuristic;
pub mod hrp;
pub mod mean_variance;
pub mod risk_parity;

pub use heuristic::{EqualWeight, InverseVolatility};
pub use hrp::{HierarchicalRiskParity, HrpLinkage};
pub use mean_variance::{GlobalMinVariance, MaxSharpe, TargetReturn, efficient_frontier};
pub use risk_parity::EqualRiskContribution;

/// Per-asset bounds and the target net weight for an allocation.
///
/// The feasible set is
///
/// ```text
/// { w : min_weights_i <= w_i <= max_weights_i,  sum_i w_i = target_sum }.
/// ```
///
/// An empty `min_weights` vector defaults every lower bound to `0.0` (long
/// only), and an empty `max_weights` vector defaults every upper bound to
/// `1.0`. The `Default` implementation therefore describes the conventional
/// long-only, fully invested portfolio. Vectors that are supplied must have one
/// entry per asset and are validated by [`OptimizationConstraints::validate`].
///
/// Bounds are expressed in the same dimensionless weight units as the optimizer
/// output: `0.25` is a 25% allocation and `-0.10` is a 10% short. Exact
/// feasibility (including `sum(min) <= target_sum <= sum(max)`) is checked when
/// the optimizer runs, since the number of assets is only known then.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::OptimizationConstraints;
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// // Long-only and fully invested (the default).
/// let long_only = OptimizationConstraints::default();
/// assert_eq!(long_only.target_sum, 1.0);
///
/// // At most 40% in any single asset.
/// let capped = OptimizationConstraints::new(
///     Vec::new(),
///     vec![0.4, 0.4, 0.4],
///     1.0,
/// )?;
/// capped.validate(3)?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct OptimizationConstraints {
    /// Per-asset lower bounds; empty means `0.0` for every asset.
    pub min_weights: Vec<f64>,
    /// Per-asset upper bounds; empty means `1.0` for every asset.
    pub max_weights: Vec<f64>,
    /// Required net weight `sum(w)`; conventionally `1.0`.
    pub target_sum: f64,
}

impl Default for OptimizationConstraints {
    fn default() -> Self {
        Self {
            min_weights: Vec::new(),
            max_weights: Vec::new(),
            target_sum: 1.0,
        }
    }
}

impl OptimizationConstraints {
    /// Creates constraints from explicit bounds.
    ///
    /// The constructor checks the structural requirements that do not depend on
    /// the number of assets: finite values, element-wise `min <= max` when both
    /// vectors are present, and matching lengths when both are present. The
    /// per-asset length and the feasibility of `target_sum` are checked by
    /// [`OptimizationConstraints::validate`] once the asset count is known.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when a bound is not finite,
    /// when `min_weights[i] > max_weights[i]`, when the two bounds have
    /// different lengths, or when `target_sum` is not finite.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::OptimizationConstraints;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let constraints = OptimizationConstraints::new(
    ///     vec![0.0, 0.0],
    ///     vec![0.7, 0.7],
    ///     1.0,
    /// )?;
    /// assert_eq!(constraints.max_weights, [0.7, 0.7]);
    /// assert!(OptimizationConstraints::new(vec![0.6], vec![0.4], 1.0).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        min_weights: Vec<f64>,
        max_weights: Vec<f64>,
        target_sum: f64,
    ) -> Result<Self, KurumiError> {
        if !target_sum.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "target_sum",
                reason: "must be finite".to_owned(),
            });
        }
        if !min_weights.is_empty() && !max_weights.is_empty() {
            if min_weights.len() != max_weights.len() {
                return Err(KurumiError::InvalidParameter {
                    parameter: "max_weights",
                    reason: format!(
                        "length {} does not match the {} lower bounds",
                        max_weights.len(),
                        min_weights.len()
                    ),
                });
            }
            for (index, (minimum, maximum)) in
                min_weights.iter().zip(max_weights.iter()).enumerate()
            {
                if !minimum.is_finite() || !maximum.is_finite() {
                    return Err(KurumiError::InvalidParameter {
                        parameter: "weights",
                        reason: format!("bound at index {index} must be finite"),
                    });
                }
                if minimum > maximum {
                    return Err(KurumiError::InvalidParameter {
                        parameter: "min_weights",
                        reason: format!(
                            "lower bound at index {index} ({minimum}) exceeds the upper bound ({maximum})"
                        ),
                    });
                }
            }
        } else {
            for (name, bounds) in [("min_weights", &min_weights), ("max_weights", &max_weights)] {
                for (index, bound) in bounds.iter().enumerate() {
                    if !bound.is_finite() {
                        return Err(KurumiError::InvalidParameter {
                            parameter: name,
                            reason: format!("bound at index {index} must be finite"),
                        });
                    }
                }
            }
        }
        Ok(Self {
            min_weights,
            max_weights,
            target_sum,
        })
    }

    /// Validates the constraints against an asset count.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InsufficientData`] for zero assets,
    /// [`KurumiError::InvalidParameter`] for a wrong-length or non-finite
    /// bound, `min > max`, or an infeasible target sum
    /// (`sum(min) > target_sum` or `target_sum > sum(max)`).
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::OptimizationConstraints;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let constraints = OptimizationConstraints::default();
    /// constraints.validate(4)?;
    /// assert!(constraints.validate(0).is_err());
    /// assert!(OptimizationConstraints::new(Vec::new(), vec![0.2, 0.2], 1.0)
    ///     .expect("finite bounds")
    ///     .validate(2)
    ///     .is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn validate(&self, assets: usize) -> Result<(), KurumiError> {
        if assets == 0 {
            return Err(KurumiError::InsufficientData {
                required: 1,
                available: 0,
            });
        }
        let (minimum, maximum) = self.bounds(assets)?;
        let lower_sum = minimum.iter().sum::<f64>();
        let upper_sum = maximum.iter().sum::<f64>();
        let scale = upper_sum.abs().max(lower_sum.abs()).max(1.0);
        if self.target_sum < lower_sum - 1e-12 * scale {
            return Err(KurumiError::InvalidParameter {
                parameter: "target_sum",
                reason: format!(
                    "target {0} is below the minimum feasible sum {lower_sum}",
                    self.target_sum
                ),
            });
        }
        if self.target_sum > upper_sum + 1e-12 * scale {
            return Err(KurumiError::InvalidParameter {
                parameter: "target_sum",
                reason: format!(
                    "target {0} is above the maximum feasible sum {upper_sum}",
                    self.target_sum
                ),
            });
        }
        Ok(())
    }

    /// Expands the constraints into concrete per-asset bounds.
    ///
    /// Empty vectors are expanded to `0.0` and `1.0` respectively. The result is
    /// not re-checked for feasibility; callers that need the full check use
    /// [`OptimizationConstraints::validate`].
    pub(crate) fn bounds(&self, assets: usize) -> Result<(Vec<f64>, Vec<f64>), KurumiError> {
        let minimum = if self.min_weights.is_empty() {
            vec![0.0; assets]
        } else if self.min_weights.len() == assets {
            self.min_weights.clone()
        } else {
            return Err(KurumiError::InvalidParameter {
                parameter: "min_weights",
                reason: format!(
                    "length {} does not match the {assets} assets",
                    self.min_weights.len()
                ),
            });
        };
        let maximum = if self.max_weights.is_empty() {
            vec![1.0; assets]
        } else if self.max_weights.len() == assets {
            self.max_weights.clone()
        } else {
            return Err(KurumiError::InvalidParameter {
                parameter: "max_weights",
                reason: format!(
                    "length {} does not match the {assets} assets",
                    self.max_weights.len()
                ),
            });
        };
        for (index, (lower, upper)) in minimum.iter().zip(maximum.iter()).enumerate() {
            if !lower.is_finite() || !upper.is_finite() {
                return Err(KurumiError::InvalidParameter {
                    parameter: "weights",
                    reason: format!("bound at index {index} must be finite"),
                });
            }
            if lower > upper {
                return Err(KurumiError::InvalidParameter {
                    parameter: "min_weights",
                    reason: format!(
                        "lower bound at index {index} ({lower}) exceeds the upper bound ({upper})"
                    ),
                });
            }
        }
        Ok((minimum, maximum))
    }
}

/// The output of a portfolio optimizer.
///
/// `weights[i]` is the fraction of portfolio value allocated to asset `i`, in
/// the same order as the expected-return vector and covariance matrix passed to
/// the optimizer. The statistics use the formulas in the [module
/// documentation](self); `iterations` reports how many solver iterations were
/// needed (`0` for closed-form strategies).
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{EqualWeight, PortfolioOptimizer};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let means = DVector::from_vec(vec![0.01, 0.02]);
/// let covariance = DMatrix::from_diagonal(&DVector::from_vec(vec![0.04, 0.09]));
/// let result = EqualWeight::new().optimize(&means, &covariance)?;
/// assert!((result.weights[0] - 0.5).abs() < 1e-15);
/// assert!((result.expected_return - 0.015).abs() < 1e-15);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct OptimizationResult {
    /// Optimal weights, one entry per asset.
    pub weights: DVector<f64>,
    /// Portfolio expected return `w^T mu`.
    pub expected_return: f64,
    /// Portfolio volatility `sqrt(w^T Sigma w)`.
    pub volatility: f64,
    /// Per-period Sharpe ratio `w^T mu / sqrt(w^T Sigma w)` with a zero
    /// risk-free rate; `0.0` when the volatility is zero.
    pub sharpe_ratio: f64,
    /// Number of solver iterations; `0` for closed-form strategies.
    pub iterations: usize,
}

impl OptimizationResult {
    /// Pairs the optimized weights with a symbol universe.
    ///
    /// The symbol count must equal the number of weights. See
    /// [`PortfolioWeights::new`] for the validation rules.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`PortfolioWeights::new`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
    /// use kurumi_portfolio::{EqualWeight, PortfolioOptimizer};
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let means = DVector::from_vec(vec![0.01, 0.02]);
    /// let covariance = DMatrix::from_diagonal(&DVector::from_vec(vec![0.04, 0.09]));
    /// let result = EqualWeight::new().optimize(&means, &covariance)?;
    /// let weights = result.to_weights(vec!["AAA".to_owned(), "BBB".to_owned()])?;
    /// assert_eq!(weights.symbols(), ["AAA", "BBB"]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn to_weights(&self, symbols: Vec<String>) -> Result<PortfolioWeights, KurumiError> {
        PortfolioWeights::new(symbols, self.weights.clone())
    }
}

/// The common interface implemented by every portfolio optimizer.
///
/// The trait deliberately takes only the expected-return vector and the
/// covariance matrix: strategy parameters such as target returns, risk budgets,
/// and linkage rules live on the implementing struct. Constraints live on the
/// implementing struct as well, so implementations can be constructed once and
/// reused across markets of the same size.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{InverseVolatility, PortfolioOptimizer};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let means = DVector::from_vec(vec![0.01, 0.02]);
/// let covariance = DMatrix::from_diagonal(&DVector::from_vec(vec![0.04, 0.09]));
/// let optimizer = InverseVolatility::new();
/// let result = optimizer.optimize(&means, &covariance)?;
/// assert!(result.weights[0] > result.weights[1]);
/// # Ok(())
/// # }
/// ```
pub trait PortfolioOptimizer {
    /// Computes the constrained optimal weights for the supplied moments.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the shapes disagree, a
    /// value is not finite, the covariance is not symmetric positive
    /// semidefinite, or the constraints are infeasible, and
    /// [`KurumiError::InsufficientData`] for an empty universe.
    fn optimize(
        &self,
        expected_returns: &DVector<f64>,
        covariance: &DMatrix<f64>,
    ) -> Result<OptimizationResult, KurumiError>;
}

/// Validates a moment pair and its constraints.
///
/// Returns the largest covariance eigenvalue, which bounds the Lipschitz
/// constant of the mean-variance objectives and therefore the stable gradient
/// step size.
pub(crate) fn validate_problem(
    expected_returns: &DVector<f64>,
    covariance: &DMatrix<f64>,
    constraints: &OptimizationConstraints,
) -> Result<f64, KurumiError> {
    let assets = expected_returns.len();
    if assets == 0 {
        return Err(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        });
    }
    if covariance.shape() != (assets, assets) {
        return Err(KurumiError::InvalidParameter {
            parameter: "covariance",
            reason: format!(
                "shape must be ({assets}, {assets}), got ({}, {})",
                covariance.nrows(),
                covariance.ncols()
            ),
        });
    }
    for (index, value) in expected_returns.iter().enumerate() {
        if !value.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "expected_returns",
                reason: format!("value at index {index} must be finite"),
            });
        }
    }
    for row in 0..assets {
        for column in 0..assets {
            let value = covariance
                .get((row, column))
                .copied()
                .ok_or_else(index_error)?;
            if !value.is_finite() {
                return Err(KurumiError::InvalidParameter {
                    parameter: "covariance",
                    reason: format!("entry ({row}, {column}) must be finite"),
                });
            }
            if row == column && value < 0.0 {
                return Err(KurumiError::InvalidParameter {
                    parameter: "covariance",
                    reason: format!("diagonal entry ({row}, {column}) must be non-negative"),
                });
            }
            if row < column {
                let transposed = covariance
                    .get((column, row))
                    .copied()
                    .ok_or_else(index_error)?;
                let scale = value.abs().max(transposed.abs()).max(1.0);
                if (value - transposed).abs() > 1e-12 * scale {
                    return Err(KurumiError::InvalidParameter {
                        parameter: "covariance",
                        reason: format!(
                            "matrix must be symmetric; entries ({row}, {column}) and ({column}, {row}) differ"
                        ),
                    });
                }
            }
        }
    }
    constraints.validate(assets)?;

    let eigen = nalgebra::linalg::SymmetricEigen::new(covariance.clone());
    let mut largest = 0.0_f64;
    for (index, value) in eigen.eigenvalues.iter().enumerate() {
        if !value.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "covariance",
                reason: format!("eigenvalue {index} must be finite"),
            });
        }
        largest = largest.max(*value);
    }
    let scale = largest.max(1.0);
    for value in eigen.eigenvalues.iter() {
        if *value < -1e-10 * scale {
            return Err(KurumiError::InvalidParameter {
                parameter: "covariance",
                reason: "matrix must be positive semidefinite".to_owned(),
            });
        }
    }
    Ok(largest)
}

/// Projects `values` onto the box-constrained simplex
/// `{ w : min_i <= w_i <= max_i, sum(w) = target }`.
///
/// The projection is the Euclidean nearest point: it exists and is unique
/// because the set is closed and convex. The implementation bisects the scalar
/// shift `tau` in `w_i = clamp(values_i - tau, min_i, max_i)`, which is
/// monotone in `tau`, then repairs the last floating-point digit of the sum.
pub(crate) fn project_box_simplex(
    values: &DVector<f64>,
    minimum: &[f64],
    maximum: &[f64],
    target: f64,
) -> Result<DVector<f64>, KurumiError> {
    let assets = values.len();
    if minimum.len() != assets || maximum.len() != assets {
        return Err(KurumiError::InvalidParameter {
            parameter: "bounds",
            reason: format!(
                "expected {assets} lower and upper bounds, got {} and {}",
                minimum.len(),
                maximum.len()
            ),
        });
    }
    if !target.is_finite() {
        return Err(KurumiError::InvalidParameter {
            parameter: "target",
            reason: "must be finite".to_owned(),
        });
    }
    let lower_sum = minimum.iter().sum::<f64>();
    let upper_sum = maximum.iter().sum::<f64>();
    let scale = upper_sum.abs().max(lower_sum.abs()).max(1.0);
    if target < lower_sum - 1e-12 * scale || target > upper_sum + 1e-12 * scale {
        return Err(KurumiError::InvalidParameter {
            parameter: "target",
            reason: format!(
                "target {target} is outside the feasible interval [{lower_sum}, {upper_sum}]"
            ),
        });
    }

    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    for (index, value) in values.iter().enumerate() {
        let (Some(&lower), Some(&upper)) = (minimum.get(index), maximum.get(index)) else {
            return Err(index_error());
        };
        low = low.min(value - upper);
        high = high.max(value - lower);
    }
    if !low.is_finite() || !high.is_finite() {
        return Err(KurumiError::InvalidParameter {
            parameter: "values",
            reason: "must be finite".to_owned(),
        });
    }

    for _ in 0..100 {
        let middle = 0.5 * (low + high);
        let mut total = 0.0;
        for (index, value) in values.iter().enumerate() {
            let (Some(&lower), Some(&upper)) = (minimum.get(index), maximum.get(index)) else {
                return Err(index_error());
            };
            total += (value - middle).clamp(lower, upper);
        }
        if total > target {
            low = middle;
        } else {
            high = middle;
        }
    }

    let mut weights = Vec::with_capacity(assets);
    let mut total = 0.0;
    for (index, value) in values.iter().enumerate() {
        let (Some(&lower), Some(&upper)) = (minimum.get(index), maximum.get(index)) else {
            return Err(index_error());
        };
        let weight = (value - 0.5 * (low + high)).clamp(lower, upper);
        total += weight;
        weights.push(weight);
    }
    repair_sum(&mut weights, minimum, maximum, target - total)?;

    let mut result = DVector::zeros(assets);
    for (index, weight) in weights.into_iter().enumerate() {
        if let Some(slot) = result.get_mut(index) {
            *slot = weight;
        } else {
            return Err(index_error());
        }
    }
    if result.iter().any(|value| !value.is_finite()) {
        return Err(KurumiError::NumericalOverflow {
            operation: "project_box_simplex",
        });
    }
    Ok(result)
}

/// Adds `residual` to a single coordinate that still has slack, keeping every
/// coordinate inside its box. The residual after bisection is at the level of
/// floating-point rounding, so one coordinate is always enough.
fn repair_sum(
    weights: &mut [f64],
    minimum: &[f64],
    maximum: &[f64],
    residual: f64,
) -> Result<(), KurumiError> {
    if !residual.is_finite() {
        return Err(KurumiError::NumericalOverflow {
            operation: "project_box_simplex",
        });
    }
    if residual.abs() <= f64::EPSILON {
        return Ok(());
    }
    let mut best_index = None;
    let mut best_slack = 0.0_f64;
    for (index, weight) in weights.iter().enumerate() {
        let (Some(&lower), Some(&upper)) = (minimum.get(index), maximum.get(index)) else {
            return Err(index_error());
        };
        let slack = if residual > 0.0 {
            upper - weight
        } else {
            weight - lower
        };
        if slack > best_slack {
            best_slack = slack;
            best_index = Some(index);
        }
    }
    let Some(index) = best_index else {
        return Ok(());
    };
    let adjustment = residual.clamp(-best_slack, best_slack);
    let Some(weight) = weights.get_mut(index) else {
        return Err(index_error());
    };
    *weight += adjustment;
    Ok(())
}

/// Computes the statistics of a weight vector and packages them as an
/// [`OptimizationResult`].
pub(crate) fn build_result(
    weights: &DVector<f64>,
    expected_returns: &DVector<f64>,
    covariance: &DMatrix<f64>,
    iterations: usize,
) -> Result<OptimizationResult, KurumiError> {
    let expected_return = weights.dot(expected_returns);
    let variance = weights.dot(&(covariance * weights));
    if !expected_return.is_finite() || !variance.is_finite() {
        return Err(KurumiError::NumericalOverflow {
            operation: "portfolio_statistics",
        });
    }
    if variance < 0.0 {
        let scale = weights
            .iter()
            .map(|value| value.abs())
            .sum::<f64>()
            .max(1.0);
        if variance < -1e-12 * scale {
            return Err(KurumiError::InvalidParameter {
                parameter: "covariance",
                reason: "quadratic form w^T Sigma w must be non-negative".to_owned(),
            });
        }
    }
    let volatility = variance.max(0.0).sqrt();
    let sharpe_ratio = if volatility > 0.0 {
        expected_return / volatility
    } else {
        0.0
    };
    if !volatility.is_finite() || !sharpe_ratio.is_finite() {
        return Err(KurumiError::NumericalOverflow {
            operation: "portfolio_statistics",
        });
    }
    Ok(OptimizationResult {
        weights: weights.clone(),
        expected_return,
        volatility,
        sharpe_ratio,
        iterations,
    })
}

/// Returns the vertex of the feasible set that maximizes `mu^T w`.
///
/// The target-return problem is linear in the objective at the boundary of the
/// efficient frontier, so the greedy water-filling assignment below is exact:
/// start at the lower bounds, then raise the highest-expected-return assets up
/// to their upper bounds until the target sum is reached.
pub(crate) fn max_return_weights(
    expected_returns: &DVector<f64>,
    constraints: &OptimizationConstraints,
    minimum: &[f64],
    maximum: &[f64],
) -> Result<DVector<f64>, KurumiError> {
    let assets = expected_returns.len();
    let mut weights = minimum.to_vec();
    let lower_sum = minimum.iter().sum::<f64>();
    let mut remaining = constraints.target_sum - lower_sum;
    if remaining < -1e-12 {
        return Err(KurumiError::InvalidParameter {
            parameter: "target_sum",
            reason: "target is below the lower-bound sum".to_owned(),
        });
    }
    let mut order = (0..assets).collect::<Vec<_>>();
    order.sort_by(|left_index, right_index| {
        let left_value = expected_returns
            .get(*left_index)
            .copied()
            .unwrap_or_default();
        let right_value = expected_returns
            .get(*right_index)
            .copied()
            .unwrap_or_default();
        right_value
            .partial_cmp(&left_value)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left_index.cmp(right_index))
    });
    for index in order {
        if remaining <= 1e-15 {
            break;
        }
        let (Some(&lower), Some(&upper)) = (minimum.get(index), maximum.get(index)) else {
            return Err(index_error());
        };
        let capacity = upper - lower;
        let Some(weight) = weights.get_mut(index) else {
            return Err(index_error());
        };
        let increase = remaining.min(capacity);
        *weight += increase;
        remaining -= increase;
    }
    if remaining > 1e-9 {
        return Err(KurumiError::InvalidParameter {
            parameter: "constraints",
            reason: "target sum cannot be reached within the box".to_owned(),
        });
    }
    Ok(DVector::from_vec(weights))
}

fn index_error() -> KurumiError {
    KurumiError::InvalidParameter {
        parameter: "index",
        reason: "matrix or vector shape is inconsistent".to_owned(),
    }
}
