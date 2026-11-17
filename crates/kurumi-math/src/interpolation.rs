//! One-dimensional interpolation.
//!
//! The [`Interpolator`] trait is implemented by three conventions:
//!
//! - [`LinearInterpolator`] joins adjacent knots with straight lines;
//! - [`LogLinearInterpolator`] interpolates the logarithm of the ordinates and
//!   exponentiates the result, which preserves positivity and is common for
//!   discount factors;
//! - [`CubicSplineInterpolator`] builds the natural cubic spline, a
//!   twice-continuously-differentiable interpolant with zero second derivative
//!   at both ends.
//!
//! All interpolators accept strictly increasing abscissae and reject
//! evaluation outside the knot domain instead of silently extrapolating.

use kurumi_core::KurumiError;

/// A one-dimensional interpolator over a fixed set of knots.
///
/// # Examples
///
/// ```
/// use kurumi_math::{Interpolator, LinearInterpolator};
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let interpolator = LinearInterpolator::new(vec![0.0, 1.0, 2.0], vec![1.0, 3.0, 5.0])?;
/// assert!((interpolator.interpolate(0.5)? - 2.0).abs() < 1e-12);
/// assert_eq!(interpolator.domain(), (0.0, 2.0));
/// # Ok(())
/// # }
/// ```
pub trait Interpolator {
    /// Evaluates the interpolant at `x`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `x` is not finite or lies
    /// outside the knot domain.
    fn interpolate(&self, x: f64) -> Result<f64, KurumiError>;

    /// Returns the inclusive `(minimum, maximum)` knot abscissae.
    fn domain(&self) -> (f64, f64);
}

/// Piecewise-linear interpolation.
///
/// On the interval `[x_i, x_{i+1}]`, with
/// `t = (x - x_i) / (x_{i+1} - x_i)`, the interpolant is
/// `y(x) = (1 - t) y_i + t y_{i+1}`.
///
/// # Examples
///
/// ```
/// use kurumi_math::{Interpolator, LinearInterpolator};
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let curve = LinearInterpolator::new(vec![0.0, 1.0], vec![10.0, 20.0])?;
/// assert!((curve.interpolate(0.25)? - 12.5).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct LinearInterpolator {
    xs: Vec<f64>,
    ys: Vec<f64>,
}

impl LinearInterpolator {
    /// Builds a linear interpolator from abscissae and ordinates.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InsufficientData`] for fewer than two knots, and
    /// [`KurumiError::InvalidParameter`] when the lengths differ, a value is not
    /// finite, or `xs` is not strictly increasing.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_math::LinearInterpolator;
    ///
    /// assert!(LinearInterpolator::new(vec![0.0, 1.0], vec![0.0, 1.0]).is_ok());
    /// assert!(LinearInterpolator::new(vec![0.0, 1.0], vec![0.0]).is_err());
    /// assert!(LinearInterpolator::new(vec![1.0, 1.0], vec![0.0, 1.0]).is_err());
    /// ```
    pub fn new(xs: Vec<f64>, ys: Vec<f64>) -> Result<Self, KurumiError> {
        validate_knots(&xs, &ys)?;
        Ok(Self { xs, ys })
    }

    /// Builds a linear interpolator by copying the supplied slices.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`LinearInterpolator::new`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_math::LinearInterpolator;
    ///
    /// let curve = LinearInterpolator::from_slices(&[0.0, 1.0], &[0.0, 2.0]);
    /// assert!(curve.is_ok());
    /// ```
    pub fn from_slices(xs: &[f64], ys: &[f64]) -> Result<Self, KurumiError> {
        Self::new(xs.to_vec(), ys.to_vec())
    }
}

impl Interpolator for LinearInterpolator {
    #[inline]
    fn interpolate(&self, x: f64) -> Result<f64, KurumiError> {
        let index = locate_interval(&self.xs, x)?;
        let x_left = value_at(&self.xs, index)?;
        let x_right = value_at(&self.xs, index + 1)?;
        let y_left = value_at(&self.ys, index)?;
        let y_right = value_at(&self.ys, index + 1)?;
        let width = x_right - x_left;
        let weight = (x - x_left) / width;
        Ok(y_left + weight * (y_right - y_left))
    }

