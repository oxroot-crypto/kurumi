//! Performance metrics computed from a periodic return series.
//!
//! Every function in this module consumes a slice of **periodic simple returns**
//! (`0.01` is a one-percent gain, `-0.02` is a two-percent loss) and a
//! `periods_per_year` factor that converts the period length into a year. Daily
//! data uses 252, weekly data 52, and monthly data 12.
//!
//! The annualization conventions are:
//!
//! - annualized return is geometric: `(prod(1 + r_i))^(periods_per_year / n) - 1`;
//! - annualized volatility is the sample standard deviation times
//!   `sqrt(periods_per_year)`;
//! - the Sharpe ratio is the annualized mean excess return divided by the
//!   annualized volatility, using the risk-free rate as the hurdle;
//! - the Sortino ratio replaces the full standard deviation with the
//!   target-downside deviation;
//! - the Calmar ratio divides the annualized return by the maximum drawdown.
//!
//! A return of `-100%` or worse is rejected because the equity curve would stop
//! being strictly positive and the geometric annualization would be undefined.
//!
//! # References
//!
//! - W. F. Sharpe, "Mutual Fund Performance", *Journal of Business* 39(1), 1966.
//! - F. A. Sortino and L. N. Price, "Performance Measurement in a Downside Risk
//!   Framework", *Journal of Investing* 3(3), 1994.
//! - T. W. Young, "Calmar Ratio: A Smoother Tool", *Futures* 20(1), 1991.

use kurumi_core::KurumiError;
use kurumi_math::{mean, sample_std_dev};

const DEFAULT_PERIODS_PER_YEAR: f64 = 252.0;

/// Annualization and hurdle settings shared by the metric functions.
///
/// Construct one with [`MetricsConfig::new`]; the fields are validated once so
/// every metric can assume a finite, strictly positive period count.
///
/// # Examples
///
/// ```
/// use kurumi_risk::MetricsConfig;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let config = MetricsConfig::new(252.0, 0.03)?;
/// assert!((config.periods_per_year() - 252.0).abs() < 1e-15);
/// assert!((config.risk_free_rate() - 0.03).abs() < 1e-15);
///
/// // The default is daily data with a zero hurdle.
/// assert!((MetricsConfig::default().periods_per_year() - 252.0).abs() < 1e-15);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricsConfig {
    periods_per_year: f64,
    risk_free_rate: f64,
}

impl MetricsConfig {
    /// Creates a configuration from a period count and an annual risk-free rate.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `periods_per_year` is not
    /// finite and strictly positive, or when `risk_free_rate` is not finite.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_risk::MetricsConfig;
    ///
    /// assert!(MetricsConfig::new(252.0, 0.0).is_ok());
    /// assert!(MetricsConfig::new(0.0, 0.0).is_err());
    /// assert!(MetricsConfig::new(252.0, f64::NAN).is_err());
    /// ```
    pub fn new(periods_per_year: f64, risk_free_rate: f64) -> Result<Self, KurumiError> {
        validate_periods_per_year(periods_per_year)?;
        if !risk_free_rate.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "risk_free_rate",
                reason: "must be finite".to_owned(),
            });
        }
        Ok(Self {
            periods_per_year,
            risk_free_rate,
        })
    }

    /// Returns the number of return periods in a year.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_risk::MetricsConfig;
    ///
    /// # fn main() -> Result<(), kurumi_risk::KurumiError> {
    /// assert!((MetricsConfig::new(12.0, 0.0)?.periods_per_year() - 12.0).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn periods_per_year(self) -> f64 {
        self.periods_per_year
    }

    /// Returns the annualized risk-free rate used as the hurdle.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_risk::MetricsConfig;
    ///
    /// # fn main() -> Result<(), kurumi_risk::KurumiError> {
    /// assert!(MetricsConfig::new(252.0, 0.05)?.risk_free_rate() > 0.0);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn risk_free_rate(self) -> f64 {
        self.risk_free_rate
    }
}

impl Default for MetricsConfig {
    /// Returns daily annualization (252 periods) with a zero risk-free rate.
    fn default() -> Self {
        Self {
            periods_per_year: DEFAULT_PERIODS_PER_YEAR,
            risk_free_rate: 0.0,
        }
    }
}

