//! Multi-asset portfolio accounting.
//!
//! [`MultiAssetAccount`] is the ledger of a portfolio that holds several
//! instruments at once. Cash, quantities, average costs, fees, slippage, traded
//! value, and equity all stay in [`rust_decimal::Decimal`]; the weighted-average
//! cost model matches the single-asset [`crate::account::PortfolioAccount`]
//! while keeping one [`AssetPosition`] per symbol in a deterministic
//! [`BTreeMap`].
//!
//! # Accounting identities
//!
//! For every applied fill,
//!
//! ```text
//! equity = initial_cash + total_realized_pnl + total_unrealized_pnl - total_fees
//! ```
//!
//! holds exactly in [`Decimal`]. Slippage is reported separately and is already
//! embedded in the execution price, so it is informational rather than a second
//! deduction from cash. A fill that would make cash negative is rejected before
//! any ledger state changes.

use std::collections::BTreeMap;

use kurumi_core::{KurumiError, Price, Quantity};
use rust_decimal::Decimal;

use crate::order::Side;
use crate::portfolio::feed::MarketSnapshot;

/// One instrument's position inside a [`MultiAssetAccount`].
///
/// The position uses the weighted-average-cost model: adding to a position
/// blends [`AssetPosition::avg_cost`], reducing it realizes profit or loss
/// against that average, and crossing through zero closes the old position and
/// opens the new one at the fill price. A negative quantity is a short position.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::AssetPosition;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let position = AssetPosition::new("AAA".to_owned(), Decimal::from(100), Decimal::from(10))?;
/// assert_eq!(position.quantity(), Decimal::from(100));
/// assert_eq!(position.market_value(Decimal::from(12))?, Decimal::from(1_200));
/// assert_eq!(position.unrealized_pnl(Decimal::from(12))?, Decimal::from(200));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct AssetPosition {
    symbol: String,
    quantity: Decimal,
    avg_cost: Decimal,
    realized_pnl: Decimal,
}

