//! Benchmark-relative performance metrics.
//!
//! Every function consumes two aligned periodic simple-return series: the
//! asset (or portfolio) and its benchmark. Series are aligned by position, so
//! callers must align them before calling; the library never forward-fills or
//! resamples implicitly.
//!
//! Ratios conventionally quoted on an annual basis take an explicit
//! `periods_per_year` argument, while Jensen's alpha is reported per period.
//!
//! # References
//!
//! - J. L. Treynor, "How to Rate Management of Investment Funds", *Harvard
//!   Business Review* 43(1), 1965.
//! - M. C. Jensen, "The Performance of Mutual Funds in the Period 1945-1964",
//!   *Journal of Finance* 23(2), 1968.
//! - W. F. Sharpe, "Asset Allocation: Management Style and Performance
//!   Measurement", *Journal of Portfolio Management* 18(2), 1992.

use kurumi_core::KurumiError;
use kurumi_math::{covariance, mean, sample_std_dev, sample_variance};

/// The beta of an asset relative to a benchmark.
///
/// ```text
/// beta = cov(r_asset, r_benchmark) / var(r_benchmark)
/// ```
///
/// Both moments use Bessel's correction, which cancels in the ratio.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when the lengths differ, a value is
/// not finite, or the benchmark has zero variance, and
/// [`KurumiError::InsufficientData`] when there are fewer than two observations.
///
/// # Examples
///
/// ```
/// use kurumi_risk::beta;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let benchmark = [0.01, -0.02, 0.03, 0.00];
/// let asset = [0.02, -0.04, 0.06, 0.00];
/// assert_eq!(beta(&asset, &benchmark)?, 2.0);
/// assert_eq!(beta(&benchmark, &benchmark)?, 1.0);
/// # Ok(())
/// # }
/// ```
#[inline]
pub fn beta(asset_returns: &[f64], benchmark_returns: &[f64]) -> Result<f64, KurumiError> {
    validate_pair(asset_returns, benchmark_returns)?;
    let benchmark_variance = sample_variance(benchmark_returns)?;
    if benchmark_variance == 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "benchmark_returns",
            reason: "must have non-zero variance".to_owned(),
        });
    }
    finite(
        covariance(asset_returns, benchmark_returns)? / benchmark_variance,
        "beta",
    )
}

/// The downside beta of an asset relative to a benchmark.
///
/// Only periods in which the benchmark return is strictly negative contribute:
///
/// ```text
/// downside_beta = cov(r_asset, r_benchmark | r_benchmark < 0)
///                 / var(r_benchmark | r_benchmark < 0)
/// ```
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] when fewer than two downside
/// periods exist, and otherwise the errors of [`beta`].
///
/// # Examples
///
/// ```
/// use kurumi_risk::downside_beta;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let benchmark = [-0.02, 0.01, -0.03, 0.02];
/// let asset = [-0.04, 0.03, -0.06, 0.01];
/// // Only the first and third observations are downside periods.
/// assert!((downside_beta(&asset, &benchmark)? - 2.0).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn downside_beta(asset_returns: &[f64], benchmark_returns: &[f64]) -> Result<f64, KurumiError> {
    conditional_beta(asset_returns, benchmark_returns, |value| value < 0.0)
}

/// The upside beta of an asset relative to a benchmark.
///
/// Only periods in which the benchmark return is strictly positive contribute:
///
/// ```text
/// upside_beta = cov(r_asset, r_benchmark | r_benchmark > 0)
///               / var(r_benchmark | r_benchmark > 0)
/// ```
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] when fewer than two upside periods
/// exist, and otherwise the errors of [`beta`].
///
/// # Examples
///
/// ```
/// use kurumi_risk::upside_beta;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let benchmark = [-0.02, 0.01, -0.03, 0.02];
/// let asset = [-0.04, 0.03, -0.06, 0.01];
/// // Only the second and fourth observations are upside periods.
/// assert!((upside_beta(&asset, &benchmark)? + 2.0).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn upside_beta(asset_returns: &[f64], benchmark_returns: &[f64]) -> Result<f64, KurumiError> {
    conditional_beta(asset_returns, benchmark_returns, |value| value > 0.0)
}