/// The result of a drawdown analysis over an equity path derived from returns.
///
/// Drawdowns are reported as a **non-negative fraction**: a value of `0.25`
/// means the equity curve lost 25% from its running peak. Durations are counted
/// in return periods.
///
/// # Examples
///
/// ```
/// use kurumi_risk::drawdown_stats;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let stats = drawdown_stats(&[0.10, -0.20, 0.05, 0.20])?;
/// // 1.00 -> 1.10 -> 0.88 is a 20% drawdown; the 1.10 peak returns at period 4.
/// assert!((stats.max_drawdown - 0.20).abs() < 1e-12);
/// assert_eq!(stats.max_drawdown_duration, 3);
/// assert!(stats.recovered);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawdownStats {
    /// Largest peak-to-trough decline as a non-negative fraction.
    pub max_drawdown: f64,
    /// Number of periods from the peak preceding the largest drawdown to its
    /// recovery, or to the end of the sample when it never recovers.
    pub max_drawdown_duration: usize,
    /// Longest number of consecutive periods spent below a running peak.
    pub longest_drawdown_duration: usize,
    /// Period index of the running peak that started the largest drawdown.
    /// `0` is the initial equity level.
    pub peak_index: usize,
    /// Period index at which the largest drawdown reached its trough.
    pub trough_index: usize,
    /// Whether the equity curve regained the peak that started the largest
    /// drawdown before the sample ended.
    pub recovered: bool,
}

/// A complete set of performance statistics for one return series.
///
/// Ratios that are undefined for the sample are `None`: the Sharpe ratio when
/// volatility is zero, the Sortino ratio when there is no downside deviation,
/// the Calmar ratio when there is no drawdown, and the profit/loss ratio when
/// there are no gains or no losses.
///
/// # Examples
///
/// ```
/// use kurumi_risk::{MetricsConfig, PerformanceMetrics};
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let returns = [0.01, -0.02, 0.015, 0.005, -0.01, 0.02];
/// let metrics = PerformanceMetrics::from_returns(&returns, MetricsConfig::default())?;
/// assert_eq!(metrics.observations, 6);
/// assert!(metrics.max_drawdown >= 0.0);
/// assert!(metrics.sharpe_ratio.is_some());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerformanceMetrics {
    /// Number of return observations.
    pub observations: usize,
    /// Cumulative simple return over the whole sample.
    pub total_return: f64,
    /// Geometric annualized return.
    pub annualized_return: f64,
    /// Annualized sample standard deviation.
    pub annualized_volatility: f64,
    /// Annualized Sharpe ratio, or `None` when volatility is zero.
    pub sharpe_ratio: Option<f64>,
    /// Annualized Sortino ratio, or `None` when downside deviation is zero.
    pub sortino_ratio: Option<f64>,
    /// Annualized return divided by the maximum drawdown.
    pub calmar_ratio: Option<f64>,
    /// Largest peak-to-trough decline as a non-negative fraction.
    pub max_drawdown: f64,
    /// Period length of the largest drawdown episode.
    pub max_drawdown_duration: usize,
    /// Longest number of consecutive periods below a running peak.
    pub longest_drawdown_duration: usize,
    /// Fraction of periods with a strictly positive return.
    pub win_rate: Option<f64>,
    /// Average gain divided by the average absolute loss.
    pub profit_loss_ratio: Option<f64>,
}

