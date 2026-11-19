//! Asset identification, time alignment, and return panels.
//!
//! Inputs arrive as one [`AssetSeries`] per symbol: a sorted or unsorted list of
//! `(timestamp, value)` observations. [`ReturnMatrix::from_price_series`] and
//! [`ReturnMatrix::from_return_series`] align those series explicitly according
//! to an [`AlignmentPolicy`] and produce a `(T, N)` matrix whose rows are
//! chronological observations and columns are assets.
//!
//! The two policies are:
//!
//! - [`AlignmentPolicy::InnerJoin`] keeps only timestamps observed for **every**
//!   asset.
//! - [`AlignmentPolicy::OuterForwardFill`] takes the union of timestamps,
//!   drops every timestamp before the last asset's first observation, and
//!   forward-fills missing prices with the last known price. Missing returns are
//!   filled with `0.0`, which is the return implied by carrying the previous
//!   price forward.
//!
//! For price inputs, [`ReturnMethod`] selects simple returns
//! `p_t / p_{t-1} - 1` or log returns `ln(p_t / p_{t-1})`. Prices must be
//! strictly positive so both definitions are real-valued and finite.
//!
//! # Examples
//!
//! ```
//! use kurumi_portfolio::chrono::{DateTime, Utc};
//! use kurumi_portfolio::{
//!     AlignmentPolicy, AssetSeries, ReturnMatrix, ReturnMethod,
//! };
//!
//! # fn at(day: i64) -> DateTime<Utc> {
//! #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
//! # }
//! # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
//! let a = AssetSeries::new("A", vec![(at(0), 100.0), (at(2), 110.0)])?;
//! let b = AssetSeries::new("B", vec![(at(0), 50.0), (at(1), 51.0), (at(2), 53.0)])?;
//! let panel = ReturnMatrix::from_price_series(
//!     &[a, b],
//!     AlignmentPolicy::OuterForwardFill,
//!     ReturnMethod::Simple,
//! )?;
//! // Union timestamps: day 0, day 1, day 2; two return observations.
//! assert_eq!(panel.dimensions(), (2, 2));
//! # Ok(())
//! # }
//! ```

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use kurumi_core::KurumiError;
use kurumi_math::{CovarianceEstimator, SampleCovariance, correlation_matrix, mean};
use nalgebra::{DMatrix, DVector};

/// A named price or return series observed at arbitrary timestamps.
///
/// The observations may be supplied in any order; every constructor in this
/// module normalizes them to ascending time internally. Duplicate timestamps
/// and non-finite values are rejected, so a validated series always describes a
/// well-defined function of time.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::chrono::{DateTime, Utc};
/// use kurumi_portfolio::AssetSeries;
///
/// # fn at(day: i64) -> DateTime<Utc> {
/// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
/// # }
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let series = AssetSeries::new("AAPL", vec![(at(0), 100.0), (at(1), 101.5)])?;
/// assert_eq!(series.symbol, "AAPL");
/// assert_eq!(series.observations.len(), 2);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct AssetSeries {
    /// Ticker or other unique identifier of the asset.
    pub symbol: String,
    /// Observed values in `(timestamp, value)` form.
    pub observations: Vec<(DateTime<Utc>, f64)>,
}

impl AssetSeries {
    /// Creates a series and validates its contents.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the symbol is empty, a
    /// value is not finite, or a timestamp appears twice, and
    /// [`KurumiError::InsufficientData`] when the observation list is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::AssetSeries;
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// assert!(AssetSeries::new("A", vec![(at(0), 100.0)]).is_ok());
    /// assert!(AssetSeries::new("", vec![(at(0), 100.0)]).is_err());
    /// assert!(AssetSeries::new("A", Vec::new()).is_err());
    /// ```
    pub fn new(
        symbol: impl Into<String>,
        observations: Vec<(DateTime<Utc>, f64)>,
    ) -> Result<Self, KurumiError> {
        let series = Self {
            symbol: symbol.into(),
            observations,
        };
        series.validate()?;
        Ok(series)
    }