/// The per-period Jensen's alpha of an asset relative to a benchmark.
///
/// ```text
/// alpha = mean(r_asset) - r_f - beta * (mean(r_benchmark) - r_f)
/// ```
///
/// `risk_free_rate_per_period` is expressed in the same period units as the
/// return series (`0.0001` is one basis point for one period). The result is a
/// per-period excess return, not an annualized rate.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when the risk-free rate is not
/// finite, and otherwise the errors of [`beta`].
///
/// # Examples
///
/// ```
/// use kurumi_risk::jensen_alpha;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let benchmark = [-0.02, -0.01, 0.00, 0.01, 0.02];
/// let asset: Vec<f64> = benchmark.iter().map(|value| 2.0 * value + 0.01).collect();
/// let alpha = jensen_alpha(&asset, &benchmark, 0.0)?;
/// assert!((alpha - 0.01).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn jensen_alpha(
    asset_returns: &[f64],
    benchmark_returns: &[f64],
    risk_free_rate_per_period: f64,
) -> Result<f64, KurumiError> {
    if !risk_free_rate_per_period.is_finite() {
        return Err(KurumiError::InvalidParameter {
            parameter: "risk_free_rate_per_period",
            reason: "must be finite".to_owned(),
        });
    }
    let asset_beta = beta(asset_returns, benchmark_returns)?;
    let asset_mean = mean(asset_returns)?;
    let benchmark_mean = mean(benchmark_returns)?;
    finite(
        asset_mean
            - risk_free_rate_per_period
            - asset_beta * (benchmark_mean - risk_free_rate_per_period),
        "jensen_alpha",
    )
}

/// The annualized tracking error of an asset relative to a benchmark.
///
/// ```text
/// active_t = r_asset,t - r_benchmark,t
/// tracking_error = std(active) * sqrt(periods_per_year)
/// ```
///
/// The standard deviation uses Bessel's correction. A perfectly tracking asset
/// has a tracking error of `0.0`.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when the lengths differ, a value is
/// not finite, or `periods_per_year` is not finite and strictly positive, and
/// [`KurumiError::InsufficientData`] when there are fewer than two observations.
///
/// # Examples
///
/// ```
/// use kurumi_risk::tracking_error;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let benchmark = [0.01, -0.02, 0.03, 0.00];
/// assert_eq!(tracking_error(&benchmark, &benchmark, 252.0)?, 0.0);
/// # Ok(())
/// # }
/// ```
pub fn tracking_error(
    asset_returns: &[f64],
    benchmark_returns: &[f64],
    periods_per_year: f64,
) -> Result<f64, KurumiError> {
    validate_pair(asset_returns, benchmark_returns)?;
    validate_periods_per_year(periods_per_year)?;
    let active = active_returns(asset_returns, benchmark_returns);
    let deviation = sample_std_dev(&active)?;
    finite(deviation * periods_per_year.sqrt(), "tracking_error")
}

/// The annualized information ratio of an asset relative to a benchmark.
///
/// ```text
/// information_ratio = mean(active) / std(active) * sqrt(periods_per_year)
/// ```
///
/// This is the annualized active return divided by the annualized tracking
/// error.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when the tracking error is zero
/// (the ratio is undefined), and otherwise the errors of [`tracking_error`].
///
/// # Examples
///
/// ```
/// use kurumi_risk::information_ratio;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let benchmark = [0.01, -0.02, 0.03, 0.00];
/// let asset = [0.02, -0.015, 0.035, 0.005];
/// assert!(information_ratio(&asset, &benchmark, 252.0)? > 0.0);
/// # Ok(())
/// # }
/// ```
pub fn information_ratio(
    asset_returns: &[f64],
    benchmark_returns: &[f64],
    periods_per_year: f64,
) -> Result<f64, KurumiError> {
    validate_pair(asset_returns, benchmark_returns)?;
    validate_periods_per_year(periods_per_year)?;
    let active = active_returns(asset_returns, benchmark_returns);
    let deviation = sample_std_dev(&active)?;
    if deviation == 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "asset_returns",
            reason: "the information ratio is undefined when the tracking error is zero".to_owned(),
        });
    }
    let active_mean = mean(&active)?;
    finite(
        active_mean / deviation * periods_per_year.sqrt(),
        "information_ratio",
    )
}

/// The annualized Treynor ratio of an asset relative to a benchmark.
///
/// ```text
/// treynor_ratio = (mean(r_asset) - r_f) * periods_per_year / beta
/// ```
///
/// `risk_free_rate_per_period` is expressed in the same period units as the
/// returns. The ratio is annualized by multiplying the per-period excess return
/// by `periods_per_year` before dividing by beta.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when the risk-free rate or the
/// period count is invalid, or when beta is zero (the ratio is undefined), and
/// otherwise the errors of [`beta`].
///
/// # Examples
///
/// ```
/// use kurumi_risk::treynor_ratio;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let benchmark = [-0.02, -0.01, 0.00, 0.01, 0.02];
/// let asset: Vec<f64> = benchmark.iter().map(|value| 2.0 * value + 0.01).collect();
/// let ratio = treynor_ratio(&asset, &benchmark, 0.0, 252.0)?;
/// assert!((ratio - 1.26).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn treynor_ratio(
    asset_returns: &[f64],
    benchmark_returns: &[f64],
    risk_free_rate_per_period: f64,
    periods_per_year: f64,
) -> Result<f64, KurumiError> {
    if !risk_free_rate_per_period.is_finite() {
        return Err(KurumiError::InvalidParameter {
            parameter: "risk_free_rate_per_period",
            reason: "must be finite".to_owned(),
        });
    }
    validate_periods_per_year(periods_per_year)?;
    let asset_beta = beta(asset_returns, benchmark_returns)?;
    if asset_beta == 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "asset_returns",
            reason: "the Treynor ratio is undefined when beta is zero".to_owned(),
        });
    }
    let asset_mean = mean(asset_returns)?;
    finite(
        (asset_mean - risk_free_rate_per_period) * periods_per_year / asset_beta,
        "treynor_ratio",
    )
}

