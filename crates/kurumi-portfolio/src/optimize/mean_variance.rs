//! Mean-variance optimization: global minimum variance, target-return, and
//! maximum-Sharpe portfolios, plus the efficient frontier.
//!
//! All three solvers work on the same convex feasible set
//! (`min_i <= w_i <= max_i`, `sum_i w_i = target_sum`) and use a projected
//! gradient method with a backtracking line search. The Euclidean projection
//! onto the box-constrained simplex is implemented by
//! `project_box_simplex` and the step size is
//! bounded by the largest covariance eigenvalue, so the method is stable for
//! singular and nearly collinear covariance matrices.
//!
//! For a risk-aversion parameter `lambda >= 0` the solver minimizes the strictly
//! convex quadratic
//!
//! ```text
//! f_lambda(w) = w^T Sigma w - lambda * mu^T w
//! ```
//!
//! over the feasible set. The target-return problem
//!
//! ```text
//! minimize w^T Sigma w   subject to   w^T mu = target
//! ```
//!
//! is solved by bisecting `lambda`: `w^T mu` is monotone non-decreasing in
//! `lambda`, ranging from the global-minimum-variance return at `lambda = 0` to
//! the maximum-return vertex as `lambda -> infinity`. The maximum-Sharpe
//! (tangency) portfolio is found by scanning and then refining the risk-aversion
//! parameter, which traces the same efficient frontier.
//!
//! # References
//!
//! - H. M. Markowitz, "Portfolio Selection", *Journal of Finance* 7(1), 1952.
//! - W. F. Sharpe, "Capital Asset Prices: A Theory of Market Equilibrium under
//!   Conditions of Risk", *Journal of Finance* 19(3), 1964.
//! - A. Beck and M. Teboulle, "A Fast Iterative Shrinkage-Thresholding
//!   Algorithm for Linear Inverse Problems", *SIAM Journal on Imaging Sciences*
//!   2(1), 2009.
//! - Y. Nesterov, *Introductory Lectures on Convex Optimization*, Kluwer, 2004.

use kurumi_core::KurumiError;
use nalgebra::{DMatrix, DVector};

use super::{
    OptimizationConstraints, OptimizationResult, PortfolioOptimizer, build_result,
    max_return_weights, project_box_simplex, validate_problem,
};

const SOLVER_TOLERANCE: f64 = 1e-12;
const SOLVER_MAX_ITERATIONS: usize = 20_000;
const TARGET_TOLERANCE: f64 = 1e-10;

/// The global minimum-variance (GMV) portfolio.
///
/// The optimizer minimizes `w^T Sigma w` over the constrained simplex. This is
/// the left-most point of the efficient frontier: it ignores expected returns
/// entirely and is therefore the allocation with the lowest achievable
/// volatility under the constraints.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{GlobalMinVariance, PortfolioOptimizer};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let means = DVector::from_vec(vec![0.01, 0.02]);
/// let covariance = DMatrix::from_row_slice(2, 2, &[0.04, 0.006, 0.006, 0.09]);
/// let result = GlobalMinVariance::new().optimize(&means, &covariance)?;
/// assert!(result.volatility <= 0.2);
/// assert!((result.weights.sum() - 1.0).abs() < 1e-10);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct GlobalMinVariance {
    /// Per-asset bounds and the target net weight.
    pub constraints: OptimizationConstraints,
}

impl GlobalMinVariance {
    /// Creates a GMV optimizer with the default long-only, fully invested
    /// constraints.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::GlobalMinVariance;
    /// assert_eq!(GlobalMinVariance::new().constraints.target_sum, 1.0);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            constraints: OptimizationConstraints::default(),
        }
    }

    /// Creates a GMV optimizer with explicit constraints.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::{GlobalMinVariance, OptimizationConstraints};
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let constraints = OptimizationConstraints::new(Vec::new(), vec![0.6, 0.6], 1.0)?;
    /// let optimizer = GlobalMinVariance::with_constraints(constraints);
    /// assert_eq!(optimizer.constraints.max_weights, [0.6, 0.6]);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_constraints(constraints: OptimizationConstraints) -> Self {
        Self { constraints }
    }
}

impl Default for GlobalMinVariance {
    fn default() -> Self {
        Self::new()
    }
}

