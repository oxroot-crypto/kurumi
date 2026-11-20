//! The cash, position, and open-order ledger.
//!
//! [`ExecutionLedger`] is the single source of truth for reserved and available
//! capital. Buy orders reserve cash and sell orders reserve quantity; a new
//! order that needs more than the unreserved balance is rejected. Execution
//! reports release the reservation, update cash and positions, and maintain
//! volume-weighted average cost and realized profit and loss.
//!
//! Every container is a [`std::collections::BTreeMap`], so iteration and
//! accounting are deterministic. There is no clock and no randomness: timestamps
//! arrive with the execution reports.
//!
//! # Examples
//!
//! ```
//! use kurumi_exec::{
//!     ClientOrderId, ExecutionLedger, OrderRequest, OrderSide, OrderType, TimeInForce,
//! };
//! use rust_decimal::Decimal;
//!
//! # fn main() -> Result<(), kurumi_exec::ExecError> {
//! let mut ledger = ExecutionLedger::new(Decimal::from(10_000))?;
//! let request = OrderRequest::new(
//!     ClientOrderId::new("buy-1"),
//!     "AAA",
//!     OrderSide::Buy,
//!     OrderType::Limit { price: Decimal::from(100) },
//!     Decimal::from(10),
//!     TimeInForce::Day,
//! );
//! ledger.reserve_for_order(&request, Decimal::from(100), Decimal::ZERO)?;
//! assert!(ledger.available_cash() < Decimal::from(10_000));
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use kurumi_core::KurumiError;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::error::ExecError;
use crate::order::{ClientOrderId, ExecutionReport, OrderRecord, OrderRequest, OrderSide};

/// A position and its reserved sell quantity.
///
/// `quantity` is signed: positive is long and negative is short. `avg_cost` is
/// the absolute entry price of the open position and is zero when the position
/// is flat.
///
/// # Examples
///
/// ```
/// use kurumi_exec::LedgerPosition;
/// use rust_decimal::Decimal;
///
/// let position = LedgerPosition::default();
/// assert!(position.quantity().is_zero());
/// assert!(position.is_flat());
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LedgerPosition {
    quantity: Decimal,
    reserved_sell_qty: Decimal,
    avg_cost: Decimal,
    realized_pnl: Decimal,
}

impl LedgerPosition {
    /// Creates a flat position.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the signed open quantity.
    #[must_use]
    #[inline]
    pub fn quantity(&self) -> Decimal {
        self.quantity
    }

    /// Returns the quantity reserved by open sell orders.
    #[must_use]
    #[inline]
    pub fn reserved_sell_qty(&self) -> Decimal {
        self.reserved_sell_qty
    }

    /// Returns the absolute average entry price, or zero when flat.
    #[must_use]
    #[inline]
    pub fn avg_cost(&self) -> Decimal {
        self.avg_cost
    }

    /// Returns the cumulative realized profit and loss.
    #[must_use]
    #[inline]
    pub fn realized_pnl(&self) -> Decimal {
        self.realized_pnl
    }

    /// Returns `true` when no quantity is open.
    #[must_use]
    #[inline]
    pub fn is_flat(&self) -> bool {
        self.quantity.is_zero()
    }
}

/// One starting position used by [`ExecutionLedger::reconcile_starting_state`].
///
/// # Examples
///
/// ```
/// use kurumi_exec::LedgerPositionSeed;
/// use rust_decimal::Decimal;
///
/// let seed = LedgerPositionSeed::new("AAA", Decimal::from(10), Decimal::from(100));
/// assert_eq!(seed.symbol, "AAA");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct LedgerPositionSeed {
    /// Instrument symbol.
    pub symbol: String,
    /// Signed starting quantity; negative is short.
    pub quantity: Decimal,
    /// Absolute average entry price, or zero for a flat seed.
    pub avg_cost: Decimal,
}

impl LedgerPositionSeed {
    /// Creates a starting-position seed.
    #[must_use]
    pub fn new(symbol: impl Into<String>, quantity: Decimal, avg_cost: Decimal) -> Self {
        Self {
            symbol: symbol.into(),
            quantity,
            avg_cost,
        }
    }
}

/// A reservation made by an open order.
#[derive(Debug, Clone, PartialEq)]
struct Reservation {
    symbol: String,
    side: OrderSide,
    cash: Decimal,
    qty: Decimal,
    order_qty: Decimal,
}

/// The deterministic execution ledger.
///
/// # Examples
///
/// ```
/// use kurumi_exec::ExecutionLedger;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let ledger = ExecutionLedger::new(Decimal::from(1_000))?;
/// assert_eq!(ledger.cash(), Decimal::from(1_000));
/// assert_eq!(ledger.available_cash(), Decimal::from(1_000));
/// assert_eq!(ledger.equity()?, Decimal::from(1_000));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct ExecutionLedger {
    initial_cash: Decimal,
    cash: Decimal,
    reserved_cash: Decimal,
    positions: BTreeMap<String, LedgerPosition>,
    last_prices: BTreeMap<String, Decimal>,
    orders: BTreeMap<ClientOrderId, OrderRecord>,
    reservations: BTreeMap<ClientOrderId, Reservation>,
    total_fees: Decimal,
    total_slippage: Decimal,
    day_start_equity: Decimal,
    allow_shorting: bool,
}

