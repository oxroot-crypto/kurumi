//! Hierarchical Risk Parity (HRP) as described by Marcos Lopez de Prado.
//!
//! HRP builds a diversified allocation in three steps:
//!
//! 1. **Tree clustering.** Correlations are converted to the distance
//!    `d_ij = sqrt(0.5 * (1 - rho_ij))` and assets are agglomerated with
//!    single or Ward linkage.
//! 2. **Quasi-diagonalization.** The dendrogram leaves are ordered by a
//!    depth-first traversal so that similar assets are adjacent.
//! 3. **Recursive bisection.** The ordered list is split in half repeatedly.
//!    Each side receives a fraction of the parent weight proportional to the
//!    inverse-variance variance of the side:
//!
//!    ```text
//!    alpha       = 1 - var_left / (var_left + var_right)
//!    w_left      = alpha * w_parent
//!    w_right     = (1 - alpha) * w_parent
//!    ```
//!
//!    where the variance of a cluster is that of its inverse-variance
//!    portfolio.
//!
//! The algorithm never inverts a covariance matrix, so it remains stable when
//! assets are highly collinear. The final weights honor the caller's
//! [`OptimizationConstraints`] through the same projection used by the other
//! optimizers; with the default long-only, fully invested constraints the HRP
//! allocation is returned unchanged.
//!
//! # References
//!
//! - M. Lopez de Prado, "Building Diversified Portfolios that Outperform Out of
//!   Sample", *Journal of Portfolio Management* 42(4), 2016.
//! - M. Lopez de Prado, *Machine Learning for Asset Managers*, Cambridge
//!   University Press, 2020.

use std::collections::HashMap;

use kurumi_core::KurumiError;
use nalgebra::{DMatrix, DVector};

use super::{
    OptimizationConstraints, OptimizationResult, PortfolioOptimizer, build_result,
    project_box_simplex, validate_problem,
};

/// The agglomerative linkage rule used to build the HRP dendrogram.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::HrpLinkage;
///
/// let default = HrpLinkage::default();
/// assert_eq!(default, HrpLinkage::Single);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HrpLinkage {
    /// Single linkage: cluster distance is the minimum pairwise distance.
    #[default]
    Single,
    /// Ward linkage: minimum-variance merge, using the Lance-Williams update.
    Ward,
}

/// The hierarchical risk-parity portfolio.
///
/// The optimizer requires strictly positive variance for every asset, because
/// the recursive-bisection variance uses an inverse-variance portfolio. Invalid
/// or non-finite correlations are clamped to `[-1, 1]`, so near-collinear input
/// matrices produce finite, valid weights rather than `NaN`.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{HierarchicalRiskParity, HrpLinkage, PortfolioOptimizer};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let means = DVector::from_vec(vec![0.01, 0.02, 0.03, 0.04]);
/// let covariance = DMatrix::from_row_slice(
///     4,
///     4,
///     &[
///         0.040, 0.030, 0.004, 0.006,
///         0.030, 0.090, 0.006, 0.008,
///         0.004, 0.006, 0.025, 0.010,
///         0.006, 0.008, 0.010, 0.050,
///     ],
/// );
/// let optimizer = HierarchicalRiskParity::with_linkage(HrpLinkage::Ward);
/// let result = optimizer.optimize(&means, &covariance)?;
/// assert!((result.weights.sum() - 1.0).abs() < 1e-9);
/// assert!(result.weights.iter().all(|weight| *weight >= 0.0));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct HierarchicalRiskParity {
    /// Per-asset bounds and the target net weight.
    pub constraints: OptimizationConstraints,
    /// Linkage rule for the agglomerative clustering.
    pub linkage: HrpLinkage,
}

impl HierarchicalRiskParity {
    /// Creates an HRP optimizer with single linkage and the default
    /// constraints.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::{HierarchicalRiskParity, HrpLinkage};
    /// assert_eq!(
    ///     HierarchicalRiskParity::new().linkage,
    ///     HrpLinkage::Single
    /// );
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            constraints: OptimizationConstraints::default(),
            linkage: HrpLinkage::Single,
        }
    }

    /// Creates an HRP optimizer with an explicit linkage rule.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::{HierarchicalRiskParity, HrpLinkage};
    /// assert_eq!(
    ///     HierarchicalRiskParity::with_linkage(HrpLinkage::Ward).linkage,
    ///     HrpLinkage::Ward
    /// );
    /// ```
    #[must_use]
    pub fn with_linkage(linkage: HrpLinkage) -> Self {
        Self {
            constraints: OptimizationConstraints::default(),
            linkage,
        }
    }

    /// Creates an HRP optimizer with explicit constraints and linkage.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::{HierarchicalRiskParity, HrpLinkage, OptimizationConstraints};
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let constraints = OptimizationConstraints::new(Vec::new(), vec![0.6, 0.6], 1.0)?;
    /// let optimizer =
    ///     HierarchicalRiskParity::with_constraints(constraints, HrpLinkage::Single);
    /// assert_eq!(optimizer.constraints.max_weights, [0.6, 0.6]);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_constraints(constraints: OptimizationConstraints, linkage: HrpLinkage) -> Self {
        Self {
            constraints,
            linkage,
        }
    }
}

