//! Portfolio weights, performance statistics, and risk decomposition.
//!
//! [`PortfolioWeights`] pairs a symbol universe with a weight vector and exposes
//! the validation checks an allocation needs: a finite sum, gross leverage, and
//! long-only status. The remaining functions take a covariance matrix (and,
//! for expected return, a mean-return vector) in the same asset order as the
//! weights, or a [`ReturnMatrix`] whose symbols are matched by name.
//!
//! The core identities are:
//!
//! ```text
//! mu_p    = w^T mu
//! sigma_p = sqrt(w^T Sigma w)
//! MCTR_i  = (Sigma w)_i / sigma_p
//! CTR_i   = w_i * MCTR_i
//! PCTR_i  = CTR_i / sigma_p
//! DR      = (w^T sigma) / sigma_p
//! ```
//!
//! Euler's theorem makes `sum(CTR) = sigma_p` and `sum(PCTR) = 1` for any
//! portfolio with strictly positive volatility. Sample moments use Bessel's
//! correction (`ddof = 1`) through `kurumi-math`.
//!
//! Parametric VaR/CVaR assumes normally distributed portfolio returns and uses
//! the closed-form normal quantile. Historical simulation reads the empirical
//! distribution of `R w`. Both report additive component allocations: the
//! parametric components are the exact Euler contributions, while historical
//! components are the average asset P&L over the VaR tail rescaled to the
//! reported total, because historical VaR has no unique additive decomposition.
//!
//! # References
//!
//! - H. M. Markowitz, "Portfolio Selection", *Journal of Finance* 7(1), 1952.
//! - P. Jorion, *Value at Risk: The New Benchmark for Managing Financial Risk*,
//!   3rd edition, McGraw-Hill, 2007.
//! - R. T. Rockafellar and S. Uryasev, "Optimization of Conditional
//!   Value-at-Risk", *Journal of Risk* 2(3), 2000.

use std::collections::BTreeSet;

use kurumi_core::KurumiError;
use kurumi_math::standard_normal_pdf;
use kurumi_risk::{
    historical_expected_shortfall, historical_var, parametric_expected_shortfall, parametric_var,
};
use nalgebra::{DMatrix, DVector};

use crate::panel::ReturnMatrix;

/// A symbol universe paired with its portfolio weights.
///
/// The weights are dimensionless fractions of portfolio value: `0.25` is a 25%
/// allocation and `-0.10` is a 10% short. The constructor enforces a matching,
/// unique symbol universe and finite weights; the allocation-level checks
/// ([`PortfolioWeights::sum`], [`PortfolioWeights::gross_leverage`],
/// [`PortfolioWeights::is_long_only`]) are separate because leverage and net
/// exposure are strategy choices rather than validity conditions.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::DVector;
/// use kurumi_portfolio::PortfolioWeights;
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let weights = PortfolioWeights::new(
///     vec!["AAA".to_owned(), "BBB".to_owned()],
///     DVector::from_vec(vec![0.6, 0.4]),
/// )?;
/// assert!((weights.sum() - 1.0).abs() < 1e-15);
/// assert!((weights.gross_leverage() - 1.0).abs() < 1e-15);
/// assert!(weights.is_long_only());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct PortfolioWeights {
    symbols: Vec<String>,
    weights: DVector<f64>,
}