/// Up- and down-market capture ratios.
///
/// A capture ratio of `1.0` means the asset moved one-for-one with the benchmark
/// in that regime; values above one indicate outperformance. Ratios use the
/// arithmetic mean of the periodic returns in the relevant regime, so
/// annualization cancels out. A regime with no observations is `None`.
///
/// # Examples
///
/// ```
/// use kurumi_risk::capture_ratios;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let benchmark = [0.01, -0.02, 0.02, -0.02];
/// let asset = [0.02, -0.03, 0.01, -0.01];
/// let ratios = capture_ratios(&asset, &benchmark)?;
/// assert!((ratios.upside_capture.unwrap_or(f64::NAN) - 1.0).abs() < 1e-12);
/// assert!((ratios.downside_capture.unwrap_or(f64::NAN) - 1.0).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureRatios {
    /// Mean asset return divided by mean benchmark return in up-benchmark
    /// periods, or `None` when there are no such periods.
    pub upside_capture: Option<f64>,
    /// Mean asset return divided by mean benchmark return in down-benchmark
    /// periods, or `None` when there are no such periods.
    pub downside_capture: Option<f64>,
}

/// Computes the up- and down-market capture ratios.
///
/// ```text
/// upside_capture   = mean(r_asset | r_benchmark > 0) / mean(r_benchmark | r_benchmark > 0)
/// downside_capture = mean(r_asset | r_benchmark < 0) / mean(r_benchmark | r_benchmark < 0)
/// ```
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when the lengths differ or a value
/// is not finite, and [`KurumiError::InsufficientData`] when there are fewer
/// than two observations. A regime with no observations yields `None` rather
/// than an error.
///
/// # Examples
///
/// ```
/// use kurumi_risk::capture_ratios;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let benchmark = [0.01, -0.02, 0.02, -0.02];
/// let asset = [0.02, -0.03, 0.01, -0.01];
/// let ratios = capture_ratios(&asset, &benchmark)?;
/// assert!(ratios.upside_capture.is_some());
/// assert!(ratios.downside_capture.is_some());
/// # Ok(())
/// # }
/// ```
pub fn capture_ratios(
    asset_returns: &[f64],
    benchmark_returns: &[f64],
) -> Result<CaptureRatios, KurumiError> {
    validate_pair(asset_returns, benchmark_returns)?;
    let mut asset_up = Vec::new();
    let mut benchmark_up = Vec::new();
    let mut asset_down = Vec::new();
    let mut benchmark_down = Vec::new();
    for (&asset, &benchmark) in asset_returns.iter().zip(benchmark_returns.iter()) {
        if benchmark > 0.0 {
            asset_up.push(asset);
            benchmark_up.push(benchmark);
        } else if benchmark < 0.0 {
            asset_down.push(asset);
            benchmark_down.push(benchmark);
        }
    }

    let upside_capture = if benchmark_up.is_empty() {
        None
    } else {
        let benchmark_mean = mean(&benchmark_up)?;
        if benchmark_mean == 0.0 {
            None
        } else {
            Some(finite(mean(&asset_up)? / benchmark_mean, "capture_ratios")?)
        }
    };
    let downside_capture = if benchmark_down.is_empty() {
        None
    } else {
        let benchmark_mean = mean(&benchmark_down)?;
        if benchmark_mean == 0.0 {
            None
        } else {
            Some(finite(
                mean(&asset_down)? / benchmark_mean,
                "capture_ratios",
            )?)
        }
    };

    Ok(CaptureRatios {
        upside_capture,
        downside_capture,
    })
}

fn conditional_beta<F>(
    asset_returns: &[f64],
    benchmark_returns: &[f64],
    keep: F,
) -> Result<f64, KurumiError>
where
    F: Fn(f64) -> bool,
{
    validate_pair(asset_returns, benchmark_returns)?;
    let mut asset = Vec::new();
    let mut benchmark = Vec::new();
    for (&asset_return, &benchmark_return) in asset_returns.iter().zip(benchmark_returns.iter()) {
        if keep(benchmark_return) {
            asset.push(asset_return);
            benchmark.push(benchmark_return);
        }
    }
    beta(&asset, &benchmark)
}

