//! Root-finding algorithms for scalar functions.
//!
//! Two methods are provided:
//!
//! - [`brent`] is a bracketing method. Given an interval whose endpoints have
//!   opposite signs, it combines bisection, secant, and inverse quadratic
//!   interpolation and converges superlinearly while remaining safeguarded.
//! - [`newton_raphson`] is an open method that uses the function and its first
//!   derivative. It converges quadratically near a simple root but needs a good
//!   starting point.
//!
//! Both methods are configured with a [`RootFinderConfig`] carrying an iteration
//! budget and a convergence tolerance, and both reject non-finite inputs and
//! function values at the public boundary.

use kurumi_core::KurumiError;

/// Configuration shared by the root-finding algorithms.
///
/// # Examples
///
/// ```
/// use kurumi_math::RootFinderConfig;
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let config = RootFinderConfig::new(64, 1e-12)?;
/// assert_eq!(config.max_iterations, 64);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct RootFinderConfig {
    /// Maximum number of iterations before [`KurumiError::ConvergenceFailed`]
    /// is returned. Must be greater than zero.
    pub max_iterations: usize,
    /// Convergence tolerance. For [`brent`] it is the absolute width of the
    /// final bracket; for [`newton_raphson`] it bounds the relative step
    /// `|x_next - x| / (1 + |x|)`. Must be finite and strictly positive.
    pub tolerance: f64,
}

impl RootFinderConfig {
    /// Creates a validated solver configuration.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `max_iterations` is zero
    /// or `tolerance` is not finite and strictly positive.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_math::RootFinderConfig;
    ///
    /// assert!(RootFinderConfig::new(100, 1e-10).is_ok());
    /// assert!(RootFinderConfig::new(0, 1e-10).is_err());
    /// assert!(RootFinderConfig::new(100, 0.0).is_err());
    /// ```
    pub fn new(max_iterations: usize, tolerance: f64) -> Result<Self, KurumiError> {
        if max_iterations == 0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "max_iterations",
                reason: "must be greater than zero".to_owned(),
            });
        }
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "tolerance",
                reason: "must be finite and strictly positive".to_owned(),
            });
        }
        Ok(Self {
            max_iterations,
            tolerance,
        })
    }
}

impl Default for RootFinderConfig {
    /// Returns a configuration with 100 iterations and a tolerance of `1e-10`.
    fn default() -> Self {
        Self {
            max_iterations: 100,
            tolerance: 1e-10,
        }
    }
}

/// Finds a root of `function` in the bracket `[lower, upper]` with Brent's
/// method.
///
/// The method maintains a shrinking bracket and at each step tries inverse
/// quadratic interpolation or a secant step, falling back to bisection whenever
/// the trial point would leave the bracket or converge too slowly. The returned
/// value is the midpoint estimate `b` of the final bracket and satisfies
/// `|b - a| <= config.tolerance` unless the bracket could not be narrowed within
/// the iteration budget.
///
/// # Errors
///
/// - [`KurumiError::InvalidParameter`] when the bounds are not finite, `lower`
///   is not less than `upper`, the endpoint values do not straddle zero, or the
///   function returns a non-finite value.
/// - [`KurumiError::ConvergenceFailed`] when the iteration budget is exhausted.
///
/// # Examples
///
/// ```
/// use kurumi_math::{brent, RootFinderConfig};
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let config = RootFinderConfig::new(100, 1e-12)?;
/// let root = brent(|x| x * x * x - 2.0 * x - 5.0, 2.0, 3.0, config)?;
/// assert!((root - 2.094_551_481_542_326_5).abs() < 1e-9);
/// # Ok(())
/// # }
/// ```
pub fn brent<F>(
    function: F,
    lower: f64,
    upper: f64,
    config: RootFinderConfig,
) -> Result<f64, KurumiError>
where
    F: Fn(f64) -> f64,
{
    ensure_finite(lower, "lower bracket")?;
    ensure_finite(upper, "upper bracket")?;
    if lower >= upper {
        return Err(KurumiError::InvalidParameter {
            parameter: "bracket",
            reason: "lower bound must be strictly less than upper bound".to_owned(),
        });
    }

    let mut a = lower;
    let mut b = upper;
    let mut fa = evaluate(&function, a)?;
    let mut fb = evaluate(&function, b)?;
    if fa == 0.0 {
        return Ok(a);
    }
    if fb == 0.0 {
        return Ok(b);
    }
    if fa.signum() == fb.signum() {
        return Err(KurumiError::InvalidParameter {
            parameter: "bracket",
            reason: "endpoint function values must have opposite signs".to_owned(),
        });
    }

    // Keep `b` as the endpoint with the smaller function magnitude.
    if fa.abs() < fb.abs() {
        core::mem::swap(&mut a, &mut b);
        core::mem::swap(&mut fa, &mut fb);
    }

    let mut c = a;
    let mut fc = fa;
    let mut d = c;
    let mut used_bisection = true;

    for _ in 0..config.max_iterations {
        if fb == 0.0 || (b - a).abs() <= config.tolerance {
            return Ok(b);
        }

        let mut trial = if fa != fc && fb != fc {
            // Inverse quadratic interpolation through (a, fa), (b, fb), (c, fc).
            a * fb * fc / ((fa - fb) * (fa - fc))
                + b * fa * fc / ((fb - fa) * (fb - fc))
                + c * fa * fb / ((fc - fa) * (fc - fb))
        } else {
            // Secant step through (a, fa) and (b, fb).
            b - fb * (b - a) / (fb - fa)
        };

        let rejects_trial = (trial - (3.0 * a + b) / 4.0) * (trial - b) > 0.0
            || (used_bisection && (trial - b).abs() >= (b - c).abs() / 2.0)
            || (!used_bisection && (trial - b).abs() >= (c - d).abs() / 2.0)
            || (used_bisection && (b - c).abs() < config.tolerance)
            || (!used_bisection && (c - d).abs() < config.tolerance);
        if rejects_trial {
            trial = (a + b) / 2.0;
            used_bisection = true;
        } else {
            used_bisection = false;
        }

        let ftrial = evaluate(&function, trial)?;
        d = c;
        c = b;
        fc = fb;
        if fa * ftrial < 0.0 {
            b = trial;
            fb = ftrial;
        } else {
            a = trial;
            fa = ftrial;
        }
        if fa.abs() < fb.abs() {
            core::mem::swap(&mut a, &mut b);
            core::mem::swap(&mut fa, &mut fb);
        }
    }

    Err(KurumiError::ConvergenceFailed {
        method: "brent",
        iterations: config.max_iterations,
    })
}