impl PortfolioWeights {
    /// Creates a weight vector and validates its structure.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InsufficientData`] for an empty universe,
    /// [`KurumiError::InvalidParameter`] for a length mismatch, a duplicate or
    /// empty symbol, or a non-finite weight.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::nalgebra::DVector;
    /// use kurumi_portfolio::PortfolioWeights;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let weights = PortfolioWeights::new(
    ///     vec!["AAA".to_owned(), "BBB".to_owned()],
    ///     DVector::from_vec(vec![0.5, 0.5]),
    /// )?;
    /// assert_eq!(weights.len(), 2);
    ///
    /// assert!(
    ///     PortfolioWeights::new(
    ///         vec!["AAA".to_owned()],
    ///         DVector::from_vec(vec![1.0, 0.0]),
    ///     )
    ///     .is_err()
    /// );
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(symbols: Vec<String>, weights: DVector<f64>) -> Result<Self, KurumiError> {
        if symbols.is_empty() {
            return Err(KurumiError::InsufficientData {
                required: 1,
                available: 0,
            });
        }
        if symbols.len() != weights.len() {
            return Err(KurumiError::InvalidParameter {
                parameter: "weights",
                reason: format!(
                    "length {} does not match the {} symbols",
                    weights.len(),
                    symbols.len()
                ),
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
        for (index, weight) in weights.iter().enumerate() {
            if !weight.is_finite() {
                return Err(KurumiError::InvalidParameter {
                    parameter: "weights",
                    reason: format!("weight at index {index} must be finite"),
                });
            }
        }
        Ok(Self { symbols, weights })
    }

    /// Revalidates the weights.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`PortfolioWeights::new`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::nalgebra::DVector;
    /// use kurumi_portfolio::PortfolioWeights;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let weights = PortfolioWeights::new(
    ///     vec!["AAA".to_owned()],
    ///     DVector::from_vec(vec![1.0]),
    /// )?;
    /// assert!(weights.validate().is_ok());
    /// # Ok(())
    /// # }
    /// ```
    pub fn validate(&self) -> Result<(), KurumiError> {
        Self::new(self.symbols.clone(), self.weights.clone()).map(|_| ())
    }

    /// Returns the asset symbols in weight order.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::nalgebra::DVector;
    /// use kurumi_portfolio::PortfolioWeights;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let weights = PortfolioWeights::new(
    ///     vec!["AAA".to_owned(), "BBB".to_owned()],
    ///     DVector::from_vec(vec![0.3, 0.7]),
    /// )?;
    /// assert_eq!(weights.symbols(), ["AAA", "BBB"]);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }

    /// Returns the weight vector in symbol order.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::nalgebra::DVector;
    /// use kurumi_portfolio::PortfolioWeights;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let weights = PortfolioWeights::new(
    ///     vec!["AAA".to_owned()],
    ///     DVector::from_vec(vec![1.0]),
    /// )?;
    /// assert_eq!(weights.weights().len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn weights(&self) -> &DVector<f64> {
        &self.weights
    }

    /// Returns the number of assets.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::nalgebra::DVector;
    /// use kurumi_portfolio::PortfolioWeights;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let weights = PortfolioWeights::new(
    ///     vec!["AAA".to_owned(), "BBB".to_owned()],
    ///     DVector::from_vec(vec![0.2, 0.8]),
    /// )?;
    /// assert_eq!(weights.len(), 2);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    /// Returns `true` when the weight vector has no assets.
    ///
    /// A validated instance is never empty; the method exists for generic code
    /// that pairs `len` with `is_empty`.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::nalgebra::DVector;
    /// use kurumi_portfolio::PortfolioWeights;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let weights = PortfolioWeights::new(
    ///     vec!["AAA".to_owned()],
    ///     DVector::from_vec(vec![1.0]),
    /// )?;
    /// assert!(!weights.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    /// Returns the weight of `symbol`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the symbol is unknown.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::nalgebra::DVector;
    /// use kurumi_portfolio::PortfolioWeights;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let weights = PortfolioWeights::new(
    ///     vec!["AAA".to_owned(), "BBB".to_owned()],
    ///     DVector::from_vec(vec![0.25, 0.75]),
    /// )?;
    /// assert!((weights.weight("BBB")? - 0.75).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    pub fn weight(&self, symbol: &str) -> Result<f64, KurumiError> {
        let index = self
            .symbols
            .iter()
            .position(|candidate| candidate == symbol)
            .ok_or_else(|| KurumiError::InvalidParameter {
                parameter: "symbol",
                reason: format!("unknown symbol `{symbol}`"),
            })?;
        self.weights
            .get(index)
            .copied()
            .ok_or_else(|| KurumiError::InvalidParameter {
                parameter: "symbol",
                reason: format!("weight for `{symbol}` is missing"),
            })
    }

    /// Returns the net weight `sum(w_i)`.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::nalgebra::DVector;
    /// use kurumi_portfolio::PortfolioWeights;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let weights = PortfolioWeights::new(
    ///     vec!["AAA".to_owned(), "BBB".to_owned()],
    ///     DVector::from_vec(vec![1.2, -0.2]),
    /// )?;
    /// assert!((weights.sum() - 1.0).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn sum(&self) -> f64 {
        self.weights.sum()
    }

    /// Returns the gross leverage `sum(|w_i|)`.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::nalgebra::DVector;
    /// use kurumi_portfolio::PortfolioWeights;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let weights = PortfolioWeights::new(
    ///     vec!["AAA".to_owned(), "BBB".to_owned()],
    ///     DVector::from_vec(vec![1.2, -0.2]),
    /// )?;
    /// assert!((weights.gross_leverage() - 1.4).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn gross_leverage(&self) -> f64 {
        self.weights.iter().map(|weight| weight.abs()).sum()
    }

    /// Returns `true` when every weight is non-negative.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::nalgebra::DVector;
    /// use kurumi_portfolio::PortfolioWeights;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let long_only = PortfolioWeights::new(
    ///     vec!["AAA".to_owned(), "BBB".to_owned()],
    ///     DVector::from_vec(vec![0.6, 0.4]),
    /// )?;
    /// let hedged = PortfolioWeights::new(
    ///     vec!["AAA".to_owned(), "BBB".to_owned()],
    ///     DVector::from_vec(vec![1.1, -0.1]),
    /// )?;
    /// assert!(long_only.is_long_only());
    /// assert!(!hedged.is_long_only());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn is_long_only(&self) -> bool {
        self.weights.iter().all(|weight| *weight >= 0.0)
    }

    /// Returns `true` when the net weight is within `tolerance` of `1.0`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `tolerance` is not finite
    /// or negative.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::nalgebra::DVector;
    /// use kurumi_portfolio::PortfolioWeights;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let weights = PortfolioWeights::new(
    ///     vec!["AAA".to_owned(), "BBB".to_owned()],
    ///     DVector::from_vec(vec![0.6, 0.4]),
    /// )?;
    /// assert!(weights.is_fully_invested(1e-12)?);
    /// # Ok(())
    /// # }
    /// ```
    pub fn is_fully_invested(&self, tolerance: f64) -> Result<bool, KurumiError> {
        if !tolerance.is_finite() || tolerance < 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "tolerance",
                reason: "must be finite and non-negative".to_owned(),
            });
        }
        Ok((self.sum() - 1.0).abs() <= tolerance)
    }

    /// Reorders the weights into `symbols` order.
    ///
    /// This is the bridge between a named allocation and a matrix whose columns
    /// are in a particular order, such as the output of
    /// [`ReturnMatrix::covariance_matrix`]. Every requested symbol must be
    /// present exactly once.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when a requested symbol is
    /// unknown, when `symbols` contains a duplicate, or when the two universes
    /// have different sizes.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::nalgebra::DVector;
    /// use kurumi_portfolio::PortfolioWeights;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// let weights = PortfolioWeights::new(
    ///     vec!["AAA".to_owned(), "BBB".to_owned()],
    ///     DVector::from_vec(vec![0.6, 0.4]),
    /// )?;
    /// let reordered = weights.aligned_to(&["BBB".to_owned(), "AAA".to_owned()])?;
    /// assert!((reordered[0] - 0.4).abs() < 1e-15);
    /// assert!((reordered[1] - 0.6).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    pub fn aligned_to(&self, symbols: &[String]) -> Result<DVector<f64>, KurumiError> {
        if symbols.len() != self.symbols.len() {
            return Err(KurumiError::InvalidParameter {
                parameter: "symbols",
                reason: format!(
                    "weights cover {} assets but {} were requested",
                    self.symbols.len(),
                    symbols.len()
                ),
            });
        }
        let mut seen = BTreeSet::new();
        let mut aligned = Vec::with_capacity(symbols.len());
        for symbol in symbols {
            if !seen.insert(symbol.clone()) {
                return Err(KurumiError::InvalidParameter {
                    parameter: "symbols",
                    reason: format!("duplicate symbol `{symbol}`"),
                });
            }
            let index = self
                .symbols
                .iter()
                .position(|candidate| candidate == symbol)
                .ok_or_else(|| KurumiError::InvalidParameter {
                    parameter: "symbols",
                    reason: format!("unknown symbol `{symbol}`"),
                })?;
            let weight =
                self.weights
                    .get(index)
                    .copied()
                    .ok_or_else(|| KurumiError::InvalidParameter {
                        parameter: "symbols",
                        reason: format!("weight for `{symbol}` is missing"),
                    })?;
            aligned.push(weight);
        }
        Ok(DVector::from_vec(aligned))
    }
}