impl PortfolioOptimizer for GlobalMinVariance {
    fn optimize(
        &self,
        expected_returns: &DVector<f64>,
        covariance: &DMatrix<f64>,
    ) -> Result<OptimizationResult, KurumiError> {
        let problem = MeanVarianceProblem::new(expected_returns, covariance, &self.constraints)?;
        build_result(
            &problem.gmv,
            expected_returns,
            covariance,
            problem.gmv_iterations,
        )
    }
}

/// The minimum-variance portfolio that achieves a required expected return.
///
/// The optimizer minimizes `w^T Sigma w` subject to `w^T mu = target_return`
/// and the constrained simplex. `target_return` must lie between the
/// global-minimum-variance return and the maximum achievable return; see
/// [`efficient_frontier`] for generating a whole curve.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{PortfolioOptimizer, TargetReturn};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let means = DVector::from_vec(vec![0.05, 0.10]);
/// let covariance = DMatrix::from_row_slice(2, 2, &[0.04, 0.0, 0.0, 0.09]);
/// let optimizer = TargetReturn::new(0.08);
/// let result = optimizer.optimize(&means, &covariance)?;
/// assert!((result.expected_return - 0.08).abs() < 1e-8);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct TargetReturn {
    /// Per-asset bounds and the target net weight.
    pub constraints: OptimizationConstraints,
    /// Required portfolio expected return `w^T mu`, in the same units as `mu`.
    pub target_return: f64,
}

impl TargetReturn {
    /// Creates a target-return optimizer with the default constraints.
    ///
    /// The target is validated for finiteness and feasibility when
    /// [`PortfolioOptimizer::optimize`] runs, because feasibility depends on the
    /// moments.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::TargetReturn;
    /// assert_eq!(TargetReturn::new(0.08).target_return, 0.08);
    /// ```
    #[must_use]
    pub fn new(target_return: f64) -> Self {
        Self {
            constraints: OptimizationConstraints::default(),
            target_return,
        }
    }

    /// Creates a target-return optimizer with explicit constraints.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::{OptimizationConstraints, TargetReturn};
    ///
    /// let optimizer = TargetReturn::with_constraints(
    ///     OptimizationConstraints::default(),
    ///     0.08,
    /// );
    /// assert_eq!(optimizer.target_return, 0.08);
    /// ```
    #[must_use]
    pub fn with_constraints(constraints: OptimizationConstraints, target_return: f64) -> Self {
        Self {
            constraints,
            target_return,
        }
    }
}

impl PortfolioOptimizer for TargetReturn {
    fn optimize(
        &self,
        expected_returns: &DVector<f64>,
        covariance: &DMatrix<f64>,
    ) -> Result<OptimizationResult, KurumiError> {
        if !self.target_return.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "target_return",
                reason: "must be finite".to_owned(),
            });
        }
        let problem = MeanVarianceProblem::new(expected_returns, covariance, &self.constraints)?;
        let initial = problem.equal_weight_start();
        problem.solve_target(self.target_return, &initial)
    }
}

/// The maximum-Sharpe (tangency) portfolio.
///
/// The optimizer maximizes `w^T mu / sqrt(w^T Sigma w)` over the constrained
/// simplex with a zero risk-free rate. The portfolio lies on the efficient
/// frontier where a ray from the origin is tangent to it. The implementation
/// scans the risk-aversion parameter of the mean-variance objective and refines
/// the best point with a ternary search in log-parameter space, which is the
/// risk-aversion-sweep alternative to the Charnes-Cooper transformation.
///
/// When every asset has zero or negative expected return the tangency slope is
/// still well defined; when the expected-return vector is exactly zero every
/// portfolio has a Sharpe ratio of `0.0` and the GMV portfolio is returned.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{MaxSharpe, PortfolioOptimizer};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let means = DVector::from_vec(vec![0.10, 0.15]);
/// let covariance = DMatrix::from_diagonal(&DVector::from_vec(vec![0.04, 0.09]));
/// let result = MaxSharpe::new().optimize(&means, &covariance)?;
/// // The tangency portfolio is proportional to Sigma^-1 mu = (2.5, 1.6667).
/// assert!((result.weights[0] - 0.6).abs() < 1e-5);
/// assert!((result.weights.sum() - 1.0).abs() < 1e-10);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MaxSharpe {
    /// Per-asset bounds and the target net weight.
    pub constraints: OptimizationConstraints,
}