/// Finds a root of `function` starting from `initial_guess` with the
/// Newton-Raphson method.
///
/// Each iteration takes the step
/// `x_next = x - f(x) / f'(x)`. Convergence is declared when
/// `|x_next - x| <= tolerance * (1 + |x|)`.
///
/// # Errors
///
/// - [`KurumiError::InvalidParameter`] when the initial guess or a function or
///   derivative value is not finite.
/// - [`KurumiError::NumericalOverflow`] when a Newton step or iterate is not
///   finite.
/// - [`KurumiError::ConvergenceFailed`] when the derivative vanishes or the
///   iteration budget is exhausted.
///
/// # Examples
///
/// ```
/// use kurumi_math::{newton_raphson, RootFinderConfig};
///
/// # fn main() -> Result<(), kurumi_math::KurumiError> {
/// let config = RootFinderConfig::new(100, 1e-12)?;
/// let root = newton_raphson(|x| x * x - 2.0, |x| 2.0 * x, 1.0, config)?;
/// assert!((root - std::f64::consts::SQRT_2).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn newton_raphson<F, DF>(
    function: F,
    derivative: DF,
    initial_guess: f64,
    config: RootFinderConfig,
) -> Result<f64, KurumiError>
where
    F: Fn(f64) -> f64,
    DF: Fn(f64) -> f64,
{
    ensure_finite(initial_guess, "initial guess")?;
    let mut x = initial_guess;

    for _ in 0..config.max_iterations {
        let fx = evaluate(&function, x)?;
        if fx == 0.0 {
            return Ok(x);
        }
        let dfx = evaluate(&derivative, x)?;
        if dfx == 0.0 {
            return Err(KurumiError::ConvergenceFailed {
                method: "newton_raphson",
                iterations: config.max_iterations,
            });
        }

        let step = fx / dfx;
        if !step.is_finite() {
            return Err(KurumiError::NumericalOverflow {
                operation: "newton_raphson step",
            });
        }
        let next = x - step;
        if !next.is_finite() {
            return Err(KurumiError::NumericalOverflow {
                operation: "newton_raphson iterate",
            });
        }
        if (next - x).abs() <= config.tolerance * (1.0 + x.abs()) {
            return Ok(next);
        }
        x = next;
    }

    Err(KurumiError::ConvergenceFailed {
        method: "newton_raphson",
        iterations: config.max_iterations,
    })
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

fn evaluate<F>(function: &F, x: f64) -> Result<f64, KurumiError>
where
    F: Fn(f64) -> f64,
{
    let value = function(x);
    if value.is_finite() {
        Ok(value)
    } else {
        Err(KurumiError::InvalidParameter {
            parameter: "function value",
            reason: format!("function returned a non-finite value at x = {x}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use std::f64::consts::{LN_2, PI};

    fn config() -> RootFinderConfig {
        RootFinderConfig::new(200, 1e-12).expect("valid config")
    }

    #[test]
    fn config_rejects_invalid_budgets_and_tolerances() {
        assert!(RootFinderConfig::new(0, 1e-8).is_err());
        assert!(RootFinderConfig::new(10, 0.0).is_err());
        assert!(RootFinderConfig::new(10, -1e-8).is_err());
        assert!(RootFinderConfig::new(10, f64::NAN).is_err());
        assert!(RootFinderConfig::new(10, f64::INFINITY).is_err());
    }

    #[test]
    fn config_default_is_usable() {
        let default = RootFinderConfig::default();
        assert_eq!(default.max_iterations, 100);
        assert_relative_eq!(default.tolerance, 1e-10, epsilon = 1e-20);
    }

    #[test]
    fn brent_finds_the_classic_cubic_root() {
        // Burden & Faires, Numerical Analysis: f(x) = x^3 - 2x - 5 has a root
        // near 2.0945514815.
        let root = brent(|x| x * x * x - 2.0 * x - 5.0, 2.0, 3.0, config())
            .expect("brent should converge");
        assert_relative_eq!(root, 2.094_551_481_542_326_5, epsilon = 1e-9);
    }

    #[test]
    fn brent_finds_pi_as_a_root_of_sine() {
        let root = brent(f64::sin, 3.0, 4.0, config()).expect("brent should converge");
        assert_relative_eq!(root, PI, epsilon = 1e-10);
    }

    #[test]
    fn brent_finds_the_root_of_an_exponential() {
        let root = brent(|x| x.exp() - 2.0, 0.0, 1.0, config()).expect("brent should converge");
        assert_relative_eq!(root, LN_2, epsilon = 1e-10);
    }

    #[test]
    fn brent_returns_a_root_at_a_bracket_endpoint() {
        assert_eq!(
            brent(|x| x - 2.0, 2.0, 3.0, config()).expect("root at lower bound"),
            2.0
        );
        assert_eq!(
            brent(|x| x - 2.0, 1.0, 2.0, config()).expect("root at upper bound"),
            2.0
        );
    }

    #[test]
    fn brent_handles_flat_and_oddly_scaled_functions() {
        let root = brent(|x| (x - 1.25).powi(3), 1.0, 2.0, config()).expect("flat root");
        assert_relative_eq!(root, 1.25, epsilon = 1e-7);
    }

    #[test]
    fn brent_rejects_a_bracket_without_a_sign_change() {
        let error = brent(|x| x * x + 1.0, -1.0, 1.0, config()).expect_err("no real root");
        assert!(matches!(error, KurumiError::InvalidParameter { .. }));
    }

    #[test]
    fn brent_rejects_invalid_bounds_and_non_finite_values() {
        assert!(brent(|x| x, f64::NAN, 1.0, config()).is_err());
        assert!(brent(|x| x, 0.0, f64::INFINITY, config()).is_err());
        assert!(brent(|x| x, 1.0, 1.0, config()).is_err());
        assert!(brent(|x| x, 2.0, 1.0, config()).is_err());
        assert!(brent(|_| f64::NAN, 0.0, 1.0, config()).is_err());
    }

    #[test]
    fn brent_reports_exhausted_iterations() {
        let tight = RootFinderConfig::new(1, 1e-15).expect("valid config");
        let error = brent(f64::sin, 3.0, 4.0, tight).expect_err("budget is too small");
        assert!(matches!(
            error,
            KurumiError::ConvergenceFailed {
                method: "brent",
                iterations: 1
            }
        ));
    }

    #[test]
    fn newton_finds_square_root_of_two() {
        let root = newton_raphson(|x| x * x - 2.0, |x| 2.0 * x, 1.0, config())
            .expect("newton should converge");
        assert_relative_eq!(root, std::f64::consts::SQRT_2, epsilon = 1e-12);
    }

    #[test]
    fn newton_finds_cube_root_of_twenty_seven() {
        let root = newton_raphson(|x| x * x * x - 27.0, |x| 3.0 * x * x, 2.0, config())
            .expect("newton should converge");
        assert_relative_eq!(root, 3.0, epsilon = 1e-12);
    }

    #[test]
    fn newton_finds_the_root_of_an_exponential() {
        let root = newton_raphson(|x| x.exp() - 2.0, f64::exp, 0.0, config())
            .expect("newton should converge");
        assert_relative_eq!(root, LN_2, epsilon = 1e-12);
    }

    #[test]
    fn newton_returns_an_exact_initial_root() {
        let root = newton_raphson(|x| x - 1.5, |_| 1.0, 1.5, config()).expect("exact root");
        assert_eq!(root, 1.5);
    }

    #[test]
    fn newton_rejects_a_zero_derivative() {
        let error = newton_raphson(|x| x * x + 1.0, |x| 2.0 * x, 0.0, config())
            .expect_err("derivative vanishes at the guess");
        assert!(matches!(error, KurumiError::ConvergenceFailed { .. }));
    }

    #[test]
    fn newton_rejects_non_finite_inputs_and_values() {
        assert!(newton_raphson(|x| x, |_| 1.0, f64::NAN, config()).is_err());
        assert!(newton_raphson(|_| f64::INFINITY, |_| 1.0, 1.0, config()).is_err());
        assert!(newton_raphson(|x| x, |_| f64::NAN, 1.0, config()).is_err());
    }

    #[test]
    fn newton_reports_failure_when_the_iteration_budget_is_exhausted() {
        let tight = RootFinderConfig::new(2, 1e-15).expect("valid config");
        let error = newton_raphson(|x| x * x + 1.0, |x| 2.0 * x, 1.0, tight)
            .expect_err("no real root for x^2 + 1");
        assert!(matches!(
            error,
            KurumiError::ConvergenceFailed { .. } | KurumiError::NumericalOverflow { .. }
        ));
    }
}