/// Euler risk contributions of a portfolio.
///
/// The fields are the volatility decomposition
/// `sigma_p = sum(CTR)` and its normalized form `1 = sum(PCTR)`. A strictly
/// positive portfolio volatility is required; a zero-volatility portfolio has
/// no well-defined marginal contributions.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{PortfolioWeights, risk_decomposition};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let weights = PortfolioWeights::new(
///     vec!["AAA".to_owned(), "BBB".to_owned()],
///     DVector::from_vec(vec![0.6, 0.4]),
/// )?;
/// let covariance = DMatrix::from_row_slice(2, 2, &[0.04, 0.006, 0.006, 0.09]);
/// let decomposition = risk_decomposition(&weights, &covariance)?;
/// assert!((decomposition.ctr.sum() - decomposition.portfolio_volatility).abs() < 1e-10);
/// assert!((decomposition.pctr.sum() - 1.0).abs() < 1e-10);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RiskDecomposition {
    /// Portfolio volatility `sqrt(w^T Sigma w)`.
    pub portfolio_volatility: f64,
    /// Marginal contribution to risk `(Sigma w)_i / sigma_p`.
    pub mctr: DVector<f64>,
    /// Component contribution to risk `w_i * MCTR_i`.
    pub ctr: DVector<f64>,
    /// Percentage contribution to risk `CTR_i / sigma_p`.
    pub pctr: DVector<f64>,
}

/// Portfolio Value at Risk, Expected Shortfall, and additive components.
///
/// `var` and `cvar` are non-negative loss magnitudes in return units. The two
/// component vectors are additive allocations of the reported totals:
/// `sum(component_var) = var` and `sum(component_cvar) = cvar`.
///
/// For the parametric estimator the components are the exact Euler
/// contributions of the normal VaR/CVaR. For historical simulation they are the
/// average asset P&L over the `1 - confidence_level` worst portfolio
/// observations, rescaled to the reported total; historical VaR has no unique
/// additive decomposition, so this allocation is stated explicitly rather than
/// implied.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{PortfolioWeights, parametric_portfolio_var};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let weights = PortfolioWeights::new(
///     vec!["AAA".to_owned(), "BBB".to_owned()],
///     DVector::from_vec(vec![0.6, 0.4]),
/// )?;
/// let means = DVector::from_vec(vec![0.0005, 0.0008]);
/// let covariance = DMatrix::from_row_slice(2, 2, &[0.0004, 0.00006, 0.00006, 0.0009]);
/// let result = parametric_portfolio_var(&weights, &means, &covariance, 0.95)?;
/// assert!((result.component_var.sum() - result.var).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct PortfolioVar {
    /// Value at Risk as a non-negative loss magnitude.
    pub var: f64,
    /// Expected Shortfall (CVaR) as a non-negative loss magnitude.
    pub cvar: f64,
    /// Additive component VaR, one entry per asset in weight order.
    pub component_var: DVector<f64>,
    /// Additive component CVaR, one entry per asset in weight order.
    pub component_cvar: DVector<f64>,
}

/// Computes the portfolio expected return `w^T mu`.
///
/// `expected_returns[i]` must correspond to `weights.symbols()[i]`.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when `expected_returns` has the
/// wrong length or contains a non-finite value.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::DVector;
/// use kurumi_portfolio::{PortfolioWeights, portfolio_expected_return};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let weights = PortfolioWeights::new(
///     vec!["AAA".to_owned(), "BBB".to_owned()],
///     DVector::from_vec(vec![0.6, 0.4]),
/// )?;
/// let means = DVector::from_vec(vec![0.01, 0.02]);
/// let expected = portfolio_expected_return(&weights, &means)?;
/// assert!((expected - 0.014).abs() < 1e-15);
/// # Ok(())
/// # }
/// ```
pub fn portfolio_expected_return(
    weights: &PortfolioWeights,
    expected_returns: &DVector<f64>,
) -> Result<f64, KurumiError> {
    validate_vector("expected_returns", expected_returns, weights.len())?;
    finite(
        weights.weights().dot(expected_returns),
        "portfolio_expected_return",
    )
}