impl ExecutionLedger {
    /// Creates a long-only ledger with `initial_cash`.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::InvalidOrder`] when `initial_cash` is not strictly
    /// positive.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::ExecutionLedger;
    /// use rust_decimal::Decimal;
    ///
    /// assert!(ExecutionLedger::new(Decimal::ZERO).is_err());
    /// ```
    pub fn new(initial_cash: Decimal) -> Result<Self, ExecError> {
        if initial_cash <= Decimal::ZERO {
            return Err(ExecError::invalid_order(
                "initial cash must be strictly positive",
            ));
        }
        Ok(Self {
            initial_cash,
            cash: initial_cash,
            reserved_cash: Decimal::ZERO,
            positions: BTreeMap::new(),
            last_prices: BTreeMap::new(),
            orders: BTreeMap::new(),
            reservations: BTreeMap::new(),
            total_fees: Decimal::ZERO,
            total_slippage: Decimal::ZERO,
            day_start_equity: initial_cash,
            allow_shorting: false,
        })
    }

    /// Enables or disables short selling.
    ///
    /// Shorting is disabled by default. When enabled, a sell order may exceed
    /// the long quantity and open a negative position.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::ExecutionLedger;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_exec::ExecError> {
    /// let ledger = ExecutionLedger::new(Decimal::from(1_000))?.with_shorting(true);
    /// assert!(ledger.shorting_enabled());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_shorting(mut self, allow_shorting: bool) -> Self {
        self.allow_shorting = allow_shorting;
        self
    }

    /// Returns the starting cash.
    #[must_use]
    #[inline]
    pub fn initial_cash(&self) -> Decimal {
        self.initial_cash
    }

    /// Returns the current cash balance.
    #[must_use]
    #[inline]
    pub fn cash(&self) -> Decimal {
        self.cash
    }

    /// Returns cash reserved by open buy orders.
    #[must_use]
    #[inline]
    pub fn reserved_cash(&self) -> Decimal {
        self.reserved_cash
    }

    /// Returns cash not reserved by open buy orders.
    #[must_use]
    #[inline]
    pub fn available_cash(&self) -> Decimal {
        self.cash
            .checked_sub(self.reserved_cash)
            .unwrap_or(Decimal::ZERO)
    }

    /// Returns the positions keyed by symbol.
    #[must_use]
    #[inline]
    pub fn positions(&self) -> &BTreeMap<String, LedgerPosition> {
        &self.positions
    }

    /// Returns one position, or `None` when the symbol has never traded.
    #[must_use]
    #[inline]
    pub fn position(&self, symbol: &str) -> Option<&LedgerPosition> {
        self.positions.get(symbol)
    }

    /// Returns the signed position quantity.
    #[must_use]
    #[inline]
    pub fn position_qty(&self, symbol: &str) -> Decimal {
        self.positions
            .get(symbol)
            .map_or(Decimal::ZERO, |position| position.quantity)
    }

    /// Returns the unreserved long quantity of `symbol`.
    ///
    /// The result is capped at zero for a short or flat position.
    #[must_use]
    #[inline]
    pub fn available_qty(&self, symbol: &str) -> Decimal {
        let Some(position) = self.positions.get(symbol) else {
            return Decimal::ZERO;
        };
        let available = position
            .quantity
            .checked_sub(position.reserved_sell_qty)
            .unwrap_or(Decimal::ZERO);
        if available.is_sign_negative() {
            Decimal::ZERO
        } else {
            available
        }
    }

    /// Returns the last mark price known for `symbol`.
    #[must_use]
    #[inline]
    pub fn last_price(&self, symbol: &str) -> Option<Decimal> {
        self.last_prices.get(symbol).copied()
    }

    /// Returns every mark price known to the ledger.
    #[must_use]
    #[inline]
    pub fn last_prices(&self) -> &BTreeMap<String, Decimal> {
        &self.last_prices
    }

    /// Returns `true` when short selling is enabled.
    #[must_use]
    #[inline]
    pub fn shorting_enabled(&self) -> bool {
        self.allow_shorting
    }

    /// Returns every order ever recorded, keyed by client order identifier.
    #[must_use]
    #[inline]
    pub fn orders(&self) -> &BTreeMap<ClientOrderId, OrderRecord> {
        &self.orders
    }

    /// Returns one order record.
    #[must_use]
    #[inline]
    pub fn order(&self, client_order_id: &ClientOrderId) -> Option<&OrderRecord> {
        self.orders.get(client_order_id)
    }

    /// Iterates over the orders that are not terminal.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::ExecutionLedger;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_exec::ExecError> {
    /// let ledger = ExecutionLedger::new(Decimal::from(1_000))?;
    /// assert_eq!(ledger.open_orders().count(), 0);
    /// # Ok(())
    /// # }
    /// ```
    pub fn open_orders(&self) -> impl Iterator<Item = &OrderRecord> {
        self.orders
            .values()
            .filter(|record| !record.status().is_terminal())
    }

    /// Returns the total fees charged by fills.
    #[must_use]
    #[inline]
    pub fn total_fees(&self) -> Decimal {
        self.total_fees
    }

    /// Returns the total slippage embedded in fills.
    #[must_use]
    #[inline]
    pub fn total_slippage(&self) -> Decimal {
        self.total_slippage
    }

    /// Returns the equity used as the daily-loss baseline.
    #[must_use]
    #[inline]
    pub fn day_start_equity(&self) -> Decimal {
        self.day_start_equity
    }

    /// Sets the daily-loss baseline.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::InvalidOrder`] when `equity` is not strictly
    /// positive.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::ExecutionLedger;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_exec::ExecError> {
    /// let mut ledger = ExecutionLedger::new(Decimal::from(1_000))?;
    /// ledger.set_day_start_equity(Decimal::from(900))?;
    /// assert_eq!(ledger.day_start_equity(), Decimal::from(900));
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_day_start_equity(&mut self, equity: Decimal) -> Result<(), ExecError> {
        if equity <= Decimal::ZERO {
            return Err(ExecError::invalid_order(
                "day-start equity must be strictly positive",
            ));
        }
        self.day_start_equity = equity;
        Ok(())
    }

    /// Marks `symbol` at `price` for equity and weight calculations.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::InvalidOrder`] when `price` is not strictly positive.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::ExecutionLedger;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_exec::ExecError> {
    /// let mut ledger = ExecutionLedger::new(Decimal::from(1_000))?;
    /// ledger.mark_to_market("AAA", Decimal::from(100))?;
    /// assert_eq!(ledger.last_price("AAA"), Some(Decimal::from(100)));
    /// # Ok(())
    /// # }
    /// ```
    pub fn mark_to_market(&mut self, symbol: &str, price: Decimal) -> Result<(), ExecError> {
        if price <= Decimal::ZERO {
            return Err(ExecError::invalid_order(format!(
                "mark price for `{symbol}` must be strictly positive"
            )));
        }
        self.last_prices.insert(symbol.to_owned(), price);
        Ok(())
    }

    /// Returns cash plus marked-to-market position value.
    ///
    /// Positions without a mark price are valued at their average cost; a
    /// non-flat position with neither a mark nor a cost is an error.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::StrategyError`] when a non-flat position has no mark
    /// price, and [`ExecError::Core`] on arithmetic overflow.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::ExecutionLedger;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_exec::ExecError> {
    /// let ledger = ExecutionLedger::new(Decimal::from(1_000))?;
    /// assert_eq!(ledger.equity()?, Decimal::from(1_000));
    /// # Ok(())
    /// # }
    /// ```
    pub fn equity(&self) -> Result<Decimal, ExecError> {
        let mut equity = self.cash;
        for (symbol, position) in &self.positions {
            if position.quantity.is_zero() {
                continue;
            }
            let price = self
                .last_prices
                .get(symbol)
                .copied()
                .or_else(|| {
                    if position.avg_cost > Decimal::ZERO {
                        Some(position.avg_cost)
                    } else {
                        None
                    }
                })
                .ok_or_else(|| {
                    ExecError::strategy(format!("no mark price is known for `{symbol}`"))
                })?;
            let value = checked_mul(position.quantity, price, "ledger equity")?;
            equity = checked_add(equity, value, "ledger equity")?;
        }
        Ok(equity)
    }

    /// Returns marked-to-market position weights as fractions of equity.
    ///
    /// The result is empty when equity is not strictly positive. Weights are
    /// keyed by symbol in deterministic order.
    ///
    /// # Errors
    ///
    /// Propagates the errors of [`ExecutionLedger::equity`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::ExecutionLedger;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_exec::ExecError> {
    /// let ledger = ExecutionLedger::new(Decimal::from(1_000))?;
    /// assert!(ledger.current_weights()?.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn current_weights(&self) -> Result<BTreeMap<String, f64>, ExecError> {
        let equity = self.equity()?;
        let mut weights = BTreeMap::new();
        if equity <= Decimal::ZERO {
            return Ok(weights);
        }
        let equity_f64 = equity.to_f64().unwrap_or(0.0);
        for (symbol, position) in &self.positions {
            if position.quantity.is_zero() {
                continue;
            }
            let price = self
                .last_prices
                .get(symbol)
                .copied()
                .unwrap_or(position.avg_cost);
            let value = checked_mul(position.quantity, price, "ledger weights")?;
            let weight = value.to_f64().unwrap_or(0.0) / equity_f64;
            weights.insert(symbol.clone(), weight);
        }
        Ok(weights)
    }

    /// Replaces the ledger with a broker account snapshot before a live run.
    ///
    /// This is the startup reconciliation step: open orders, reservations, and
    /// fee counters are cleared, then the broker's cash and positions become the
    /// new starting state. Every non-flat seed must carry a strictly positive
    /// average cost so equity can be valued before the first market event.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::InvalidOrder`] for negative cash, a non-positive
    /// day-start equity, an empty/duplicate symbol, or a non-flat seed without a
    /// positive average cost.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::{ExecutionLedger, LedgerPositionSeed};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_exec::ExecError> {
    /// let mut ledger = ExecutionLedger::new(Decimal::from(1_000))?;
    /// ledger.reconcile_starting_state(
    ///     Decimal::from(900),
    ///     Decimal::from(1_000),
    ///     vec![LedgerPositionSeed::new("AAA", Decimal::from(1), Decimal::from(100))],
    /// )?;
    /// assert_eq!(ledger.position_qty("AAA"), Decimal::from(1));
    /// assert_eq!(ledger.equity()?, Decimal::from(1_000));
    /// # Ok(())
    /// # }
    /// ```
    pub fn reconcile_starting_state(
        &mut self,
        cash: Decimal,
        day_start_equity: Decimal,
        positions: Vec<LedgerPositionSeed>,
    ) -> Result<(), ExecError> {
        if cash < Decimal::ZERO {
            return Err(ExecError::invalid_order(
                "reconciled cash must be non-negative",
            ));
        }
        if day_start_equity <= Decimal::ZERO {
            return Err(ExecError::invalid_order(
                "reconciled day-start equity must be strictly positive",
            ));
        }
        let mut next_positions = BTreeMap::new();
        for seed in positions {
            if seed.symbol.trim().is_empty() {
                return Err(ExecError::invalid_order(
                    "reconciled position symbol must not be empty",
                ));
            }
            if next_positions.contains_key(&seed.symbol) {
                return Err(ExecError::invalid_order(format!(
                    "duplicate reconciled position `{}`",
                    seed.symbol
                )));
            }
            if !seed.quantity.is_zero() && seed.avg_cost <= Decimal::ZERO {
                return Err(ExecError::invalid_order(format!(
                    "reconciled position `{}` must have a positive average cost",
                    seed.symbol
                )));
            }
            next_positions.insert(
                seed.symbol,
                LedgerPosition {
                    quantity: seed.quantity,
                    reserved_sell_qty: Decimal::ZERO,
                    avg_cost: if seed.quantity.is_zero() {
                        Decimal::ZERO
                    } else {
                        seed.avg_cost
                    },
                    realized_pnl: Decimal::ZERO,
                },
            );
        }
        self.initial_cash = cash;
        self.cash = cash;
        self.reserved_cash = Decimal::ZERO;
        self.positions = next_positions;
        self.last_prices.clear();
        self.orders.clear();
        self.reservations.clear();
        self.total_fees = Decimal::ZERO;
        self.total_slippage = Decimal::ZERO;
        self.day_start_equity = day_start_equity;
        Ok(())
    }

    /// Reserves cash for a buy order or quantity for a sell order.
    ///
    /// The buy reservation is
    ///
    /// ```text
    /// ref_price * quantity * (1 + fee_buffer_bps / 10_000)
    /// ```
    ///
    /// and the sell reservation is the order quantity. A reservation creates the
    /// order record in [`crate::OrderStatus::PendingSubmit`].
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::DuplicateClientOrderId`] when the identifier is
    /// already known, [`ExecError::InsufficientFunds`] when the cash reservation
    /// exceeds available cash, [`ExecError::InsufficientPosition`] when a
    /// long-only sell exceeds the unreserved quantity, and
    /// [`ExecError::InvalidOrder`] for a malformed request or non-positive
    /// reference price.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::{
    ///     ClientOrderId, ExecutionLedger, OrderRequest, OrderSide, OrderType, TimeInForce,
    /// };
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_exec::ExecError> {
    /// let mut ledger = ExecutionLedger::new(Decimal::from(1_000))?;
    /// let request = OrderRequest::new(
    ///     ClientOrderId::new("sell-1"), "AAA", OrderSide::Sell, OrderType::Market,
    ///     Decimal::ONE, TimeInForce::Day,
    /// );
    /// assert!(ledger.reserve_for_order(&request, Decimal::from(100), Decimal::ZERO).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn reserve_for_order(
        &mut self,
        request: &OrderRequest,
        ref_price: Decimal,
        fee_buffer_bps: Decimal,
    ) -> Result<(), ExecError> {
        request.validate()?;
        if ref_price <= Decimal::ZERO {
            return Err(ExecError::invalid_order(
                "reference price must be strictly positive",
            ));
        }
        if fee_buffer_bps < Decimal::ZERO {
            return Err(ExecError::invalid_order("fee buffer must be non-negative"));
        }
        if self.orders.contains_key(&request.client_order_id) {
            return Err(ExecError::DuplicateClientOrderId {
                client_order_id: request.client_order_id.to_string(),
            });
        }
        let current_position = self.position_qty(&request.symbol);
        if request.reduce_only {
            let reducing = match request.side {
                OrderSide::Buy => {
                    current_position < Decimal::ZERO && request.quantity <= current_position.abs()
                }
                OrderSide::Sell => {
                    current_position > Decimal::ZERO && request.quantity <= current_position
                }
            };
            if !reducing {
                return Err(ExecError::InsufficientPosition {
                    symbol: request.symbol.clone(),
                    required: request.quantity,
                    available: match request.side {
                        OrderSide::Buy => (-current_position).max(Decimal::ZERO),
                        OrderSide::Sell => current_position.max(Decimal::ZERO),
                    },
                });
            }
        }

        let (reserved_cash, reserved_qty) = match request.side {
            OrderSide::Buy => {
                let buffer = Decimal::ONE
                    + checked_div(fee_buffer_bps, Decimal::from(10_000), "ledger reservation")?;
                let notional = checked_mul(ref_price, request.quantity, "ledger reservation")?;
                let required = checked_mul(notional, buffer, "ledger reservation")?;
                let available = self.available_cash();
                if required > available {
                    return Err(ExecError::InsufficientFunds {
                        required,
                        available,
                    });
                }
                (required, Decimal::ZERO)
            }
            OrderSide::Sell => {
                let available = self.available_qty(&request.symbol);
                if !self.allow_shorting && request.quantity > available {
                    return Err(ExecError::InsufficientPosition {
                        symbol: request.symbol.clone(),
                        required: request.quantity,
                        available,
                    });
                }
                (Decimal::ZERO, request.quantity)
            }
        };

        if reserved_cash > Decimal::ZERO {
            self.reserved_cash = checked_add(self.reserved_cash, reserved_cash, "ledger reserve")?;
        }
        if reserved_qty > Decimal::ZERO {
            let position = self.positions.entry(request.symbol.clone()).or_default();
            position.reserved_sell_qty =
                checked_add(position.reserved_sell_qty, reserved_qty, "ledger reserve")?;
        }
        self.reservations.insert(
            request.client_order_id.clone(),
            Reservation {
                symbol: request.symbol.clone(),
                side: request.side,
                cash: reserved_cash,
                qty: reserved_qty,
                order_qty: request.quantity,
            },
        );
        self.orders.insert(
            request.client_order_id.clone(),
            OrderRecord::new(request.clone()),
        );
        Ok(())
    }

    /// Applies one execution report to the order and the accounting.
    ///
    /// A replayed fill with an already-applied execution identifier is ignored,
    /// leaving every balance unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::OrderNotFound`] for an unknown order,
    /// [`ExecError::InvalidStateTransition`] for an illegal transition, and
    /// [`ExecError::InsufficientFunds`] or
    /// [`ExecError::InsufficientPosition`] when a fill would overdraw the
    /// account. The ledger is left unchanged on error.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::{ClientOrderId, ExecutionLedger, ExecutionReport};
    /// use chrono::{DateTime, Utc};
    ///
    /// # fn main() -> Result<(), kurumi_exec::ExecError> {
    /// let mut ledger = ExecutionLedger::new(rust_decimal::Decimal::from(1_000))?;
    /// let ts = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
    /// assert!(
    ///     ledger
    ///         .apply_execution_report(&ExecutionReport::Accepted {
    ///             client_order_id: ClientOrderId::new("missing"),
    ///             ts,
    ///         })
    ///         .is_err()
    /// );
    /// # Ok(())
    /// # }
    /// ```
    pub fn apply_execution_report(&mut self, report: &ExecutionReport) -> Result<(), ExecError> {
        let client_order_id = report.client_order_id().clone();
        let current =
            self.orders
                .get(&client_order_id)
                .cloned()
                .ok_or_else(|| ExecError::OrderNotFound {
                    client_order_id: client_order_id.to_string(),
                })?;

        match report {
            ExecutionReport::Accepted { .. } => {
                let mut updated = current;
                updated.apply_report(report)?;
                self.orders.insert(client_order_id, updated);
                Ok(())
            }
            ExecutionReport::Fill {
                symbol,
                fill_price,
                fill_qty,
                fee,
                slippage,
                ..
            } => {
                // Apply the report to a copy so a failed accounting check
                // leaves both the order record and the balances untouched.
                let mut updated = current;
                let applied = updated.apply_report(report)?;
                if !applied {
                    return Ok(());
                }
                let order_side = updated.side();
                let order_qty = updated.quantity();
                let terminal = updated.remaining_qty().is_zero();

                // A sell fill may only exceed the current long position when
                // shorting is enabled.
                let available_position = self.position_qty(symbol);
                if order_side == OrderSide::Sell
                    && !self.allow_shorting
                    && *fill_qty > available_position
                {
                    return Err(ExecError::InsufficientPosition {
                        symbol: symbol.clone(),
                        required: *fill_qty,
                        available: available_position.max(Decimal::ZERO),
                    });
                }

                let reservation = self.reservations.get(&client_order_id).cloned();
                let (release_cash, release_qty) = match &reservation {
                    Some(reservation) => {
                        if terminal {
                            (reservation.cash, reservation.qty)
                        } else {
                            let release_cash = checked_div(
                                checked_mul(reservation.cash, *fill_qty, "ledger fill release")?,
                                order_qty,
                                "ledger fill release",
                            )?;
                            let release_qty = checked_div(
                                checked_mul(reservation.qty, *fill_qty, "ledger fill release")?,
                                order_qty,
                                "ledger fill release",
                            )?;
                            (
                                release_cash.min(reservation.cash),
                                release_qty.min(reservation.qty),
                            )
                        }
                    }
                    None => (Decimal::ZERO, Decimal::ZERO),
                };

                let notional = checked_mul(*fill_price, *fill_qty, "ledger fill notional")?;
                let cash_delta = match order_side {
                    OrderSide::Buy => {
                        let debit = checked_add(notional, *fee, "ledger buy cash")?;
                        // The fill may consume its own released reservation and
                        // any genuinely unreserved cash, but never cash that is
                        // still reserved for other open buy orders.
                        let capacity =
                            checked_add(self.available_cash(), release_cash, "ledger buy cash")?;
                        if debit > capacity {
                            return Err(ExecError::InsufficientFunds {
                                required: debit,
                                available: capacity,
                            });
                        }
                        -debit
                    }
                    OrderSide::Sell => {
                        notional
                            .checked_sub(*fee)
                            .ok_or(KurumiError::NumericalOverflow {
                                operation: "ledger sell cash",
                            })?
                    }
                };
                let new_cash = checked_add(self.cash, cash_delta, "ledger cash")?;
                let new_reserved_cash = subtract_non_negative(
                    self.reserved_cash,
                    release_cash,
                    "ledger reserve release",
                )?;
                let new_fees = checked_add(self.total_fees, *fee, "ledger fees")?;
                let new_slippage = checked_add(self.total_slippage, *slippage, "ledger slippage")?;

                let mut position = self
                    .positions
                    .get(symbol)
                    .cloned()
                    .unwrap_or_else(LedgerPosition::new);
                position.reserved_sell_qty = subtract_non_negative(
                    position.reserved_sell_qty,
                    release_qty,
                    "ledger sell reservation",
                )?;
                apply_fill_to_position(&mut position, order_side, *fill_price, *fill_qty)?;
                let remaining = match &reservation {
                    Some(reservation) => Some((
                        subtract_non_negative(
                            reservation.cash,
                            release_cash,
                            "ledger reserve release",
                        )?,
                        subtract_non_negative(
                            reservation.qty,
                            release_qty,
                            "ledger sell reservation",
                        )?,
                    )),
                    None => None,
                };

                // Every fallible operation is complete; commit atomically.
                self.orders.insert(client_order_id.clone(), updated);
                self.cash = new_cash;
                self.reserved_cash = new_reserved_cash;
                self.total_fees = new_fees;
                self.total_slippage = new_slippage;
                self.positions.insert(symbol.clone(), position);
                if let Some((remaining_cash, remaining_qty)) = remaining {
                    if remaining_cash.is_zero() && remaining_qty.is_zero() {
                        self.reservations.remove(&client_order_id);
                    } else if let Some(entry) = self.reservations.get_mut(&client_order_id) {
                        entry.cash = remaining_cash;
                        entry.qty = remaining_qty;
                    }
                }
                Ok(())
            }
            ExecutionReport::Canceled { .. }
            | ExecutionReport::Rejected { .. }
            | ExecutionReport::Expired { .. } => {
                let mut updated = current;
                updated.apply_report(report)?;
                self.orders.insert(client_order_id.clone(), updated);
                self.release_reservation(&client_order_id);
                Ok(())
            }
        }
    }

    /// Requests cancellation of an order and releases its reservation.
    ///
    /// The returned record is in [`crate::OrderStatus::PendingCancel`]; the caller is
    /// responsible for delivering the final `Canceled` report.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::OrderNotFound`] for an unknown order and
    /// [`ExecError::InvalidStateTransition`] for a terminal order.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::{ClientOrderId, ExecutionLedger};
    /// use chrono::{DateTime, Utc};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_exec::ExecError> {
    /// let ts = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
    /// let mut ledger = ExecutionLedger::new(Decimal::from(1_000))?;
    /// assert!(
    ///     ledger
    ///         .request_cancel(&ClientOrderId::new("missing"), ts)
    ///         .is_err()
    /// );
    /// # Ok(())
    /// # }
    /// ```
    pub fn request_cancel(
        &mut self,
        client_order_id: &ClientOrderId,
        ts: DateTime<Utc>,
    ) -> Result<OrderRecord, ExecError> {
        {
            let record =
                self.orders
                    .get_mut(client_order_id)
                    .ok_or_else(|| ExecError::OrderNotFound {
                        client_order_id: client_order_id.to_string(),
                    })?;
            record.request_cancel(ts)?;
        }
        let record =
            self.orders
                .get(client_order_id)
                .cloned()
                .ok_or_else(|| ExecError::OrderNotFound {
                    client_order_id: client_order_id.to_string(),
                })?;
        self.release_reservation(client_order_id);
        Ok(record)
    }

    fn release_reservation(&mut self, client_order_id: &ClientOrderId) {
        if let Some(reservation) = self.reservations.remove(client_order_id) {
            self.reserved_cash = subtract_non_negative(
                self.reserved_cash,
                reservation.cash,
                "ledger cancel release",
            )
            .unwrap_or(Decimal::ZERO);
            if let Some(position) = self.positions.get_mut(&reservation.symbol) {
                position.reserved_sell_qty = subtract_non_negative(
                    position.reserved_sell_qty,
                    reservation.qty,
                    "ledger cancel release",
                )
                .unwrap_or(Decimal::ZERO);
            }
        }
    }
}