    /// Revalidates a series that was built with struct-literal field syntax.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`AssetSeries::new`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::AssetSeries;
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// let series = AssetSeries {
    ///     symbol: "A".to_owned(),
    ///     observations: vec![(at(0), 100.0)],
    /// };
    /// assert!(series.validate().is_ok());
    /// ```
    pub fn validate(&self) -> Result<(), KurumiError> {
        if self.symbol.trim().is_empty() {
            return Err(KurumiError::InvalidParameter {
                parameter: "symbol",
                reason: "must not be empty".to_owned(),
            });
        }
        if self.observations.is_empty() {
            return Err(KurumiError::InsufficientData {
                required: 1,
                available: 0,
            });
        }
        let mut seen = BTreeSet::new();
        for (timestamp, value) in &self.observations {
            if !value.is_finite() {
                return Err(KurumiError::InvalidParameter {
                    parameter: "observations",
                    reason: format!("value for `{}` at {timestamp} must be finite", self.symbol),
                });
            }
            if !seen.insert(*timestamp) {
                return Err(KurumiError::InvalidParameter {
                    parameter: "observations",
                    reason: format!("duplicate timestamp {timestamp} for `{}`", self.symbol),
                });
            }
        }
        Ok(())
    }
}

/// How independently observed series are placed on a common time axis.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::AlignmentPolicy;
///
/// assert_ne!(AlignmentPolicy::InnerJoin, AlignmentPolicy::OuterForwardFill);
/// assert_eq!(AlignmentPolicy::default(), AlignmentPolicy::InnerJoin);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AlignmentPolicy {
    /// Keep only timestamps observed for every asset.
    ///
    /// This is the strictest policy: no value is ever invented, but one sparse
    /// series can shrink the panel substantially.
    #[default]
    InnerJoin,
    /// Use the union of timestamps and forward-fill missing values.
    ///
    /// Prices are carried forward from the last observation. Returns are filled
    /// with zero, the return implied by a carried-forward price. Timestamps
    /// before the last asset's first observation are dropped because at least
    /// one asset has no valid value there.
    OuterForwardFill,
}

/// The return definition used when a price panel is converted to returns.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::ReturnMethod;
///
/// // 100 -> 110: a simple return of 10% is a log return of ln(1.10).
/// assert!((110.0_f64 / 100.0 - 1.0 - 0.10).abs() < 1e-15);
/// assert!(((110.0_f64 / 100.0).ln() - (1.1_f64).ln()).abs() < 1e-15);
/// assert_eq!(ReturnMethod::default(), ReturnMethod::Simple);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReturnMethod {
    /// Arithmetic return `p_t / p_{t-1} - 1`.
    #[default]
    Simple,
    /// Continuously compounded return `ln(p_t / p_{t-1})`.
    Log,
}

/// A `(T, N)` matrix of periodic returns on a common time axis.
///
/// Rows are chronological return observations and columns are assets, matching
/// the workspace-wide multi-asset convention. The matrix never contains `NaN`
/// or infinity: every constructor validates the panel, and
/// [`ReturnMatrix::covariance_matrix`] / [`ReturnMatrix::correlation_matrix`]
/// reuse the validated estimators in `kurumi-math`.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::chrono::{DateTime, Utc};
/// use kurumi_portfolio::{
///     AlignmentPolicy, AssetSeries, ReturnMatrix, ReturnMethod,
/// };
///
/// # fn at(day: i64) -> DateTime<Utc> {
/// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
/// # }
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let a = AssetSeries::new(
///     "A",
///     vec![(at(0), 100.0), (at(1), 110.0), (at(2), 121.0)],
/// )?;
/// let b = AssetSeries::new(
///     "B",
///     vec![(at(0), 50.0), (at(1), 50.0), (at(2), 52.0)],
/// )?;
/// let panel = ReturnMatrix::from_price_series(
///     &[a, b],
///     AlignmentPolicy::InnerJoin,
///     ReturnMethod::Simple,
/// )?;
/// assert_eq!(panel.symbols(), ["A", "B"]);
/// assert_eq!(panel.dimensions(), (2, 2));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnMatrix {
    symbols: Vec<String>,
    timestamps: Vec<DateTime<Utc>>,
    data: DMatrix<f64>,
}