impl MaxSharpe {
    /// Creates a maximum-Sharpe optimizer with the default constraints.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::MaxSharpe;
    /// assert_eq!(MaxSharpe::new().constraints.target_sum, 1.0);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            constraints: OptimizationConstraints::default(),
        }
    }

    /// Creates a maximum-Sharpe optimizer with explicit constraints.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::{MaxSharpe, OptimizationConstraints};
    ///
    /// let optimizer = MaxSharpe::with_constraints(OptimizationConstraints::default());
    /// assert!(optimizer.constraints.min_weights.is_empty());
    /// ```
    #[must_use]
    pub fn with_constraints(constraints: OptimizationConstraints) -> Self {
        Self { constraints }
    }
}

impl Default for MaxSharpe {
    fn default() -> Self {
        Self::new()
    }
}

impl PortfolioOptimizer for MaxSharpe {
    fn optimize(
        &self,
        expected_returns: &DVector<f64>,
        covariance: &DMatrix<f64>,
    ) -> Result<OptimizationResult, KurumiError> {
        let problem = MeanVarianceProblem::new(expected_returns, covariance, &self.constraints)?;
        problem.solve_max_sharpe()
    }
}

/// Generates `points` portfolios along the constrained efficient frontier.
///
/// The targets are evenly spaced in expected return between the
/// global-minimum-variance return and the maximum achievable return, and each
/// point is the minimum-variance portfolio at that target. The first point is
/// the GMV portfolio and the last point is the maximum-return vertex; both are
/// included. Portfolios are returned in increasing target-return order, so the
/// expected returns and volatilities are non-decreasing along the vector.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when `points < 2` or the moments
/// and constraints are invalid.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{OptimizationConstraints, efficient_frontier};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let means = DVector::from_vec(vec![0.05, 0.10, 0.15]);
/// let covariance = DMatrix::from_diagonal(&DVector::from_vec(vec![0.04, 0.09, 0.16]));
/// let frontier = efficient_frontier(
///     &OptimizationConstraints::default(),
///     &means,
///     &covariance,
///     5,
/// )?;
/// assert_eq!(frontier.len(), 5);
/// assert!(frontier[4].expected_return >= frontier[0].expected_return);
/// # Ok(())
/// # }
/// ```
pub fn efficient_frontier(
    constraints: &OptimizationConstraints,
    expected_returns: &DVector<f64>,
    covariance: &DMatrix<f64>,
    points: usize,
) -> Result<Vec<OptimizationResult>, KurumiError> {
    if points < 2 {
        return Err(KurumiError::InvalidParameter {
            parameter: "points",
            reason: "must be at least 2".to_owned(),
        });
    }
    let problem = MeanVarianceProblem::new(expected_returns, covariance, constraints)?;
    let mut results = Vec::with_capacity(points);
    let mut warm_start = problem.gmv.clone();
    for index in 0..points {
        let fraction = index as f64 / (points - 1) as f64;
        let target = problem.min_return + fraction * (problem.max_return - problem.min_return);
        let result = problem.solve_target(target, &warm_start)?;
        warm_start = result.weights.clone();
        results.push(result);
    }
    Ok(results)
}

/// The shared mean-variance problem: validated moments, constraints, and the
/// two boundary portfolios of the efficient frontier.
struct MeanVarianceProblem<'a> {
    expected_returns: &'a DVector<f64>,
    covariance: &'a DMatrix<f64>,
    constraints: &'a OptimizationConstraints,
    minimum: Vec<f64>,
    maximum: Vec<f64>,
    lipschitz: f64,
    gmv: DVector<f64>,
    gmv_iterations: usize,
    min_return: f64,
    max_return: f64,
    max_return_portfolio: DVector<f64>,
}

