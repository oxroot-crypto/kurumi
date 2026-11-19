//! Numerical methods for Kurumi.
//!
//! This crate provides the reusable computational building blocks used by
//! pricing, risk, and indicator code:
//!
//! - [`interpolation`] — linear, log-linear, and natural cubic-spline curves;
//! - [`multivariate`] — correlation, rolling correlation, and covariance
//!   estimators;
//! - [`regression`] — simple and SVD-based multiple ordinary least squares;
//! - [`roots`] — safeguarded Brent brackets and Newton-Raphson iteration;
//! - [`stats`] — standard normal CDF/PDF, sample moments, and covariance
//!   matrices.
//!
//! It sits directly above `kurumi-core` and must not depend on any higher-layer
//! crate. All public functions reject `NaN` and infinite inputs and return the
//! shared [`KurumiError`] type.
//!
//! # Examples
//!
//! ```
//! use kurumi_math::{brent, RootFinderConfig};
//!
//! # fn main() -> Result<(), kurumi_math::KurumiError> {
//! let config = RootFinderConfig::new(100, 1e-12)?;
//! let root = brent(|x| x * x - 2.0, 1.0, 2.0, config)?;
//! assert!((root - std::f64::consts::SQRT_2).abs() < 1e-10);
//! # Ok(())
//! # }
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

pub mod interpolation;
pub mod multivariate;
pub mod regression;
pub mod roots;
pub mod stats;

pub use interpolation::{
    CubicSplineInterpolator, Interpolator, LinearInterpolator, LogLinearInterpolator,
};
pub use multivariate::{
    CovarianceEstimator, EwmaCovariance, LedoitWolfShrinkage, SampleCovariance, correlation_matrix,
    pearson_correlation, rolling_correlation, spearman_correlation,
};
pub use regression::{MultipleRegressionResult, SimpleRegressionResult, ols_multiple, ols_simple};
pub use roots::{RootFinderConfig, brent, newton_raphson};
pub use stats::{
    covariance, covariance_matrix, mean, sample_std_dev, sample_variance, standard_normal_cdf,
    standard_normal_pdf,
};

pub use kurumi_core::KurumiError;