impl ReturnMatrix {
    /// Builds a return matrix from already aligned data.
    ///
    /// `data` must have shape `(timestamps.len(), symbols.len())`, rows must be
    /// chronological, symbols must be unique and non-empty, and every value must
    /// be finite. A panel may hold a single observation; sample covariance and
    /// correlation require at least two and report
    /// [`KurumiError::InsufficientData`] themselves.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] for an invalid symbol, a
    /// non-increasing timestamp, a shape mismatch, a duplicate symbol, or a
    /// non-finite value, and [`KurumiError::InsufficientData`] when the panel is
    /// empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::nalgebra::DMatrix;
    /// use kurumi_portfolio::ReturnMatrix;
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let panel = ReturnMatrix::new(
    ///     vec!["A".to_owned(), "B".to_owned()],
    ///     vec![at(0), at(1)],
    ///     DMatrix::from_row_slice(2, 2, &[0.01, 0.02, -0.01, 0.00]),
    /// )?;
    /// assert_eq!(panel.dimensions(), (2, 2));
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        symbols: Vec<String>,
        timestamps: Vec<DateTime<Utc>>,
        data: DMatrix<f64>,
    ) -> Result<Self, KurumiError> {
        if symbols.is_empty() {
            return Err(KurumiError::InsufficientData {
                required: 1,
                available: 0,
            });
        }
        if timestamps.is_empty() {
            return Err(KurumiError::InsufficientData {
                required: 1,
                available: 0,
            });
        }
        let mut unique = BTreeSet::new();
        for symbol in &symbols {
            if symbol.trim().is_empty() {
                return Err(KurumiError::InvalidParameter {
                    parameter: "symbols",
                    reason: "must not contain an empty symbol".to_owned(),
                });
            }
            if !unique.insert(symbol.clone()) {
                return Err(KurumiError::InvalidParameter {
                    parameter: "symbols",
                    reason: format!("duplicate symbol `{symbol}`"),
                });
            }
        }
        if timestamps.windows(2).any(|pair| match pair {
            [earlier, later] => earlier >= later,
            _ => true,
        }) {
            return Err(KurumiError::InvalidParameter {
                parameter: "timestamps",
                reason: "must be strictly increasing".to_owned(),
            });
        }
        if data.shape() != (timestamps.len(), symbols.len()) {
            return Err(KurumiError::InvalidParameter {
                parameter: "data",
                reason: format!(
                    "shape must be ({}, {}), got ({}, {})",
                    timestamps.len(),
                    symbols.len(),
                    data.nrows(),
                    data.ncols()
                ),
            });
        }
        for (row, column) in data.iter().enumerate() {
            if !column.is_finite() {
                return Err(KurumiError::InvalidParameter {
                    parameter: "data",
                    reason: format!("value at flat index {row} must be finite"),
                });
            }
        }
        Ok(Self {
            symbols,
            timestamps,
            data,
        })
    }

    /// Aligns price series and converts them into periodic returns.
    ///
    /// The aligned price panel defines `T - 1` returns; the returned matrix's
    /// timestamps are the interval-end timestamps of those returns. Prices must
    /// be strictly positive.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InsufficientData`] when the universe is empty or
    /// the aligned panel has fewer than two timestamps, and
    /// [`KurumiError::InvalidParameter`] for duplicate symbols, invalid
    /// observations, non-positive prices, or a non-finite return.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix, ReturnMethod,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let a = AssetSeries::new(
    ///     "A",
    ///     vec![(at(0), 100.0), (at(1), 110.0), (at(2), 121.0)],
    /// )?;
    /// let b = AssetSeries::new(
    ///     "B",
    ///     vec![(at(0), 50.0), (at(1), 49.0), (at(2), 51.0)],
    /// )?;
    /// let panel = ReturnMatrix::from_price_series(
    ///     &[a, b],
    ///     AlignmentPolicy::InnerJoin,
    ///     ReturnMethod::Simple,
    /// )?;
    /// assert_eq!(panel.dimensions(), (2, 2));
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_price_series(
        series: &[AssetSeries],
        alignment: AlignmentPolicy,
        method: ReturnMethod,
    ) -> Result<Self, KurumiError> {
        let prepared = prepare_universe(series)?;
        let aligned = align(&prepared, alignment, false)?;
        if aligned.timestamps.len() < 2 {
            return Err(KurumiError::InsufficientData {
                required: 2,
                available: aligned.timestamps.len(),
            });
        }
        validate_prices(&aligned)?;

        let rows = aligned.timestamps.len() - 1;
        let columns = prepared.len();
        let mut values = Vec::with_capacity(rows * columns);
        for row in 1..aligned.timestamps.len() {
            let previous = aligned.values.get(row - 1).ok_or_else(panel_error)?;
            let current = aligned.values.get(row).ok_or_else(panel_error)?;
            for column in 0..columns {
                let previous_price = previous.get(column).copied().ok_or_else(panel_error)?;
                let current_price = current.get(column).copied().ok_or_else(panel_error)?;
                let value = match method {
                    ReturnMethod::Simple => current_price / previous_price - 1.0,
                    ReturnMethod::Log => (current_price / previous_price).ln(),
                };
                if !value.is_finite() {
                    return Err(KurumiError::InvalidParameter {
                        parameter: "series",
                        reason: "produced a non-finite return".to_owned(),
                    });
                }
                values.push(value);
            }
        }

        let timestamps = aligned
            .timestamps
            .get(1..)
            .ok_or_else(panel_error)?
            .to_vec();
        let symbols = prepared
            .iter()
            .map(|asset| asset.symbol.clone())
            .collect::<Vec<_>>();
        Self::new(
            symbols,
            timestamps,
            DMatrix::from_row_slice(rows, columns, &values),
        )
    }