impl AssetPosition {
    /// Creates a position from its symbol, signed quantity, and average cost.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the symbol is empty or
    /// `avg_cost` is negative.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::AssetPosition;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// assert!(AssetPosition::new("AAA".to_owned(), Decimal::ONE, Decimal::ONE).is_ok());
    /// assert!(AssetPosition::new(String::new(), Decimal::ONE, Decimal::ONE).is_err());
    /// assert!(AssetPosition::new("AAA".to_owned(), Decimal::ONE, Decimal::NEGATIVE_ONE).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        symbol: impl Into<String>,
        quantity: Decimal,
        avg_cost: Decimal,
    ) -> Result<Self, KurumiError> {
        let symbol = symbol.into();
        if symbol.trim().is_empty() {
            return Err(KurumiError::InvalidParameter {
                parameter: "symbol",
                reason: "must not be empty".to_owned(),
            });
        }
        if avg_cost.is_sign_negative() {
            return Err(KurumiError::InvalidParameter {
                parameter: "avg_cost",
                reason: "must be non-negative".to_owned(),
            });
        }
        Ok(Self {
            symbol,
            quantity,
            avg_cost,
            realized_pnl: Decimal::ZERO,
        })
    }

    /// Returns the instrument symbol.
    #[must_use]
    #[inline]
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// Returns the signed quantity (positive long, negative short).
    #[must_use]
    #[inline]
    pub fn quantity(&self) -> Decimal {
        self.quantity
    }

    /// Returns the weighted-average entry price, or zero when flat.
    #[must_use]
    #[inline]
    pub fn avg_cost(&self) -> Decimal {
        self.avg_cost
    }

    /// Returns the cumulative realized profit and loss, gross of fees.
    #[must_use]
    #[inline]
    pub fn realized_pnl(&self) -> Decimal {
        self.realized_pnl
    }

    /// Returns `true` when the position is flat.
    #[must_use]
    #[inline]
    pub fn is_flat(&self) -> bool {
        self.quantity.is_zero()
    }

    /// Returns the signed market value at `price`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::NumericalOverflow`] when the product is not
    /// representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::AssetPosition;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let position = AssetPosition::new("AAA".to_owned(), Decimal::from(10), Decimal::from(5))?;
    /// assert_eq!(position.market_value(Decimal::from(7))?, Decimal::from(70));
    /// # Ok(())
    /// # }
    /// ```
    pub fn market_value(&self, price: Decimal) -> Result<Decimal, KurumiError> {
        self.quantity
            .checked_mul(price)
            .ok_or(overflow("AssetPosition::market_value"))
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
    /// use kurumi_backtest::portfolio::AssetPosition;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let position = AssetPosition::new("AAA".to_owned(), Decimal::from(10), Decimal::from(5))?;
    /// assert_eq!(position.unrealized_pnl(Decimal::from(7))?, Decimal::from(20));
    /// # Ok(())
    /// # }
    /// ```
    pub fn unrealized_pnl(&self, price: Decimal) -> Result<Decimal, KurumiError> {
        if self.quantity.is_zero() {
            return Ok(Decimal::ZERO);
        }
        price
            .checked_sub(self.avg_cost)
            .and_then(|difference| difference.checked_mul(self.quantity))
            .ok_or(overflow("AssetPosition::unrealized_pnl"))
    }

    /// Applies a signed trade to the weighted-average-cost position.
    ///
    /// `quantity` is a positive magnitude and `side` carries the direction.
    fn apply_trade(
        &mut self,
        side: Side,
        price: Decimal,
        quantity: Decimal,
    ) -> Result<(), KurumiError> {
        let signed_quantity = match side {
            Side::Buy => quantity,
            Side::Sell => Decimal::ZERO
                .checked_sub(quantity)
                .ok_or(overflow("AssetPosition::apply_trade"))?,
        };
        let old_position = self.quantity;
        let old_absolute = old_position.abs();
        let new_position = old_position
            .checked_add(signed_quantity)
            .ok_or(overflow("AssetPosition::apply_trade position"))?;

        if old_position.is_zero() {
            self.avg_cost = price;
        } else if old_position.is_sign_negative() == signed_quantity.is_sign_negative() {
            let weighted = self
                .avg_cost
                .checked_mul(old_absolute)
                .and_then(|existing| {
                    price
                        .checked_mul(quantity)
                        .and_then(|added| existing.checked_add(added))
                })
                .ok_or(overflow("AssetPosition::apply_trade average"))?;
            let total = old_absolute
                .checked_add(quantity)
                .ok_or(overflow("AssetPosition::apply_trade average"))?;
            self.avg_cost = weighted
                .checked_div(total)
                .ok_or(overflow("AssetPosition::apply_trade average"))?;
        } else {
            let closed = old_absolute.min(quantity);
            let direction = if old_position.is_sign_negative() {
                Decimal::NEGATIVE_ONE
            } else {
                Decimal::ONE
            };
            let realized = price
                .checked_sub(self.avg_cost)
                .and_then(|difference| difference.checked_mul(closed))
                .and_then(|pnl| pnl.checked_mul(direction))
                .ok_or(overflow("AssetPosition::apply_trade realized"))?;
            self.realized_pnl = self
                .realized_pnl
                .checked_add(realized)
                .ok_or(overflow("AssetPosition::apply_trade realized"))?;
            if quantity > old_absolute {
                self.avg_cost = price;
            } else if quantity == old_absolute {
                self.avg_cost = Decimal::ZERO;
            }
        }

        self.quantity = new_position;
        Ok(())
    }

    /// Creates the empty position used the first time a symbol is filled.
    fn flat(symbol: &str) -> Self {
        Self {
            symbol: symbol.to_owned(),
            quantity: Decimal::ZERO,
            avg_cost: Decimal::ZERO,
            realized_pnl: Decimal::ZERO,
        }
    }
}