/// Computes the portfolio volatility `sqrt(w^T Sigma w)`.
///
/// `covariance` is an `(N, N)` matrix whose rows and columns follow the same
/// asset order as `weights.symbols()`.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when `covariance` is not a
/// symmetric finite `(N, N)` matrix with non-negative diagonal, or when the
/// quadratic form is materially negative.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{PortfolioWeights, portfolio_volatility};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let weights = PortfolioWeights::new(
///     vec!["AAA".to_owned(), "BBB".to_owned()],
///     DVector::from_vec(vec![0.6, 0.4]),
/// )?;
/// let covariance = DMatrix::from_row_slice(2, 2, &[0.04, 0.006, 0.006, 0.09]);
/// let volatility = portfolio_volatility(&weights, &covariance)?;
/// assert!((volatility - 0.031_68_f64.sqrt()).abs() < 1e-15);
/// # Ok(())
/// # }
/// ```
pub fn portfolio_volatility(
    weights: &PortfolioWeights,
    covariance: &DMatrix<f64>,
) -> Result<f64, KurumiError> {
    let variance = quadratic_variance(weights.weights(), covariance)?;
    Ok(variance.sqrt())
}

/// Computes the annualized Sharpe ratio of the portfolio.
///
/// With `mu_p` the per-period portfolio mean, `sigma_p` the per-period
/// volatility, `rf` the annual risk-free rate, and `m` the number of periods per
/// year:
///
/// ```text
/// Sharpe = (mu_p * m - rf) / (sigma_p * sqrt(m))
/// ```
///
/// `expected_returns` and `covariance` follow the asset order of `weights`.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] for invalid inputs, a
/// non-positive portfolio volatility, a non-finite result, or a
/// `periods_per_year` that is not finite and strictly positive.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{PortfolioWeights, annualized_sharpe_ratio};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let weights = PortfolioWeights::new(
///     vec!["AAA".to_owned()],
///     DVector::from_vec(vec![1.0]),
/// )?;
/// let means = DVector::from_vec(vec![0.001]);
/// let covariance = DMatrix::from_row_slice(1, 1, &[0.0001]);
/// let sharpe = annualized_sharpe_ratio(&weights, &means, &covariance, 0.02, 252.0)?;
/// assert!(sharpe > 0.0);
/// # Ok(())
/// # }
/// ```
pub fn annualized_sharpe_ratio(
    weights: &PortfolioWeights,
    expected_returns: &DVector<f64>,
    covariance: &DMatrix<f64>,
    risk_free_rate: f64,
    periods_per_year: f64,
) -> Result<f64, KurumiError> {
    if !risk_free_rate.is_finite() {
        return Err(KurumiError::InvalidParameter {
            parameter: "risk_free_rate",
            reason: "must be finite".to_owned(),
        });
    }
    if !periods_per_year.is_finite() || periods_per_year <= 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "periods_per_year",
            reason: "must be finite and strictly positive".to_owned(),
        });
    }
    let expected = portfolio_expected_return(weights, expected_returns)?;
    let volatility = portfolio_volatility(weights, covariance)?;
    if volatility <= 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "covariance",
            reason: "portfolio volatility must be strictly positive".to_owned(),
        });
    }
    let numerator = expected * periods_per_year - risk_free_rate;
    finite(
        numerator / (volatility * periods_per_year.sqrt()),
        "annualized_sharpe_ratio",
    )
}

/// Computes the diversification ratio `(w^T sigma) / sigma_p`.
///
/// `sigma` is the vector of per-asset standard deviations taken from the
/// covariance diagonal. The ratio is `1` for a single asset or a perfectly
/// correlated portfolio and rises with diversification. The covariance follows
/// the asset order of `weights`.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] for an invalid covariance matrix or
/// a portfolio volatility that is not strictly positive.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{PortfolioWeights, diversification_ratio};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let weights = PortfolioWeights::new(
///     vec!["AAA".to_owned(), "BBB".to_owned()],
///     DVector::from_vec(vec![0.6, 0.4]),
/// )?;
/// let covariance = DMatrix::from_row_slice(2, 2, &[0.04, 0.006, 0.006, 0.09]);
/// let ratio = diversification_ratio(&weights, &covariance)?;
/// assert!(ratio > 1.0);
/// # Ok(())
/// # }
/// ```
pub fn diversification_ratio(
    weights: &PortfolioWeights,
    covariance: &DMatrix<f64>,
) -> Result<f64, KurumiError> {
    let variance = quadratic_variance(weights.weights(), covariance)?;
    let volatility = variance.sqrt();
    if volatility <= 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "covariance",
            reason: "portfolio volatility must be strictly positive".to_owned(),
        });
    }
    let mut weighted_volatility = 0.0;
    for index in 0..weights.len() {
        let weight = weights
            .weights()
            .get(index)
            .copied()
            .ok_or_else(index_error)?;
        let variance = covariance
            .get((index, index))
            .copied()
            .ok_or_else(index_error)?;
        weighted_volatility += weight * variance.sqrt();
    }
    finite(weighted_volatility / volatility, "diversification_ratio")
}