    fn domain(&self) -> (f64, f64) {
        domain_of(&self.xs)
    }
}

/// Log-linear interpolation: linear in `x`, linear in `ln(y)`.
///
/// On `[x_i, x_{i+1}]`, with `t = (x - x_i) / (x_{i+1} - x_i)`, the
/// interpolant is
///
/// ```text
/// y(x) = y_i^(1 - t) * y_{i+1}^t = exp((1 - t) ln y_i + t ln y_{i+1}).
/// ```
///
/// Every ordinate must be strictly positive, which makes this convention
/// suitable for discount factors and survival probabilities.
///
/// # Examples
///
/// ```
/// use kurumi_math::{Interpolator, LogLinearInterpolator};
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let curve = LogLinearInterpolator::new(vec![0.0, 1.0], vec![1.0, 2.0])?;
/// let midpoint = curve.interpolate(0.5)?;
/// assert!((midpoint - 2.0_f64.sqrt()).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct LogLinearInterpolator {
    xs: Vec<f64>,
    ys: Vec<f64>,
}

impl LogLinearInterpolator {
    /// Builds a log-linear interpolator from abscissae and ordinates.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InsufficientData`] for fewer than two knots, and
    /// [`KurumiError::InvalidParameter`] when the lengths differ, a value is not
    /// finite, `xs` is not strictly increasing, or an ordinate is not strictly
    /// positive.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_math::LogLinearInterpolator;
    ///
    /// assert!(LogLinearInterpolator::new(vec![0.0, 1.0], vec![1.0, 0.0]).is_err());
    /// ```
    pub fn new(xs: Vec<f64>, ys: Vec<f64>) -> Result<Self, KurumiError> {
        validate_knots(&xs, &ys)?;
        for (index, y) in ys.iter().enumerate() {
            if *y <= 0.0 {
                return Err(KurumiError::InvalidParameter {
                    parameter: "ys",
                    reason: format!("ordinate at index {index} must be strictly positive"),
                });
            }
        }
        Ok(Self { xs, ys })
    }

    /// Builds a log-linear interpolator by copying the supplied slices.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`LogLinearInterpolator::new`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_math::LogLinearInterpolator;
    ///
    /// assert!(LogLinearInterpolator::from_slices(&[0.0, 1.0], &[1.0, 2.0]).is_ok());
    /// ```
    pub fn from_slices(xs: &[f64], ys: &[f64]) -> Result<Self, KurumiError> {
        Self::new(xs.to_vec(), ys.to_vec())
    }
}

impl Interpolator for LogLinearInterpolator {
    #[inline]
    fn interpolate(&self, x: f64) -> Result<f64, KurumiError> {
        let index = locate_interval(&self.xs, x)?;
        let x_left = value_at(&self.xs, index)?;
        let x_right = value_at(&self.xs, index + 1)?;
        let y_left = value_at(&self.ys, index)?;
        let y_right = value_at(&self.ys, index + 1)?;
        let width = x_right - x_left;
        let weight = (x - x_left) / width;
        let log_value = y_left.ln() * (1.0 - weight) + y_right.ln() * weight;
        let value = log_value.exp();
        if value.is_finite() {
            Ok(value)
        } else {
            Err(KurumiError::NumericalOverflow {
                operation: "log-linear interpolation",
            })
        }
    }

    fn domain(&self) -> (f64, f64) {
        domain_of(&self.xs)
    }
}

