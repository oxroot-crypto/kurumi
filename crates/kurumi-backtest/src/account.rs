//! Portfolio accounting: cash, signed positions, average cost, and the equity
//! curve.
//!
//! The account uses a **weighted-average-cost** model. Adding to a position
//! blends the average entry price; reducing it realizes profit or loss against
//! that average; crossing through zero closes the old position and opens the new
//! one at the fill price. Fees are paid from cash immediately and tracked
//! separately; [`PortfolioAccount::realized_pnl`] is gross of fees.
//!
//! All ledger arithmetic uses [`rust_decimal::Decimal`] and checked operations.
//! No margin check is enforced: a buy whose cost exceeds cash is allowed and
//! produces negative cash, so callers remain responsible for position sizing.

use kurumi_core::{Currency, KurumiError, Price};
use rust_decimal::Decimal;

use crate::order::{Fill, Side};

/// One mark-to-market observation of the account.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::{EquityPoint, PortfolioAccount};
/// use kurumi_core::{Currency, Price};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let mut account = PortfolioAccount::new(Decimal::from(1_000), Currency::Usd)?;
/// account.record_equity(0, Price::new(Decimal::from(100))?)?;
/// let point = account.equity_curve().first().expect("one point");
/// assert_eq!(point.equity, Decimal::from(1_000));
/// assert_eq!(point.position, Decimal::ZERO);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EquityPoint {
    /// Bar timestamp in Unix milliseconds.
    pub timestamp: i64,
    /// Mark price used for this observation.
    pub price: Decimal,
    /// Cash balance after all executions up to this observation.
    pub cash: Decimal,
    /// Signed position at this observation.
    pub position: Decimal,
    /// Mark-to-market equity: `cash + position * price`.
    pub equity: Decimal,
}

/// A single-instrument portfolio ledger.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::PortfolioAccount;
/// use kurumi_core::Currency;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let account = PortfolioAccount::new(Decimal::from(10_000), Currency::Usd)?;
/// assert_eq!(account.cash(), Decimal::from(10_000));
/// assert_eq!(account.position(), Decimal::ZERO);
/// assert_eq!(account.average_price(), Decimal::ZERO);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct PortfolioAccount {
    currency: Currency,
    initial_cash: Decimal,
    cash: Decimal,
    position: Decimal,
    average_price: Decimal,
    realized_pnl: Decimal,
    total_fees: Decimal,
    equity_curve: Vec<EquityPoint>,
}

impl PortfolioAccount {
    /// Creates an account with an initial cash balance.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `initial_cash` is
    /// negative.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::PortfolioAccount;
    /// use kurumi_core::Currency;
    /// use rust_decimal::Decimal;
    ///
    /// assert!(PortfolioAccount::new(Decimal::from(1_000), Currency::Usd).is_ok());
    /// assert!(PortfolioAccount::new(Decimal::NEGATIVE_ONE, Currency::Usd).is_err());
    /// ```
    pub fn new(initial_cash: Decimal, currency: Currency) -> Result<Self, KurumiError> {
        if initial_cash.is_sign_negative() {
            return Err(KurumiError::InvalidParameter {
                parameter: "initial_cash",
                reason: "must be non-negative".to_owned(),
            });
        }
        Ok(Self {
            currency,
            initial_cash,
            cash: initial_cash,
            position: Decimal::ZERO,
            average_price: Decimal::ZERO,
            realized_pnl: Decimal::ZERO,
            total_fees: Decimal::ZERO,
            equity_curve: Vec::new(),
        })
    }

