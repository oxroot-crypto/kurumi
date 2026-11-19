//! Market data for option pricing.

use kurumi_core::KurumiError;

/// The market inputs required by the option-pricing engines.
///
/// The values are continuous model inputs: spot and strike are expressed in the
/// same currency, rates and the dividend yield are annualized decimals (`0.05`
/// is 5%), and volatility is an annualized standard deviation (`0.20` is 20%).
///
/// # Examples
///
/// ```
/// use kurumi_pricing::MarketData;
///
/// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
/// let market = MarketData::new(100.0, 0.05, 0.02, 0.20)?;
/// assert!((market.spot() - 100.0).abs() < 1e-15);
/// assert!((market.volatility() - 0.20).abs() < 1e-15);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketData {
    spot: f64,
    risk_free_rate: f64,
    dividend_yield: f64,
    volatility: f64,
}

impl MarketData {
    /// Creates a market-data snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `spot` is not finite and
    /// strictly positive, when a rate or the dividend yield is not finite, or
    /// when `volatility` is not finite and non-negative.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::MarketData;
    ///
    /// assert!(MarketData::new(100.0, 0.05, 0.02, 0.20).is_ok());
    /// assert!(MarketData::new(0.0, 0.05, 0.02, 0.20).is_err());
    /// assert!(MarketData::new(100.0, 0.05, 0.02, -0.20).is_err());
    /// ```
    pub fn new(
        spot: f64,
        risk_free_rate: f64,
        dividend_yield: f64,
        volatility: f64,
    ) -> Result<Self, KurumiError> {
        if !spot.is_finite() || spot <= 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "spot",
                reason: "must be finite and strictly positive".to_owned(),
            });
        }
        if !risk_free_rate.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "risk_free_rate",
                reason: "must be finite".to_owned(),
            });
        }
        if !dividend_yield.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "dividend_yield",
                reason: "must be finite".to_owned(),
            });
        }
        if !volatility.is_finite() || volatility < 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "volatility",
                reason: "must be finite and non-negative".to_owned(),
            });
        }
        Ok(Self {
            spot,
            risk_free_rate,
            dividend_yield,
            volatility,
        })
    }

    /// Returns the underlying spot price.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::MarketData;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let market = MarketData::new(100.0, 0.05, 0.0, 0.2)?;
    /// assert!((market.spot() - 100.0).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn spot(self) -> f64 {
        self.spot
    }

    /// Returns the annualized risk-free rate.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::MarketData;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let market = MarketData::new(100.0, 0.05, 0.0, 0.2)?;
    /// assert!((market.risk_free_rate() - 0.05).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn risk_free_rate(self) -> f64 {
        self.risk_free_rate
    }

    /// Returns the annualized continuous dividend yield.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::MarketData;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let market = MarketData::new(100.0, 0.05, 0.02, 0.2)?;
    /// assert!((market.dividend_yield() - 0.02).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn dividend_yield(self) -> f64 {
        self.dividend_yield
    }

    /// Returns the annualized volatility.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::MarketData;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let market = MarketData::new(100.0, 0.05, 0.02, 0.2)?;
    /// assert!((market.volatility() - 0.2).abs() < 1e-15);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn volatility(self) -> f64 {
        self.volatility
    }

    /// Returns a copy of this market snapshot with a different volatility.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `volatility` is not finite
    /// and non-negative.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_pricing::MarketData;
    ///
    /// # fn main() -> Result<(), kurumi_pricing::KurumiError> {
    /// let market = MarketData::new(100.0, 0.05, 0.0, 0.2)?;
    /// let stressed = market.with_volatility(0.4)?;
    /// assert!((stressed.volatility() - 0.4).abs() < 1e-15);
    /// assert!(market.with_volatility(-0.1).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_volatility(self, volatility: f64) -> Result<Self, KurumiError> {
        Self::new(
            self.spot,
            self.risk_free_rate,
            self.dividend_yield,
            volatility,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn market_data_round_trips_its_inputs() {
        let market = MarketData::new(101.5, 0.04, 0.015, 0.25).expect("valid market data");
        assert_relative_eq!(market.spot(), 101.5, epsilon = 1e-15);
        assert_relative_eq!(market.risk_free_rate(), 0.04, epsilon = 1e-15);
        assert_relative_eq!(market.dividend_yield(), 0.015, epsilon = 1e-15);
        assert_relative_eq!(market.volatility(), 0.25, epsilon = 1e-15);
    }

    #[test]
    fn market_data_rejects_invalid_inputs() {
        assert!(MarketData::new(-1.0, 0.05, 0.0, 0.2).is_err());
        assert!(MarketData::new(f64::NAN, 0.05, 0.0, 0.2).is_err());
        assert!(MarketData::new(100.0, f64::NAN, 0.0, 0.2).is_err());
        assert!(MarketData::new(100.0, 0.05, f64::INFINITY, 0.2).is_err());
        assert!(MarketData::new(100.0, 0.05, 0.0, -0.2).is_err());
        assert!(MarketData::new(100.0, 0.05, 0.0, f64::NAN).is_err());
    }

    #[test]
    fn volatility_can_be_bumped_and_zero_volatility_is_valid() {
        let market = MarketData::new(100.0, 0.05, 0.0, 0.2).expect("valid market data");
        assert!(market.with_volatility(0.0).is_ok());
        assert!(market.with_volatility(f64::INFINITY).is_err());
    }
}