impl PerformanceMetrics {
    /// Computes every metric from a return series and an annualization config.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InsufficientData`] for fewer than two
    /// observations, and [`KurumiError::InvalidParameter`] when a return is not
    /// finite or is `-100%` or worse, or when annualization fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_risk::{MetricsConfig, PerformanceMetrics};
    ///
    /// # fn main() -> Result<(), kurumi_risk::KurumiError> {
    /// let returns = [0.02, -0.01, 0.03, -0.02];
    /// let metrics =
    ///     PerformanceMetrics::from_returns(&returns, MetricsConfig::new(252.0, 0.0)?)?;
    /// assert!(metrics.annualized_volatility > 0.0);
    /// assert!(metrics.max_drawdown > 0.0);
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_returns(returns: &[f64], config: MetricsConfig) -> Result<Self, KurumiError> {
        validate_returns(returns)?;
        if returns.len() < 2 {
            return Err(KurumiError::InsufficientData {
                required: 2,
                available: returns.len(),
            });
        }
        let periods_per_year = config.periods_per_year();
        let total = total_return(returns)?;
        let annualized = annualized_return(returns, periods_per_year)?;
        let volatility = annualized_volatility(returns, periods_per_year)?;
        let drawdown = drawdown_stats(returns)?;

        let sharpe = sharpe_ratio_value(returns, config)?;
        let sortino = sortino_ratio_value(returns, config)?;
        let calmar = if drawdown.max_drawdown > 0.0 {
            Some(finite(annualized / drawdown.max_drawdown, "calmar_ratio")?)
        } else {
            None
        };
        let wins = win_rate(returns)?;
        let profit_loss = profit_loss_ratio_value(returns)?;

        Ok(Self {
            observations: returns.len(),
            total_return: total,
            annualized_return: annualized,
            annualized_volatility: volatility,
            sharpe_ratio: sharpe,
            sortino_ratio: sortino,
            calmar_ratio: calmar,
            max_drawdown: drawdown.max_drawdown,
            max_drawdown_duration: drawdown.max_drawdown_duration,
            longest_drawdown_duration: drawdown.longest_drawdown_duration,
            win_rate: Some(wins),
            profit_loss_ratio: profit_loss,
        })
    }
}

/// The cumulative simple return of a series.
///
/// ```text
/// R = prod_i (1 + r_i) - 1
/// ```
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for an empty series and
/// [`KurumiError::InvalidParameter`] when a return is not finite or is `-100%`
/// or worse.
///
/// # Examples
///
/// ```
/// use kurumi_risk::total_return;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let total = total_return(&[0.10, -0.10])?;
/// assert!((total - (-0.01)).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn total_return(returns: &[f64]) -> Result<f64, KurumiError> {
    validate_returns(returns)?;
    if returns.is_empty() {
        return Err(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        });
    }
    let growth = growth_factor(returns)?;
    finite(growth - 1.0, "total_return")
}

/// The geometric annualized return.
///
/// ```text
/// R_annual = (prod_i (1 + r_i))^(periods_per_year / n) - 1
/// ```
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for an empty series,
/// [`KurumiError::InvalidParameter`] when the period count is invalid or a
/// return is not finite or is `-100%` or worse, and
/// [`KurumiError::NumericalOverflow`] when compounding overflows.
///
/// # Examples
///
/// ```
/// use kurumi_risk::annualized_return;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// // One monthly 1% gain annualizes to (1.01)^12 - 1.
/// let annualized = annualized_return(&[0.01], 12.0)?;
/// assert!((annualized - (1.01_f64.powf(12.0) - 1.0)).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn annualized_return(returns: &[f64], periods_per_year: f64) -> Result<f64, KurumiError> {
    validate_returns(returns)?;
    validate_periods_per_year(periods_per_year)?;
    if returns.is_empty() {
        return Err(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        });
    }
    let growth = growth_factor(returns)?;
    let periods = returns.len() as f64;
    let annualized = growth.powf(periods_per_year / periods) - 1.0;
    finite(annualized, "annualized_return")
}

/// The annualized sample standard deviation of returns.
///
/// ```text
/// sigma_annual = sqrt(periods_per_year) * sqrt(sum_i (r_i - mean)^2 / (n - 1))
/// ```
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for fewer than two observations,
/// [`KurumiError::InvalidParameter`] when the period count is invalid or a
/// return is not finite or is `-100%` or worse, and
/// [`KurumiError::NumericalOverflow`] when the variance overflows.
///
/// # Examples
///
/// ```
/// use kurumi_risk::annualized_volatility;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let volatility = annualized_volatility(&[0.01, -0.01, 0.02, -0.02], 252.0)?;
/// // Constant-magnitude two-sided returns have a known sample deviation.
/// assert!(volatility > 0.0);
/// # Ok(())
/// # }
/// ```
pub fn annualized_volatility(returns: &[f64], periods_per_year: f64) -> Result<f64, KurumiError> {
    validate_returns(returns)?;
    validate_periods_per_year(periods_per_year)?;
    let deviation = sample_std_dev(returns)?;
    finite(deviation * periods_per_year.sqrt(), "annualized_volatility")
}