/// One simulated execution of a multi-asset account.
///
/// Prices, fees, and slippage are decimal. `slippage` is the non-negative cost
/// already embedded in `price`; it is accumulated for reporting and never
/// deducted from cash a second time.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::MultiAssetFill;
/// use kurumi_backtest::Side;
/// use kurumi_core::{Price, Quantity};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let fill = MultiAssetFill::new(
///     "AAA",
///     Side::Buy,
///     Price::new(Decimal::from(10))?,
///     Quantity::new(Decimal::from(100))?,
///     Decimal::ONE,
///     Decimal::ZERO,
/// )?;
/// assert_eq!(fill.notional()?, Decimal::from(1_000));
/// assert_eq!(fill.cash_flow()?, Decimal::from(-1_001));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MultiAssetFill {
    symbol: String,
    side: Side,
    price: Price,
    quantity: Quantity,
    fee: Decimal,
    slippage: Decimal,
}

impl MultiAssetFill {
    /// Creates a validated multi-asset fill.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the symbol is empty, the
    /// quantity is zero, or the fee or slippage is negative.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::MultiAssetFill;
    /// use kurumi_backtest::Side;
    /// use kurumi_core::{Price, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let price = Price::new(Decimal::ONE)?;
    /// let quantity = Quantity::new(Decimal::ONE)?;
    /// assert!(MultiAssetFill::new("AAA", Side::Buy, price, quantity, Decimal::ZERO, Decimal::ZERO).is_ok());
    /// assert!(MultiAssetFill::new("AAA", Side::Buy, price, Quantity::new(Decimal::ZERO)?, Decimal::ZERO, Decimal::ZERO).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        symbol: impl Into<String>,
        side: Side,
        price: Price,
        quantity: Quantity,
        fee: Decimal,
        slippage: Decimal,
    ) -> Result<Self, KurumiError> {
        let symbol = symbol.into();
        if symbol.trim().is_empty() {
            return Err(KurumiError::InvalidParameter {
                parameter: "symbol",
                reason: "must not be empty".to_owned(),
            });
        }
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
        if slippage.is_sign_negative() {
            return Err(KurumiError::InvalidParameter {
                parameter: "slippage",
                reason: "must be non-negative".to_owned(),
            });
        }
        Ok(Self {
            symbol,
            side,
            price,
            quantity,
            fee,
            slippage,
        })
    }

    /// Returns the instrument symbol.
    #[must_use]
    #[inline]
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// Returns the fill side.
    #[must_use]
    #[inline]
    pub const fn side(&self) -> Side {
        self.side
    }

    /// Returns the execution price after slippage.
    #[must_use]
    #[inline]
    pub const fn price(&self) -> Price {
        self.price
    }

    /// Returns the executed quantity as a positive magnitude.
    #[must_use]
    #[inline]
    pub const fn quantity(&self) -> Quantity {
        self.quantity
    }

    /// Returns the fee charged for the fill.
    #[must_use]
    #[inline]
    pub const fn fee(&self) -> Decimal {
        self.fee
    }

    /// Returns the non-negative slippage cost embedded in the execution price.
    #[must_use]
    #[inline]
    pub const fn slippage(&self) -> Decimal {
        self.slippage
    }

    /// Returns the pre-fee notional value `price * quantity`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::NumericalOverflow`] when the product is not
    /// representable.
    pub fn notional(&self) -> Result<Decimal, KurumiError> {
        self.price
            .value()
            .checked_mul(self.quantity.value())
            .ok_or(overflow("MultiAssetFill::notional"))
    }

    /// Returns the signed cash flow: negative for a buy including its fee and
    /// positive for a sell net of its fee.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::NumericalOverflow`] when the arithmetic is not
    /// representable.
    pub fn cash_flow(&self) -> Result<Decimal, KurumiError> {
        let notional = self.notional()?;
        match self.side {
            Side::Buy => notional
                .checked_add(self.fee)
                .and_then(|total| Decimal::ZERO.checked_sub(total)),
            Side::Sell => notional.checked_sub(self.fee),
        }
        .ok_or(overflow("MultiAssetFill::cash_flow"))
    }
}

