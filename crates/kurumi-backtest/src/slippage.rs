//! Slippage models.
//!
//! A [`SlippageModel`] adjusts a reference execution price for the cost of
//! crossing the spread and consuming liquidity. The model is applied to every
//! fill, including limit orders, so a limit fill can only improve on its limit
//! price, never violate it (the engine clamps the result).

use kurumi_core::{KurumiError, Price};
use rust_decimal::Decimal;

use crate::order::Side;

/// A model that maps a reference price to an execution price.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::{NoSlippage, SlippageModel, Side};
/// use kurumi_core::Price;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let filled = NoSlippage.fill_price(Side::Buy, Price::new(Decimal::from(100))?)?;
/// assert_eq!(filled.value(), Decimal::from(100));
/// # Ok(())
/// # }
/// ```
pub trait SlippageModel {
    /// Returns the execution price for `reference_price` on `side`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the adjusted price is not
    /// strictly positive, and [`KurumiError::NumericalOverflow`] when the
    /// adjustment is not representable.
    fn fill_price(&self, side: Side, reference_price: Price) -> Result<Price, KurumiError>;
}

/// A slippage model that leaves the reference price unchanged.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::{NoSlippage, SlippageModel, Side};
/// use kurumi_core::Price;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let price = Price::new(Decimal::from(50))?;
/// assert_eq!(NoSlippage.fill_price(Side::Sell, price)?.value(), Decimal::from(50));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoSlippage;

impl SlippageModel for NoSlippage {
    #[inline]
    fn fill_price(&self, _side: Side, reference_price: Price) -> Result<Price, KurumiError> {
        Ok(reference_price)
    }
}

/// Fixed slippage in basis points.
///
/// A buy fills at `reference * (1 + bps / 10_000)` and a sell at
/// `reference * (1 - bps / 10_000)`, so the cost is always adverse.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::{FixedBpsSlippage, SlippageModel, Side};
/// use kurumi_core::Price;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let model = FixedBpsSlippage::new(Decimal::from(10))?;
/// let reference = Price::new(Decimal::from(100))?;
/// assert_eq!(model.fill_price(Side::Buy, reference)?.value(), Decimal::new(1001, 1));
/// assert_eq!(model.fill_price(Side::Sell, reference)?.value(), Decimal::new(999, 1));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixedBpsSlippage {
    basis_points: Decimal,
}

impl FixedBpsSlippage {
    /// Creates a fixed basis-point slippage model.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] unless `basis_points` is
    /// non-negative and strictly below 10,000 (100%). The upper bound keeps a
    /// sell price strictly positive.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::FixedBpsSlippage;
    /// use rust_decimal::Decimal;
    ///
    /// assert!(FixedBpsSlippage::new(Decimal::from(10)).is_ok());
    /// assert!(FixedBpsSlippage::new(Decimal::NEGATIVE_ONE).is_err());
    /// assert!(FixedBpsSlippage::new(Decimal::from(10_000)).is_err());
    /// ```
    pub fn new(basis_points: Decimal) -> Result<Self, KurumiError> {
        if basis_points.is_sign_negative() || basis_points >= Decimal::from(10_000) {
            return Err(KurumiError::InvalidParameter {
                parameter: "basis_points",
                reason: "must be non-negative and strictly below 10,000".to_owned(),
            });
        }
        Ok(Self { basis_points })
    }

    /// Returns the configured slippage in basis points.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::FixedBpsSlippage;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// assert_eq!(FixedBpsSlippage::new(Decimal::from(10))?.basis_points(), Decimal::from(10));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn basis_points(self) -> Decimal {
        self.basis_points
    }
}

impl SlippageModel for FixedBpsSlippage {
    fn fill_price(&self, side: Side, reference_price: Price) -> Result<Price, KurumiError> {
        let fraction = self.basis_points.checked_div(Decimal::from(10_000)).ok_or(
            KurumiError::NumericalOverflow {
                operation: "FixedBpsSlippage::fill_price",
            },
        )?;
        let factor = match side {
            Side::Buy => {
                Decimal::ONE
                    .checked_add(fraction)
                    .ok_or(KurumiError::NumericalOverflow {
                        operation: "FixedBpsSlippage::fill_price",
                    })?
            }
            Side::Sell => {
                Decimal::ONE
                    .checked_sub(fraction)
                    .ok_or(KurumiError::NumericalOverflow {
                        operation: "FixedBpsSlippage::fill_price",
                    })?
            }
        };
        let adjusted =
            reference_price
                .value()
                .checked_mul(factor)
                .ok_or(KurumiError::NumericalOverflow {
                    operation: "FixedBpsSlippage::fill_price",
                })?;
        Price::new(adjusted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price(value: i64) -> Price {
        Price::new(Decimal::from(value)).expect("valid test price")
    }

    #[test]
    fn no_slippage_preserves_the_reference_price() {
        let reference = price(100);
        assert_eq!(
            NoSlippage
                .fill_price(Side::Buy, reference)
                .expect("valid price"),
            reference
        );
        assert_eq!(
            NoSlippage
                .fill_price(Side::Sell, reference)
                .expect("valid price"),
            reference
        );
    }

    #[test]
    fn fixed_bps_slippage_is_adverse_on_both_sides() {
        let model = FixedBpsSlippage::new(Decimal::from(10)).expect("valid slippage");
        let reference = price(100);
        let buy = model.fill_price(Side::Buy, reference).expect("valid price");
        let sell = model
            .fill_price(Side::Sell, reference)
            .expect("valid price");
        assert_eq!(buy.value(), Decimal::new(1001, 1));
        assert_eq!(sell.value(), Decimal::new(999, 1));
        assert!(buy > reference);
        assert!(sell < reference);
    }

    #[test]
    fn zero_basis_points_is_allowed() {
        let model = FixedBpsSlippage::new(Decimal::ZERO).expect("valid slippage");
        let reference = price(100);
        assert_eq!(
            model.fill_price(Side::Buy, reference).expect("valid price"),
            reference
        );
    }

    #[test]
    fn fixed_bps_slippage_rejects_invalid_configuration() {
        assert!(FixedBpsSlippage::new(Decimal::NEGATIVE_ONE).is_err());
        assert!(FixedBpsSlippage::new(Decimal::from(10_000)).is_err());
    }
}