/// The annualized Sharpe ratio.
///
/// ```text
/// Sharpe = (mean(r) - risk_free / periods_per_year) / sigma * sqrt(periods_per_year)
/// ```
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for fewer than two observations,
/// [`KurumiError::InvalidParameter`] when a return is not finite or is `-100%`
/// or worse, or when the return volatility is zero (the ratio is undefined),
/// and [`KurumiError::NumericalOverflow`] when the variance overflows.
///
/// # Examples
///
/// ```
/// use kurumi_risk::{sharpe_ratio, MetricsConfig};
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let returns = [0.02, -0.01, 0.03, 0.00, -0.02, 0.01];
/// let sharpe = sharpe_ratio(&returns, MetricsConfig::new(252.0, 0.0)?)?;
/// assert!(sharpe.is_finite());
/// # Ok(())
/// # }
/// ```
pub fn sharpe_ratio(returns: &[f64], config: MetricsConfig) -> Result<f64, KurumiError> {
    sharpe_ratio_value(returns, config)?.ok_or(KurumiError::InvalidParameter {
        parameter: "returns",
        reason: "the Sharpe ratio is undefined when volatility is zero".to_owned(),
    })
}

fn sharpe_ratio_value(returns: &[f64], config: MetricsConfig) -> Result<Option<f64>, KurumiError> {
    validate_returns(returns)?;
    let deviation = sample_std_dev(returns)?;
    if deviation == 0.0 {
        return Ok(None);
    }
    let excess_mean = mean(returns)? - config.risk_free_rate() / config.periods_per_year();
    Ok(Some(finite(
        excess_mean / deviation * config.periods_per_year().sqrt(),
        "sharpe_ratio",
    )?))
}

/// The annualized Sortino ratio.
///
/// The target is the per-period risk-free rate. The downside deviation is the
/// root-mean-square of the excess returns below the target:
///
/// ```text
/// DD = sqrt((1 / n) * sum_i min(r_i - target, 0)^2)
/// Sortino = (mean(r) - target) / DD * sqrt(periods_per_year)
/// ```
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for fewer than two observations,
/// [`KurumiError::InvalidParameter`] when a return is not finite or is `-100%`
/// or worse, or when there is no downside deviation (the ratio is undefined).
///
/// # Examples
///
/// ```
/// use kurumi_risk::{sortino_ratio, MetricsConfig};
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let returns = [0.02, -0.01, 0.03, 0.00, -0.02, 0.01];
/// let sortino = sortino_ratio(&returns, MetricsConfig::new(252.0, 0.0)?)?;
/// // The Sortino ratio exceeds the Sharpe ratio when only downside moves are
/// // penalized.
/// let sharpe = kurumi_risk::sharpe_ratio(&returns, MetricsConfig::new(252.0, 0.0)?)?;
/// assert!(sortino > sharpe);
/// # Ok(())
/// # }
/// ```
pub fn sortino_ratio(returns: &[f64], config: MetricsConfig) -> Result<f64, KurumiError> {
    sortino_ratio_value(returns, config)?.ok_or(KurumiError::InvalidParameter {
        parameter: "returns",
        reason: "the Sortino ratio is undefined when downside deviation is zero".to_owned(),
    })
}

fn sortino_ratio_value(returns: &[f64], config: MetricsConfig) -> Result<Option<f64>, KurumiError> {
    validate_returns(returns)?;
    let target = config.risk_free_rate() / config.periods_per_year();
    let mut excess_sum = 0.0;
    let mut downside_sum = 0.0;
    for &value in returns {
        let excess = value - target;
        excess_sum += excess;
        if excess < 0.0 {
            downside_sum += excess * excess;
        }
    }
    let count = returns.len() as f64;
    let downside = (downside_sum / count).sqrt();
    if downside == 0.0 {
        return Ok(None);
    }
    let excess_mean = excess_sum / count;
    Ok(Some(finite(
        excess_mean / downside * config.periods_per_year().sqrt(),
        "sortino_ratio",
    )?))
}