/// A multi-asset portfolio ledger.
///
/// Positions, prices, and cash stay in [`Decimal`] and are held in [`BTreeMap`]
/// containers, so the account's iteration order never depends on hashing. The
/// account accepts signed fills but rejects any fill that would leave cash
/// negative: there is no implicit margin.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::{MultiAssetAccount, MultiAssetFill};
/// use kurumi_backtest::Side;
/// use kurumi_core::{Price, Quantity};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let mut account = MultiAssetAccount::new(Decimal::from(1_000))?;
/// let fill = MultiAssetFill::new(
///     "AAA",
///     Side::Buy,
///     Price::new(Decimal::from(10))?,
///     Quantity::new(Decimal::from(50))?,
///     Decimal::ONE,
///     Decimal::ZERO,
/// )?;
/// account.apply_fill(&fill)?;
/// assert_eq!(account.cash(), Decimal::from(499));
/// assert_eq!(account.equity()?, Decimal::from(999));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MultiAssetAccount {
    initial_cash: Decimal,
    cash: Decimal,
    positions: BTreeMap<String, AssetPosition>,
    last_known_prices: BTreeMap<String, Decimal>,
    total_fees: Decimal,
    total_slippage: Decimal,
    total_traded_value: Decimal,
}

impl MultiAssetAccount {
    /// Creates a flat account with `initial_cash`.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `initial_cash` is
    /// negative.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::MultiAssetAccount;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// assert!(MultiAssetAccount::new(Decimal::from(1_000)).is_ok());
    /// assert!(MultiAssetAccount::new(Decimal::NEGATIVE_ONE).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(initial_cash: Decimal) -> Result<Self, KurumiError> {
        if initial_cash < Decimal::ZERO {
            return Err(KurumiError::InvalidParameter {
                parameter: "initial_cash",
                reason: "must be non-negative".to_owned(),
            });
        }
        Ok(Self {
            initial_cash,
            cash: initial_cash,
            positions: BTreeMap::new(),
            last_known_prices: BTreeMap::new(),
            total_fees: Decimal::ZERO,
            total_slippage: Decimal::ZERO,
            total_traded_value: Decimal::ZERO,
        })
    }

    /// Returns the opening cash balance.
    #[must_use]
    #[inline]
    pub const fn initial_cash(&self) -> Decimal {
        self.initial_cash
    }

    /// Returns the current cash balance.
    #[must_use]
    #[inline]
    pub const fn cash(&self) -> Decimal {
        self.cash
    }

    /// Returns the per-symbol positions in symbol order.
    #[must_use]
    #[inline]
    pub fn positions(&self) -> &BTreeMap<String, AssetPosition> {
        &self.positions
    }

    /// Returns the position of `symbol`, if the symbol has ever been filled.
    #[must_use]
    #[inline]
    pub fn position(&self, symbol: &str) -> Option<&AssetPosition> {
        self.positions.get(symbol)
    }

    /// Returns the last known prices in symbol order.
    #[must_use]
    #[inline]
    pub fn last_known_prices(&self) -> &BTreeMap<String, Decimal> {
        &self.last_known_prices
    }

    /// Returns the last known price of `symbol`, if any.
    #[must_use]
    #[inline]
    pub fn last_known_price(&self, symbol: &str) -> Option<Decimal> {
        self.last_known_prices.get(symbol).copied()
    }

    /// Returns cumulative fees charged to the account.
    #[must_use]
    #[inline]
    pub const fn total_fees(&self) -> Decimal {
        self.total_fees
    }

    /// Returns cumulative slippage cost embedded in execution prices.
    #[must_use]
    #[inline]
    pub const fn total_slippage(&self) -> Decimal {
        self.total_slippage
    }

    /// Returns cumulative one-way traded notional.
    #[must_use]
    #[inline]
    pub const fn total_traded_value(&self) -> Decimal {
        self.total_traded_value
    }

    /// Marks the universe with the close prices of `snapshot`.
    ///
    /// Symbols without a bar in the snapshot keep their previous price, so a
    /// market holiday or trading halt does not mark a position to zero.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when a close price cannot be
    /// represented as a strictly positive [`Decimal`]. The update is atomic:
    /// no price changes when any conversion fails.
    pub fn update_market_prices(&mut self, snapshot: &MarketSnapshot) -> Result<(), KurumiError> {
        let prices = snapshot.close_prices()?;
        for (symbol, price) in prices {
            self.last_known_prices.insert(symbol, price);
        }
        Ok(())
    }

    /// Returns the mark price of `position`.
    fn mark_price(&self, position: &AssetPosition) -> Decimal {
        self.last_known_prices
            .get(position.symbol())
            .copied()
            .unwrap_or_else(|| position.avg_cost())
    }

    /// Returns total equity: cash plus every position at its last known price.
    ///
    /// Positions without a market snapshot are marked at their average cost,
    /// because [`MultiAssetAccount::apply_fill`] records each execution price
    /// as the symbol's last known price.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::NumericalOverflow`] when the arithmetic is not
    /// representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::MultiAssetAccount;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let account = MultiAssetAccount::new(Decimal::from(1_000))?;
    /// assert_eq!(account.equity()?, Decimal::from(1_000));
    /// # Ok(())
    /// # }
    /// ```
    pub fn equity(&self) -> Result<Decimal, KurumiError> {
        let mut total = self.cash;
        for position in self.positions.values() {
            if position.is_flat() {
                continue;
            }
            let value = position.market_value(self.mark_price(position))?;
            total = total
                .checked_add(value)
                .ok_or(overflow("MultiAssetAccount::equity"))?;
        }
        Ok(total)
    }

    /// Returns cumulative realized profit and loss, gross of fees.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::NumericalOverflow`] when the sum is not
    /// representable.
    pub fn realized_pnl(&self) -> Result<Decimal, KurumiError> {
        let mut total = Decimal::ZERO;
        for position in self.positions.values() {
            total = total
                .checked_add(position.realized_pnl())
                .ok_or(overflow("MultiAssetAccount::realized_pnl"))?;
        }
        Ok(total)
    }

    /// Returns cumulative unrealized profit and loss at the last known prices.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::NumericalOverflow`] when the arithmetic is not
    /// representable.
    pub fn unrealized_pnl(&self) -> Result<Decimal, KurumiError> {
        let mut total = Decimal::ZERO;
        for position in self.positions.values() {
            let pnl = position.unrealized_pnl(self.mark_price(position))?;
            total = total
                .checked_add(pnl)
                .ok_or(overflow("MultiAssetAccount::unrealized_pnl"))?;
        }
        Ok(total)
    }

    /// Returns each non-flat position's market value as a fraction of equity.
    ///
    /// Weights are `market_value / equity` and are returned as `f64` because
    /// the portfolio optimizers consume `f64` weights. Cash is not represented
    /// in the map, so the weights sum to at most one.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when equity is not strictly
    /// positive, and [`KurumiError::NumericalOverflow`] when a weight cannot be
    /// represented.
    pub fn current_weights(&self) -> Result<BTreeMap<String, f64>, KurumiError> {
        let equity = self.equity()?;
        if equity <= Decimal::ZERO {
            return Err(KurumiError::InvalidParameter {
                parameter: "equity",
                reason: "must be strictly positive to compute weights".to_owned(),
            });
        }
        let mut weights = BTreeMap::new();
        for (symbol, position) in &self.positions {
            if position.is_flat() {
                continue;
            }
            let value = position.market_value(self.mark_price(position))?;
            let weight = value
                .checked_div(equity)
                .ok_or(overflow("MultiAssetAccount::current_weights"))?;
            let weight = f64::try_from(weight)
                .map_err(|_| overflow("MultiAssetAccount::current_weights"))?;
            weights.insert(symbol.clone(), weight);
        }
        Ok(weights)
    }

    /// Applies a fill to cash, the symbol's position, and the cost totals.
    ///
    /// The fill is rejected when it would leave cash negative; in that case
    /// the ledger is unchanged. A sell may open or extend a short position, but
    /// the account never borrows cash implicitly.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the fill would overdraw
    /// cash, and [`KurumiError::NumericalOverflow`] when ledger arithmetic is
    /// not representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::{MultiAssetAccount, MultiAssetFill};
    /// use kurumi_backtest::Side;
    /// use kurumi_core::{Price, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let mut account = MultiAssetAccount::new(Decimal::from(1_000))?;
    /// let fill = MultiAssetFill::new(
    ///     "AAA",
    ///     Side::Buy,
    ///     Price::new(Decimal::from(10))?,
    ///     Quantity::new(Decimal::from(50))?,
    ///     Decimal::ONE,
    ///     Decimal::ZERO,
    /// )?;
    /// account.apply_fill(&fill)?;
    /// assert_eq!(account.position("AAA").map(|p| p.quantity()), Some(Decimal::from(50)));
    /// assert_eq!(account.total_traded_value(), Decimal::from(500));
    /// # Ok(())
    /// # }
    /// ```
    pub fn apply_fill(&mut self, fill: &MultiAssetFill) -> Result<(), KurumiError> {
        let notional = fill.notional()?;
        let cash_flow = fill.cash_flow()?;
        let new_cash = self
            .cash
            .checked_add(cash_flow)
            .ok_or(overflow("MultiAssetAccount::apply_fill cash"))?;
        if new_cash < Decimal::ZERO {
            return Err(KurumiError::InvalidParameter {
                parameter: "cash",
                reason: format!(
                    "fill for `{}` would leave cash at {new_cash}; implicit overdraft is forbidden",
                    fill.symbol()
                ),
            });
        }

        let mut position = match self.positions.get(fill.symbol()) {
            Some(position) => position.clone(),
            None => AssetPosition::flat(fill.symbol()),
        };
        position.apply_trade(fill.side(), fill.price().value(), fill.quantity().value())?;

        let new_fees = self
            .total_fees
            .checked_add(fill.fee())
            .ok_or(overflow("MultiAssetAccount::apply_fill fees"))?;
        let new_slippage = self
            .total_slippage
            .checked_add(fill.slippage())
            .ok_or(overflow("MultiAssetAccount::apply_fill slippage"))?;
        let new_traded = self
            .total_traded_value
            .checked_add(notional)
            .ok_or(overflow("MultiAssetAccount::apply_fill traded value"))?;

        self.positions.insert(fill.symbol().to_owned(), position);
        self.cash = new_cash;
        self.total_fees = new_fees;
        self.total_slippage = new_slippage;
        self.total_traded_value = new_traded;
        self.last_known_prices
            .insert(fill.symbol().to_owned(), fill.price().value());
        Ok(())
    }
}