/// Computes the Euler volatility decomposition of a portfolio.
///
/// The covariance matrix follows the asset order of `weights`; the returned
/// vectors use that same order.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] for an invalid covariance matrix or
/// a portfolio volatility that is not strictly positive, and
/// [`KurumiError::NumericalOverflow`] when a contribution is not finite.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{PortfolioWeights, risk_decomposition};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let weights = PortfolioWeights::new(
///     vec!["AAA".to_owned(), "BBB".to_owned()],
///     DVector::from_vec(vec![0.6, 0.4]),
/// )?;
/// let covariance = DMatrix::from_row_slice(2, 2, &[0.04, 0.006, 0.006, 0.09]);
/// let decomposition = risk_decomposition(&weights, &covariance)?;
/// assert!((decomposition.pctr.sum() - 1.0).abs() < 1e-10);
/// # Ok(())
/// # }
/// ```
pub fn risk_decomposition(
    weights: &PortfolioWeights,
    covariance: &DMatrix<f64>,
) -> Result<RiskDecomposition, KurumiError> {
    let variance = quadratic_variance(weights.weights(), covariance)?;
    let volatility = variance.sqrt();
    if volatility <= 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "covariance",
            reason: "portfolio volatility must be strictly positive for risk decomposition"
                .to_owned(),
        });
    }
    let sigma_weighted = covariance * weights.weights();
    let mctr = sigma_weighted.map(|value| value / volatility);
    let ctr = weights.weights().component_mul(&mctr);
    let pctr = ctr.map(|value| value / volatility);
    if pctr.iter().any(|value| !value.is_finite()) {
        return Err(KurumiError::NumericalOverflow {
            operation: "risk_decomposition",
        });
    }
    Ok(RiskDecomposition {
        portfolio_volatility: volatility,
        mctr,
        ctr,
        pctr,
    })
}

/// Computes parametric portfolio VaR and CVaR from a mean vector and covariance
/// matrix.
///
/// The portfolio return is normal with mean `mu_p = w^T mu` and standard
/// deviation `sigma_p = sqrt(w^T Sigma w)`. With `q = -Phi^-1(1 - c)` the
/// one-sided loss quantile and `alpha = 1 - c`:
///
/// ```text
/// VaR  = max(0, q * sigma_p - mu_p)
/// CVaR = max(0, (phi(q) / alpha) * sigma_p - mu_p)
/// ```
///
/// `confidence_level` must lie strictly between `0.5` and `1.0`. The mean
/// vector and covariance matrix follow the asset order of `weights`.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] for invalid inputs or a
/// non-positive portfolio volatility, and propagates the validation errors of
/// the `kurumi-risk` normal estimators.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::nalgebra::{DMatrix, DVector};
/// use kurumi_portfolio::{PortfolioWeights, parametric_portfolio_var};
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let weights = PortfolioWeights::new(
///     vec!["AAA".to_owned()],
///     DVector::from_vec(vec![1.0]),
/// )?;
/// let means = DVector::from_vec(vec![0.0]);
/// let covariance = DMatrix::from_row_slice(1, 1, &[0.0001]);
/// let result = parametric_portfolio_var(&weights, &means, &covariance, 0.95)?;
/// assert!((result.var - 0.016_448_536_269_514_722).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn parametric_portfolio_var(
    weights: &PortfolioWeights,
    expected_returns: &DVector<f64>,
    covariance: &DMatrix<f64>,
    confidence_level: f64,
) -> Result<PortfolioVar, KurumiError> {
    validate_confidence(confidence_level)?;
    validate_vector("expected_returns", expected_returns, weights.len())?;
    let variance = quadratic_variance(weights.weights(), covariance)?;
    let volatility = variance.sqrt();
    if volatility <= 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "covariance",
            reason: "portfolio volatility must be strictly positive".to_owned(),
        });
    }
    let mean = weights.weights().dot(expected_returns);
    let var = parametric_var(mean, volatility, confidence_level)?;
    let cvar = parametric_expected_shortfall(mean, volatility, confidence_level)?;
    let quantile = parametric_var(0.0, 1.0, confidence_level)?;
    let tail_probability = 1.0 - confidence_level;
    let density = standard_normal_pdf(quantile)?;
    let sigma_weighted = covariance * weights.weights();
    let marginal_volatility = sigma_weighted.map(|value| value / volatility);

    let mut raw_var = Vec::with_capacity(weights.len());
    let mut raw_cvar = Vec::with_capacity(weights.len());
    for index in 0..weights.len() {
        let weight = weights
            .weights()
            .get(index)
            .copied()
            .ok_or_else(index_error)?;
        let asset_mean = expected_returns
            .get(index)
            .copied()
            .ok_or_else(index_error)?;
        let marginal = marginal_volatility
            .get(index)
            .copied()
            .ok_or_else(index_error)?;
        raw_var.push(weight * (quantile * marginal - asset_mean));
        raw_cvar.push(weight * ((density / tail_probability) * marginal - asset_mean));
    }
    let component_var = scale_components(DVector::from_vec(raw_var), var)?;
    let component_cvar = scale_components(DVector::from_vec(raw_cvar), cvar)?;
    Ok(PortfolioVar {
        var,
        cvar,
        component_var,
        component_cvar,
    })
}