    /// Aligns already-computed return series without changing their units.
    ///
    /// [`AlignmentPolicy::InnerJoin`] keeps the common timestamps.
    /// [`AlignmentPolicy::OuterForwardFill`] takes the union of timestamps,
    /// drops timestamps before the last asset's first observation, and fills a
    /// missing return with `0.0`; this is consistent with carrying the previous
    /// price forward. A panel may hold a single aligned observation.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InsufficientData`] when the universe is empty or
    /// the aligned panel has no timestamps, and
    /// [`KurumiError::InvalidParameter`] for duplicate symbols, invalid
    /// observations, or a non-finite value.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let a = AssetSeries::new("A", vec![(at(0), 0.01), (at(2), 0.02)])?;
    /// let b = AssetSeries::new("B", vec![(at(0), -0.01), (at(1), 0.03), (at(2), 0.00)])?;
    /// let panel = ReturnMatrix::from_return_series(
    ///     &[a, b],
    ///     AlignmentPolicy::OuterForwardFill,
    /// )?;
    /// // Day 1 has no observation for A, so its return is filled with zero.
    /// assert_eq!(panel.dimensions(), (3, 2));
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_return_series(
        series: &[AssetSeries],
        alignment: AlignmentPolicy,
    ) -> Result<Self, KurumiError> {
        let prepared = prepare_universe(series)?;
        let aligned = align(&prepared, alignment, true)?;
        if aligned.timestamps.is_empty() {
            return Err(KurumiError::InsufficientData {
                required: 1,
                available: 0,
            });
        }
        let columns = prepared.len();
        let mut values = Vec::with_capacity(aligned.timestamps.len() * columns);
        for row in &aligned.values {
            for value in row {
                if !value.is_finite() {
                    return Err(KurumiError::InvalidParameter {
                        parameter: "series",
                        reason: "must contain finite returns".to_owned(),
                    });
                }
                values.push(*value);
            }
        }
        let symbols = prepared
            .iter()
            .map(|asset| asset.symbol.clone())
            .collect::<Vec<_>>();
        let rows = aligned.timestamps.len();
        Self::new(
            symbols,
            aligned.timestamps,
            DMatrix::from_row_slice(rows, columns, &values),
        )
    }

    /// Returns the asset symbols in column order.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let a = AssetSeries::new("A", vec![(at(0), 0.01), (at(1), 0.02)])?;
    /// let panel = ReturnMatrix::from_return_series(&[a], AlignmentPolicy::InnerJoin)?;
    /// assert_eq!(panel.symbols(), ["A"]);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }

    /// Returns the interval-end timestamps in row order.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let a = AssetSeries::new("A", vec![(at(0), 0.01), (at(1), 0.02)])?;
    /// let panel = ReturnMatrix::from_return_series(&[a], AlignmentPolicy::InnerJoin)?;
    /// assert_eq!(panel.timestamps().len(), 2);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn timestamps(&self) -> &[DateTime<Utc>] {
        &self.timestamps
    }

    /// Returns the underlying `(T, N)` return matrix.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let a = AssetSeries::new("A", vec![(at(0), 0.01), (at(1), 0.02)])?;
    /// let panel = ReturnMatrix::from_return_series(&[a], AlignmentPolicy::InnerJoin)?;
    /// assert_eq!(panel.data().shape(), (2, 1));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn data(&self) -> &DMatrix<f64> {
        &self.data
    }

    /// Returns `(observations, assets)`, the shape of the return matrix.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let a = AssetSeries::new("A", vec![(at(0), 0.01), (at(1), 0.02)])?;
    /// let panel = ReturnMatrix::from_return_series(&[a], AlignmentPolicy::InnerJoin)?;
    /// assert_eq!(panel.dimensions(), (2, 1));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn dimensions(&self) -> (usize, usize) {
        self.data.shape()
    }

    /// Returns the column index of `symbol`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the symbol is unknown.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let a = AssetSeries::new("A", vec![(at(0), 0.01), (at(1), 0.02)])?;
    /// let panel = ReturnMatrix::from_return_series(&[a], AlignmentPolicy::InnerJoin)?;
    /// assert_eq!(panel.asset_index("A")?, 0);
    /// # Ok(())
    /// # }
    /// ```
    pub fn asset_index(&self, symbol: &str) -> Result<usize, KurumiError> {
        self.symbols
            .iter()
            .position(|candidate| candidate == symbol)
            .ok_or_else(|| KurumiError::InvalidParameter {
                parameter: "symbol",
                reason: format!("unknown symbol `{symbol}`"),
            })
    }

    /// Returns one asset's return column as a vector in time order.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the symbol is unknown.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let a = AssetSeries::new("A", vec![(at(0), 0.01), (at(1), 0.02)])?;
    /// let panel = ReturnMatrix::from_return_series(&[a], AlignmentPolicy::InnerJoin)?;
    /// assert_eq!(panel.column("A")?.len(), 2);
    /// # Ok(())
    /// # }
    /// ```
    pub fn column(&self, symbol: &str) -> Result<DVector<f64>, KurumiError> {
        let index = self.asset_index(symbol)?;
        self.column_by_index(index)
    }

    /// Returns the column at `index` as a vector in time order.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the index is out of bounds.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let a = AssetSeries::new("A", vec![(at(0), 0.01), (at(1), 0.02)])?;
    /// let panel = ReturnMatrix::from_return_series(&[a], AlignmentPolicy::InnerJoin)?;
    /// assert_eq!(panel.column_by_index(0)?.len(), 2);
    /// # Ok(())
    /// # }
    /// ```
    pub fn column_by_index(&self, index: usize) -> Result<DVector<f64>, KurumiError> {
        if index >= self.symbols.len() {
            return Err(KurumiError::InvalidParameter {
                parameter: "index",
                reason: format!(
                    "column index {index} is out of bounds for {} assets",
                    self.symbols.len()
                ),
            });
        }
        Ok(DVector::from_iterator(
            self.data.nrows(),
            self.data.column(index).iter().copied(),
        ))
    }

    /// Returns one time observation as a vector in asset order.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the index is out of bounds.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let a = AssetSeries::new("A", vec![(at(0), 0.01), (at(1), 0.02)])?;
    /// let panel = ReturnMatrix::from_return_series(&[a], AlignmentPolicy::InnerJoin)?;
    /// assert_eq!(panel.row(0)?.len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    pub fn row(&self, index: usize) -> Result<DVector<f64>, KurumiError> {
        if index >= self.data.nrows() {
            return Err(KurumiError::InvalidParameter {
                parameter: "index",
                reason: format!(
                    "row index {index} is out of bounds for {} observations",
                    self.data.nrows()
                ),
            });
        }
        Ok(DVector::from_iterator(
            self.data.ncols(),
            self.data.row(index).iter().copied(),
        ))
    }

    /// Returns the arithmetic mean return of every asset in column order.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InsufficientData`] when there are no observations.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let a = AssetSeries::new("A", vec![(at(0), 0.01), (at(1), 0.03)])?;
    /// let panel = ReturnMatrix::from_return_series(&[a], AlignmentPolicy::InnerJoin)?;
    /// assert!((panel.mean_returns()?.get(0).copied().unwrap_or_default() - 0.02).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    pub fn mean_returns(&self) -> Result<DVector<f64>, KurumiError> {
        let mut means = Vec::with_capacity(self.symbols.len());
        for index in 0..self.symbols.len() {
            let column = self.column_by_index(index)?;
            means.push(mean(column.as_slice())?);
        }
        Ok(DVector::from_vec(means))
    }

    /// Estimates the `(N, N)` sample covariance matrix (`ddof = 1`).
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InsufficientData`] when there are fewer than two
    /// observations and [`KurumiError::InvalidParameter`] when a value is not
    /// finite. The constructor guarantees both, so this is a defensive check.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let a = AssetSeries::new("A", vec![(at(0), 0.01), (at(1), 0.03)])?;
    /// let panel = ReturnMatrix::from_return_series(&[a], AlignmentPolicy::InnerJoin)?;
    /// assert_eq!(panel.covariance_matrix()?.shape(), (1, 1));
    /// # Ok(())
    /// # }
    /// ```
    pub fn covariance_matrix(&self) -> Result<DMatrix<f64>, KurumiError> {
        SampleCovariance.estimate(&self.data)
    }

    /// Estimates the `(N, N)` Pearson correlation matrix.
    ///
    /// The diagonal is pinned to exactly `1.0` and off-diagonal entries are
    /// clamped to `[-1.0, 1.0]`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InsufficientData`] when there are fewer than two
    /// observations and [`KurumiError::InvalidParameter`] when a column has zero
    /// variance.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::chrono::{DateTime, Utc};
    /// use kurumi_portfolio::{
    ///     AlignmentPolicy, AssetSeries, ReturnMatrix,
    /// };
    ///
    /// # fn at(day: i64) -> DateTime<Utc> {
    /// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    /// # }
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let a = AssetSeries::new("A", vec![(at(0), 0.01), (at(1), 0.03)])?;
    /// let b = AssetSeries::new("B", vec![(at(0), 0.02), (at(1), 0.06)])?;
    /// let panel = ReturnMatrix::from_return_series(&[a, b], AlignmentPolicy::InnerJoin)?;
    /// let correlation = panel.correlation_matrix()?;
    /// let off_diagonal = correlation.get((0, 1)).copied().unwrap_or_default();
    /// assert!((off_diagonal - 1.0).abs() < 1e-12);
    /// # Ok(())
    /// # }
    /// ```
    pub fn correlation_matrix(&self) -> Result<DMatrix<f64>, KurumiError> {
        correlation_matrix(&self.data)
    }
}