fn overflow(operation: &'static str) -> KurumiError {
    KurumiError::NumericalOverflow { operation }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use kurumi_ta::OhlcvBar;

    const DAY: i64 = 86_400_000;

    fn price(value: i64) -> Price {
        Price::new(Decimal::from(value)).expect("valid test price")
    }

    fn quantity(value: i64) -> Quantity {
        Quantity::new(Decimal::from(value)).expect("valid test quantity")
    }

    fn named_fill(
        symbol: &str,
        side: Side,
        fill_price: i64,
        size: i64,
        fee: i64,
        slippage: i64,
    ) -> MultiAssetFill {
        MultiAssetFill::new(
            symbol,
            side,
            price(fill_price),
            quantity(size),
            Decimal::from(fee),
            Decimal::from(slippage),
        )
        .expect("valid test fill")
    }

    fn fill(side: Side, fill_price: i64, size: i64, fee: i64, slippage: i64) -> MultiAssetFill {
        named_fill("AAA", side, fill_price, size, fee, slippage)
    }

    fn snapshot(day: i64, closes: &[(&str, f64)]) -> MarketSnapshot {
        let timestamp = DateTime::<Utc>::from_timestamp_millis(day * DAY).expect("valid day");
        let mut bars = BTreeMap::new();
        for (symbol, close) in closes {
            bars.insert(
                (*symbol).to_owned(),
                OhlcvBar::new(
                    day * DAY,
                    *close,
                    *close + 1.0,
                    *close - 1.0,
                    *close,
                    1_000.0,
                    *close * 1_000.0,
                )
                .expect("valid test bar"),
            );
        }
        MarketSnapshot::new(timestamp, bars).expect("valid snapshot")
    }

    #[test]
    fn complete_buy_sell_cycle_preserves_the_equity_identity() {
        let mut account = MultiAssetAccount::new(Decimal::from(10_000)).expect("valid account");
        account
            .update_market_prices(&snapshot(0, &[("AAA", 10.0), ("BBB", 20.0)]))
            .expect("day zero prices");

        let buy_a = named_fill("AAA", Side::Buy, 10, 100, 2, 1);
        let buy_b = named_fill("BBB", Side::Buy, 20, 100, 3, 2);
        account.apply_fill(&buy_a).expect("buy AAA");
        account.apply_fill(&buy_b).expect("buy BBB");

        assert_eq!(account.cash(), Decimal::from(6_995));
        account
            .update_market_prices(&snapshot(1, &[("AAA", 12.0), ("BBB", 18.0)]))
            .expect("day one prices");
        assert_eq!(account.equity().expect("equity"), Decimal::from(9_995));
        assert_eq!(account.realized_pnl().expect("realized"), Decimal::ZERO);
        assert_eq!(account.unrealized_pnl().expect("unrealized"), Decimal::ZERO);

        let sell_a = named_fill("AAA", Side::Sell, 12, 100, 1, 1);
        account.apply_fill(&sell_a).expect("sell AAA");
        assert_eq!(account.cash(), Decimal::from(8_194));
        assert_eq!(
            account.realized_pnl().expect("realized"),
            Decimal::from(200)
        );

        account
            .update_market_prices(&snapshot(2, &[("BBB", 22.0)]))
            .expect("day two prices");
        assert_eq!(account.equity().expect("equity"), Decimal::from(10_394));
        assert_eq!(
            account.unrealized_pnl().expect("unrealized"),
            Decimal::from(200)
        );

        let sell_b = named_fill("BBB", Side::Sell, 22, 100, 1, 1);
        account.apply_fill(&sell_b).expect("sell BBB");

        let equity = account.equity().expect("equity");
        let realized = account.realized_pnl().expect("realized");
        let unrealized = account.unrealized_pnl().expect("unrealized");
        assert_eq!(equity, Decimal::from(10_393));
        assert_eq!(realized, Decimal::from(400));
        assert_eq!(unrealized, Decimal::ZERO);
        assert_eq!(account.total_fees(), Decimal::from(7));
        assert_eq!(account.total_slippage(), Decimal::from(5));
        assert_eq!(account.total_traded_value(), Decimal::from(6_400));
        assert_eq!(
            equity,
            account.initial_cash() + realized + unrealized - account.total_fees()
        );
    }

    #[test]
    fn market_snapshots_mark_positions_and_keep_stale_prices() {
        let mut account = MultiAssetAccount::new(Decimal::from(1_000)).expect("valid account");
        account
            .update_market_prices(&snapshot(0, &[("AAA", 10.0), ("BBB", 20.0)]))
            .expect("day zero prices");

        let buy_a = named_fill("AAA", Side::Buy, 10, 10, 0, 0);
        let buy_b = named_fill("BBB", Side::Buy, 20, 10, 0, 0);
        account.apply_fill(&buy_a).expect("buy AAA");
        account.apply_fill(&buy_b).expect("buy BBB");

        account
            .update_market_prices(&snapshot(1, &[("AAA", 12.0)]))
            .expect("AAA only");
        assert_eq!(account.last_known_price("AAA"), Some(Decimal::from(12)));
        assert_eq!(account.last_known_price("BBB"), Some(Decimal::from(20)));
        assert_eq!(account.equity().expect("equity"), Decimal::from(1_020));
        assert_eq!(
            account.unrealized_pnl().expect("unrealized"),
            Decimal::from(20)
        );
    }

    #[test]
    fn current_weights_are_market_values_over_equity() {
        let mut account = MultiAssetAccount::new(Decimal::from(1_000)).expect("valid account");
        account
            .update_market_prices(&snapshot(0, &[("AAA", 10.0), ("BBB", 20.0)]))
            .expect("day zero prices");
        let buy_a = named_fill("AAA", Side::Buy, 10, 50, 0, 0);
        let buy_b = named_fill("BBB", Side::Buy, 20, 25, 0, 0);
        account.apply_fill(&buy_a).expect("buy AAA");
        account.apply_fill(&buy_b).expect("buy BBB");

        let weights = account.current_weights().expect("weights");
        assert_eq!(weights.get("AAA"), Some(&0.5));
        assert_eq!(weights.get("BBB"), Some(&0.5));
        let sum: f64 = weights.values().sum();
        assert!((sum - 1.0).abs() < 1e-12);

        let empty = MultiAssetAccount::new(Decimal::ZERO).expect("zero cash account");
        assert!(empty.current_weights().is_err());
    }

    #[test]
    fn a_fill_that_would_overdraw_cash_is_rejected_without_mutating_the_ledger() {
        let mut account = MultiAssetAccount::new(Decimal::from(500)).expect("valid account");
        let buy = fill(Side::Buy, 10, 100, 0, 0);
        assert!(account.apply_fill(&buy).is_err());
        assert_eq!(account.cash(), Decimal::from(500));
        assert!(account.positions().is_empty());
        assert_eq!(account.total_traded_value(), Decimal::ZERO);
        assert_eq!(account.total_fees(), Decimal::ZERO);

        let mut empty = MultiAssetAccount::new(Decimal::ZERO).expect("zero cash account");
        let sell_with_fee = fill(Side::Sell, 1, 1, 5, 0);
        assert!(empty.apply_fill(&sell_with_fee).is_err());
        assert_eq!(empty.cash(), Decimal::ZERO);
        assert!(empty.positions().is_empty());
    }

    #[test]
    fn weighted_average_cost_blends_and_realizes_per_symbol() {
        let mut account = MultiAssetAccount::new(Decimal::from(10_000)).expect("valid account");
        account
            .apply_fill(&fill(Side::Buy, 10, 100, 0, 0))
            .expect("first buy");
        account
            .apply_fill(&fill(Side::Buy, 20, 100, 0, 0))
            .expect("second buy");
        let position = account.position("AAA").expect("AAA position");
        assert_eq!(position.quantity(), Decimal::from(200));
        assert_eq!(position.avg_cost(), Decimal::from(15));

        account
            .apply_fill(&fill(Side::Sell, 25, 150, 0, 0))
            .expect("partial sell");
        let position = account.position("AAA").expect("AAA position");
        assert_eq!(position.quantity(), Decimal::from(50));
        assert_eq!(position.avg_cost(), Decimal::from(15));
        assert_eq!(position.realized_pnl(), Decimal::from(1_500));

        account
            .apply_fill(&fill(Side::Sell, 20, 50, 0, 0))
            .expect("closing sell");
        let position = account.position("AAA").expect("AAA position");
        assert!(position.is_flat());
        assert_eq!(position.avg_cost(), Decimal::ZERO);
        assert_eq!(position.realized_pnl(), Decimal::from(1_750));
        assert_eq!(account.cash(), Decimal::from(11_750));
        assert_eq!(account.equity().expect("equity"), Decimal::from(11_750));
    }

    #[test]
    fn fill_constructors_reject_invalid_symbols_quantities_and_costs() {
        assert!(
            MultiAssetFill::new(
                "",
                Side::Buy,
                price(1),
                quantity(1),
                Decimal::ZERO,
                Decimal::ZERO
            )
            .is_err()
        );
        assert!(
            MultiAssetFill::new(
                "AAA",
                Side::Buy,
                price(1),
                quantity(0),
                Decimal::ZERO,
                Decimal::ZERO
            )
            .is_err()
        );
        assert!(
            MultiAssetFill::new(
                "AAA",
                Side::Buy,
                price(1),
                quantity(1),
                Decimal::NEGATIVE_ONE,
                Decimal::ZERO
            )
            .is_err()
        );
        assert!(
            MultiAssetFill::new(
                "AAA",
                Side::Buy,
                price(1),
                quantity(1),
                Decimal::ZERO,
                Decimal::NEGATIVE_ONE
            )
            .is_err()
        );
        assert!(AssetPosition::new("", Decimal::ZERO, Decimal::ZERO).is_err());
        assert!(AssetPosition::new("AAA", Decimal::ZERO, Decimal::NEGATIVE_ONE).is_err());
    }
}