/// Natural cubic spline interpolation.
///
/// The interpolant is the piecewise cubic `S` that passes through every knot,
/// has continuous first and second derivatives at interior knots, and satisfies
/// `S''(x_0) = S''(x_n) = 0`. On `[x_i, x_{i+1}]`, with
/// `h_i = x_{i+1} - x_i` and second derivatives `M_i = S''(x_i)`,
///
/// ```text
/// S(x) = M_i (x_{i+1} - x)^3 / (6 h_i)
///      + M_{i+1} (x - x_i)^3 / (6 h_i)
///      + (y_i - M_i h_i^2 / 6) (x_{i+1} - x) / h_i
///      + (y_{i+1} - M_{i+1} h_i^2 / 6) (x - x_i) / h_i.
/// ```
///
/// The interior second derivatives solve the tridiagonal system
///
/// ```text
/// h_{i-1} M_{i-1} + 2 (h_{i-1} + h_i) M_i + h_i M_{i+1}
///     = 6 ((y_{i+1} - y_i) / h_i - (y_i - y_{i-1}) / h_{i-1}),
/// ```
///
/// with `M_0 = M_n = 0`, solved by the Thomas algorithm.
///
/// # Examples
///
/// ```
/// use kurumi_math::{CubicSplineInterpolator, Interpolator};
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let spline = CubicSplineInterpolator::new(vec![0.0, 1.0, 2.0], vec![0.0, 1.0, 0.0])?;
/// assert!((spline.interpolate(0.5)? - 0.6875).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct CubicSplineInterpolator {
    xs: Vec<f64>,
    ys: Vec<f64>,
    second_derivatives: Vec<f64>,
}

impl CubicSplineInterpolator {
    /// Builds a natural cubic spline from abscissae and ordinates.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InsufficientData`] for fewer than two knots,
    /// [`KurumiError::InvalidParameter`] when the lengths differ, a value is not
    /// finite, or `xs` is not strictly increasing, and
    /// [`KurumiError::NumericalOverflow`] when the tridiagonal solve is not
    /// numerically well defined.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_math::CubicSplineInterpolator;
    ///
    /// assert!(CubicSplineInterpolator::new(vec![0.0, 1.0, 2.0], vec![0.0, 1.0, 0.0]).is_ok());
    /// assert!(CubicSplineInterpolator::new(vec![0.0], vec![0.0]).is_err());
    /// ```
    pub fn new(xs: Vec<f64>, ys: Vec<f64>) -> Result<Self, KurumiError> {
        validate_knots(&xs, &ys)?;
        let second_derivatives = natural_second_derivatives(&xs, &ys)?;
        Ok(Self {
            xs,
            ys,
            second_derivatives,
        })
    }

    /// Builds a natural cubic spline by copying the supplied slices.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`CubicSplineInterpolator::new`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_math::CubicSplineInterpolator;
    ///
    /// assert!(CubicSplineInterpolator::from_slices(&[0.0, 1.0], &[0.0, 1.0]).is_ok());
    /// ```
    pub fn from_slices(xs: &[f64], ys: &[f64]) -> Result<Self, KurumiError> {
        Self::new(xs.to_vec(), ys.to_vec())
    }
}

impl Interpolator for CubicSplineInterpolator {
    #[inline]
    fn interpolate(&self, x: f64) -> Result<f64, KurumiError> {
        let index = locate_interval(&self.xs, x)?;
        let x_left = value_at(&self.xs, index)?;
        let x_right = value_at(&self.xs, index + 1)?;
        let y_left = value_at(&self.ys, index)?;
        let y_right = value_at(&self.ys, index + 1)?;
        let m_left = value_at(&self.second_derivatives, index)?;
        let m_right = value_at(&self.second_derivatives, index + 1)?;

        let width = x_right - x_left;
        let from_right = x_right - x;
        let from_left = x - x_left;
        let value = m_left * from_right.powi(3) / (6.0 * width)
            + m_right * from_left.powi(3) / (6.0 * width)
            + (y_left - m_left * width * width / 6.0) * from_right / width
            + (y_right - m_right * width * width / 6.0) * from_left / width;
        if value.is_finite() {
            Ok(value)
        } else {
            Err(KurumiError::NumericalOverflow {
                operation: "cubic spline interpolation",
            })
        }
    }

    fn domain(&self) -> (f64, f64) {
        domain_of(&self.xs)
    }
}