/// The Calmar ratio: annualized return divided by maximum drawdown.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when the maximum drawdown is zero
/// (the ratio is undefined), and propagates the errors of
/// [`annualized_return`] and [`drawdown_stats`].
///
/// # Examples
///
/// ```
/// use kurumi_risk::calmar_ratio;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let returns = [0.10, -0.20, 0.05, 0.20];
/// let calmar = calmar_ratio(&returns, 12.0)?;
/// assert!(calmar.is_finite());
/// # Ok(())
/// # }
/// ```
pub fn calmar_ratio(returns: &[f64], periods_per_year: f64) -> Result<f64, KurumiError> {
    calmar_ratio_value(returns, periods_per_year)?.ok_or(KurumiError::InvalidParameter {
        parameter: "returns",
        reason: "the Calmar ratio is undefined without a drawdown".to_owned(),
    })
}

fn calmar_ratio_value(returns: &[f64], periods_per_year: f64) -> Result<Option<f64>, KurumiError> {
    let annualized = annualized_return(returns, periods_per_year)?;
    let drawdown = max_drawdown(returns)?;
    if drawdown == 0.0 {
        return Ok(None);
    }
    Ok(Some(finite(annualized / drawdown, "calmar_ratio")?))
}

/// The largest peak-to-trough equity decline and its duration.
///
/// The equity path starts at `1.0` and is compounded by the returns. Durations
/// are measured in return periods; the peak index `0` is the initial level.
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for an empty series and
/// [`KurumiError::InvalidParameter`] when a return is not finite or is `-100%`
/// or worse.
///
/// # Examples
///
/// ```
/// use kurumi_risk::drawdown_stats;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let stats = drawdown_stats(&[0.10, -0.20, 0.05, 0.20])?;
/// assert!((stats.max_drawdown - 0.20).abs() < 1e-12);
/// assert_eq!(stats.trough_index, 2);
/// # Ok(())
/// # }
/// ```
pub fn drawdown_stats(returns: &[f64]) -> Result<DrawdownStats, KurumiError> {
    validate_returns(returns)?;
    if returns.is_empty() {
        return Err(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        });
    }

    let mut equity = 1.0;
    let mut peak = 1.0;
    let mut peak_level = 0usize;
    let mut longest = 0usize;

    let mut max_drawdown = 0.0;
    let mut max_peak_level = 0usize;
    let mut max_peak_value = 1.0;
    let mut max_trough_level = 0usize;
    let mut max_recovery_level: Option<usize> = None;

    for (index, &value) in returns.iter().enumerate() {
        let level = index + 1;
        equity = finite(equity * (1.0 + value), "drawdown_stats equity")?;

        if equity >= peak {
            let stretch = level - peak_level;
            longest = longest.max(stretch);
            peak = equity;
            peak_level = level;
        } else {
            let drawdown = 1.0 - equity / peak;
            if drawdown > max_drawdown {
                max_drawdown = drawdown;
                max_peak_level = peak_level;
                max_peak_value = peak;
                max_trough_level = level;
                max_recovery_level = None;
            }
        }

        if max_drawdown > 0.0
            && max_recovery_level.is_none()
            && level > max_trough_level
            && equity >= max_peak_value
        {
            max_recovery_level = Some(level);
        }
    }

    longest = longest.max(returns.len() - peak_level);
    let recovered = max_recovery_level.is_some();
    let end_level = max_recovery_level.unwrap_or(returns.len());
    let max_drawdown_duration = end_level.saturating_sub(max_peak_level);

    Ok(DrawdownStats {
        max_drawdown,
        max_drawdown_duration,
        longest_drawdown_duration: longest,
        peak_index: max_peak_level,
        trough_index: max_trough_level,
        recovered,
    })
}

/// The maximum drawdown as a non-negative fraction.
///
/// This is the `max_drawdown` field of [`drawdown_stats`].
///
/// # Errors
///
/// Returns the errors of [`drawdown_stats`].
///
/// # Examples
///
/// ```
/// use kurumi_risk::max_drawdown;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// assert!((max_drawdown(&[0.10, -0.20, 0.05])? - 0.20).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn max_drawdown(returns: &[f64]) -> Result<f64, KurumiError> {
    Ok(drawdown_stats(returns)?.max_drawdown)
}