impl Default for HierarchicalRiskParity {
    fn default() -> Self {
        Self::new()
    }
}

impl PortfolioOptimizer for HierarchicalRiskParity {
    fn optimize(
        &self,
        expected_returns: &DVector<f64>,
        covariance: &DMatrix<f64>,
    ) -> Result<OptimizationResult, KurumiError> {
        let assets = expected_returns.len();
        let _ = validate_problem(expected_returns, covariance, &self.constraints)?;
        let (minimum, maximum) = self.constraints.bounds(assets)?;

        if assets == 1 {
            let raw = DVector::from_element(1, self.constraints.target_sum);
            let weights =
                project_box_simplex(&raw, &minimum, &maximum, self.constraints.target_sum)?;
            return build_result(&weights, expected_returns, covariance, 0);
        }

        let (variances, distance) = correlation_distance(covariance)?;
        let order = hierarchical_order(&distance, self.linkage)?;
        let scales = recursive_bisection(&order, covariance, &variances)?;
        let mut raw = vec![0.0; assets];
        for (position, asset) in order.iter().enumerate() {
            let (Some(&scale), Some(slot)) = (scales.get(position), raw.get_mut(*asset)) else {
                return Err(index_error());
            };
            *slot = scale;
        }
        let raw = DVector::from_vec(raw);
        let weights = project_box_simplex(&raw, &minimum, &maximum, self.constraints.target_sum)?;
        build_result(&weights, expected_returns, covariance, 0)
    }
}

/// Converts a covariance matrix into per-asset variances and the correlation
/// distance matrix.
fn correlation_distance(
    covariance: &DMatrix<f64>,
) -> Result<(Vec<f64>, DMatrix<f64>), KurumiError> {
    let assets = covariance.nrows();
    let mut variances = Vec::with_capacity(assets);
    for index in 0..assets {
        let variance = covariance
            .get((index, index))
            .copied()
            .ok_or_else(index_error)?;
        if variance <= 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "covariance",
                reason: format!("asset {index} has non-positive variance, so HRP is undefined"),
            });
        }
        variances.push(variance);
    }
    let mut data = vec![0.0; assets * assets];
    for row in 0..assets {
        for column in 0..assets {
            let Some(&variance_row) = variances.get(row) else {
                return Err(index_error());
            };
            let Some(&variance_column) = variances.get(column) else {
                return Err(index_error());
            };
            let value = if row == column {
                0.0
            } else {
                let covariance_value = covariance
                    .get((row, column))
                    .copied()
                    .ok_or_else(index_error)?;
                let correlation = (covariance_value
                    / (variance_row.sqrt() * variance_column.sqrt()))
                .clamp(-1.0, 1.0);
                (0.5 * (1.0 - correlation)).max(0.0).sqrt()
            };
            if !value.is_finite() {
                return Err(KurumiError::NumericalOverflow {
                    operation: "hrp_correlation_distance",
                });
            }
            let Some(slot) = data.get_mut(row * assets + column) else {
                return Err(index_error());
            };
            *slot = value;
        }
    }
    Ok((variances, DMatrix::from_row_slice(assets, assets, &data)))
}

