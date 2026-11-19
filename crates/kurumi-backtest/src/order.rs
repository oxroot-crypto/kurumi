//! Orders, sides, and simulated executions.
//!
//! An [`Order`] carries a side, a size, and an execution style: [`OrderKind::Market`]
//! executes at the next bar's open, while [`OrderKind::Limit`] executes only when
//! the bar trades through the limit price.
//!
//! A [`Fill`] is the immutable record of one simulated execution. Its notional
//! and cash flow are computed once, with checked decimal arithmetic, so the
//! account ledger never has to redo the multiplication.

use std::fmt;

use kurumi_core::{KurumiError, Price, Quantity};
use rust_decimal::Decimal;

use crate::fee::Liquidity;

/// The direction of an order or position.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::Side;
///
/// assert_eq!(Side::Buy.sign(), 1);
/// assert_eq!(Side::Sell.sign(), -1);
/// assert_eq!(Side::Buy.opposite(), Side::Sell);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// Buy the instrument, increasing the position.
    Buy,
    /// Sell the instrument, decreasing or shorting the position.
    Sell,
}

impl Side {
    /// Returns `1` for a buy and `-1` for a sell.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::Side;
    ///
    /// assert_eq!(Side::Buy.sign(), 1);
    /// assert_eq!(Side::Sell.sign(), -1);
    /// ```
    #[must_use]
    #[inline]
    pub const fn sign(self) -> i8 {
        match self {
            Self::Buy => 1,
            Self::Sell => -1,
        }
    }

    /// Returns the opposite side.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::Side;
    ///
    /// assert_eq!(Side::Buy.opposite(), Side::Sell);
    /// assert_eq!(Side::Sell.opposite(), Side::Buy);
    /// ```
    #[must_use]
    #[inline]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Buy => Self::Sell,
            Self::Sell => Self::Buy,
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Buy => "BUY",
            Self::Sell => "SELL",
        })
    }
}

/// How an order should be matched against the next bar.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::OrderKind;
/// use kurumi_core::Price;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let limit = OrderKind::Limit {
///     price: Price::new(Decimal::from(100))?,
/// };
/// assert_eq!(limit.limit_price().map(Price::value), Some(Decimal::from(100)));
/// assert!(OrderKind::Market.limit_price().is_none());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderKind {
    /// Execute at the next bar's opening price after slippage.
    Market,
    /// Execute only when the bar's range reaches the limit price.
    Limit {
        /// The worst acceptable execution price for the order.
        price: Price,
    },
}

impl OrderKind {
    /// Returns `true` for a market order.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::OrderKind;
    ///
    /// assert!(OrderKind::Market.is_market());
    /// ```
    #[must_use]
    #[inline]
    pub const fn is_market(self) -> bool {
        matches!(self, Self::Market)
    }

    /// Returns the limit price, or `None` for a market order.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::OrderKind;
    ///
    /// assert!(OrderKind::Market.limit_price().is_none());
    /// ```
    #[must_use]
    #[inline]
    pub const fn limit_price(self) -> Option<Price> {
        match self {
            Self::Market => None,
            Self::Limit { price } => Some(price),
        }
    }
}

/// A request to trade one instrument.
///
/// Quantities are always non-negative; the side carries the direction.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::{Order, Side};
/// use kurumi_core::Quantity;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let order = Order::market(Side::Buy, Quantity::new(Decimal::from(10))?);
/// assert_eq!(order.side(), Side::Buy);
/// assert!(order.is_market());
/// assert_eq!(order.quantity().value(), Decimal::from(10));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Order {
    side: Side,
    kind: OrderKind,
    quantity: Quantity,
}

impl Order {
    /// Creates a market order.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Order, Side};
    /// use kurumi_core::Quantity;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let order = Order::market(Side::Sell, Quantity::new(Decimal::ONE)?);
    /// assert_eq!(order.side(), Side::Sell);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn market(side: Side, quantity: Quantity) -> Self {
        Self {
            side,
            kind: OrderKind::Market,
            quantity,
        }
    }

    /// Creates a limit order.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Order, OrderKind, Side};
    /// use kurumi_core::{Price, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let order = Order::limit(
    ///     Side::Buy,
    ///     Quantity::new(Decimal::ONE)?,
    ///     Price::new(Decimal::from(100))?,
    /// );
    /// assert_eq!(order.kind().limit_price().map(Price::value), Some(Decimal::from(100)));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn limit(side: Side, quantity: Quantity, price: Price) -> Self {
        Self {
            side,
            kind: OrderKind::Limit { price },
            quantity,
        }
    }

    /// Returns the order side.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Order, Side};
    /// use kurumi_core::Quantity;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let order = Order::market(Side::Buy, Quantity::new(Decimal::ONE)?);
    /// assert_eq!(order.side(), Side::Buy);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn side(self) -> Side {
        self.side
    }

    /// Returns the execution style.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Order, OrderKind, Side};
    /// use kurumi_core::Quantity;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let order = Order::market(Side::Buy, Quantity::new(Decimal::ONE)?);
    /// assert_eq!(order.kind(), OrderKind::Market);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn kind(self) -> OrderKind {
        self.kind
    }

    /// Returns the order quantity.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Order, Side};
    /// use kurumi_core::Quantity;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let order = Order::market(Side::Buy, Quantity::new(Decimal::from(3))?);
    /// assert_eq!(order.quantity().value(), Decimal::from(3));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn quantity(self) -> Quantity {
        self.quantity
    }

    /// Returns `true` for a market order.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Order, Side};
    /// use kurumi_core::Quantity;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let order = Order::market(Side::Buy, Quantity::new(Decimal::ONE)?);
    /// assert!(order.is_market());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn is_market(self) -> bool {
        self.kind.is_market()
    }
}