/// A validated series together with a timestamp-ordered observation map.
struct PreparedSeries {
    symbol: String,
    observations: BTreeMap<DateTime<Utc>, f64>,
}

/// A panel after alignment: `values[row][column]` with `row` chronological.
struct AlignedPanel {
    timestamps: Vec<DateTime<Utc>>,
    values: Vec<Vec<f64>>,
}

fn prepare_universe(series: &[AssetSeries]) -> Result<Vec<PreparedSeries>, KurumiError> {
    if series.is_empty() {
        return Err(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        });
    }
    let mut symbols = BTreeSet::new();
    let mut prepared = Vec::with_capacity(series.len());
    for asset in series {
        asset.validate()?;
        if !symbols.insert(asset.symbol.clone()) {
            return Err(KurumiError::InvalidParameter {
                parameter: "series",
                reason: format!("duplicate symbol `{}`", asset.symbol),
            });
        }
        let observations = asset
            .observations
            .iter()
            .map(|(timestamp, value)| (*timestamp, *value))
            .collect();
        prepared.push(PreparedSeries {
            symbol: asset.symbol.clone(),
            observations,
        });
    }
    Ok(prepared)
}

/// Aligns prepared series. When `fill_returns` is true, missing observations
/// after the start of the panel are filled with zero; otherwise the last price
/// is carried forward.
fn align(
    prepared: &[PreparedSeries],
    policy: AlignmentPolicy,
    fill_returns: bool,
) -> Result<AlignedPanel, KurumiError> {
    match policy {
        AlignmentPolicy::InnerJoin => align_inner(prepared),
        AlignmentPolicy::OuterForwardFill => align_outer(prepared, fill_returns),
    }
}

fn align_inner(prepared: &[PreparedSeries]) -> Result<AlignedPanel, KurumiError> {
    let mut common = prepared
        .first()
        .map(|asset| asset.observations.keys().cloned().collect::<BTreeSet<_>>())
        .ok_or(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        })?;
    for asset in prepared.iter().skip(1) {
        common.retain(|timestamp| asset.observations.contains_key(timestamp));
    }
    let timestamps = common.into_iter().collect::<Vec<_>>();
    let mut values = Vec::with_capacity(timestamps.len());
    for timestamp in &timestamps {
        let mut row = Vec::with_capacity(prepared.len());
        for asset in prepared {
            let value = asset
                .observations
                .get(timestamp)
                .copied()
                .ok_or_else(panel_error)?;
            row.push(value);
        }
        values.push(row);
    }
    Ok(AlignedPanel { timestamps, values })
}

fn align_outer(
    prepared: &[PreparedSeries],
    fill_returns: bool,
) -> Result<AlignedPanel, KurumiError> {
    let start = prepared
        .iter()
        .filter_map(|asset| asset.observations.keys().next().cloned())
        .max()
        .ok_or(KurumiError::InsufficientData {
            required: 1,
            available: 0,
        })?;
    let mut union = BTreeSet::new();
    for asset in prepared {
        union.extend(asset.observations.keys().cloned());
    }
    let mut timestamps = Vec::new();
    let mut values = Vec::new();
    for timestamp in union {
        if timestamp < start {
            continue;
        }
        let mut row = Vec::with_capacity(prepared.len());
        for asset in prepared {
            let value = if fill_returns {
                asset.observations.get(&timestamp).copied().unwrap_or(0.0)
            } else {
                asset
                    .observations
                    .range(..=&timestamp)
                    .next_back()
                    .map(|(_, value)| *value)
                    .ok_or_else(panel_error)?
            };
            row.push(value);
        }
        timestamps.push(timestamp);
        values.push(row);
    }
    Ok(AlignedPanel { timestamps, values })
}