/// Builds the dendrogram with agglomerative clustering and returns the leaf
/// order produced by a depth-first traversal.
fn hierarchical_order(
    distance: &DMatrix<f64>,
    linkage: HrpLinkage,
) -> Result<Vec<usize>, KurumiError> {
    let assets = distance.nrows();
    let mut members: Vec<Vec<usize>> = (0..assets).map(|index| vec![index]).collect();
    let mut left_child: Vec<Option<usize>> = vec![None; assets];
    let mut right_child: Vec<Option<usize>> = vec![None; assets];
    let mut active: Vec<usize> = (0..assets).collect();
    let mut distances: HashMap<(usize, usize), f64> = HashMap::new();
    for row in 0..assets {
        for column in (row + 1)..assets {
            let value = distance
                .get((row, column))
                .copied()
                .ok_or_else(index_error)?;
            distances.insert((row, column), value);
        }
    }

    while active.len() > 1 {
        let mut best: Option<(usize, usize, f64)> = None;
        for first_position in 0..active.len() {
            for second_position in (first_position + 1)..active.len() {
                let (Some(&first), Some(&second)) =
                    (active.get(first_position), active.get(second_position))
                else {
                    return Err(index_error());
                };
                let Some(&value) = distances.get(&ordered_pair(first, second)) else {
                    return Err(index_error());
                };
                let replace = match best {
                    None => true,
                    Some((best_first, best_second, best_value)) => {
                        value < best_value
                            || (value == best_value && (first, second) < (best_first, best_second))
                    }
                };
                if replace {
                    best = Some((first_position, second_position, value));
                }
            }
        }
        let Some((first_position, second_position, merge_distance)) = best else {
            return Err(index_error());
        };
        let (Some(&first), Some(&second)) =
            (active.get(first_position), active.get(second_position))
        else {
            return Err(index_error());
        };
        let (Some(first_members), Some(second_members)) = (members.get(first), members.get(second))
        else {
            return Err(index_error());
        };
        let first_size = first_members.len() as f64;
        let second_size = second_members.len() as f64;
        let mut combined = first_members.clone();
        combined.extend_from_slice(second_members);
        let merged = members.len();
        members.push(combined);
        left_child.push(Some(first));
        right_child.push(Some(second));

        let mut updated: Vec<((usize, usize), f64)> = Vec::new();
        for &other in &active {
            if other == first || other == second {
                continue;
            }
            let Some(&first_distance) = distances.get(&ordered_pair(first, other)) else {
                return Err(index_error());
            };
            let Some(&second_distance) = distances.get(&ordered_pair(second, other)) else {
                return Err(index_error());
            };
            let value = match linkage {
                HrpLinkage::Single => first_distance.min(second_distance),
                HrpLinkage::Ward => {
                    let other_size = members
                        .get(other)
                        .map(|members| members.len() as f64)
                        .ok_or_else(index_error)?;
                    let denominator = first_size + second_size + other_size;
                    if denominator <= 0.0 {
                        return Err(index_error());
                    }
                    let numerator = (first_size + other_size) * first_distance * first_distance
                        + (second_size + other_size) * second_distance * second_distance
                        - other_size * merge_distance * merge_distance;
                    (numerator / denominator).max(0.0).sqrt()
                }
            };
            if !value.is_finite() {
                return Err(KurumiError::NumericalOverflow {
                    operation: "hrp_clustering",
                });
            }
            updated.push((ordered_pair(merged, other), value));
        }
        for (key, value) in updated {
            distances.insert(key, value);
        }
        active.remove(second_position);
        active.remove(first_position);
        active.push(merged);
    }

    let Some(&root) = active.first() else {
        return Err(index_error());
    };
    let mut order = Vec::with_capacity(assets);
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node < assets {
            order.push(node);
            continue;
        }
        let (Some(&left), Some(&right)) = (left_child.get(node), right_child.get(node)) else {
            return Err(index_error());
        };
        let (Some(left), Some(right)) = (left, right) else {
            return Err(index_error());
        };
        stack.push(right);
        stack.push(left);
    }
    if order.len() != assets {
        return Err(index_error());
    }
    Ok(order)
}

/// Assigns weights by recursively bisecting the ordered leaf list.
fn recursive_bisection(
    order: &[usize],
    covariance: &DMatrix<f64>,
    variances: &[f64],
) -> Result<Vec<f64>, KurumiError> {
    let assets = order.len();
    let mut scales = vec![1.0; assets];
    let mut stack = vec![(0_usize, assets)];
    while let Some((start, end)) = stack.pop() {
        if end <= start + 1 {
            continue;
        }
        let middle = start + (end - start) / 2;
        let (Some(left_members), Some(right_members)) =
            (order.get(start..middle), order.get(middle..end))
        else {
            return Err(index_error());
        };
        let left_variance = cluster_variance(covariance, left_members, variances)?;
        let right_variance = cluster_variance(covariance, right_members, variances)?;
        let total = left_variance + right_variance;
        let alpha = if total > 0.0 {
            (1.0 - left_variance / total).clamp(0.0, 1.0)
        } else {
            0.5
        };
        let Some(left_scales) = scales.get_mut(start..middle) else {
            return Err(index_error());
        };
        for scale in left_scales.iter_mut() {
            *scale *= alpha;
        }
        let Some(right_scales) = scales.get_mut(middle..end) else {
            return Err(index_error());
        };
        for scale in right_scales.iter_mut() {
            *scale *= 1.0 - alpha;
        }
        stack.push((start, middle));
        stack.push((middle, end));
    }
    Ok(scales)
}