/// A simulated execution.
///
/// The notional (`price * quantity`) and the signed cash flow are validated and
/// stored at construction. Fees are never negative.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::{Fill, Liquidity, Side};
/// use kurumi_core::{Price, Quantity};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let fill = Fill::new(
///     1,
///     0,
///     Side::Buy,
///     Price::new(Decimal::from(100))?,
///     Quantity::new(Decimal::from(2))?,
///     Decimal::from(1),
///     Liquidity::Taker,
/// )?;
/// assert_eq!(fill.notional(), Decimal::from(200));
/// assert_eq!(fill.cash_flow(), Decimal::from(-201));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fill {
    order_id: u64,
    timestamp: i64,
    side: Side,
    price: Price,
    quantity: Quantity,
    fee: Decimal,
    notional: Decimal,
    cash_flow: Decimal,
    liquidity: Liquidity,
}

impl Fill {
    /// Creates a fill, validating its decimal arithmetic.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `quantity` is zero or
    /// `fee` is negative, and [`KurumiError::NumericalOverflow`] when the
    /// notional or cash flow is not representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Fill, Liquidity, Side};
    /// use kurumi_core::{Price, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let price = Price::new(Decimal::from(100))?;
    /// assert!(Fill::new(1, 0, Side::Buy, price, Quantity::new(Decimal::ZERO)?, Decimal::ZERO, Liquidity::Maker).is_err());
    /// assert!(Fill::new(1, 0, Side::Buy, price, Quantity::new(Decimal::ONE)?, Decimal::NEGATIVE_ONE, Liquidity::Maker).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        order_id: u64,
        timestamp: i64,
        side: Side,
        price: Price,
        quantity: Quantity,
        fee: Decimal,
        liquidity: Liquidity,
    ) -> Result<Self, KurumiError> {
        if quantity.value() <= Decimal::ZERO {
            return Err(KurumiError::InvalidParameter {
                parameter: "quantity",
                reason: "a fill quantity must be strictly positive".to_owned(),
            });
        }
        if fee.is_sign_negative() {
            return Err(KurumiError::InvalidParameter {
                parameter: "fee",
                reason: "must be non-negative".to_owned(),
            });
        }
        let notional =
            price
                .value()
                .checked_mul(quantity.value())
                .ok_or(KurumiError::NumericalOverflow {
                    operation: "Fill::notional",
                })?;
        let cash_flow = match side {
            Side::Buy => notional
                .checked_add(fee)
                .and_then(|total| Decimal::ZERO.checked_sub(total)),
            Side::Sell => notional.checked_sub(fee),
        }
        .ok_or(KurumiError::NumericalOverflow {
            operation: "Fill::cash_flow",
        })?;
        Ok(Self {
            order_id,
            timestamp,
            side,
            price,
            quantity,
            fee,
            notional,
            cash_flow,
            liquidity,
        })
    }

    /// Returns the identifier of the order that produced this fill.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Fill, Liquidity, Side};
    /// use kurumi_core::{Price, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let fill = Fill::new(7, 0, Side::Buy, Price::new(Decimal::ONE)?, Quantity::new(Decimal::ONE)?, Decimal::ZERO, Liquidity::Taker)?;
    /// assert_eq!(fill.order_id(), 7);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn order_id(self) -> u64 {
        self.order_id
    }

    /// Returns the bar timestamp at which the fill occurred.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Fill, Liquidity, Side};
    /// use kurumi_core::{Price, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let fill = Fill::new(1, 1_700_000_000_000, Side::Buy, Price::new(Decimal::ONE)?, Quantity::new(Decimal::ONE)?, Decimal::ZERO, Liquidity::Taker)?;
    /// assert_eq!(fill.timestamp(), 1_700_000_000_000);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn timestamp(self) -> i64 {
        self.timestamp
    }

    /// Returns the fill side.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Fill, Liquidity, Side};
    /// use kurumi_core::{Price, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let fill = Fill::new(1, 0, Side::Sell, Price::new(Decimal::ONE)?, Quantity::new(Decimal::ONE)?, Decimal::ZERO, Liquidity::Taker)?;
    /// assert_eq!(fill.side(), Side::Sell);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn side(self) -> Side {
        self.side
    }

    /// Returns the execution price after slippage.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Fill, Liquidity, Side};
    /// use kurumi_core::{Price, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let fill = Fill::new(1, 0, Side::Buy, Price::new(Decimal::from(101))?, Quantity::new(Decimal::ONE)?, Decimal::ZERO, Liquidity::Taker)?;
    /// assert_eq!(fill.price().value(), Decimal::from(101));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn price(self) -> Price {
        self.price
    }

    /// Returns the executed quantity.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Fill, Liquidity, Side};
    /// use kurumi_core::{Price, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let fill = Fill::new(1, 0, Side::Buy, Price::new(Decimal::ONE)?, Quantity::new(Decimal::from(5))?, Decimal::ZERO, Liquidity::Taker)?;
    /// assert_eq!(fill.quantity().value(), Decimal::from(5));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn quantity(self) -> Quantity {
        self.quantity
    }

    /// Returns the fee charged for this fill.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Fill, Liquidity, Side};
    /// use kurumi_core::{Price, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let fill = Fill::new(1, 0, Side::Buy, Price::new(Decimal::ONE)?, Quantity::new(Decimal::ONE)?, Decimal::new(5, 2), Liquidity::Taker)?;
    /// assert_eq!(fill.fee(), Decimal::new(5, 2));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn fee(self) -> Decimal {
        self.fee
    }

    /// Returns the pre-fee notional value (`price * quantity`).
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Fill, Liquidity, Side};
    /// use kurumi_core::{Price, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let fill = Fill::new(1, 0, Side::Buy, Price::new(Decimal::from(100))?, Quantity::new(Decimal::from(2))?, Decimal::ZERO, Liquidity::Taker)?;
    /// assert_eq!(fill.notional(), Decimal::from(200));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn notional(self) -> Decimal {
        self.notional
    }

    /// Returns the signed cash flow: negative for a buy including its fee, and
    /// positive for a sell net of its fee.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Fill, Liquidity, Side};
    /// use kurumi_core::{Price, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let fill = Fill::new(1, 0, Side::Sell, Price::new(Decimal::from(100))?, Quantity::new(Decimal::from(2))?, Decimal::ONE, Liquidity::Taker)?;
    /// assert_eq!(fill.cash_flow(), Decimal::from(199));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn cash_flow(self) -> Decimal {
        self.cash_flow
    }

    /// Returns whether the fill provided maker or taker liquidity.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Fill, Liquidity, Side};
    /// use kurumi_core::{Price, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let fill = Fill::new(1, 0, Side::Buy, Price::new(Decimal::ONE)?, Quantity::new(Decimal::ONE)?, Decimal::ZERO, Liquidity::Maker)?;
    /// assert_eq!(fill.liquidity(), Liquidity::Maker);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn liquidity(self) -> Liquidity {
        self.liquidity
    }
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
    fn side_sign_and_opposite_round_trip() {
        assert_eq!(Side::Buy.sign(), 1);
        assert_eq!(Side::Sell.sign(), -1);
        assert_eq!(Side::Buy.opposite().opposite(), Side::Buy);
        assert_eq!(Side::Buy.to_string(), "BUY");
    }

    #[test]
    fn order_constructors_preserve_their_terms() {
        let market = Order::market(Side::Buy, quantity(3));
        assert_eq!(market.side(), Side::Buy);
        assert_eq!(market.quantity().value(), Decimal::from(3));
        assert!(market.is_market());
        assert!(market.kind().limit_price().is_none());

        let limit = Order::limit(Side::Sell, quantity(4), price(250));
        assert_eq!(
            limit.kind().limit_price().map(Price::value),
            Some(Decimal::from(250))
        );
        assert!(!limit.is_market());
    }

    #[test]
    fn fill_computes_notional_and_signed_cash_flow() {
        let buy = Fill::new(
            1,
            0,
            Side::Buy,
            price(100),
            quantity(2),
            Decimal::from(2),
            Liquidity::Taker,
        )
        .expect("valid fill");
        assert_eq!(buy.notional(), Decimal::from(200));
        assert_eq!(buy.cash_flow(), Decimal::from(-202));

        let sell = Fill::new(
            2,
            0,
            Side::Sell,
            price(100),
            quantity(2),
            Decimal::from(2),
            Liquidity::Maker,
        )
        .expect("valid fill");
        assert_eq!(sell.cash_flow(), Decimal::from(198));
    }

    #[test]
    fn fill_rejects_zero_quantity_and_negative_fee() {
        assert!(
            Fill::new(
                1,
                0,
                Side::Buy,
                price(100),
                quantity(0),
                Decimal::ZERO,
                Liquidity::Taker
            )
            .is_err()
        );
        assert!(
            Fill::new(
                1,
                0,
                Side::Buy,
                price(100),
                quantity(1),
                Decimal::NEGATIVE_ONE,
                Liquidity::Taker
            )
            .is_err()
        );
    }
}