impl<'a> MeanVarianceProblem<'a> {
    fn new(
        expected_returns: &'a DVector<f64>,
        covariance: &'a DMatrix<f64>,
        constraints: &'a OptimizationConstraints,
    ) -> Result<Self, KurumiError> {
        let lipschitz = validate_problem(expected_returns, covariance, constraints)?;
        let assets = expected_returns.len();
        let (minimum, maximum) = constraints.bounds(assets)?;
        let initial = DVector::from_element(assets, constraints.target_sum / assets as f64);
        let (gmv, gmv_iterations) = projected_gradient(
            expected_returns,
            covariance,
            &minimum,
            &maximum,
            constraints.target_sum,
            0.0,
            &initial,
            lipschitz,
            SOLVER_TOLERANCE,
            SOLVER_MAX_ITERATIONS,
        )?;
        let max_return_portfolio =
            max_return_weights(expected_returns, constraints, &minimum, &maximum)?;
        let min_return = expected_returns.dot(&gmv);
        let max_return = expected_returns.dot(&max_return_portfolio);
        Ok(Self {
            expected_returns,
            covariance,
            constraints,
            minimum,
            maximum,
            lipschitz,
            gmv,
            gmv_iterations,
            min_return,
            max_return,
            max_return_portfolio,
        })
    }

    fn equal_weight_start(&self) -> DVector<f64> {
        let assets = self.expected_returns.len();
        DVector::from_element(assets, self.constraints.target_sum / assets as f64)
    }

    fn solve_risk_aversion(
        &self,
        lambda: f64,
        initial: &DVector<f64>,
    ) -> Result<(DVector<f64>, usize), KurumiError> {
        projected_gradient(
            self.expected_returns,
            self.covariance,
            &self.minimum,
            &self.maximum,
            self.constraints.target_sum,
            lambda,
            initial,
            self.lipschitz,
            SOLVER_TOLERANCE,
            SOLVER_MAX_ITERATIONS,
        )
    }

    fn solve_target(
        &self,
        target_return: f64,
        initial: &DVector<f64>,
    ) -> Result<OptimizationResult, KurumiError> {
        let scale = self.min_return.abs().max(self.max_return.abs()).max(1.0);
        let feasibility_tolerance = 1e-10 * scale;
        if target_return < self.min_return - feasibility_tolerance {
            return Err(KurumiError::InvalidParameter {
                parameter: "target_return",
                reason: format!(
                    "target {target_return} is below the minimum-variance return {}",
                    self.min_return
                ),
            });
        }
        if target_return > self.max_return + feasibility_tolerance {
            return Err(KurumiError::InvalidParameter {
                parameter: "target_return",
                reason: format!(
                    "target {target_return} is above the maximum achievable return {}",
                    self.max_return
                ),
            });
        }
        if target_return <= self.min_return + 1e-12 * scale {
            return build_result(
                &self.gmv,
                self.expected_returns,
                self.covariance,
                self.gmv_iterations,
            );
        }
        if target_return >= self.max_return - 1e-12 * scale {
            return build_result(
                &self.max_return_portfolio,
                self.expected_returns,
                self.covariance,
                self.gmv_iterations,
            );
        }

        let tolerance = TARGET_TOLERANCE * scale.max(target_return.abs());
        let mut iterations = 0_usize;
        let mut warm_start = initial.clone();

        let lambda_scale = if self.lipschitz > 1e-15 {
            self.expected_returns.norm() / self.lipschitz
        } else {
            1.0
        };
        let mut lambda_low = 0.0_f64;
        let mut lambda_high = lambda_scale.max(1e-6);
        let mut best = self.gmv.clone();
        let mut best_gap = (self.min_return - target_return).abs();
        for _ in 0..64 {
            let (candidate, used) = self.solve_risk_aversion(lambda_high, &warm_start)?;
            iterations += used;
            warm_start = candidate.clone();
            let candidate_return = self.expected_returns.dot(&candidate);
            let gap = (candidate_return - target_return).abs();
            if gap < best_gap {
                best_gap = gap;
                best = candidate;
            }
            if candidate_return >= target_return {
                break;
            }
            lambda_high *= 4.0;
        }

        for _ in 0..64 {
            let middle = 0.5 * (lambda_low + lambda_high);
            let (candidate, used) = self.solve_risk_aversion(middle, &warm_start)?;
            iterations += used;
            warm_start = candidate.clone();
            let candidate_return = self.expected_returns.dot(&candidate);
            let gap = (candidate_return - target_return).abs();
            if gap < best_gap {
                best_gap = gap;
                best = candidate.clone();
            }
            if candidate_return < target_return {
                lambda_low = middle;
            } else {
                lambda_high = middle;
            }
            if best_gap <= tolerance {
                break;
            }
        }

        if best_gap > tolerance * 100.0 {
            return Err(KurumiError::ConvergenceFailed {
                method: "target_return_bisection",
                iterations,
            });
        }
        build_result(&best, self.expected_returns, self.covariance, iterations)
    }