/// The fraction of periods with a strictly positive return.
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for an empty series and
/// [`KurumiError::InvalidParameter`] when a return is not finite or is `-100%`
/// or worse.
///
/// # Examples
///
/// ```
/// use kurumi_risk::win_rate;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let rate = win_rate(&[0.01, -0.02, 0.0, 0.03])?;
/// assert!((rate - 0.5).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn win_rate(returns: &[f64]) -> Result<f64, KurumiError> {
    validate_returns(returns)?;
    if returns.is_empty() {
        return Err(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        });
    }
    let wins = returns.iter().filter(|value| **value > 0.0).count();
    Ok(wins as f64 / returns.len() as f64)
}

/// The average gain divided by the average absolute loss.
///
/// This is the classic payoff ratio used with [`win_rate`] to reconstruct the
/// Kelly fraction. Zero-return periods are ignored.
///
/// # Errors
///
/// Returns [`KurumiError::InsufficientData`] for an empty series,
/// [`KurumiError::InvalidParameter`] when a return is not finite or is `-100%`
/// or worse, or when the sample contains no gains or no losses (the ratio is
/// undefined).
///
/// # Examples
///
/// ```
/// use kurumi_risk::profit_loss_ratio;
///
/// # fn main() -> Result<(), kurumi_risk::KurumiError> {
/// let ratio = profit_loss_ratio(&[0.02, 0.04, -0.01, -0.03])?;
/// // Average gain 0.03 divided by average absolute loss 0.02.
/// assert!((ratio - 1.5).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn profit_loss_ratio(returns: &[f64]) -> Result<f64, KurumiError> {
    profit_loss_ratio_value(returns)?.ok_or(KurumiError::InvalidParameter {
        parameter: "returns",
        reason: "the profit/loss ratio requires at least one gain and one loss".to_owned(),
    })
}

fn profit_loss_ratio_value(returns: &[f64]) -> Result<Option<f64>, KurumiError> {
    validate_returns(returns)?;
    if returns.is_empty() {
        return Err(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        });
    }
    let mut gain_sum = 0.0;
    let mut gain_count = 0usize;
    let mut loss_sum = 0.0;
    let mut loss_count = 0usize;
    for &value in returns {
        if value > 0.0 {
            gain_sum += value;
            gain_count += 1;
        } else if value < 0.0 {
            loss_sum += value;
            loss_count += 1;
        }
    }
    if gain_count == 0 || loss_count == 0 {
        return Ok(None);
    }
    let average_gain = gain_sum / gain_count as f64;
    let average_loss = (loss_sum / loss_count as f64).abs();
    Ok(Some(finite(
        average_gain / average_loss,
        "profit_loss_ratio",
    )?))
}

fn growth_factor(returns: &[f64]) -> Result<f64, KurumiError> {
    let mut growth = 1.0;
    for &value in returns {
        growth = finite(growth * (1.0 + value), "compounded growth")?;
    }
    Ok(growth)
}