    /// Returns the account currency.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::PortfolioAccount;
    /// use kurumi_core::Currency;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let account = PortfolioAccount::new(Decimal::ONE, Currency::Eur)?;
    /// assert_eq!(account.currency(), Currency::Eur);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn currency(&self) -> Currency {
        self.currency
    }

    /// Returns the opening cash balance.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::PortfolioAccount;
    /// use kurumi_core::Currency;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let account = PortfolioAccount::new(Decimal::from(500), Currency::Usd)?;
    /// assert_eq!(account.initial_cash(), Decimal::from(500));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn initial_cash(&self) -> Decimal {
        self.initial_cash
    }

    /// Returns the current cash balance.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::PortfolioAccount;
    /// use kurumi_core::Currency;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let account = PortfolioAccount::new(Decimal::from(500), Currency::Usd)?;
    /// assert_eq!(account.cash(), Decimal::from(500));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn cash(&self) -> Decimal {
        self.cash
    }

    /// Returns the signed position (positive long, negative short).
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::PortfolioAccount;
    /// use kurumi_core::Currency;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let account = PortfolioAccount::new(Decimal::ZERO, Currency::Usd)?;
    /// assert_eq!(account.position(), Decimal::ZERO);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn position(&self) -> Decimal {
        self.position
    }

    /// Returns the average entry price of the open position, or zero when flat.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::PortfolioAccount;
    /// use kurumi_core::Currency;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let account = PortfolioAccount::new(Decimal::ZERO, Currency::Usd)?;
    /// assert_eq!(account.average_price(), Decimal::ZERO);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn average_price(&self) -> Decimal {
        self.average_price
    }

    /// Returns realized profit and loss, gross of fees.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::PortfolioAccount;
    /// use kurumi_core::Currency;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let account = PortfolioAccount::new(Decimal::from(100), Currency::Usd)?;
    /// assert_eq!(account.realized_pnl(), Decimal::ZERO);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn realized_pnl(&self) -> Decimal {
        self.realized_pnl
    }

    /// Returns the cumulative fees charged to the account.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::PortfolioAccount;
    /// use kurumi_core::Currency;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let account = PortfolioAccount::new(Decimal::from(100), Currency::Usd)?;
    /// assert_eq!(account.total_fees(), Decimal::ZERO);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn total_fees(&self) -> Decimal {
        self.total_fees
    }

    /// Returns the recorded equity curve.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::PortfolioAccount;
    /// use kurumi_core::Currency;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let account = PortfolioAccount::new(Decimal::ONE, Currency::Usd)?;
    /// assert!(account.equity_curve().is_empty());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn equity_curve(&self) -> &[EquityPoint] {
        &self.equity_curve
    }

    /// Returns the most recent equity observation, if any.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::PortfolioAccount;
    /// use kurumi_core::Currency;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let account = PortfolioAccount::new(Decimal::ONE, Currency::Usd)?;
    /// assert!(account.last_equity().is_none());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn last_equity(&self) -> Option<&EquityPoint> {
        self.equity_curve.last()
    }

    /// Returns the signed market value of the position at `price`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::NumericalOverflow`] when the product is not
    /// representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::PortfolioAccount;
    /// use kurumi_core::{Currency, Price};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let account = PortfolioAccount::new(Decimal::ZERO, Currency::Usd)?;
    /// let value = account.market_value(Price::new(Decimal::from(100))?)?;
    /// assert_eq!(value, Decimal::ZERO);
    /// # Ok(())
    /// # }
    /// ```
    pub fn market_value(&self, price: Price) -> Result<Decimal, KurumiError> {
        self.position
            .checked_mul(price.value())
            .ok_or(KurumiError::NumericalOverflow {
                operation: "PortfolioAccount::market_value",
            })
    }

    /// Returns the unrealized profit and loss at `price` (zero when flat).
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::NumericalOverflow`] when the arithmetic is not
    /// representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::PortfolioAccount;
    /// use kurumi_core::{Currency, Price};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let account = PortfolioAccount::new(Decimal::ZERO, Currency::Usd)?;
    /// let pnl = account.unrealized_pnl(Price::new(Decimal::from(100))?)?;
    /// assert_eq!(pnl, Decimal::ZERO);
    /// # Ok(())
    /// # }
    /// ```
    pub fn unrealized_pnl(&self, price: Price) -> Result<Decimal, KurumiError> {
        if self.position.is_zero() {
            return Ok(Decimal::ZERO);
        }
        price
            .value()
            .checked_sub(self.average_price)
            .and_then(|difference| difference.checked_mul(self.position))
            .ok_or(KurumiError::NumericalOverflow {
                operation: "PortfolioAccount::unrealized_pnl",
            })
    }

    /// Returns the mark-to-market equity at `price`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::NumericalOverflow`] when the arithmetic is not
    /// representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::PortfolioAccount;
    /// use kurumi_core::{Currency, Price};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let account = PortfolioAccount::new(Decimal::from(1_000), Currency::Usd)?;
    /// let equity = account.equity(Price::new(Decimal::from(100))?)?;
    /// assert_eq!(equity, Decimal::from(1_000));
    /// # Ok(())
    /// # }
    /// ```
    pub fn equity(&self, price: Price) -> Result<Decimal, KurumiError> {
        let value = self.market_value(price)?;
        self.cash
            .checked_add(value)
            .ok_or(KurumiError::NumericalOverflow {
                operation: "PortfolioAccount::equity",
            })
    }

    /// Appends a mark-to-market observation for `timestamp` at `price`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::NumericalOverflow`] when the equity is not
    /// representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::PortfolioAccount;
    /// use kurumi_core::{Currency, Price};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let mut account = PortfolioAccount::new(Decimal::from(1_000), Currency::Usd)?;
    /// account.record_equity(1_700_000_000_000, Price::new(Decimal::from(100))?)?;
    /// assert_eq!(account.equity_curve().len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    pub fn record_equity(&mut self, timestamp: i64, price: Price) -> Result<(), KurumiError> {
        let equity = self.equity(price)?;
        self.equity_curve.push(EquityPoint {
            timestamp,
            price: price.value(),
            cash: self.cash,
            position: self.position,
            equity,
        });
        Ok(())
    }

    /// Applies a simulated fill to cash, position, average cost, and realized
    /// profit and loss.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::NumericalOverflow`] when any ledger arithmetic is
    /// not representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{Fill, Liquidity, PortfolioAccount, Side};
    /// use kurumi_core::{Currency, Price, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let mut account = PortfolioAccount::new(Decimal::from(10_000), Currency::Usd)?;
    /// let fill = Fill::new(
    ///     1,
    ///     0,
    ///     Side::Buy,
    ///     Price::new(Decimal::from(10))?,
    ///     Quantity::new(Decimal::from(100))?,
    ///     Decimal::ZERO,
    ///     Liquidity::Taker,
    /// )?;
    /// account.apply_fill(&fill)?;
    /// assert_eq!(account.position(), Decimal::from(100));
    /// assert_eq!(account.average_price(), Decimal::from(10));
    /// assert_eq!(account.cash(), Decimal::from(9_000));
    /// # Ok(())
    /// # }
    /// ```
    pub fn apply_fill(&mut self, fill: &Fill) -> Result<(), KurumiError> {
        let quantity = fill.quantity().value();
        let signed_quantity = match fill.side() {
            Side::Buy => quantity,
            Side::Sell => {
                Decimal::ZERO
                    .checked_sub(quantity)
                    .ok_or(KurumiError::NumericalOverflow {
                        operation: "PortfolioAccount::apply_fill",
                    })?
            }
        };
        let price = fill.price().value();

        self.cash =
            self.cash
                .checked_add(fill.cash_flow())
                .ok_or(KurumiError::NumericalOverflow {
                    operation: "PortfolioAccount::apply_fill cash",
                })?;
        self.total_fees =
            self.total_fees
                .checked_add(fill.fee())
                .ok_or(KurumiError::NumericalOverflow {
                    operation: "PortfolioAccount::apply_fill fees",
                })?;

        let old_position = self.position;
        let old_absolute = old_position.abs();
        let new_position =
            old_position
                .checked_add(signed_quantity)
                .ok_or(KurumiError::NumericalOverflow {
                    operation: "PortfolioAccount::apply_fill position",
                })?;

        if old_position.is_zero() {
            self.average_price = price;
        } else if old_position.is_sign_negative() == signed_quantity.is_sign_negative() {
            // Adding to the position: blend the average over absolute sizes.
            let weighted = self
                .average_price
                .checked_mul(old_absolute)
                .and_then(|existing| {
                    price
                        .checked_mul(quantity)
                        .and_then(|added| existing.checked_add(added))
                })
                .ok_or(KurumiError::NumericalOverflow {
                    operation: "PortfolioAccount::apply_fill average",
                })?;
            let total =
                old_absolute
                    .checked_add(quantity)
                    .ok_or(KurumiError::NumericalOverflow {
                        operation: "PortfolioAccount::apply_fill average",
                    })?;
            self.average_price =
                weighted
                    .checked_div(total)
                    .ok_or(KurumiError::NumericalOverflow {
                        operation: "PortfolioAccount::apply_fill average",
                    })?;
        } else {
            // Reducing: realize the closed part against the average cost.
            let closed = old_absolute.min(quantity);
            let direction = if old_position.is_sign_negative() {
                Decimal::NEGATIVE_ONE
            } else {
                Decimal::ONE
            };
            let realized = price
                .checked_sub(self.average_price)
                .and_then(|difference| difference.checked_mul(closed))
                .and_then(|pnl| pnl.checked_mul(direction))
                .ok_or(KurumiError::NumericalOverflow {
                    operation: "PortfolioAccount::apply_fill realized",
                })?;
            self.realized_pnl =
                self.realized_pnl
                    .checked_add(realized)
                    .ok_or(KurumiError::NumericalOverflow {
                        operation: "PortfolioAccount::apply_fill realized",
                    })?;
            if quantity > old_absolute {
                // The position flipped: the remainder opens at the fill price.
                self.average_price = price;
            } else if quantity == old_absolute {
                self.average_price = Decimal::ZERO;
            }
        }

        self.position = new_position;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurumi_core::Quantity;

    fn price(value: i64) -> Price {
        Price::new(Decimal::from(value)).expect("valid test price")
    }

    fn fill(side: Side, price_value: i64, quantity: i64, fee: i64) -> Fill {
        Fill::new(
            1,
            0,
            side,
            price(price_value),
            Quantity::new(Decimal::from(quantity)).expect("valid test quantity"),
            Decimal::from(fee),
            crate::fee::Liquidity::Taker,
        )
        .expect("valid test fill")
    }

    #[test]
    fn new_account_rejects_negative_cash_and_starts_flat() {
        assert!(PortfolioAccount::new(Decimal::NEGATIVE_ONE, Currency::Usd).is_err());
        let account =
            PortfolioAccount::new(Decimal::from(1_000), Currency::Usd).expect("valid account");
        assert_eq!(account.cash(), Decimal::from(1_000));
        assert_eq!(account.position(), Decimal::ZERO);
        assert_eq!(account.average_price(), Decimal::ZERO);
        assert_eq!(account.realized_pnl(), Decimal::ZERO);
        assert!(account.equity_curve().is_empty());
    }

    #[test]
    fn buying_updates_cash_position_and_unrealized_pnl() {
        let mut account =
            PortfolioAccount::new(Decimal::from(10_000), Currency::Usd).expect("valid account");
        account
            .apply_fill(&fill(Side::Buy, 10, 100, 0))
            .expect("buy fill");
        assert_eq!(account.cash(), Decimal::from(9_000));
        assert_eq!(account.position(), Decimal::from(100));
        assert_eq!(account.average_price(), Decimal::from(10));
        assert_eq!(
            account.unrealized_pnl(price(12)).expect("unrealized pnl"),
            Decimal::from(200)
        );
        assert_eq!(
            account.market_value(price(12)).expect("market value"),
            Decimal::from(1_200)
        );
        assert_eq!(
            account.equity(price(12)).expect("equity"),
            Decimal::from(10_200)
        );
    }

    #[test]
    fn adding_to_a_position_blends_the_average_price() {
        let mut account =
            PortfolioAccount::new(Decimal::from(10_000), Currency::Usd).expect("valid account");
        account
            .apply_fill(&fill(Side::Buy, 10, 100, 0))
            .expect("first buy");
        account
            .apply_fill(&fill(Side::Buy, 20, 100, 0))
            .expect("second buy");
        assert_eq!(account.position(), Decimal::from(200));
        assert_eq!(account.average_price(), Decimal::from(15));
        assert_eq!(account.cash(), Decimal::from(7_000));
    }

    #[test]
    fn partial_reduction_realizes_pnl_and_keeps_the_average() {
        let mut account =
            PortfolioAccount::new(Decimal::from(10_000), Currency::Usd).expect("valid account");
        account
            .apply_fill(&fill(Side::Buy, 10, 100, 0))
            .expect("buy fill");
        account
            .apply_fill(&fill(Side::Sell, 12, 40, 0))
            .expect("sell fill");
        assert_eq!(account.position(), Decimal::from(60));
        assert_eq!(account.average_price(), Decimal::from(10));
        assert_eq!(account.realized_pnl(), Decimal::from(80));
        assert_eq!(account.cash(), Decimal::from(9_480));
        assert_eq!(
            account.equity(price(12)).expect("equity"),
            Decimal::from(10_200)
        );
    }

    #[test]
    fn crossing_through_zero_opens_the_new_position_at_the_fill_price() {
        let mut account =
            PortfolioAccount::new(Decimal::from(10_000), Currency::Usd).expect("valid account");
        account
            .apply_fill(&fill(Side::Buy, 10, 100, 0))
            .expect("buy fill");
        account
            .apply_fill(&fill(Side::Sell, 12, 150, 0))
            .expect("sell fill");
        assert_eq!(account.position(), Decimal::from(-50));
        assert_eq!(account.average_price(), Decimal::from(12));
        assert_eq!(account.realized_pnl(), Decimal::from(200));
        assert_eq!(account.cash(), Decimal::from(10_800));
        assert_eq!(
            account.equity(price(12)).expect("equity"),
            Decimal::from(10_200)
        );
    }

    #[test]
    fn short_positions_realize_profit_when_prices_fall() {
        let mut account =
            PortfolioAccount::new(Decimal::from(10_000), Currency::Usd).expect("valid account");
        account
            .apply_fill(&fill(Side::Sell, 10, 100, 0))
            .expect("short fill");
        assert_eq!(account.position(), Decimal::from(-100));
        assert_eq!(account.average_price(), Decimal::from(10));
        assert_eq!(account.cash(), Decimal::from(11_000));
        assert_eq!(
            account.unrealized_pnl(price(8)).expect("unrealized pnl"),
            Decimal::from(200)
        );
        account
            .apply_fill(&fill(Side::Buy, 8, 100, 0))
            .expect("cover fill");
        assert_eq!(account.position(), Decimal::ZERO);
        assert_eq!(account.average_price(), Decimal::ZERO);
        assert_eq!(account.realized_pnl(), Decimal::from(200));
    }

    #[test]
    fn fees_leave_cash_and_are_tracked_separately() {
        let mut account =
            PortfolioAccount::new(Decimal::from(10_000), Currency::Usd).expect("valid account");
        account
            .apply_fill(&fill(Side::Buy, 10, 100, 5))
            .expect("buy fill");
        assert_eq!(account.cash(), Decimal::from(8_995));
        assert_eq!(account.total_fees(), Decimal::from(5));
        assert_eq!(account.realized_pnl(), Decimal::ZERO);
    }

    #[test]
    fn equity_curve_records_each_mark() {
        let mut account =
            PortfolioAccount::new(Decimal::from(10_000), Currency::Usd).expect("valid account");
        account
            .apply_fill(&fill(Side::Buy, 10, 100, 0))
            .expect("buy fill");
        account.record_equity(1, price(11)).expect("first mark");
        account.record_equity(2, price(12)).expect("second mark");
        assert_eq!(account.equity_curve().len(), 2);
        let latest = account.last_equity().expect("latest mark");
        assert_eq!(latest.timestamp, 2);
        assert_eq!(latest.position, Decimal::from(100));
        assert_eq!(latest.equity, Decimal::from(10_200));
    }
}