    fn solve_max_sharpe(&self) -> Result<OptimizationResult, KurumiError> {
        let mut best = build_result(
            &self.gmv,
            self.expected_returns,
            self.covariance,
            self.gmv_iterations,
        )?;
        let expected_norm = self.expected_returns.norm();
        if expected_norm <= f64::EPSILON {
            return Ok(best);
        }
        let average_variance = {
            let mut sum = 0.0;
            for index in 0..self.expected_returns.len() {
                sum += self
                    .covariance
                    .get((index, index))
                    .copied()
                    .unwrap_or_default();
            }
            (sum / self.expected_returns.len() as f64).max(1e-12)
        };
        let lambda_scale = expected_norm / average_variance;
        let mut iterations = self.gmv_iterations;
        let mut warm_start = self.gmv.clone();
        let mut best_lambda = None;

        // A half-decade grid from 1e-6 to 1e6 times the natural risk-aversion
        // scale brackets the tangency parameter for every well-scaled problem.
        let grid_points = 25_usize;
        for index in 0..=grid_points {
            let exponent = -6.0 + 12.0 * index as f64 / grid_points as f64;
            let lambda = lambda_scale * 10.0_f64.powf(exponent);
            let (candidate, used) = self.solve_risk_aversion(lambda, &warm_start)?;
            iterations += used;
            warm_start = candidate.clone();
            let sharpe = sharpe_ratio(&candidate, self.expected_returns, self.covariance)?;
            if sharpe > best.sharpe_ratio {
                best = build_result(
                    &candidate,
                    self.expected_returns,
                    self.covariance,
                    iterations,
                )?;
                best_lambda = Some(lambda);
            }
        }

        let Some(mut best_lambda) = best_lambda else {
            best.iterations = iterations;
            return Ok(best);
        };
        let mut low = (best_lambda / 10.0_f64.powf(0.5)).max(lambda_scale * 1e-9);
        let mut high = (best_lambda * 10.0_f64.powf(0.5)).min(lambda_scale * 1e9);
        if high <= low {
            low = best_lambda * 0.5;
            high = best_lambda * 2.0;
        }
        warm_start = best.weights.clone();
        let mut low_log = low.ln();
        let mut high_log = high.ln();
        for _ in 0..60 {
            if high_log <= low_log {
                break;
            }
            let first = low_log + (high_log - low_log) / 3.0;
            let second = high_log - (high_log - low_log) / 3.0;
            let (first_portfolio, first_used) =
                self.solve_risk_aversion(first.exp(), &warm_start)?;
            iterations += first_used;
            let first_sharpe =
                sharpe_ratio(&first_portfolio, self.expected_returns, self.covariance)?;
            if first_sharpe > best.sharpe_ratio {
                best = build_result(
                    &first_portfolio,
                    self.expected_returns,
                    self.covariance,
                    iterations,
                )?;
                best_lambda = first.exp();
            }
            let (second_portfolio, second_used) =
                self.solve_risk_aversion(second.exp(), &first_portfolio)?;
            iterations += second_used;
            let second_sharpe =
                sharpe_ratio(&second_portfolio, self.expected_returns, self.covariance)?;
            if second_sharpe > best.sharpe_ratio {
                best = build_result(
                    &second_portfolio,
                    self.expected_returns,
                    self.covariance,
                    iterations,
                )?;
                best_lambda = second.exp();
            }
            warm_start = second_portfolio;
            if first_sharpe < second_sharpe {
                low_log = first;
            } else {
                high_log = second;
            }
        }
        let (refined, used) = self.solve_risk_aversion(best_lambda, &warm_start)?;
        iterations += used;
        let refined_sharpe = sharpe_ratio(&refined, self.expected_returns, self.covariance)?;
        if refined_sharpe > best.sharpe_ratio {
            best = build_result(&refined, self.expected_returns, self.covariance, iterations)?;
        }
        best.iterations = iterations;
        Ok(best)
    }
}

fn sharpe_ratio(
    weights: &DVector<f64>,
    expected_returns: &DVector<f64>,
    covariance: &DMatrix<f64>,
) -> Result<f64, KurumiError> {
    let result = build_result(weights, expected_returns, covariance, 0)?;
    Ok(result.sharpe_ratio)
}