/// Applies one fill to a position, maintaining average cost and realized P&L.
///
/// Positions are signed: a negative quantity is a short. Selling a long keeps
/// its average cost until the position flips; the opening part of a flip takes
/// the fill price as its cost. The same rule applies in reverse when a buy
/// covers a short.
fn apply_fill_to_position(
    position: &mut LedgerPosition,
    side: OrderSide,
    price: Decimal,
    qty: Decimal,
) -> Result<(), ExecError> {
    let current = position.quantity;
    let signed_qty = checked_mul(qty, Decimal::from(side.sign()), "ledger position")?;
    let new_quantity = checked_add(current, signed_qty, "ledger position")?;

    if current >= Decimal::ZERO && signed_qty >= Decimal::ZERO {
        // Adding to a long position.
        if new_quantity.is_zero() {
            position.avg_cost = Decimal::ZERO;
        } else {
            let existing = checked_mul(current, position.avg_cost, "ledger average cost")?;
            let added = checked_mul(qty, price, "ledger average cost")?;
            position.avg_cost = checked_div(
                checked_add(existing, added, "ledger average cost")?,
                new_quantity,
                "ledger average cost",
            )?;
        }
    } else if current >= Decimal::ZERO && signed_qty < Decimal::ZERO {
        // Selling a long position, possibly flipping short.
        let closing = qty.min(current);
        if !closing.is_zero() {
            let gain = checked_mul(
                price
                    .checked_sub(position.avg_cost)
                    .ok_or(KurumiError::NumericalOverflow {
                        operation: "ledger realized pnl",
                    })?,
                closing,
                "ledger realized pnl",
            )?;
            position.realized_pnl =
                checked_add(position.realized_pnl, gain, "ledger realized pnl")?;
        }
        if new_quantity < Decimal::ZERO {
            position.avg_cost = price;
        } else if new_quantity.is_zero() {
            position.avg_cost = Decimal::ZERO;
        }
    } else if current < Decimal::ZERO && signed_qty > Decimal::ZERO {
        // Buying back a short position, possibly flipping long.
        let closing = qty.min(current.abs());
        if !closing.is_zero() {
            let gain = checked_mul(
                position
                    .avg_cost
                    .checked_sub(price)
                    .ok_or(KurumiError::NumericalOverflow {
                        operation: "ledger realized pnl",
                    })?,
                closing,
                "ledger realized pnl",
            )?;
            position.realized_pnl =
                checked_add(position.realized_pnl, gain, "ledger realized pnl")?;
        }
        if new_quantity > Decimal::ZERO {
            position.avg_cost = price;
        } else if new_quantity.is_zero() {
            position.avg_cost = Decimal::ZERO;
        }
    } else {
        // Adding to a short position.
        let existing = checked_mul(current.abs(), position.avg_cost, "ledger average cost")?;
        let added = checked_mul(qty, price, "ledger average cost")?;
        position.avg_cost = checked_div(
            checked_add(existing, added, "ledger average cost")?,
            new_quantity.abs(),
            "ledger average cost",
        )?;
    }

    position.quantity = new_quantity;
    Ok(())
}