/// Variance of the equal-inverse-variance portfolio of a cluster.
fn cluster_variance(
    covariance: &DMatrix<f64>,
    members: &[usize],
    variances: &[f64],
) -> Result<f64, KurumiError> {
    let mut inverse_total = 0.0;
    let mut inverse_variances = Vec::with_capacity(members.len());
    for &asset in members {
        let Some(&variance) = variances.get(asset) else {
            return Err(index_error());
        };
        let inverse = 1.0 / variance;
        inverse_total += inverse;
        inverse_variances.push(inverse);
    }
    if !inverse_total.is_finite() || inverse_total <= 0.0 {
        return Err(KurumiError::NumericalOverflow {
            operation: "hrp_cluster_variance",
        });
    }
    let weights = inverse_variances
        .iter()
        .map(|inverse| inverse / inverse_total)
        .collect::<Vec<_>>();
    let mut value = 0.0;
    for (first_position, &first_asset) in members.iter().enumerate() {
        let Some(&first_weight) = weights.get(first_position) else {
            return Err(index_error());
        };
        for (second_position, &second_asset) in members.iter().enumerate() {
            let Some(&second_weight) = weights.get(second_position) else {
                return Err(index_error());
            };
            let entry = covariance
                .get((first_asset, second_asset))
                .copied()
                .ok_or_else(index_error)?;
            value += first_weight * second_weight * entry;
        }
    }
    if !value.is_finite() {
        return Err(KurumiError::NumericalOverflow {
            operation: "hrp_cluster_variance",
        });
    }
    Ok(value.max(0.0))
}

fn ordered_pair(first: usize, second: usize) -> (usize, usize) {
    if first < second {
        (first, second)
    } else {
        (second, first)
    }
}

fn index_error() -> KurumiError {
    KurumiError::InvalidParameter {
        parameter: "index",
        reason: "matrix or vector shape is inconsistent".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn four_asset_problem() -> (DVector<f64>, DMatrix<f64>) {
        (
            DVector::from_vec(vec![0.01, 0.02, 0.03, 0.04]),
            DMatrix::from_row_slice(
                4,
                4,
                &[
                    0.040, 0.030, 0.004, 0.006, 0.030, 0.090, 0.006, 0.008, 0.004, 0.006, 0.025,
                    0.010, 0.006, 0.008, 0.010, 0.050,
                ],
            ),
        )
    }

    #[test]
    fn hrp_weights_are_long_only_and_fully_invested() {
        let (means, covariance) = four_asset_problem();
        for linkage in [HrpLinkage::Single, HrpLinkage::Ward] {
            let result = HierarchicalRiskParity::with_linkage(linkage)
                .optimize(&means, &covariance)
                .expect("HRP succeeds");
            assert_relative_eq!(result.weights.sum(), 1.0, epsilon = 1e-9);
            assert!(result.weights.iter().all(|weight| weight.is_finite()));
            assert!(result.weights.iter().all(|weight| *weight >= 0.0));
            assert!(result.volatility > 0.0);
        }
    }

    #[test]
    fn hrp_is_stable_for_nearly_collinear_assets() {
        let means = DVector::from_vec(vec![0.01, 0.02, 0.015]);
        let volatility_one = 0.20_f64;
        let volatility_two = 0.30_f64;
        let volatility_three = 0.25_f64;
        let correlation = 0.999_f64;
        let covariance = DMatrix::from_row_slice(
            3,
            3,
            &[
                volatility_one * volatility_one,
                correlation * volatility_one * volatility_two,
                correlation * volatility_one * volatility_three,
                correlation * volatility_one * volatility_two,
                volatility_two * volatility_two,
                correlation * volatility_two * volatility_three,
                correlation * volatility_one * volatility_three,
                correlation * volatility_two * volatility_three,
                volatility_three * volatility_three,
            ],
        );
        let result = HierarchicalRiskParity::new()
            .optimize(&means, &covariance)
            .expect("collinear HRP succeeds");
        assert_relative_eq!(result.weights.sum(), 1.0, epsilon = 1e-9);
        assert!(result.weights.iter().all(|weight| weight.is_finite()));
        assert!(result.weights.iter().all(|weight| *weight >= 0.0));
    }

    #[test]
    fn hrp_respects_a_single_asset_constraint() {
        let means = DVector::from_vec(vec![0.01, 0.02]);
        let covariance = DMatrix::from_row_slice(2, 2, &[0.04, 0.006, 0.006, 0.09]);
        let constraints =
            OptimizationConstraints::new(Vec::new(), vec![0.4, 1.0], 1.0).expect("valid bounds");
        let result = HierarchicalRiskParity::with_constraints(constraints, HrpLinkage::Single)
            .optimize(&means, &covariance)
            .expect("constrained HRP succeeds");
        assert!(result.weights[0] <= 0.4 + 1e-12);
        assert_relative_eq!(result.weights.sum(), 1.0, epsilon = 1e-9);
    }
}
