//! Rolling benchmark betas and correlations across a return matrix.
//!
//! A rolling window of `window` consecutive return observations produces one
//! output value per window. The output timestamp is the **window-end**
//! timestamp, so `values[k]` describes the window ending at `timestamps[k]`.
//!
//! A window with zero benchmark variance has no defined beta, and a window with
//! zero variance in either column has no defined correlation. Those windows are
//! reported as `None` instead of `NaN` or infinity.
//!
//! # Examples
//!
//! ```
//! use kurumi_portfolio::chrono::{DateTime, Utc};
//! use kurumi_portfolio::{
//!     AlignmentPolicy, AssetSeries, ReturnMatrix, rolling_beta_and_correlation,
//! };
//!
//! # fn at(day: i64) -> DateTime<Utc> {
//! #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
//! # }
//! # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
//! let benchmark = AssetSeries::new(
//!     "BM",
//!     vec![(at(0), -0.02), (at(1), 0.01), (at(2), 0.03), (at(3), -0.01)],
//! )?;
//! let asset = AssetSeries::new(
//!     "AAA",
//!     vec![(at(0), -0.04), (at(1), 0.02), (at(2), 0.06), (at(3), -0.02)],
//! )?;
//! let returns = ReturnMatrix::from_return_series(&[benchmark, asset], AlignmentPolicy::InnerJoin)?;
//! let panel = rolling_beta_and_correlation(&returns, "BM", 3)?;
//! assert_eq!(panel.betas.timestamps.len(), 2);
//! assert_eq!(panel.betas.series("AAA").and_then(|series| series[0]), Some(2.0));
//! # Ok(())
//! # }
//! ```

use chrono::{DateTime, Utc};
use kurumi_core::KurumiError;
use kurumi_math::rolling_correlation;
use kurumi_risk::beta;
use nalgebra::DVector;

use crate::panel::ReturnMatrix;

/// A rolling statistic panel: one series per non-benchmark asset.
///
/// `values[asset][time]` is `Some(value)` when the statistic is defined for
/// that window and `None` when a zero-variance window makes it undefined.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::chrono::{DateTime, Utc};
/// use kurumi_portfolio::{
///     AlignmentPolicy, AssetSeries, ReturnMatrix, rolling_betas,
/// };
///
/// # fn at(day: i64) -> DateTime<Utc> {
/// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
/// # }
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let benchmark = AssetSeries::new(
///     "BM",
///     vec![(at(0), 0.01), (at(1), 0.02), (at(2), 0.03)],
/// )?;
/// let asset = AssetSeries::new(
///     "AAA",
///     vec![(at(0), 0.02), (at(1), 0.04), (at(2), 0.06)],
/// )?;
/// let returns = ReturnMatrix::from_return_series(&[benchmark, asset], AlignmentPolicy::InnerJoin)?;
/// let panel = rolling_betas(&returns, "BM", 2)?;
/// assert_eq!(panel.symbols, ["AAA"]);
/// assert_eq!(panel.values.len(), 1);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RollingPanel {
    /// Asset symbols, excluding the benchmark, in matrix column order.
    pub symbols: Vec<String>,
    /// Window-end timestamps, one per output observation.
    pub timestamps: Vec<DateTime<Utc>>,
    /// Rolling values in `values[asset][time]` form.
    pub values: Vec<Vec<Option<f64>>>,
}

impl RollingPanel {
    /// Returns the series for `symbol`, or `None` when it is not in the panel.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix, rolling_betas,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let benchmark = AssetSeries::new(
    ///     "BM",
    ///     vec![(at(0), 0.01), (at(1), 0.02), (at(2), 0.03)],
    /// )?;
    /// let asset = AssetSeries::new(
    ///     "AAA",
    ///     vec![(at(0), 0.02), (at(1), 0.04), (at(2), 0.06)],
    /// )?;
    /// let returns = ReturnMatrix::from_return_series(&[benchmark, asset], AlignmentPolicy::InnerJoin)?;
    /// let panel = rolling_betas(&returns, "BM", 2)?;
    /// assert!(panel.series("AAA").is_some());
    /// assert!(panel.series("BM").is_none());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn series(&self, symbol: &str) -> Option<&[Option<f64>]> {
        self.symbols
            .iter()
            .position(|candidate| candidate == symbol)
            .and_then(|index| self.values.get(index))
            .map(Vec::as_slice)
    }

    /// Returns the value for `symbol` at output index `time`.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix, rolling_betas,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let benchmark = AssetSeries::new(
    ///     "BM",
    ///     vec![(at(0), 0.01), (at(1), 0.02), (at(2), 0.03)],
    /// )?;
    /// let asset = AssetSeries::new(
    ///     "AAA",
    ///     vec![(at(0), 0.02), (at(1), 0.04), (at(2), 0.06)],
    /// )?;
    /// let returns = ReturnMatrix::from_return_series(&[benchmark, asset], AlignmentPolicy::InnerJoin)?;
    /// let panel = rolling_betas(&returns, "BM", 2)?;
    /// assert_eq!(panel.value("AAA", 0), Some(2.0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn value(&self, symbol: &str, time: usize) -> Option<f64> {
        self.series(symbol)
            .and_then(|series| series.get(time))
            .copied()
            .flatten()
    }

    /// Returns the output index of the window ending at `timestamp`.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix, rolling_betas,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let benchmark = AssetSeries::new(
    ///     "BM",
    ///     vec![(at(0), 0.01), (at(1), 0.02), (at(2), 0.03)],
    /// )?;
    /// let returns = ReturnMatrix::from_return_series(&[benchmark], AlignmentPolicy::InnerJoin)?;
    /// let panel = rolling_betas(&returns, "BM", 2)?;
    /// assert_eq!(panel.time_index(&at(2)), Some(1));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn time_index(&self, timestamp: &DateTime<Utc>) -> Option<usize> {
        self.timestamps
            .iter()
            .position(|candidate| candidate == timestamp)
    }
}