/// The projected gradient loop shared by the mean-variance solvers.
///
/// Each iteration takes a gradient step on
/// `f(w) = w^T Sigma w - lambda * mu^T w`, projects the candidate back onto the
/// box-constrained simplex, and halves the step until the Armijo-style descent
/// condition holds. The base step `1 / (2 * lambda_max(Sigma))` guarantees the
/// descent lemma for a backtracking-free step; the line search protects against
/// finite-precision overshoot.
///
/// The argument list is long because the loop is shared by three public
/// optimizers; bundling the immutable problem data in a borrow struct would not
/// reduce the state it has to mutate.
#[allow(clippy::too_many_arguments)]
fn projected_gradient(
    expected_returns: &DVector<f64>,
    covariance: &DMatrix<f64>,
    minimum: &[f64],
    maximum: &[f64],
    target_sum: f64,
    risk_aversion: f64,
    initial: &DVector<f64>,
    lipschitz: f64,
    tolerance: f64,
    max_iterations: usize,
) -> Result<(DVector<f64>, usize), KurumiError> {
    let mut weights = project_box_simplex(initial, minimum, maximum, target_sum)?;
    if lipschitz <= 1e-15 {
        return Ok((weights, 0));
    }
    let base_step = 1.0 / (2.0 * lipschitz);
    let mut iterations = 0;
    for index in 0..max_iterations {
        iterations = index + 1;
        let gradient = (covariance * &weights) * 2.0 - expected_returns * risk_aversion;
        let gradient_norm_squared = gradient.norm_squared();
        if !gradient_norm_squared.is_finite() {
            return Err(KurumiError::NumericalOverflow {
                operation: "projected_gradient",
            });
        }
        if gradient_norm_squared <= f64::EPSILON * f64::EPSILON {
            break;
        }
        let current = objective(&weights, expected_returns, covariance, risk_aversion)?;
        let mut step = base_step;
        let mut candidate =
            project_box_simplex(&(&weights - &gradient * step), minimum, maximum, target_sum)?;
        // The base step already satisfies the descent lemma, so accepting any
        // non-increase (up to rounding) is sufficient; halving protects against
        // a numerically larger trial value. Comparing against the full-gradient
        // norm would stall near the optimum, where projection cancels the
        // component of the gradient normal to the active constraints.
        for _ in 0..60 {
            let trial = objective(&candidate, expected_returns, covariance, risk_aversion)?;
            if trial <= current + 1e-15 * (1.0 + current.abs()) {
                break;
            }
            step *= 0.5;
            candidate =
                project_box_simplex(&(&weights - &gradient * step), minimum, maximum, target_sum)?;
        }
        let difference = &candidate - &weights;
        let difference_norm = difference.norm();
        weights = candidate;
        if difference_norm <= tolerance * (1.0 + weights.norm()) {
            break;
        }
    }
    Ok((weights, iterations))
}