fn validate_prices(panel: &AlignedPanel) -> Result<(), KurumiError> {
    for row in &panel.values {
        for value in row {
            if !value.is_finite() || *value <= 0.0 {
                return Err(KurumiError::InvalidParameter {
                    parameter: "series",
                    reason: "prices must be finite and strictly positive".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn panel_error() -> KurumiError {
    KurumiError::InvalidParameter {
        parameter: "series",
        reason: "aligned panel shape is inconsistent".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn at(day: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    }

    fn price_series(symbol: &str, values: &[(i64, f64)]) -> AssetSeries {
        AssetSeries::new(
            symbol,
            values
                .iter()
                .map(|(day, value)| (at(*day), *value))
                .collect(),
        )
        .expect("valid price series")
    }

    fn return_series(symbol: &str, values: &[(i64, f64)]) -> AssetSeries {
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
    fn inner_join_keeps_only_common_timestamps() {
        // A is missing Tuesday, so the inner join drops Tuesday entirely.
        let a = price_series("A", &[(0, 100.0), (2, 110.0), (3, 121.0)]);
        let b = price_series("B", &[(0, 50.0), (1, 52.0), (2, 55.0), (3, 55.0)]);
        let panel = ReturnMatrix::from_price_series(
            &[a, b],
            AlignmentPolicy::InnerJoin,
            ReturnMethod::Simple,
        )
        .expect("inner join succeeds");

        assert_eq!(panel.timestamps(), [at(2), at(3)]);
        assert_eq!(panel.dimensions(), (2, 2));
        assert_relative_eq!(
            panel.data().get((0, 0)).copied().unwrap_or_default(),
            0.10,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            panel.data().get((0, 1)).copied().unwrap_or_default(),
            0.10,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            panel.data().get((1, 0)).copied().unwrap_or_default(),
            0.10,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            panel.data().get((1, 1)).copied().unwrap_or_default(),
            0.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn outer_forward_fill_carries_prices_over_missing_timestamps() {
        let a = price_series("A", &[(0, 100.0), (2, 110.0), (3, 121.0)]);
        let b = price_series("B", &[(0, 50.0), (1, 52.0), (2, 55.0), (3, 55.0)]);
        let panel = ReturnMatrix::from_price_series(
            &[a, b],
            AlignmentPolicy::OuterForwardFill,
            ReturnMethod::Simple,
        )
        .expect("outer join succeeds");

        assert_eq!(panel.timestamps(), [at(1), at(2), at(3)]);
        // A carries 100 into Tuesday, so its Tuesday return is zero.
        assert_relative_eq!(
            panel.data().get((0, 0)).copied().unwrap_or_default(),
            0.0,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            panel.data().get((0, 1)).copied().unwrap_or_default(),
            0.04,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            panel.data().get((1, 0)).copied().unwrap_or_default(),
            0.10,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            panel.data().get((1, 1)).copied().unwrap_or_default(),
            55.0 / 52.0 - 1.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn outer_forward_fill_drops_leading_partial_timestamps() {
        let a = price_series("A", &[(2, 110.0), (3, 121.0), (4, 133.1)]);
        let b = price_series(
            "B",
            &[(0, 50.0), (1, 52.0), (2, 55.0), (3, 55.0), (4, 55.0)],
        );
        let panel = ReturnMatrix::from_price_series(
            &[a, b],
            AlignmentPolicy::OuterForwardFill,
            ReturnMethod::Simple,
        )
        .expect("outer join succeeds");

        // Everything before A's first observation at day 2 is dropped.
        assert_eq!(panel.timestamps(), [at(3), at(4)]);
        assert_relative_eq!(
            panel.data().get((0, 0)).copied().unwrap_or_default(),
            0.10,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            panel.data().get((0, 1)).copied().unwrap_or_default(),
            0.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn log_returns_match_the_continuous_compounding_definition() {
        let a = price_series("A", &[(0, 100.0), (1, 110.0), (2, 99.0)]);
        let panel =
            ReturnMatrix::from_price_series(&[a], AlignmentPolicy::InnerJoin, ReturnMethod::Log)
                .expect("log returns succeed");

        assert_relative_eq!(
            panel.data().get((0, 0)).copied().unwrap_or_default(),
            (1.1_f64).ln(),
            epsilon = 1e-15
        );
        assert_relative_eq!(
            panel.data().get((1, 0)).copied().unwrap_or_default(),
            (0.9_f64).ln(),
            epsilon = 1e-15
        );
    }

    #[test]
    fn return_series_outer_fill_uses_zero_for_missing_periods() {
        let a = return_series("A", &[(0, 0.01), (2, 0.03)]);
        let b = return_series("B", &[(0, -0.01), (1, 0.02), (2, 0.00)]);
        let panel = ReturnMatrix::from_return_series(&[a, b], AlignmentPolicy::OuterForwardFill)
            .expect("return alignment succeeds");

        assert_eq!(panel.dimensions(), (3, 2));
        assert_eq!(panel.timestamps(), [at(0), at(1), at(2)]);
        assert_relative_eq!(
            panel.data().get((1, 0)).copied().unwrap_or_default(),
            0.0,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            panel.data().get((2, 0)).copied().unwrap_or_default(),
            0.03,
            epsilon = 1e-15
        );
    }

    #[test]
    fn inner_join_with_one_common_timestamp_is_insufficient_for_returns() {
        let a = price_series("A", &[(0, 100.0), (1, 110.0)]);
        let b = price_series("B", &[(0, 50.0)]);
        let error = ReturnMatrix::from_price_series(
            &[a, b],
            AlignmentPolicy::InnerJoin,
            ReturnMethod::Simple,
        )
        .expect_err("one aligned timestamp cannot produce returns");

        assert!(matches!(
            error,
            KurumiError::InsufficientData {
                required: 2,
                available: 1
            }
        ));
    }

    #[test]
    fn duplicate_symbols_and_duplicate_timestamps_are_rejected() {
        let a = price_series("A", &[(0, 100.0), (1, 110.0), (2, 121.0)]);
        let duplicate = price_series("A", &[(0, 100.0), (1, 110.0), (2, 121.0)]);
        let error = ReturnMatrix::from_price_series(
            &[a, duplicate],
            AlignmentPolicy::InnerJoin,
            ReturnMethod::Simple,
        )
        .expect_err("duplicate symbols are rejected");
        assert!(matches!(error, KurumiError::InvalidParameter { .. }));

        let repeated = AssetSeries {
            symbol: "B".to_owned(),
            observations: vec![(at(0), 1.0), (at(0), 2.0)],
        };
        assert!(repeated.validate().is_err());
    }

    #[test]
    fn non_positive_prices_are_rejected() {
        let a = price_series("A", &[(0, 100.0), (1, 0.0), (2, 121.0)]);
        let error =
            ReturnMatrix::from_price_series(&[a], AlignmentPolicy::InnerJoin, ReturnMethod::Simple)
                .expect_err("zero prices are rejected");
        assert!(matches!(error, KurumiError::InvalidParameter { .. }));
    }

    #[test]
    fn a_single_return_observation_is_a_valid_panel() {
        // A two-price inner join yields one return observation.
        let a = price_series("A", &[(0, 100.0), (2, 110.0)]);
        let b = price_series("B", &[(0, 50.0), (1, 52.0), (2, 55.0)]);
        let panel = ReturnMatrix::from_price_series(
            &[a, b],
            AlignmentPolicy::InnerJoin,
            ReturnMethod::Simple,
        )
        .expect("a single aligned return is a valid panel");
        assert_eq!(panel.dimensions(), (1, 2));
        // Sample covariance still needs two observations.
        assert!(panel.covariance_matrix().is_err());

        // The same rule applies to already-computed returns.
        let first = return_series("A", &[(0, 0.01)]);
        let second = return_series("B", &[(0, 0.02)]);
        let panel = ReturnMatrix::from_return_series(&[first, second], AlignmentPolicy::InnerJoin)
            .expect("one return timestamp is valid");
        assert_eq!(panel.dimensions(), (1, 2));
    }

    #[test]
    fn column_and_matrix_queries_expose_the_panel_contract() {
        let a = return_series("A", &[(0, 0.01), (1, 0.02), (2, 0.03)]);
        let b = return_series("B", &[(0, 0.02), (1, 0.04), (2, 0.06)]);
        let panel = ReturnMatrix::from_return_series(&[a, b], AlignmentPolicy::InnerJoin)
            .expect("panel is valid");

        assert_eq!(panel.asset_index("B").unwrap(), 1);
        assert!(panel.asset_index("C").is_err());
        assert_eq!(panel.column("A").unwrap().len(), 3);
        assert_eq!(panel.row(1).unwrap().len(), 2);
        assert_relative_eq!(
            panel
                .mean_returns()
                .unwrap()
                .get(0)
                .copied()
                .unwrap_or_default(),
            0.02,
            epsilon = 1e-15
        );
        assert_eq!(panel.covariance_matrix().unwrap().shape(), (2, 2));
        assert_eq!(panel.correlation_matrix().unwrap().shape(), (2, 2));
    }
}