fn validate_returns(returns: &[f64]) -> Result<(), KurumiError> {
    for (index, &value) in returns.iter().enumerate() {
        if !value.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "returns",
                reason: format!("value at index {index} must be finite"),
            });
        }
        if value <= -1.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "returns",
                reason: format!("value at index {index} must be greater than -100%"),
            });
        }
    }
    Ok(())
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

    fn daily(risk_free_rate: f64) -> MetricsConfig {
        MetricsConfig::new(252.0, risk_free_rate).expect("valid config")
    }

    #[test]
    fn config_rejects_invalid_periods_and_rates() {
        assert!(MetricsConfig::new(0.0, 0.0).is_err());
        assert!(MetricsConfig::new(-1.0, 0.0).is_err());
        assert!(MetricsConfig::new(f64::NAN, 0.0).is_err());
        assert!(MetricsConfig::new(252.0, f64::INFINITY).is_err());
        assert_eq!(MetricsConfig::default().periods_per_year(), 252.0);
    }

    #[test]
    fn annualized_return_matches_compound_growth() {
        // A 10% first period and a -10% second period grow 1.10 * 0.90 = 0.99.
        let annualized = annualized_return(&[0.10, -0.10], 2.0).expect("valid sample");
        assert_relative_eq!(annualized, 0.99_f64.powf(1.0) - 1.0, epsilon = 1e-12);

        // Twelve monthly 1% gains compound to (1.01)^12 - 1.
        let monthly = annualized_return(&[0.01; 12], 12.0).expect("valid sample");
        assert_relative_eq!(monthly, 1.01_f64.powf(12.0) - 1.0, epsilon = 1e-12);
    }

    #[test]
    fn annualized_return_rejects_ruin_and_empty_series() {
        assert!(annualized_return(&[-1.0], 252.0).is_err());
        assert!(matches!(
            annualized_return(&[], 252.0),
            Err(KurumiError::InsufficientData { .. })
        ));
        assert!(annualized_return(&[0.01], 0.0).is_err());
    }

    #[test]
    fn annualized_volatility_scales_the_sample_deviation() {
        let returns = [0.01, -0.01, 0.02, -0.02];
        let expected = sample_std_dev(&returns).expect("valid sample") * 252.0_f64.sqrt();
        let actual = annualized_volatility(&returns, 252.0).expect("valid sample");
        assert_relative_eq!(actual, expected, epsilon = 1e-12);
    }

    #[test]
    fn sharpe_ratio_matches_the_textbook_formula() {
        let returns = [0.02, 0.00, -0.01, 0.03, 0.01];
        let config = daily(0.0);
        let expected_mean = mean(&returns).expect("valid sample");
        let expected_std = sample_std_dev(&returns).expect("valid sample");
        let expected = expected_mean / expected_std * 252.0_f64.sqrt();
        let actual = sharpe_ratio(&returns, config).expect("valid sample");
        assert_relative_eq!(actual, expected, epsilon = 1e-12);
    }

    #[test]
    fn sharpe_ratio_subtracts_the_risk_free_hurdle() {
        let returns = [0.02, 0.01, 0.03, 0.00];
        let with_zero = sharpe_ratio(&returns, daily(0.0)).expect("valid sample");
        let with_hurdle = sharpe_ratio(&returns, daily(0.05)).expect("valid sample");
        assert!(with_hurdle < with_zero);
    }

    #[test]
    fn sharpe_ratio_is_undefined_with_zero_volatility() {
        assert!(sharpe_ratio(&[0.01, 0.01, 0.01], daily(0.0)).is_err());
        assert!(sharpe_ratio(&[0.01], daily(0.0)).is_err());
    }

    #[test]
    fn sortino_ratio_penalizes_only_downside_deviation() {
        let returns = [0.02, -0.01, 0.03, 0.00, -0.02, 0.01];
        let config = daily(0.0);
        let sortino = sortino_ratio(&returns, config).expect("valid sample");
        let sharpe = sharpe_ratio(&returns, config).expect("valid sample");
        assert!(sortino > sharpe);

        // Hand-computed downside deviation: sqrt((0.01^2 + 0.02^2) / 6).
        let downside = ((0.0001 + 0.0004) / 6.0_f64).sqrt();
        let expected = (mean(&returns).expect("valid sample") / downside) * 252.0_f64.sqrt();
        assert_relative_eq!(sortino, expected, epsilon = 1e-12);
    }

    #[test]
    fn sortino_ratio_is_undefined_without_downside_returns() {
        assert!(sortino_ratio(&[0.01, 0.02, 0.03], daily(0.0)).is_err());
    }

    #[test]
    fn calmar_ratio_divides_annualized_return_by_drawdown() {
        let returns = [0.10, -0.20, 0.05, 0.20];
        let annualized = annualized_return(&returns, 12.0).expect("valid sample");
        let drawdown = max_drawdown(&returns).expect("valid sample");
        let calmar = calmar_ratio(&returns, 12.0).expect("valid sample");
        assert_relative_eq!(calmar, annualized / drawdown, epsilon = 1e-12);
    }

    #[test]
    fn calmar_ratio_is_undefined_without_a_drawdown() {
        assert!(calmar_ratio(&[0.01, 0.02, 0.03], 12.0).is_err());
    }

    #[test]
    fn drawdown_stats_track_the_largest_decline_and_recovery() {
        // 1.00 -> 1.10 -> 0.88 -> 0.924 -> 1.1088. The 20% decline from the
        // period-1 peak recovers in period 4, three periods after the peak.
        let stats = drawdown_stats(&[0.10, -0.20, 0.05, 0.20]).expect("valid sample");
        assert_relative_eq!(stats.max_drawdown, 0.20, epsilon = 1e-12);
        assert_eq!(stats.peak_index, 1);
        assert_eq!(stats.trough_index, 2);
        assert_eq!(stats.max_drawdown_duration, 3);
        assert!(stats.recovered);
    }

    #[test]
    fn drawdown_stats_report_an_unrecovered_decline_through_the_end() {
        // 1.00 -> 0.90 -> 0.99: the decline never regains the initial peak.
        let stats = drawdown_stats(&[-0.10, 0.10]).expect("valid sample");
        assert_relative_eq!(stats.max_drawdown, 0.10, epsilon = 1e-12);
        assert_eq!(stats.max_drawdown_duration, 2);
        assert!(!stats.recovered);
    }

    #[test]
    fn drawdown_stats_report_the_longest_underwater_stretch() {
        // Peaks at periods 1 and 3; the second stretch is two periods long.
        let stats = drawdown_stats(&[0.05, -0.01, 0.02, -0.01, 0.00]).expect("valid sample");
        assert!(stats.longest_drawdown_duration >= stats.max_drawdown_duration);
    }

    #[test]
    fn win_rate_counts_strictly_positive_periods() {
        let rate = win_rate(&[0.01, -0.02, 0.0, 0.03]).expect("valid sample");
        assert_relative_eq!(rate, 0.5, epsilon = 1e-12);
        assert!(win_rate(&[]).is_err());
    }

    #[test]
    fn profit_loss_ratio_matches_the_payoff_average() {
        let ratio = profit_loss_ratio(&[0.02, 0.04, -0.01, -0.03]).expect("valid sample");
        assert_relative_eq!(ratio, 1.5, epsilon = 1e-12);
        assert!(profit_loss_ratio(&[0.01, 0.02]).is_err());
        assert!(profit_loss_ratio(&[-0.01, -0.02]).is_err());
    }

    #[test]
    fn performance_metrics_aggregate_the_individual_statistics() {
        let returns = [0.02, -0.01, 0.03, 0.00, -0.02, 0.01];
        let config = daily(0.0);
        let metrics = PerformanceMetrics::from_returns(&returns, config).expect("valid sample");
        assert_eq!(metrics.observations, returns.len());
        assert_relative_eq!(
            metrics.total_return,
            total_return(&returns).expect("valid sample"),
            epsilon = 1e-12
        );
        assert_relative_eq!(
            metrics.annualized_return,
            annualized_return(&returns, 252.0).expect("valid sample"),
            epsilon = 1e-12
        );
        assert_relative_eq!(
            metrics.max_drawdown,
            max_drawdown(&returns).expect("valid sample"),
            epsilon = 1e-12
        );
        assert!(metrics.sharpe_ratio.is_some());
        assert!(metrics.sortino_ratio.is_some());
        assert!(metrics.calmar_ratio.is_some());
        assert!(metrics.win_rate.is_some());
        assert!(metrics.profit_loss_ratio.is_some());
    }

    #[test]
    fn performance_metrics_leave_undefined_ratios_empty() {
        let metrics = PerformanceMetrics::from_returns(&[0.01, 0.01, 0.01], daily(0.0))
            .expect("valid sample");
        assert!(metrics.sharpe_ratio.is_none());
        assert!(metrics.sortino_ratio.is_none());
        assert!(metrics.calmar_ratio.is_none());
        assert!(metrics.profit_loss_ratio.is_none());
        assert_relative_eq!(metrics.max_drawdown, 0.0, epsilon = 1e-15);
    }

    #[test]
    fn performance_metrics_require_two_observations_and_finite_returns() {
        assert!(matches!(
            PerformanceMetrics::from_returns(&[0.01], daily(0.0)),
            Err(KurumiError::InsufficientData {
                required: 2,
                available: 1
            })
        ));
        assert!(PerformanceMetrics::from_returns(&[0.01, f64::NAN], daily(0.0)).is_err());
    }
}