fn active_returns(asset_returns: &[f64], benchmark_returns: &[f64]) -> Vec<f64> {
    asset_returns
        .iter()
        .zip(benchmark_returns.iter())
        .map(|(&asset, &benchmark)| asset - benchmark)
        .collect()
}

fn validate_pair(asset_returns: &[f64], benchmark_returns: &[f64]) -> Result<(), KurumiError> {
    if asset_returns.len() != benchmark_returns.len() {
        return Err(KurumiError::InvalidParameter {
            parameter: "benchmark_returns",
            reason: "must have the same length as asset_returns".to_owned(),
        });
    }
    if asset_returns.len() < 2 {
        return Err(KurumiError::InsufficientData {
            required: 2,
            available: asset_returns.len(),
        });
    }
    ensure_finite(asset_returns, "asset_returns")?;
    ensure_finite(benchmark_returns, "benchmark_returns")
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

fn validate_periods_per_year(periods_per_year: f64) -> Result<(), KurumiError> {
    if !periods_per_year.is_finite() || periods_per_year <= 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "periods_per_year",
            reason: "must be finite and strictly positive".to_owned(),
        });
    }
    Ok(())
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
    fn beta_of_a_series_against_itself_is_exactly_one() {
        let returns = [0.01, -0.02, 0.03, 0.00, 0.015, -0.005];
        assert_eq!(beta(&returns, &returns).expect("valid"), 1.0);
    }

    #[test]
    fn beta_and_alpha_recover_a_linear_scaling() {
        let benchmark = [-2.0, -1.0, 0.0, 1.0, 2.0];
        let asset = [-3.99, -1.99, 0.01, 2.01, 4.01];
        assert_eq!(beta(&asset, &benchmark).expect("valid"), 2.0);
        assert_relative_eq!(
            jensen_alpha(&asset, &benchmark, 0.0).expect("valid"),
            0.01,
            epsilon = 1e-12
        );
    }

    #[test]
    fn conditional_betas_use_only_the_matching_benchmark_periods() {
        let benchmark = [-0.02, 0.01, -0.03, 0.02, -0.01];
        let asset = [-0.04, 0.03, -0.06, 0.01, -0.02];
        let downside_benchmark = [-0.02, -0.03, -0.01];
        let downside_asset = [-0.04, -0.06, -0.02];
        let upside_benchmark = [0.01, 0.02];
        let upside_asset = [0.03, 0.01];
        assert_relative_eq!(
            downside_beta(&asset, &benchmark).expect("valid"),
            beta(&downside_asset, &downside_benchmark).expect("valid"),
            epsilon = 1e-15
        );
        assert_relative_eq!(
            upside_beta(&asset, &benchmark).expect("valid"),
            beta(&upside_asset, &upside_benchmark).expect("valid"),
            epsilon = 1e-15
        );
    }

    #[test]
    fn tracking_error_of_a_perfect_tracker_is_zero() {
        let benchmark = [0.01, -0.02, 0.03, 0.00];
        assert_eq!(
            tracking_error(&benchmark, &benchmark, 252.0).expect("valid"),
            0.0
        );
    }

    #[test]
    fn information_ratio_and_treynor_ratio_are_finite_for_valid_inputs() {
        let benchmark = [-0.02, -0.01, 0.00, 0.01, 0.02];
        let asset = [-0.03, -0.01, 0.01, 0.02, 0.05];
        let ratio = information_ratio(&asset, &benchmark, 252.0).expect("valid");
        assert!(ratio.is_finite());
        let treynor = treynor_ratio(&asset, &benchmark, 0.0, 252.0).expect("valid");
        assert!(treynor.is_finite());
    }

    #[test]
    fn information_ratio_rejects_zero_tracking_error() {
        let benchmark = [0.01, -0.02, 0.03, 0.00];
        assert!(matches!(
            information_ratio(&benchmark, &benchmark, 252.0),
            Err(KurumiError::InvalidParameter { .. })
        ));
    }

    #[test]
    fn capture_ratios_use_the_matching_benchmark_regimes() {
        let benchmark = [0.01, -0.02, 0.02, -0.02];
        let asset = [0.02, -0.03, 0.01, -0.01];
        let ratios = capture_ratios(&asset, &benchmark).expect("valid");
        assert_relative_eq!(
            ratios.upside_capture.expect("up periods"),
            1.0,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            ratios.downside_capture.expect("down periods"),
            1.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn capture_ratios_report_missing_regimes_as_none() {
        let benchmark = [0.01, 0.02, 0.03, 0.04];
        let asset = [0.01, 0.02, 0.03, 0.04];
        let ratios = capture_ratios(&asset, &benchmark).expect("valid");
        assert!(ratios.upside_capture.is_some());
        assert!(ratios.downside_capture.is_none());
    }
}