fn validate_knots(xs: &[f64], ys: &[f64]) -> Result<(), KurumiError> {
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
    for (index, x) in xs.iter().enumerate() {
        if !x.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "xs",
                reason: format!("abscissa at index {index} must be finite"),
            });
        }
    }
    for (index, y) in ys.iter().enumerate() {
        if !y.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "ys",
                reason: format!("ordinate at index {index} must be finite"),
            });
        }
    }
    for window in xs.windows(2) {
        if let [left, right] = window {
            if left >= right {
                return Err(KurumiError::InvalidParameter {
                    parameter: "xs",
                    reason: "abscissae must be strictly increasing".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn domain_of(xs: &[f64]) -> (f64, f64) {
    match (xs.first(), xs.last()) {
        (Some(first), Some(last)) => (*first, *last),
        _ => (0.0, 0.0),
    }
}

fn locate_interval(xs: &[f64], x: f64) -> Result<usize, KurumiError> {
    if !x.is_finite() {
        return Err(KurumiError::InvalidParameter {
            parameter: "x",
            reason: "must be finite".to_owned(),
        });
    }
    let (first, last) = domain_of(xs);
    if xs.len() < 2 {
        return Err(KurumiError::InsufficientData {
            required: 2,
            available: xs.len(),
        });
    }
    if x < first || x > last {
        return Err(KurumiError::InvalidParameter {
            parameter: "x",
            reason: format!("value {x} is outside the interpolation domain [{first}, {last}]"),
        });
    }
    let position = xs.partition_point(|knot| *knot <= x);
    let upper = position.min(xs.len() - 1);
    Ok(upper.saturating_sub(1))
}

fn value_at(values: &[f64], index: usize) -> Result<f64, KurumiError> {
    values
        .get(index)
        .copied()
        .ok_or_else(|| KurumiError::InvalidParameter {
            parameter: "knot index",
            reason: format!("index {index} is out of bounds"),
        })
}

fn natural_second_derivatives(xs: &[f64], ys: &[f64]) -> Result<Vec<f64>, KurumiError> {
    let count = xs.len();
    if count < 2 {
        return Err(KurumiError::InsufficientData {
            required: 2,
            available: count,
        });
    }
    if count == 2 {
        return Ok(vec![0.0, 0.0]);
    }

    let mut lower = vec![0.0; count];
    let mut diagonal = vec![0.0; count];
    let mut upper = vec![0.0; count];
    let mut rhs = vec![0.0; count];

    for index in 1..count - 1 {
        let x_previous = value_at(xs, index - 1)?;
        let x_current = value_at(xs, index)?;
        let x_next = value_at(xs, index + 1)?;
        let y_previous = value_at(ys, index - 1)?;
        let y_current = value_at(ys, index)?;
        let y_next = value_at(ys, index + 1)?;

        let h_previous = x_current - x_previous;
        let h_next = x_next - x_current;

        set_at(&mut lower, index, h_previous)?;
        set_at(&mut diagonal, index, 2.0 * (h_previous + h_next))?;
        set_at(&mut upper, index, h_next)?;
        set_at(
            &mut rhs,
            index,
            6.0 * ((y_next - y_current) / h_next - (y_current - y_previous) / h_previous),
        )?;
    }

    set_at(&mut diagonal, 0, 1.0)?;
    set_at(&mut rhs, 0, 0.0)?;
    set_at(&mut diagonal, count - 1, 1.0)?;
    set_at(&mut rhs, count - 1, 0.0)?;

    solve_tridiagonal(&lower, &diagonal, &upper, &rhs)
}

fn solve_tridiagonal(
    lower: &[f64],
    diagonal: &[f64],
    upper: &[f64],
    rhs: &[f64],
) -> Result<Vec<f64>, KurumiError> {
    let count = diagonal.len();
    if count == 0 || lower.len() != count || upper.len() != count || rhs.len() != count {
        return Err(KurumiError::InvalidParameter {
            parameter: "spline system",
            reason: "tridiagonal arrays must have the same non-zero length".to_owned(),
        });
    }

    let mut modified_upper = vec![0.0; count];
    let mut modified_rhs = vec![0.0; count];

    let first_pivot = value_at(diagonal, 0)?;
    if first_pivot == 0.0 {
        return Err(KurumiError::NumericalOverflow {
            operation: "cubic spline tridiagonal solve",
        });
    }
    set_at(&mut modified_upper, 0, value_at(upper, 0)? / first_pivot)?;
    set_at(&mut modified_rhs, 0, value_at(rhs, 0)? / first_pivot)?;

    for index in 1..count {
        let pivot = value_at(diagonal, index)?
            - value_at(lower, index)? * value_at(&modified_upper, index - 1)?;
        if pivot == 0.0 || !pivot.is_finite() {
            return Err(KurumiError::NumericalOverflow {
                operation: "cubic spline tridiagonal solve",
            });
        }
        let upper_value = value_at(upper, index)? / pivot;
        let rhs_value = (value_at(rhs, index)?
            - value_at(lower, index)? * value_at(&modified_rhs, index - 1)?)
            / pivot;
        set_at(&mut modified_upper, index, upper_value)?;
        set_at(&mut modified_rhs, index, rhs_value)?;
    }

    let mut solution = vec![0.0; count];
    let last = count - 1;
    set_at(&mut solution, last, value_at(&modified_rhs, last)?)?;
    for index in (0..last).rev() {
        let value = value_at(&modified_rhs, index)?
            - value_at(&modified_upper, index)? * value_at(&solution, index + 1)?;
        set_at(&mut solution, index, value)?;
    }
    Ok(solution)
}

fn set_at(values: &mut [f64], index: usize, value: f64) -> Result<(), KurumiError> {
    let slot = values
        .get_mut(index)
        .ok_or_else(|| KurumiError::InvalidParameter {
            parameter: "spline system",
            reason: format!("index {index} is out of bounds"),
        })?;
    *slot = value;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn linear() -> LinearInterpolator {
        LinearInterpolator::new(vec![0.0, 1.0, 2.0], vec![1.0, 3.0, 5.0])
            .expect("valid linear knots")
    }

    #[test]
    fn linear_interpolation_matches_the_line_through_the_knots() {
        let curve = linear();
        assert_relative_eq!(
            curve.interpolate(0.0).expect("inside"),
            1.0,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            curve.interpolate(0.5).expect("inside"),
            2.0,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            curve.interpolate(1.5).expect("inside"),
            4.0,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            curve.interpolate(2.0).expect("inside"),
            5.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn linear_interpolation_uses_the_correct_segment() {
        let curve = LinearInterpolator::new(vec![0.0, 1.0, 3.0], vec![0.0, 10.0, 14.0])
            .expect("valid knots");
        assert_relative_eq!(
            curve.interpolate(2.0).expect("inside"),
            12.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn log_linear_interpolation_is_geometric_on_the_ordinate() {
        let curve = LogLinearInterpolator::new(vec![0.0, 1.0, 2.0], vec![1.0, 2.0, 4.0])
            .expect("valid log-linear knots");
        assert_relative_eq!(
            curve.interpolate(0.5).expect("inside"),
            2.0_f64.sqrt(),
            epsilon = 1e-15
        );
        assert_relative_eq!(
            curve.interpolate(1.5).expect("inside"),
            8.0_f64.sqrt(),
            epsilon = 1e-15
        );
        assert_relative_eq!(
            curve.interpolate(0.0).expect("inside"),
            1.0,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            curve.interpolate(2.0).expect("inside"),
            4.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn log_linear_interpolation_rejects_non_positive_ordinates() {
        assert!(LogLinearInterpolator::new(vec![0.0, 1.0], vec![1.0, 0.0]).is_err());
        assert!(LogLinearInterpolator::new(vec![0.0, 1.0], vec![-1.0, 1.0]).is_err());
    }

    #[test]
    fn cubic_spline_reproduces_the_textbook_natural_spline() {
        // Burden & Faires style example: knots (0,0), (1,1), (2,0). The natural
        // spline has M_1 = -3 and value 0.6875 at x = 0.5 and x = 1.5.
        let spline = CubicSplineInterpolator::new(vec![0.0, 1.0, 2.0], vec![0.0, 1.0, 0.0])
            .expect("valid spline knots");
        assert_relative_eq!(
            spline.interpolate(0.5).expect("inside"),
            0.6875,
            epsilon = 1e-12
        );
        assert_relative_eq!(
            spline.interpolate(1.5).expect("inside"),
            0.6875,
            epsilon = 1e-12
        );
    }

    #[test]
    fn cubic_spline_passes_through_every_knot() {
        let spline = CubicSplineInterpolator::new(
            vec![0.0, 0.5, 1.5, 3.0, 4.0],
            vec![1.0, 2.0, 0.0, -1.0, 3.0],
        )
        .expect("valid spline knots");
        for (x, y) in [(0.0, 1.0), (0.5, 2.0), (1.5, 0.0), (3.0, -1.0), (4.0, 3.0)] {
            assert_relative_eq!(spline.interpolate(x).expect("knot"), y, epsilon = 1e-12);
        }
    }

    #[test]
    fn cubic_spline_reproduces_linear_data_exactly() {
        let spline =
            CubicSplineInterpolator::new(vec![0.0, 1.0, 2.0, 3.0], vec![1.0, 3.0, 5.0, 7.0])
                .expect("valid spline knots");
        for x in [0.25, 1.25, 2.75] {
            assert_relative_eq!(
                spline.interpolate(x).expect("inside"),
                2.0 * x + 1.0,
                epsilon = 1e-12
            );
        }
    }

    #[test]
    fn cubic_spline_handles_two_knots_as_a_line() {
        let spline = CubicSplineInterpolator::new(vec![1.0, 3.0], vec![2.0, 6.0])
            .expect("valid spline knots");
        assert_relative_eq!(
            spline.interpolate(2.0).expect("inside"),
            4.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn interpolation_rejects_coordinates_outside_the_domain() {
        let curve = linear();
        assert!(curve.interpolate(-0.001).is_err());
        assert!(curve.interpolate(2.001).is_err());
        assert!(curve.interpolate(f64::NAN).is_err());
        assert!(curve.interpolate(f64::INFINITY).is_err());
    }

    #[test]
    fn construction_validates_knots() {
        assert!(LinearInterpolator::new(vec![0.0], vec![1.0]).is_err());
        assert!(LinearInterpolator::new(vec![0.0, 1.0], vec![1.0]).is_err());
        assert!(LinearInterpolator::new(vec![0.0, 0.0], vec![1.0, 2.0]).is_err());
        assert!(LinearInterpolator::new(vec![1.0, 0.0], vec![1.0, 2.0]).is_err());
        assert!(LinearInterpolator::new(vec![0.0, f64::NAN], vec![1.0, 2.0]).is_err());
        assert!(LinearInterpolator::new(vec![0.0, 1.0], vec![1.0, f64::INFINITY]).is_err());
        assert!(CubicSplineInterpolator::new(vec![0.0, 1.0], vec![1.0]).is_err());
    }

    #[test]
    fn interpolators_are_usable_as_trait_objects() {
        let interpolators: [Box<dyn Interpolator>; 2] = [
            Box::new(
                LinearInterpolator::new(vec![0.0, 1.0, 2.0], vec![0.0, 1.0, 2.0])
                    .expect("valid linear knots"),
            ),
            Box::new(
                CubicSplineInterpolator::new(vec![0.0, 1.0, 2.0], vec![0.0, 1.0, 2.0])
                    .expect("valid spline knots"),
            ),
        ];
        for interpolator in interpolators {
            assert_relative_eq!(
                interpolator.interpolate(0.5).expect("inside"),
                0.5,
                epsilon = 1e-12
            );
            assert_eq!(interpolator.domain(), (0.0, 2.0));
        }
    }
}