fn subtract_non_negative(
    left: Decimal,
    right: Decimal,
    operation: &'static str,
) -> Result<Decimal, ExecError> {
    if right >= left {
        Ok(Decimal::ZERO)
    } else {
        left.checked_sub(right)
            .ok_or_else(|| KurumiError::NumericalOverflow { operation }.into())
    }
}

fn checked_mul(
    left: Decimal,
    right: Decimal,
    operation: &'static str,
) -> Result<Decimal, ExecError> {
    left.checked_mul(right)
        .ok_or_else(|| KurumiError::NumericalOverflow { operation }.into())
}

fn checked_add(
    left: Decimal,
    right: Decimal,
    operation: &'static str,
) -> Result<Decimal, ExecError> {
    left.checked_add(right)
        .ok_or_else(|| KurumiError::NumericalOverflow { operation }.into())
}

fn checked_div(
    left: Decimal,
    right: Decimal,
    operation: &'static str,
) -> Result<Decimal, ExecError> {
    if right.is_zero() {
        return Err(KurumiError::InvalidParameter {
            parameter: "divisor",
            reason: "must be non-zero".to_owned(),
        }
        .into());
    }
    left.checked_div(right)
        .ok_or_else(|| KurumiError::NumericalOverflow { operation }.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{ExecId, ExecutionReport};

    fn ts(millis: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp_millis(millis).expect("valid timestamp")
    }

    fn request(id: &str, side: OrderSide, qty: i64) -> OrderRequest {
        OrderRequest::new(
            ClientOrderId::new(id),
            "AAA",
            side,
            crate::OrderType::Limit {
                price: Decimal::from(100),
            },
            Decimal::from(qty),
            crate::TimeInForce::Gtc,
        )
    }

    fn fill(id: &str, exec: &str, side: OrderSide, qty: i64, price: i64) -> ExecutionReport {
        ExecutionReport::Fill {
            client_order_id: ClientOrderId::new(id),
            exec_id: ExecId::new(exec),
            symbol: "AAA".to_owned(),
            side,
            fill_price: Decimal::from(price),
            fill_qty: Decimal::from(qty),
            fee: Decimal::ONE,
            slippage: Decimal::ZERO,
            ts: ts(0),
        }
    }

    #[test]
    fn second_sell_is_rejected_until_the_first_is_cancelled() {
        let mut ledger = ExecutionLedger::new(Decimal::from(20_000)).expect("ledger");
        let seed = request("seed", OrderSide::Buy, 100);
        ledger
            .reserve_for_order(&seed, Decimal::from(100), Decimal::ZERO)
            .expect("seed reserve");
        ledger
            .apply_execution_report(&ExecutionReport::Accepted {
                client_order_id: ClientOrderId::new("seed"),
                ts: ts(0),
            })
            .expect("seed accept");
        ledger
            .apply_execution_report(&fill("seed", "e-seed", OrderSide::Buy, 100, 100))
            .expect("seed fill");
        let first = request("s-1", OrderSide::Sell, 100);
        ledger
            .reserve_for_order(&first, Decimal::from(100), Decimal::ZERO)
            .expect("first sell reserves");
        let second = request("s-2", OrderSide::Sell, 100);
        assert!(matches!(
            ledger.reserve_for_order(&second, Decimal::from(100), Decimal::ZERO),
            Err(ExecError::InsufficientPosition { .. })
        ));
        ledger
            .request_cancel(&ClientOrderId::new("s-1"), ts(1))
            .expect("cancel first");
        ledger
            .reserve_for_order(&second, Decimal::from(100), Decimal::ZERO)
            .expect("second sell now reserves");
        assert_eq!(ledger.available_qty("AAA"), Decimal::ZERO);
    }

    #[test]
    fn buy_reservation_and_fill_conserve_cash() {
        let mut ledger = ExecutionLedger::new(Decimal::from(10_000)).expect("ledger");
        let order = request("b-1", OrderSide::Buy, 10);
        ledger
            .reserve_for_order(&order, Decimal::from(100), Decimal::from(100))
            .expect("reserve");
        assert_eq!(ledger.reserved_cash(), Decimal::from(1010));
        assert_eq!(ledger.available_cash(), Decimal::from(8_990));
        ledger
            .apply_execution_report(&ExecutionReport::Accepted {
                client_order_id: ClientOrderId::new("b-1"),
                ts: ts(0),
            })
            .expect("accept");
        ledger
            .apply_execution_report(&fill("b-1", "e-1", OrderSide::Buy, 10, 100))
            .expect("fill");
        assert_eq!(ledger.cash(), Decimal::from(8_999));
        assert_eq!(ledger.reserved_cash(), Decimal::ZERO);
        assert_eq!(ledger.position_qty("AAA"), Decimal::from(10));
        assert_eq!(ledger.total_fees(), Decimal::ONE);
        assert_eq!(ledger.equity().expect("equity"), Decimal::from(9_999));
    }

    #[test]
    fn sell_after_buy_realizes_profit_and_rejects_oversell() {
        let mut ledger = ExecutionLedger::new(Decimal::from(10_000)).expect("ledger");
        ledger
            .mark_to_market("AAA", Decimal::from(100))
            .expect("mark");
        let buy = request("b-1", OrderSide::Buy, 10);
        ledger
            .reserve_for_order(&buy, Decimal::from(100), Decimal::ZERO)
            .expect("reserve");
        ledger
            .apply_execution_report(&ExecutionReport::Accepted {
                client_order_id: ClientOrderId::new("b-1"),
                ts: ts(0),
            })
            .expect("accept");
        ledger
            .apply_execution_report(&fill("b-1", "e-1", OrderSide::Buy, 10, 100))
            .expect("fill");
        let sell = request("s-1", OrderSide::Sell, 4);
        ledger
            .reserve_for_order(&sell, Decimal::from(100), Decimal::ZERO)
            .expect("reserve sell");
        ledger
            .apply_execution_report(&ExecutionReport::Accepted {
                client_order_id: ClientOrderId::new("s-1"),
                ts: ts(1),
            })
            .expect("accept");
        ledger
            .apply_execution_report(&fill("s-1", "e-2", OrderSide::Sell, 4, 110))
            .expect("fill");
        assert_eq!(ledger.position_qty("AAA"), Decimal::from(6));
        assert_eq!(
            ledger.position("AAA").expect("position").realized_pnl(),
            Decimal::from(40)
        );
        assert!(ledger.position("AAA").expect("position").avg_cost() == Decimal::from(100));
    }

    #[test]
    fn duplicate_exec_id_does_not_change_cash() {
        let mut ledger = ExecutionLedger::new(Decimal::from(10_000)).expect("ledger");
        let order = request("b-1", OrderSide::Buy, 10);
        ledger
            .reserve_for_order(&order, Decimal::from(100), Decimal::ZERO)
            .expect("reserve");
        ledger
            .apply_execution_report(&ExecutionReport::Accepted {
                client_order_id: ClientOrderId::new("b-1"),
                ts: ts(0),
            })
            .expect("accept");
        let report = fill("b-1", "e-1", OrderSide::Buy, 5, 100);
        ledger.apply_execution_report(&report).expect("fill once");
        let cash = ledger.cash();
        ledger.apply_execution_report(&report).expect("fill twice");
        assert_eq!(ledger.cash(), cash);
        assert_eq!(ledger.position_qty("AAA"), Decimal::from(5));
    }

    #[test]
    fn shorting_is_opt_in() {
        let mut ledger = ExecutionLedger::new(Decimal::from(10_000))
            .expect("ledger")
            .with_shorting(true);
        let order = request("s-1", OrderSide::Sell, 10);
        ledger
            .reserve_for_order(&order, Decimal::from(100), Decimal::ZERO)
            .expect("short reserve");
        ledger
            .apply_execution_report(&ExecutionReport::Accepted {
                client_order_id: ClientOrderId::new("s-1"),
                ts: ts(0),
            })
            .expect("accept");
        ledger
            .apply_execution_report(&fill("s-1", "e-1", OrderSide::Sell, 10, 100))
            .expect("short fill");
        assert_eq!(ledger.position_qty("AAA"), Decimal::from(-10));
        assert_eq!(
            ledger.position("AAA").expect("position").avg_cost(),
            Decimal::from(100)
        );
    }
}