/// Rolling betas and rolling correlations against one benchmark column.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::chrono::{DateTime, Utc};
/// use kurumi_portfolio::{
///     AlignmentPolicy, AssetSeries, ReturnMatrix, rolling_beta_and_correlation,
/// };
///
/// # fn at(day: i64) -> DateTime<Utc> {
/// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
/// # }
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let benchmark = AssetSeries::new(
///     "BM",
///     vec![(at(0), -0.02), (at(1), 0.01), (at(2), 0.03), (at(3), -0.01)],
/// )?;
/// let asset = AssetSeries::new(
///     "AAA",
///     vec![(at(0), -0.04), (at(1), 0.02), (at(2), 0.06), (at(3), -0.02)],
/// )?;
/// let returns = ReturnMatrix::from_return_series(&[benchmark, asset], AlignmentPolicy::InnerJoin)?;
/// let panel = rolling_beta_and_correlation(&returns, "BM", 3)?;
/// assert_eq!(panel.betas.symbols, panel.correlations.symbols);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RollingBetaPanel {
    /// Rolling betas of every non-benchmark asset.
    pub betas: RollingPanel,
    /// Rolling Pearson correlations of every non-benchmark asset.
    pub correlations: RollingPanel,
}

/// Computes rolling betas of every non-benchmark asset against `benchmark`.
///
/// The beta of a window is `cov(r_asset, r_benchmark) / var(r_benchmark)` with
/// Bessel's correction, computed by [`kurumi_risk::beta`]. Windows with zero
/// benchmark variance yield `None`.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when the benchmark is unknown or
/// `window < 2`, and [`KurumiError::InsufficientData`] when there are fewer
/// observations than the window length.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::chrono::{DateTime, Utc};
/// use kurumi_portfolio::{
///     AlignmentPolicy, AssetSeries, ReturnMatrix, rolling_betas,
/// };
///
/// # fn at(day: i64) -> DateTime<Utc> {
/// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
/// # }
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let benchmark = AssetSeries::new(
///     "BM",
///     vec![(at(0), 0.01), (at(1), 0.02), (at(2), 0.03)],
/// )?;
/// let asset = AssetSeries::new(
///     "AAA",
///     vec![(at(0), 0.02), (at(1), 0.04), (at(2), 0.06)],
/// )?;
/// let returns = ReturnMatrix::from_return_series(&[benchmark, asset], AlignmentPolicy::InnerJoin)?;
/// let panel = rolling_betas(&returns, "BM", 2)?;
/// assert_eq!(panel.value("AAA", 0), Some(2.0));
/// # Ok(())
/// # }
/// ```
pub fn rolling_betas(
    returns: &ReturnMatrix,
    benchmark: &str,
    window: usize,
) -> Result<RollingPanel, KurumiError> {
    let (benchmark_index, columns) = validate_window(returns, benchmark, window)?;
    let observations = returns.dimensions().0;
    let benchmark_values = column_values(&columns, benchmark_index)?;
    let output_len = observations - window + 1;
    let mut symbols = Vec::new();
    let mut values = Vec::new();
    for (index, column) in columns.iter().enumerate() {
        if index == benchmark_index {
            continue;
        }
        let symbol = returns
            .symbols()
            .get(index)
            .cloned()
            .ok_or_else(panel_shape_error)?;
        symbols.push(symbol);
        let mut series = Vec::with_capacity(output_len);
        for end in window..=observations {
            let asset_window = column
                .as_slice()
                .get(end - window..end)
                .ok_or_else(panel_shape_error)?;
            let benchmark_window = benchmark_values
                .get(end - window..end)
                .ok_or_else(panel_shape_error)?;
            series.push(beta(asset_window, benchmark_window).ok());
        }
        values.push(series);
    }
    let timestamps = window_end_timestamps(returns, window)?;
    Ok(RollingPanel {
        symbols,
        timestamps,
        values,
    })
}

/// Computes rolling Pearson correlations of every non-benchmark asset against
/// `benchmark`.
///
/// Windows with zero variance in either series yield `None`.
///
/// # Errors
///
/// Returns the same errors as [`rolling_betas`].
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::chrono::{DateTime, Utc};
/// use kurumi_portfolio::{
///     AlignmentPolicy, AssetSeries, ReturnMatrix, rolling_correlations,
/// };
///
/// # fn at(day: i64) -> DateTime<Utc> {
/// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
/// # }
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let benchmark = AssetSeries::new(
///     "BM",
///     vec![(at(0), 0.01), (at(1), 0.02), (at(2), 0.03)],
/// )?;
/// let asset = AssetSeries::new(
///     "AAA",
///     vec![(at(0), 0.02), (at(1), 0.04), (at(2), 0.06)],
/// )?;
/// let returns = ReturnMatrix::from_return_series(&[benchmark, asset], AlignmentPolicy::InnerJoin)?;
/// let panel = rolling_correlations(&returns, "BM", 2)?;
/// assert_eq!(panel.value("AAA", 0), Some(1.0));
/// # Ok(())
/// # }
/// ```
pub fn rolling_correlations(
    returns: &ReturnMatrix,
    benchmark: &str,
    window: usize,
) -> Result<RollingPanel, KurumiError> {
    let (benchmark_index, columns) = validate_window(returns, benchmark, window)?;
    let benchmark_values = column_values(&columns, benchmark_index)?;
    let mut symbols = Vec::new();
    let mut values = Vec::new();
    for (index, column) in columns.iter().enumerate() {
        if index == benchmark_index {
            continue;
        }
        let symbol = returns
            .symbols()
            .get(index)
            .cloned()
            .ok_or_else(panel_shape_error)?;
        symbols.push(symbol);
        values.push(rolling_correlation(
            column.as_slice(),
            benchmark_values,
            window,
        )?);
    }
    let timestamps = window_end_timestamps(returns, window)?;
    Ok(RollingPanel {
        symbols,
        timestamps,
        values,
    })
}

/// Computes both rolling betas and rolling correlations in one call.
///
/// This is equivalent to calling [`rolling_betas`] and
/// [`rolling_correlations`] with the same arguments; the two panels share the
/// same symbols and window-end timestamps.
///
/// # Errors
///
/// Returns the same errors as [`rolling_betas`].
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::chrono::{DateTime, Utc};
/// use kurumi_portfolio::{
///     AlignmentPolicy, AssetSeries, ReturnMatrix, rolling_beta_and_correlation,
/// };
///
/// # fn at(day: i64) -> DateTime<Utc> {
/// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
/// # }
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let benchmark = AssetSeries::new(
///     "BM",
///     vec![(at(0), 0.01), (at(1), 0.02), (at(2), 0.03)],
/// )?;
/// let asset = AssetSeries::new(
///     "AAA",
///     vec![(at(0), 0.02), (at(1), 0.04), (at(2), 0.06)],
/// )?;
/// let returns = ReturnMatrix::from_return_series(&[benchmark, asset], AlignmentPolicy::InnerJoin)?;
/// let panel = rolling_beta_and_correlation(&returns, "BM", 2)?;
/// assert_eq!(panel.betas.timestamps, panel.correlations.timestamps);
/// # Ok(())
/// # }
/// ```
pub fn rolling_beta_and_correlation(
    returns: &ReturnMatrix,
    benchmark: &str,
    window: usize,
) -> Result<RollingBetaPanel, KurumiError> {
    Ok(RollingBetaPanel {
        betas: rolling_betas(returns, benchmark, window)?,
        correlations: rolling_correlations(returns, benchmark, window)?,
    })
}

fn validate_window(
    returns: &ReturnMatrix,
    benchmark: &str,
    window: usize,
) -> Result<(usize, Vec<DVector<f64>>), KurumiError> {
    if window < 2 {
        return Err(KurumiError::InvalidParameter {
            parameter: "window",
            reason: "must be at least 2".to_owned(),
        });
    }
    let (observations, assets) = returns.dimensions();
    if observations < window {
        return Err(KurumiError::InsufficientData {
            required: window,
            available: observations,
        });
    }
    let benchmark_index = returns.asset_index(benchmark)?;
    let mut columns = Vec::with_capacity(assets);
    for index in 0..assets {
        columns.push(returns.column_by_index(index)?);
    }
    Ok((benchmark_index, columns))
}

fn column_values(columns: &[DVector<f64>], index: usize) -> Result<&[f64], KurumiError> {
    columns
        .get(index)
        .map(|column| column.as_slice())
        .ok_or_else(panel_shape_error)
}

fn window_end_timestamps(
    returns: &ReturnMatrix,
    window: usize,
) -> Result<Vec<DateTime<Utc>>, KurumiError> {
    returns
        .timestamps()
        .get(window - 1..)
        .map(|slice| slice.to_vec())
        .ok_or_else(panel_shape_error)
}

fn panel_shape_error() -> KurumiError {
    KurumiError::InvalidParameter {
        parameter: "returns",
        reason: "rolling panel shape is inconsistent".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    use crate::panel::{AlignmentPolicy, AssetSeries};

    fn at(day: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    }

    fn panel(values: &[(i64, f64)], symbol: &str) -> AssetSeries {
        AssetSeries::new(
            symbol,
            values
                .iter()
                .map(|(day, value)| (at(*day), *value))
                .collect(),
        )
        .expect("valid return series")
    }

    #[test]
    fn rolling_beta_and_correlation_recover_a_two_times_benchmark() {
        let benchmark = panel(
            &[(0, -0.02), (1, 0.01), (2, 0.03), (3, -0.01), (4, 0.02)],
            "BM",
        );
        let asset = panel(
            &[(0, -0.04), (1, 0.02), (2, 0.06), (3, -0.02), (4, 0.04)],
            "AAA",
        );
        let returns =
            ReturnMatrix::from_return_series(&[benchmark, asset], AlignmentPolicy::InnerJoin)
                .expect("valid panel");
        let result = rolling_beta_and_correlation(&returns, "BM", 3).expect("rolling succeeds");

        assert_eq!(result.betas.symbols, ["AAA"]);
        assert_eq!(result.betas.timestamps, [at(2), at(3), at(4)]);
        for time in 0..3 {
            assert_relative_eq!(
                result.betas.value("AAA", time).unwrap_or(f64::NAN),
                2.0,
                epsilon = 1e-12
            );
            assert_relative_eq!(
                result.correlations.value("AAA", time).unwrap_or(f64::NAN),
                1.0,
                epsilon = 1e-12
            );
        }
    }

    #[test]
    fn zero_variance_windows_are_reported_as_none() {
        let benchmark = panel(&[(0, 0.0), (1, 0.0), (2, 0.02), (3, 0.01)], "BM");
        let asset = panel(&[(0, 0.0), (1, 0.0), (2, 0.03), (3, 0.01)], "AAA");
        let returns =
            ReturnMatrix::from_return_series(&[benchmark, asset], AlignmentPolicy::InnerJoin)
                .expect("valid panel");
        let result = rolling_beta_and_correlation(&returns, "BM", 2).expect("rolling succeeds");

        // The first two-point window is all zeros for both series.
        assert_eq!(result.betas.value("AAA", 0), None);
        assert_eq!(result.correlations.value("AAA", 0), None);
        assert!(result.betas.value("AAA", 1).is_some());
    }

    #[test]
    fn invalid_windows_and_benchmarks_are_rejected() {
        let benchmark = panel(&[(0, 0.01), (1, 0.02), (2, 0.03)], "BM");
        let returns = ReturnMatrix::from_return_series(&[benchmark], AlignmentPolicy::InnerJoin)
            .expect("valid panel");

        assert!(rolling_betas(&returns, "BM", 1).is_err());
        assert!(rolling_betas(&returns, "BM", 4).is_err());
        assert!(rolling_betas(&returns, "MISSING", 2).is_err());
        assert!(rolling_correlations(&returns, "BM", 2).is_ok());
    }

    #[test]
    fn benchmark_column_is_not_part_of_the_output_panel() {
        let benchmark = panel(&[(0, 0.01), (1, 0.02), (2, 0.03)], "BM");
        let first = panel(&[(0, 0.01), (1, 0.02), (2, 0.03)], "AAA");
        let second = panel(&[(0, -0.01), (1, 0.00), (2, 0.01)], "BBB");
        let returns = ReturnMatrix::from_return_series(
            &[benchmark, first, second],
            AlignmentPolicy::InnerJoin,
        )
        .expect("valid panel");
        let panel = rolling_betas(&returns, "BM", 2).expect("rolling succeeds");

        assert_eq!(panel.symbols, ["AAA", "BBB"]);
        assert_eq!(panel.values.len(), 2);
        assert_eq!(panel.series("BM"), None);
    }
}