fn objective(
    weights: &DVector<f64>,
    expected_returns: &DVector<f64>,
    covariance: &DMatrix<f64>,
    risk_aversion: f64,
) -> Result<f64, KurumiError> {
    let value =
        weights.dot(&(covariance * weights)) - risk_aversion * expected_returns.dot(weights);
    if value.is_finite() {
        Ok(value)
    } else {
        Err(KurumiError::NumericalOverflow {
            operation: "mean_variance_objective",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn two_asset_problem() -> (DVector<f64>, DMatrix<f64>) {
        (
            DVector::from_vec(vec![0.10, 0.15]),
            DMatrix::from_row_slice(2, 2, &[0.04, 0.006, 0.006, 0.09]),
        )
    }

    #[test]
    fn global_minimum_variance_matches_the_closed_form_solution() {
        let (means, covariance) = two_asset_problem();
        let result = GlobalMinVariance::new()
            .optimize(&means, &covariance)
            .expect("GMV succeeds");
        let variance_one = 0.04;
        let variance_two = 0.09;
        let covariance_value = 0.006;
        let denominator = variance_one + variance_two - 2.0 * covariance_value;
        let expected_first = (variance_two - covariance_value) / denominator;
        assert_relative_eq!(result.weights[0], expected_first, epsilon = 1e-6);
        assert_relative_eq!(result.weights[1], 1.0 - expected_first, epsilon = 1e-6);
    }

    #[test]
    fn global_minimum_variance_volatility_beats_single_assets_and_equal_weight() {
        let (means, covariance) = two_asset_problem();
        let result = GlobalMinVariance::new()
            .optimize(&means, &covariance)
            .expect("GMV succeeds");
        let single_one = covariance.get((0, 0)).copied().unwrap_or_default().sqrt();
        let single_two = covariance.get((1, 1)).copied().unwrap_or_default().sqrt();
        let equal = DVector::from_vec(vec![0.5, 0.5]);
        let equal_variance = equal.dot(&(covariance * &equal));
        assert!(result.volatility <= single_one + 1e-12);
        assert!(result.volatility <= single_two + 1e-12);
        assert!(result.volatility <= equal_variance.sqrt() + 1e-12);
    }

    #[test]
    fn target_return_solver_hits_the_requested_return() {
        let (means, covariance) = two_asset_problem();
        let optimizer = TargetReturn::new(0.12);
        let result = optimizer
            .optimize(&means, &covariance)
            .expect("target succeeds");
        assert_relative_eq!(result.expected_return, 0.12, epsilon = 1e-8);
        assert_relative_eq!(result.weights.sum(), 1.0, epsilon = 1e-8);
        assert!(result.volatility > 0.0);
    }

    #[test]
    fn target_return_outside_the_frontier_is_rejected() {
        let (means, covariance) = two_asset_problem();
        assert!(
            TargetReturn::new(0.5)
                .optimize(&means, &covariance)
                .is_err()
        );
        assert!(
            TargetReturn::new(-0.5)
                .optimize(&means, &covariance)
                .is_err()
        );
    }

    #[test]
    fn max_sharpe_recovers_the_analytic_tangency_portfolio() {
        let means = DVector::from_vec(vec![0.10, 0.15]);
        let covariance = DMatrix::from_diagonal(&DVector::from_vec(vec![0.04, 0.09]));
        let result = MaxSharpe::new()
            .optimize(&means, &covariance)
            .expect("max sharpe succeeds");
        // Sigma^-1 mu = (2.5, 1.6667), normalized to (0.6, 0.4).
        assert_relative_eq!(result.weights[0], 0.6, epsilon = 2e-4);
        assert_relative_eq!(result.weights[1], 0.4, epsilon = 2e-4);
        assert_relative_eq!(result.weights.sum(), 1.0, epsilon = 1e-9);
    }

    #[test]
    fn max_sharpe_dominates_the_other_strategies() {
        let (means, covariance) = two_asset_problem();
        let maximum = MaxSharpe::new()
            .optimize(&means, &covariance)
            .expect("max sharpe succeeds");
        let gmv = GlobalMinVariance::new()
            .optimize(&means, &covariance)
            .expect("GMV succeeds");
        assert!(maximum.sharpe_ratio >= gmv.sharpe_ratio - 1e-9);
    }

    #[test]
    fn efficient_frontier_is_monotone() {
        let means = DVector::from_vec(vec![0.05, 0.10, 0.15]);
        let covariance = DMatrix::from_diagonal(&DVector::from_vec(vec![0.04, 0.09, 0.16]));
        let frontier =
            efficient_frontier(&OptimizationConstraints::default(), &means, &covariance, 7)
                .expect("frontier succeeds");
        assert_eq!(frontier.len(), 7);
        for pair in frontier.windows(2) {
            let (Some(lower), Some(higher)) = (pair.first(), pair.get(1)) else {
                panic!("frontier windows have two entries");
            };
            assert!(higher.expected_return >= lower.expected_return - 1e-10);
            assert!(higher.volatility >= lower.volatility - 1e-9);
        }
    }

    #[test]
    fn constraints_limit_each_weight() {
        let (means, covariance) = two_asset_problem();
        let constraints =
            OptimizationConstraints::new(Vec::new(), vec![0.5, 0.5], 1.0).expect("valid bounds");
        let result = GlobalMinVariance::with_constraints(constraints)
            .optimize(&means, &covariance)
            .expect("constrained GMV succeeds");
        assert_relative_eq!(result.weights[0], 0.5, epsilon = 1e-9);
        assert_relative_eq!(result.weights[1], 0.5, epsilon = 1e-9);
    }
}
