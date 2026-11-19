//! Transaction-cost models.
//!
//! A [`FeeModel`] turns an execution price, quantity, side, and liquidity role
//! into a non-negative fee in the quote currency. Two standard models are
//! provided:
//!
//! - [`FixedRateFee`] charges the same proportional rate to every fill;
//! - [`MakerTakerFee`] charges different rates to resting (maker) and aggressive
//!   (taker) fills.
//!
//! A market order always provides taker liquidity. A limit order that executes
//! against the bar is treated as a maker fill; a more detailed exchange model
//! belongs in a custom [`FeeModel`] implementation.

use kurumi_core::{KurumiError, Price, Quantity};
use rust_decimal::Decimal;

use crate::order::Side;

/// The liquidity role of a simulated fill.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::Liquidity;
///
/// assert_ne!(Liquidity::Maker, Liquidity::Taker);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Liquidity {
    /// The order added liquidity to the book (a resting limit order).
    Maker,
    /// The order removed liquidity from the book (a market order).
    Taker,
}

/// A transaction-cost model.
///
/// Implementations must return a non-negative fee and may use the side and the
/// liquidity role to price different flow differently.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::{FeeModel, Liquidity, Side};
/// use kurumi_core::{KurumiError, Price, Quantity};
/// use rust_decimal::Decimal;
///
/// struct OneCentPerUnit;
///
/// impl FeeModel for OneCentPerUnit {
///     fn fee(
///         &self,
///         _price: Price,
///         quantity: Quantity,
///         _side: Side,
///         _liquidity: Liquidity,
///     ) -> Result<Decimal, KurumiError> {
///         Ok(Decimal::new(1, 2) * quantity.value())
///     }
/// }
/// # fn main() -> Result<(), KurumiError> {
/// let fee = OneCentPerUnit.fee(
///     Price::new(Decimal::from(100))?,
///     Quantity::new(Decimal::from(10))?,
///     Side::Buy,
///     Liquidity::Taker,
/// )?;
/// assert_eq!(fee, Decimal::new(10, 2));
/// # Ok(())
/// # }
/// ```
pub trait FeeModel {
    /// Returns the fee for one fill in the quote currency.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::NumericalOverflow`] when the notional or fee is
    /// not representable, or [`KurumiError::InvalidParameter`] when the model's
    /// configuration makes the fee invalid.
    fn fee(
        &self,
        price: Price,
        quantity: Quantity,
        side: Side,
        liquidity: Liquidity,
    ) -> Result<Decimal, KurumiError>;
}

/// A fee model that charges nothing.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::{FeeModel, Liquidity, NoFee, Side};
/// use kurumi_core::{Price, Quantity};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let fee = NoFee.fee(
///     Price::new(Decimal::from(100))?,
///     Quantity::new(Decimal::ONE)?,
///     Side::Buy,
///     Liquidity::Taker,
/// )?;
/// assert_eq!(fee, Decimal::ZERO);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoFee;

impl FeeModel for NoFee {
    #[inline]
    fn fee(
        &self,
        _price: Price,
        _quantity: Quantity,
        _side: Side,
        _liquidity: Liquidity,
    ) -> Result<Decimal, KurumiError> {
        Ok(Decimal::ZERO)
    }
}

/// A proportional fee charged identically to every fill.
///
/// The fee is `price * quantity * rate`; a rate of `0.001` charges ten basis
/// points of notional.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::{FeeModel, FixedRateFee, Liquidity, Side};
/// use kurumi_core::{Price, Quantity};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let model = FixedRateFee::new(Decimal::new(1, 3))?;
/// let fee = model.fee(
///     Price::new(Decimal::from(100))?,
///     Quantity::new(Decimal::from(10))?,
///     Side::Buy,
///     Liquidity::Taker,
/// )?;
/// assert_eq!(fee, Decimal::ONE);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixedRateFee {
    rate: Decimal,
}

impl FixedRateFee {
    /// Creates a fixed proportional fee model.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] unless `rate` is non-negative
    /// and strictly below one.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::FixedRateFee;
    /// use rust_decimal::Decimal;
    ///
    /// assert!(FixedRateFee::new(Decimal::new(5, 3)).is_ok());
    /// assert!(FixedRateFee::new(Decimal::NEGATIVE_ONE).is_err());
    /// assert!(FixedRateFee::new(Decimal::ONE).is_err());
    /// ```
    pub fn new(rate: Decimal) -> Result<Self, KurumiError> {
        validate_rate(rate, "rate")?;
        Ok(Self { rate })
    }

    /// Returns the proportional rate.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::FixedRateFee;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let model = FixedRateFee::new(Decimal::new(1, 3))?;
    /// assert_eq!(model.rate(), Decimal::new(1, 3));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn rate(self) -> Decimal {
        self.rate
    }
}

impl FeeModel for FixedRateFee {
    fn fee(
        &self,
        price: Price,
        quantity: Quantity,
        _side: Side,
        _liquidity: Liquidity,
    ) -> Result<Decimal, KurumiError> {
        let notional = notional(price, quantity)?;
        notional
            .checked_mul(self.rate)
            .ok_or(KurumiError::NumericalOverflow {
                operation: "FixedRateFee::fee",
            })
    }
}