/// Computes historical-simulation portfolio VaR and CVaR from a return matrix.
///
/// The portfolio return in each row is `r_p,t = sum_i w_i r_i,t`; VaR and CVaR
/// are read from the empirical distribution exactly as
/// [`kurumi_risk::historical_var`] and
/// [`kurumi_risk::historical_expected_shortfall`] do. Weights are matched to
/// matrix columns by symbol, so their order need not be identical.
///
/// Component allocations use the `ceil(T * (1 - confidence_level))` worst
/// portfolio observations. `component_cvar` is the average asset P&L over that
/// tail and sums to `cvar`; `component_var` rescales the same allocation to the
/// reported `var`. This states an explicit convention because historical VaR
/// has no unique additive decomposition.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when the weights do not match the
/// matrix symbols, when `confidence_level` is not strictly between `0.5` and
/// `1.0`, or when a portfolio return is not finite.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::chrono::{DateTime, Utc};
/// use kurumi_portfolio::nalgebra::DVector;
/// use kurumi_portfolio::{
///     AlignmentPolicy, AssetSeries, PortfolioWeights, ReturnMatrix,
///     historical_portfolio_var,
/// };
///
/// # fn at(day: i64) -> DateTime<Utc> {
/// #     DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
/// # }
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let a = AssetSeries::new("AAA", vec![(at(0), -0.02), (at(1), 0.01), (at(2), 0.02)])?;
/// let b = AssetSeries::new("BBB", vec![(at(0), -0.01), (at(1), 0.00), (at(2), 0.01)])?;
/// let returns = ReturnMatrix::from_return_series(&[a, b], AlignmentPolicy::InnerJoin)?;
/// let weights = PortfolioWeights::new(
///     vec!["BBB".to_owned(), "AAA".to_owned()],
///     DVector::from_vec(vec![0.4, 0.6]),
/// )?;
/// let result = historical_portfolio_var(&returns, &weights, 0.95)?;
/// assert!(result.var >= 0.0);
/// assert!((result.component_var.sum() - result.var).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
pub fn historical_portfolio_var(
    returns: &ReturnMatrix,
    weights: &PortfolioWeights,
    confidence_level: f64,
) -> Result<PortfolioVar, KurumiError> {
    validate_confidence(confidence_level)?;
    let aligned = weights.aligned_to(returns.symbols())?;
    let (observations, assets) = returns.dimensions();
    let mut portfolio_returns = Vec::with_capacity(observations);
    for row in 0..observations {
        let observation = returns.data().row(row);
        let mut value = 0.0;
        for (weight, asset_return) in aligned.iter().zip(observation.iter()) {
            value += weight * asset_return;
        }
        if !value.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "returns",
                reason: format!("portfolio return at row {row} must be finite"),
            });
        }
        portfolio_returns.push(value);
    }
    let var = historical_var(&portfolio_returns, confidence_level)?;
    let cvar = historical_expected_shortfall(&portfolio_returns, confidence_level)?;
    let tail = tail_count(portfolio_returns.len(), confidence_level);
    let mut order = (0..portfolio_returns.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| {
        let left = portfolio_returns.get(*left).copied().unwrap_or_default();
        let right = portfolio_returns.get(*right).copied().unwrap_or_default();
        left.total_cmp(&right)
    });

    let mut loss_sums = vec![0.0; assets];
    let mut portfolio_loss_sum = 0.0;
    for index in order.iter().take(tail) {
        let observation = returns.data().row(*index);
        for (sum, (weight, asset_return)) in loss_sums
            .iter_mut()
            .zip(aligned.iter().zip(observation.iter()))
        {
            *sum -= weight * asset_return;
        }
        portfolio_loss_sum -= portfolio_returns
            .get(*index)
            .copied()
            .ok_or_else(index_error)?;
    }
    if !portfolio_loss_sum.is_finite() {
        return Err(KurumiError::NumericalOverflow {
            operation: "historical_portfolio_var",
        });
    }
    let divisor = tail as f64;
    let raw_cvar = portfolio_loss_sum / divisor;
    let raw_components = loss_sums
        .into_iter()
        .map(|sum| sum / divisor)
        .collect::<Vec<_>>();
    let raw_components = DVector::from_vec(raw_components);
    let component_cvar = scale_components(raw_components.clone(), cvar)?;
    let component_var = if raw_cvar > 0.0 {
        scale_components(raw_components, var)?
    } else {
        DVector::zeros(assets)
    };
    Ok(PortfolioVar {
        var,
        cvar,
        component_var,
        component_cvar,
    })
}

fn validate_vector(
    parameter: &'static str,
    values: &DVector<f64>,
    expected_len: usize,
) -> Result<(), KurumiError> {
    if values.len() != expected_len {
        return Err(KurumiError::InvalidParameter {
            parameter,
            reason: format!(
                "length {} does not match the {} assets",
                values.len(),
                expected_len
            ),
        });
    }
    for (index, value) in values.iter().enumerate() {
        if !value.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter,
                reason: format!("value at index {index} must be finite"),
            });
        }
    }
    Ok(())
}