/// A fee model with separate maker and taker rates.
///
/// This is the standard crypto-exchange schedule: resting limit orders pay the
/// lower maker rate and aggressive market orders pay the taker rate.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::{FeeModel, Liquidity, MakerTakerFee, Side};
/// use kurumi_core::{Price, Quantity};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let model = MakerTakerFee::new(Decimal::new(1, 4), Decimal::new(5, 4))?;
/// let price = Price::new(Decimal::from(100))?;
/// let quantity = Quantity::new(Decimal::from(10))?;
/// assert_eq!(model.fee(price, quantity, Side::Buy, Liquidity::Maker)?, Decimal::new(1, 1));
/// assert_eq!(model.fee(price, quantity, Side::Buy, Liquidity::Taker)?, Decimal::new(5, 1));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MakerTakerFee {
    maker_rate: Decimal,
    taker_rate: Decimal,
}

impl MakerTakerFee {
    /// Creates a maker/taker fee model.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] unless both rates are
    /// non-negative and strictly below one.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::MakerTakerFee;
    /// use rust_decimal::Decimal;
    ///
    /// assert!(MakerTakerFee::new(Decimal::new(2, 4), Decimal::new(5, 4)).is_ok());
    /// assert!(MakerTakerFee::new(Decimal::new(5, 4), Decimal::new(2, 4)).is_err());
    /// assert!(MakerTakerFee::new(Decimal::ONE, Decimal::ZERO).is_err());
    /// ```
    pub fn new(maker_rate: Decimal, taker_rate: Decimal) -> Result<Self, KurumiError> {
        validate_rate(maker_rate, "maker_rate")?;
        validate_rate(taker_rate, "taker_rate")?;
        if maker_rate > taker_rate {
            return Err(KurumiError::InvalidParameter {
                parameter: "maker_rate",
                reason: "must not exceed taker_rate".to_owned(),
            });
        }
        Ok(Self {
            maker_rate,
            taker_rate,
        })
    }

    /// Returns the maker rate.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::MakerTakerFee;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let model = MakerTakerFee::new(Decimal::new(1, 4), Decimal::new(5, 4))?;
    /// assert_eq!(model.maker_rate(), Decimal::new(1, 4));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn maker_rate(self) -> Decimal {
        self.maker_rate
    }

    /// Returns the taker rate.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::MakerTakerFee;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let model = MakerTakerFee::new(Decimal::new(1, 4), Decimal::new(5, 4))?;
    /// assert_eq!(model.taker_rate(), Decimal::new(5, 4));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn taker_rate(self) -> Decimal {
        self.taker_rate
    }
}

impl FeeModel for MakerTakerFee {
    fn fee(
        &self,
        price: Price,
        quantity: Quantity,
        _side: Side,
        liquidity: Liquidity,
    ) -> Result<Decimal, KurumiError> {
        let rate = match liquidity {
            Liquidity::Maker => self.maker_rate,
            Liquidity::Taker => self.taker_rate,
        };
        let notional = notional(price, quantity)?;
        notional
            .checked_mul(rate)
            .ok_or(KurumiError::NumericalOverflow {
                operation: "MakerTakerFee::fee",
            })
    }
}

fn validate_rate(rate: Decimal, parameter: &'static str) -> Result<(), KurumiError> {
    if rate.is_sign_negative() || rate >= Decimal::ONE {
        return Err(KurumiError::InvalidParameter {
            parameter,
            reason: "must be non-negative and strictly below one".to_owned(),
        });
    }
    Ok(())
}

fn notional(price: Price, quantity: Quantity) -> Result<Decimal, KurumiError> {
    price
        .value()
        .checked_mul(quantity.value())
        .ok_or(KurumiError::NumericalOverflow {
            operation: "fee notional",
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price(value: i64) -> Price {
        Price::new(Decimal::from(value)).expect("valid test price")
    }

    fn quantity(value: i64) -> Quantity {
        Quantity::new(Decimal::from(value)).expect("valid test quantity")
    }

    #[test]
    fn no_fee_always_charges_zero() {
        let fee = NoFee
            .fee(price(100), quantity(10), Side::Buy, Liquidity::Taker)
            .expect("no fee");
        assert_eq!(fee, Decimal::ZERO);
    }

    #[test]
    fn fixed_rate_fee_charges_ten_basis_points() {
        let model = FixedRateFee::new(Decimal::new(1, 3)).expect("valid rate");
        let fee = model
            .fee(price(100), quantity(10), Side::Buy, Liquidity::Taker)
            .expect("valid fee");
        assert_eq!(fee, Decimal::ONE);
    }

    #[test]
    fn maker_taker_fee_uses_the_liquidity_role() {
        let model =
            MakerTakerFee::new(Decimal::new(1, 4), Decimal::new(5, 4)).expect("valid rates");
        let maker = model
            .fee(price(100), quantity(10), Side::Buy, Liquidity::Maker)
            .expect("valid fee");
        let taker = model
            .fee(price(100), quantity(10), Side::Sell, Liquidity::Taker)
            .expect("valid fee");
        assert_eq!(maker, Decimal::new(1, 1));
        assert_eq!(taker, Decimal::new(5, 1));
        assert!(maker < taker);
    }

    #[test]
    fn fee_models_reject_invalid_rates() {
        assert!(FixedRateFee::new(Decimal::NEGATIVE_ONE).is_err());
        assert!(FixedRateFee::new(Decimal::ONE).is_err());
        assert!(MakerTakerFee::new(Decimal::ONE, Decimal::ZERO).is_err());
        assert!(MakerTakerFee::new(Decimal::new(5, 4), Decimal::new(2, 4)).is_err());
    }
}