fn validate_covariance(order: usize, covariance: &DMatrix<f64>) -> Result<(), KurumiError> {
    if covariance.shape() != (order, order) {
        return Err(KurumiError::InvalidParameter {
            parameter: "covariance",
            reason: format!(
                "shape must be ({order}, {order}), got ({}, {})",
                covariance.nrows(),
                covariance.ncols()
            ),
        });
    }
    for row in 0..order {
        for column in 0..order {
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
    Ok(())
}

fn quadratic_variance(
    weights: &DVector<f64>,
    covariance: &DMatrix<f64>,
) -> Result<f64, KurumiError> {
    validate_covariance(weights.len(), covariance)?;
    let sigma_weighted = covariance * weights;
    let variance = weights.dot(&sigma_weighted);
    if !variance.is_finite() {
        return Err(KurumiError::NumericalOverflow {
            operation: "portfolio_variance",
        });
    }
    if variance < 0.0 {
        let scale = sigma_weighted
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
        return Ok(0.0);
    }
    Ok(variance)
}

fn scale_components(components: DVector<f64>, target: f64) -> Result<DVector<f64>, KurumiError> {
    if target <= 0.0 {
        return Ok(DVector::zeros(components.len()));
    }
    let sum = components.sum();
    if !sum.is_finite() {
        return Err(KurumiError::NumericalOverflow {
            operation: "portfolio_component_var",
        });
    }
    if sum <= 0.0 {
        return Ok(DVector::zeros(components.len()));
    }
    let factor = target / sum;
    let scaled = components * factor;
    if scaled.iter().any(|value| !value.is_finite()) {
        return Err(KurumiError::NumericalOverflow {
            operation: "portfolio_component_var",
        });
    }
    Ok(scaled)
}

fn validate_confidence(confidence_level: f64) -> Result<(), KurumiError> {
    if !confidence_level.is_finite() || confidence_level <= 0.5 || confidence_level >= 1.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "confidence_level",
            reason: "must be finite and strictly between 0.5 and 1.0".to_owned(),
        });
    }
    Ok(())
}

fn tail_count(length: usize, confidence_level: f64) -> usize {
    let raw = ((1.0 - confidence_level) * length as f64).ceil();
    if raw < 1.0 {
        1
    } else {
        (raw as usize).min(length)
    }
}

fn finite(value: f64, operation: &'static str) -> Result<f64, KurumiError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(KurumiError::NumericalOverflow { operation })
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

    use crate::panel::{AlignmentPolicy, AssetSeries, ReturnMatrix};
    use chrono::{DateTime, Utc};

    fn at(day: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
    }

    fn weights(values: &[f64]) -> PortfolioWeights {
        let symbols = values
            .iter()
            .enumerate()
            .map(|(index, _)| format!("A{index}"))
            .collect::<Vec<_>>();
        PortfolioWeights::new(symbols, DVector::from_vec(values.to_vec())).expect("valid weights")
    }

    #[test]
    fn weight_summary_reports_net_gross_and_long_only_status() {
        let long_only = weights(&[0.6, 0.4]);
        assert_relative_eq!(long_only.sum(), 1.0, epsilon = 1e-15);
        assert_relative_eq!(long_only.gross_leverage(), 1.0, epsilon = 1e-15);
        assert!(long_only.is_long_only());
        assert!(long_only.is_fully_invested(1e-12).unwrap());

        let hedged = weights(&[1.2, -0.2]);
        assert_relative_eq!(hedged.sum(), 1.0, epsilon = 1e-15);
        assert_relative_eq!(hedged.gross_leverage(), 1.4, epsilon = 1e-15);
        assert!(!hedged.is_long_only());
    }

    #[test]
    fn weights_reject_invalid_universes() {
        assert!(PortfolioWeights::new(Vec::new(), DVector::from_vec(Vec::new())).is_err());
        assert!(
            PortfolioWeights::new(vec!["A".to_owned()], DVector::from_vec(vec![1.0, 0.0]),)
                .is_err()
        );
        assert!(
            PortfolioWeights::new(
                vec!["A".to_owned(), "A".to_owned()],
                DVector::from_vec(vec![0.5, 0.5]),
            )
            .is_err()
        );
        assert!(
            PortfolioWeights::new(vec!["A".to_owned()], DVector::from_vec(vec![f64::NAN]),)
                .is_err()
        );
    }

    #[test]
    fn aligned_weights_follow_matrix_column_order() {
        let weights = weights(&[0.6, 0.4]);
        let aligned = weights
            .aligned_to(&["A1".to_owned(), "A0".to_owned()])
            .expect("aligned");
        assert_relative_eq!(aligned[0], 0.4, epsilon = 1e-15);
        assert_relative_eq!(aligned[1], 0.6, epsilon = 1e-15);
        assert!(weights.aligned_to(&["A0".to_owned()]).is_err());
    }

    #[test]
    fn risk_decomposition_sums_to_volatility_and_one() {
        let weights = weights(&[0.6, 0.4]);
        let covariance = DMatrix::from_row_slice(2, 2, &[0.04, 0.006, 0.006, 0.09]);
        let decomposition =
            risk_decomposition(&weights, &covariance).expect("decomposition succeeds");

        let expected_variance: f64 = 0.6 * 0.6 * 0.04 + 2.0 * 0.6 * 0.4 * 0.006 + 0.4 * 0.4 * 0.09;
        assert_relative_eq!(
            decomposition.portfolio_volatility,
            expected_variance.sqrt(),
            epsilon = 1e-15
        );
        assert_relative_eq!(
            decomposition.ctr.sum(),
            decomposition.portfolio_volatility,
            epsilon = 1e-10
        );
        assert_relative_eq!(decomposition.pctr.sum(), 1.0, epsilon = 1e-10);
        assert_relative_eq!(
            decomposition.ctr[0] + decomposition.ctr[1],
            decomposition.portfolio_volatility,
            epsilon = 1e-10
        );
    }

    #[test]
    fn risk_decomposition_sums_hold_for_a_four_asset_positive_definite_covariance() {
        // Sigma = A A^T + I is symmetric positive definite by construction.
        let factor = DMatrix::from_row_slice(
            4,
            4,
            &[
                0.20, 0.00, 0.00, 0.00, 0.05, 0.30, 0.00, 0.00, -0.04, 0.02, 0.25, 0.00, 0.01,
                -0.03, 0.04, 0.15,
            ],
        );
        let covariance = &factor * factor.transpose() + DMatrix::identity(4, 4) * 1e-4;
        let weights = weights(&[0.35, -0.15, 0.50, 0.30]);
        let decomposition =
            risk_decomposition(&weights, &covariance).expect("decomposition succeeds");

        assert_relative_eq!(
            decomposition.ctr.sum(),
            decomposition.portfolio_volatility,
            epsilon = 1e-10
        );
        assert_relative_eq!(decomposition.pctr.sum(), 1.0, epsilon = 1e-10);
        assert!(decomposition.portfolio_volatility > 0.0);
    }

    #[test]
    fn expected_return_sharpe_and_diversification_ratio_match_definitions() {
        let weights = weights(&[0.6, 0.4]);
        let means = DVector::from_vec(vec![0.01, 0.02]);
        let covariance = DMatrix::from_row_slice(2, 2, &[0.04, 0.006, 0.006, 0.09]);

        assert_relative_eq!(
            portfolio_expected_return(&weights, &means).unwrap(),
            0.014,
            epsilon = 1e-15
        );
        let volatility = portfolio_volatility(&weights, &covariance).unwrap();
        let sharpe = annualized_sharpe_ratio(&weights, &means, &covariance, 0.02, 252.0).unwrap();
        let expected_sharpe = (0.014 * 252.0 - 0.02) / (volatility * 252.0_f64.sqrt());
        assert_relative_eq!(sharpe, expected_sharpe, epsilon = 1e-12);

        let ratio = diversification_ratio(&weights, &covariance).unwrap();
        let expected_ratio = (0.6 * 0.2 + 0.4 * 0.3) / volatility;
        assert_relative_eq!(ratio, expected_ratio, epsilon = 1e-12);
    }

    #[test]
    fn parametric_var_components_are_additive() {
        let weights = weights(&[0.6, 0.4]);
        let means = DVector::from_vec(vec![0.0005, 0.0008]);
        let covariance = DMatrix::from_row_slice(2, 2, &[0.0004, 0.00006, 0.00006, 0.0009]);
        let result =
            parametric_portfolio_var(&weights, &means, &covariance, 0.95).expect("VaR succeeds");

        let portfolio_mean = 0.6 * 0.0005 + 0.4 * 0.0008;
        let portfolio_volatility = portfolio_volatility(&weights, &covariance).unwrap();
        assert_relative_eq!(
            result.var,
            (1.644_853_626_951_472_2 * portfolio_volatility - portfolio_mean).max(0.0),
            epsilon = 1e-12
        );
        assert_relative_eq!(result.component_var.sum(), result.var, epsilon = 1e-12);
        assert_relative_eq!(result.component_cvar.sum(), result.cvar, epsilon = 1e-12);
        assert!(result.var > 0.0);
        assert!(result.cvar >= result.var);
    }

    #[test]
    fn historical_var_components_are_additive_and_match_the_portfolio_series() {
        let a = AssetSeries::new(
            "AAA",
            vec![
                (at(0), -0.03),
                (at(1), -0.01),
                (at(2), 0.02),
                (at(3), 0.04),
                (at(4), 0.01),
            ],
        )
        .unwrap();
        let b = AssetSeries::new(
            "BBB",
            vec![
                (at(0), -0.01),
                (at(1), 0.00),
                (at(2), 0.01),
                (at(3), 0.02),
                (at(4), -0.02),
            ],
        )
        .unwrap();
        let returns = ReturnMatrix::from_return_series(&[a, b], AlignmentPolicy::InnerJoin)
            .expect("valid panel");
        let weights = PortfolioWeights::new(
            vec!["BBB".to_owned(), "AAA".to_owned()],
            DVector::from_vec(vec![0.4, 0.6]),
        )
        .unwrap();
        let result = historical_portfolio_var(&returns, &weights, 0.95).expect("VaR succeeds");

        let portfolio = (0..5)
            .map(|row| 0.4 * returns.data()[(row, 1)] + 0.6 * returns.data()[(row, 0)])
            .collect::<Vec<_>>();
        assert_relative_eq!(
            result.var,
            historical_var(&portfolio, 0.95).unwrap(),
            epsilon = 1e-15
        );
        assert_relative_eq!(
            result.cvar,
            historical_expected_shortfall(&portfolio, 0.95).unwrap(),
            epsilon = 1e-15
        );
        assert_relative_eq!(result.component_var.sum(), result.var, epsilon = 1e-12);
        assert_relative_eq!(result.component_cvar.sum(), result.cvar, epsilon = 1e-12);
    }

    #[test]
    fn covariance_validation_rejects_bad_shapes_and_asymmetry() {
        let weights = weights(&[0.5, 0.5]);
        let wrong_shape =
            DMatrix::from_row_slice(3, 3, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        assert!(portfolio_volatility(&weights, &wrong_shape).is_err());

        let asymmetric = DMatrix::from_row_slice(2, 2, &[0.04, 0.01, 0.02, 0.09]);
        assert!(portfolio_volatility(&weights, &asymmetric).is_err());

        let negative_diagonal = DMatrix::from_row_slice(2, 2, &[-0.01, 0.0, 0.0, 0.09]);
        assert!(portfolio_volatility(&weights, &negative_diagonal).is_err());
    }

    #[test]
    fn confidence_level_must_be_above_one_half() {
        let weights = weights(&[1.0]);
        let means = DVector::from_vec(vec![0.0]);
        let covariance = DMatrix::from_row_slice(1, 1, &[0.0001]);
        assert!(parametric_portfolio_var(&weights, &means, &covariance, 0.5).is_err());
        assert!(parametric_portfolio_var(&weights, &means, &covariance, 0.999).is_ok());
    }
}
