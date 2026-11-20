//! The Sans-I/O paper matching engine.
//!
//! [`PaperMatchingEngine`] fills working orders against trades, quotes, bars, or
//! multi-asset snapshots using the same [`FeeModel`] and [`SlippageModel`]
//! abstractions as the rest of `kurumi-backtest`. It owns no runtime, no
//! network connection, and no clock: every report carries the event timestamp
//! and a deterministic execution identifier.
//!
//! # Matching rules
//!
//! - A **market** order fills at the ask (buy) or bid (sell) of the latest quote,
//!   or at the trade/bar price when no quote is available, adjusted by the
//!   slippage model.
//! - A **limit** order fills when the quote crosses the limit, or when a bar's
//!   low (buy) or high (sell) reaches it. A gapped bar fills at
//!   `min(limit, open)` for a buy and `max(limit, open)` for a sell, and slippage
//!   can never move a limit fill through its limit.
//! - A **stop** order activates once the trigger price is touched and then
//!   behaves as a market order; a **stop-limit** activates once its stop is
//!   touched and then behaves as a limit order.
//! - An unfilled **IOC** or **FOK** order expires immediately. A **Day** order
//!   expires when the session date advances.
//!
//! The engine fills the full remaining quantity when a match occurs; it does not
//! model queue position or partial depth. Partial fills are still supported by
//! [`OrderRecord`](crate::OrderRecord) and by custom matching implementations.
//!
//! # Examples
//!
//! ```
//! use kurumi_backtest::{NoFee, NoSlippage};
//! use kurumi_exec::PaperMatchingEngine;
//!
//! let engine = PaperMatchingEngine::new(Box::new(NoFee), Box::new(NoSlippage));
//! assert_eq!(engine.working_order_ids().count(), 0);
//! ```

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use kurumi_backtest::{FeeModel, Liquidity, NoFee, NoSlippage, SlippageModel};
use kurumi_core::{Price, Quantity};
use kurumi_ta::OhlcvBar;
use rust_decimal::Decimal;

use crate::error::ExecError;
use crate::order::{
    ClientOrderId, ExecId, ExecutionReport, OrderRequest, OrderSide, OrderType, TimeInForce,
};
use crate::strategy::ExecMarketEvent;
use crate::util::decimal_from_f64;

/// The top of one instrument's book.
#[derive(Debug, Clone, Copy, PartialEq)]
struct QuoteState {
    bid: Decimal,
    ask: Decimal,
}

/// The market context used to match one order.
#[derive(Debug, Clone, Copy)]
enum MarketView {
    Quote(QuoteState),
    Trade(Decimal),
    Bar(OhlcvBar),
}

/// A live order known to the matching engine.
#[derive(Debug, Clone)]
struct WorkingOrder {
    request: OrderRequest,
    activated: bool,
    remaining: Decimal,
}

/// The synchronous simulated broker.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::{FixedRateFee, NoSlippage};
/// use kurumi_exec::PaperMatchingEngine;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let engine = PaperMatchingEngine::new(
///     Box::new(FixedRateFee::new(Decimal::new(1, 3))?),
///     Box::new(NoSlippage),
/// );
/// assert!(engine.last_price("AAA").is_none());
/// # Ok(())
/// # }
/// ```
pub struct PaperMatchingEngine {
    fee_model: Box<dyn FeeModel>,
    slippage_model: Box<dyn SlippageModel>,
    working: BTreeMap<ClientOrderId, WorkingOrder>,
    last_prices: BTreeMap<String, Decimal>,
    quotes: BTreeMap<String, QuoteState>,
    next_exec_sequence: u64,
    session_date: Option<chrono::NaiveDate>,
}

impl fmt::Debug for PaperMatchingEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PaperMatchingEngine")
            .field("working", &self.working)
            .field("last_prices", &self.last_prices)
            .finish_non_exhaustive()
    }
}

impl PaperMatchingEngine {
    /// Creates an engine from the transaction-cost models.
    #[must_use]
    pub fn new(fee_model: Box<dyn FeeModel>, slippage_model: Box<dyn SlippageModel>) -> Self {
        Self {
            fee_model,
            slippage_model,
            working: BTreeMap::new(),
            last_prices: BTreeMap::new(),
            quotes: BTreeMap::new(),
            next_exec_sequence: 0,
            session_date: None,
        }
    }

    /// Creates an engine with no fees and no slippage.
    #[must_use]
    pub fn frictionless() -> Self {
        Self::new(Box::new(NoFee), Box::new(NoSlippage))
    }

    /// Registers a newly accepted order.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::DuplicateClientOrderId`] when the identifier is
    /// already working and [`ExecError::InvalidOrder`] when the request is
    /// malformed.
    pub fn register_order(
        &mut self,
        request: OrderRequest,
        reference_price: Decimal,
    ) -> Result<(), ExecError> {
        request.validate()?;
        if reference_price <= Decimal::ZERO {
            return Err(ExecError::invalid_order(
                "reference price must be strictly positive",
            ));
        }
        if self.working.contains_key(&request.client_order_id) {
            return Err(ExecError::DuplicateClientOrderId {
                client_order_id: request.client_order_id.to_string(),
            });
        }
        let remaining = request.quantity;
        self.working.insert(
            request.client_order_id.clone(),
            WorkingOrder {
                request,
                activated: false,
                remaining,
            },
        );
        Ok(())
    }

    /// Removes a working order and returns its request.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::OrderNotFound`] when the order is not working.
    pub fn cancel(&mut self, client_order_id: &ClientOrderId) -> Result<OrderRequest, ExecError> {
        self.working
            .remove(client_order_id)
            .map(|working| working.request)
            .ok_or_else(|| ExecError::OrderNotFound {
                client_order_id: client_order_id.to_string(),
            })
    }

    /// Iterates over working order identifiers in deterministic order.
    pub fn working_order_ids(&self) -> impl Iterator<Item = &ClientOrderId> {
        self.working.keys()
    }

    /// Returns the request of a working order.
    #[must_use]
    pub fn working_request(&self, client_order_id: &ClientOrderId) -> Option<&OrderRequest> {
        self.working
            .get(client_order_id)
            .map(|working| &working.request)
    }

    /// Returns the remaining quantity of a working order.
    #[must_use]
    pub fn remaining_qty(&self, client_order_id: &ClientOrderId) -> Option<Decimal> {
        self.working
            .get(client_order_id)
            .map(|working| working.remaining)
    }

    /// Returns `true` when the order is still working.
    #[must_use]
    pub fn is_working(&self, client_order_id: &ClientOrderId) -> bool {
        self.working.contains_key(client_order_id)
    }

    /// Returns the latest known price of `symbol`.
    #[must_use]
    pub fn last_price(&self, symbol: &str) -> Option<Decimal> {
        self.last_prices.get(symbol).copied()
    }

    /// Returns the latest known top-of-book quote of `symbol`.
    pub fn quote(&self, symbol: &str) -> Option<(Decimal, Decimal)> {
        self.quotes.get(symbol).map(|quote| (quote.bid, quote.ask))
    }

    /// Returns the number of execution identifiers generated so far.
    #[must_use]
    pub fn execution_count(&self) -> u64 {
        self.next_exec_sequence
    }

    /// Processes one market event and returns every execution report it
    /// produced.
    ///
    /// Reports are ordered by client order identifier, making the output
    /// deterministic even when several orders match on the same event.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::BrokerError`] for a malformed quote or cost-model
    /// failure and [`ExecError::Core`] for arithmetic overflow.
    pub fn on_market_event(
        &mut self,
        event: &ExecMarketEvent,
    ) -> Result<Vec<ExecutionReport>, ExecError> {
        let mut reports = Vec::new();
        let timestamp = event.timestamp();
        let date = timestamp.date_naive();
        if self.session_date.is_some_and(|previous| date > previous) {
            reports.extend(self.expire_day_orders(timestamp)?);
        }
        self.session_date = Some(date);
        self.update_market_state(event)?;

        let identifiers = self.working.keys().cloned().collect::<Vec<_>>();
        for client_order_id in identifiers {
            let Some(working) = self.working.get(&client_order_id) else {
                continue;
            };
            let request = working.request.clone();
            let view = event_view(event, &request.symbol).or_else(|| match event {
                ExecMarketEvent::Snapshot(_) => self
                    .quotes
                    .get(&request.symbol)
                    .copied()
                    .map(MarketView::Quote),
                _ => None,
            });
            let Some(view) = view else {
                let symbol_scoped = matches!(event, ExecMarketEvent::Snapshot(_))
                    || event.symbol() == Some(request.symbol.as_str());
                if symbol_scoped && request.tif.is_immediate() {
                    self.working.remove(&client_order_id);
                    reports.push(ExecutionReport::Expired {
                        client_order_id,
                        ts: timestamp,
                    });
                }
                continue;
            };
            let mut activated = working.activated;
            let outcome = match_order(&request, &view, &mut activated)?;
            match outcome {
                MatchOutcome::Fill {
                    base_price,
                    liquidity,
                } => {
                    let Some(working) = self.working.remove(&client_order_id) else {
                        continue;
                    };
                    let report = self.build_fill(
                        &request,
                        working.remaining,
                        base_price,
                        liquidity,
                        timestamp,
                    )?;
                    reports.push(report);
                }
                MatchOutcome::Expired => {
                    self.working.remove(&client_order_id);
                    reports.push(ExecutionReport::Expired {
                        client_order_id,
                        ts: timestamp,
                    });
                }
                MatchOutcome::Pending => {
                    if let Some(working) = self.working.get_mut(&client_order_id) {
                        working.activated = activated;
                    }
                }
            }
        }
        Ok(reports)
    }

    /// Expires every working Day order.
    ///
    /// The runner calls this at the end of a run; [`PaperMatchingEngine::on_market_event`]
    /// calls it automatically when the session date advances.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::Core`] only on internal arithmetic failure.
    pub fn expire_day_orders(
        &mut self,
        timestamp: DateTime<Utc>,
    ) -> Result<Vec<ExecutionReport>, ExecError> {
        let identifiers = self
            .working
            .iter()
            .filter(|(_, working)| working.request.tif == TimeInForce::Day)
            .map(|(identifier, _)| identifier.clone())
            .collect::<Vec<_>>();
        let mut reports = Vec::with_capacity(identifiers.len());
        for client_order_id in identifiers {
            self.working.remove(&client_order_id);
            reports.push(ExecutionReport::Expired {
                client_order_id,
                ts: timestamp,
            });
        }
        Ok(reports)
    }

    fn update_market_state(&mut self, event: &ExecMarketEvent) -> Result<(), ExecError> {
        match event {
            ExecMarketEvent::Trade { symbol, price, .. } => {
                if *price <= Decimal::ZERO {
                    return Err(ExecError::BrokerError {
                        message: format!("trade price of `{symbol}` must be positive"),
                    });
                }
                self.last_prices.insert(symbol.clone(), *price);
            }
            ExecMarketEvent::Quote {
                symbol,
                bid_price,
                bid_size,
                ask_price,
                ask_size,
                ..
            } => {
                if *bid_price <= Decimal::ZERO
                    || *ask_price <= Decimal::ZERO
                    || *bid_price > *ask_price
                    || *bid_size < Decimal::ZERO
                    || *ask_size < Decimal::ZERO
                {
                    return Err(ExecError::BrokerError {
                        message: format!("invalid quote for `{symbol}`"),
                    });
                }
                self.quotes.insert(
                    symbol.clone(),
                    QuoteState {
                        bid: *bid_price,
                        ask: *ask_price,
                    },
                );
                let mid = bid_price
                    .checked_add(*ask_price)
                    .ok_or(kurumi_core::KurumiError::NumericalOverflow {
                        operation: "quote mid",
                    })?
                    .checked_div(Decimal::from(2))
                    .ok_or(kurumi_core::KurumiError::NumericalOverflow {
                        operation: "quote mid",
                    })?;
                self.last_prices.insert(symbol.clone(), mid);
            }
            ExecMarketEvent::Bar { symbol, bar, .. } => {
                self.last_prices
                    .insert(symbol.clone(), decimal_from_f64(bar.close(), symbol)?);
            }
            ExecMarketEvent::Snapshot(snapshot) => {
                for (symbol, bar) in snapshot.bars() {
                    self.last_prices
                        .insert(symbol.clone(), decimal_from_f64(bar.close(), symbol)?);
                }
            }
        }
        Ok(())
    }

    fn build_fill(
        &mut self,
        request: &OrderRequest,
        quantity: Decimal,
        base_price: Decimal,
        liquidity: Liquidity,
        timestamp: DateTime<Utc>,
    ) -> Result<ExecutionReport, ExecError> {
        let reference = Price::new(base_price)?;
        let adjusted = self
            .slippage_model
            .fill_price(request.side.to_backtest_side(), reference)?
            .value();
        let fill_price = match request.order_type.limit_price() {
            Some(limit) if request.side.is_buy() => adjusted.min(limit),
            Some(limit) => adjusted.max(limit),
            None => adjusted,
        };
        if fill_price <= Decimal::ZERO {
            return Err(ExecError::BrokerError {
                message: "slippage model produced a non-positive fill price".to_owned(),
            });
        }
        let price = Price::new(fill_price)?;
        let quantity = Quantity::new(quantity)?;
        let fee =
            self.fee_model
                .fee(price, quantity, request.side.to_backtest_side(), liquidity)?;
        let slippage = fill_price
            .checked_sub(base_price)
            .ok_or(kurumi_core::KurumiError::NumericalOverflow {
                operation: "paper fill slippage",
            })?
            .abs()
            .checked_mul(quantity.value())
            .ok_or(kurumi_core::KurumiError::NumericalOverflow {
                operation: "paper fill slippage",
            })?;
        let exec_id = self.next_exec_id()?;
        Ok(ExecutionReport::Fill {
            client_order_id: request.client_order_id.clone(),
            exec_id,
            symbol: request.symbol.clone(),
            side: request.side,
            fill_price,
            fill_qty: quantity.value(),
            fee,
            slippage,
            ts: timestamp,
        })
    }

    fn next_exec_id(&mut self) -> Result<ExecId, ExecError> {
        let sequence = self.next_exec_sequence;
        self.next_exec_sequence = self.next_exec_sequence.checked_add(1).ok_or(
            kurumi_core::KurumiError::NumericalOverflow {
                operation: "paper execution id",
            },
        )?;
        Ok(ExecId::new(format!("exec-{sequence:012}")))
    }
}

/// The result of matching one order against one event.
enum MatchOutcome {
    Fill {
        base_price: Decimal,
        liquidity: Liquidity,
    },
    Expired,
    Pending,
}

fn event_view(event: &ExecMarketEvent, symbol: &str) -> Option<MarketView> {
    match event {
        ExecMarketEvent::Trade {
            symbol: event_symbol,
            price,
            ..
        } => {
            if event_symbol == symbol {
                Some(MarketView::Trade(*price))
            } else {
                None
            }
        }
        ExecMarketEvent::Quote {
            symbol: event_symbol,
            bid_price,
            ask_price,
            ..
        } => {
            if event_symbol == symbol {
                Some(MarketView::Quote(QuoteState {
                    bid: *bid_price,
                    ask: *ask_price,
                }))
            } else {
                None
            }
        }
        ExecMarketEvent::Bar {
            symbol: event_symbol,
            bar,
            ..
        } => {
            if event_symbol == symbol {
                Some(MarketView::Bar(*bar))
            } else {
                None
            }
        }
        ExecMarketEvent::Snapshot(snapshot) => {
            snapshot.bar(symbol).map(|bar| MarketView::Bar(*bar))
        }
    }
}

fn match_order(
    request: &OrderRequest,
    view: &MarketView,
    activated: &mut bool,
) -> Result<MatchOutcome, ExecError> {
    if request.order_type.is_stop() && !*activated {
        let Some(stop_price) = request.order_type.stop_price() else {
            return Ok(MatchOutcome::Pending);
        };
        let trigger = trigger_price(*view, request.side);
        let touched = match request.side {
            OrderSide::Buy => trigger >= stop_price,
            OrderSide::Sell => trigger <= stop_price,
        };
        if !touched {
            return Ok(if request.tif.is_immediate() {
                MatchOutcome::Expired
            } else {
                MatchOutcome::Pending
            });
        }
        *activated = true;
    }

    let liquidity = if request.is_market_like() {
        Liquidity::Taker
    } else {
        Liquidity::Maker
    };
    let base_price = match request.order_type {
        OrderType::Market | OrderType::Stop { .. } => market_price(*view, request.side),
        OrderType::Limit { price } => limit_fill_price(*view, request.side, price),
        OrderType::StopLimit { limit_price, .. } => {
            limit_fill_price(*view, request.side, limit_price)
        }
    };
    match base_price {
        Some(base_price) => Ok(MatchOutcome::Fill {
            base_price,
            liquidity,
        }),
        None => Ok(if request.tif.is_immediate() {
            MatchOutcome::Expired
        } else {
            MatchOutcome::Pending
        }),
    }
}

fn market_price(view: MarketView, side: OrderSide) -> Option<Decimal> {
    match view {
        MarketView::Quote(quote) => Some(match side {
            OrderSide::Buy => quote.ask,
            OrderSide::Sell => quote.bid,
        }),
        MarketView::Trade(price) => Some(price),
        MarketView::Bar(bar) => Decimal::from_f64_retain(bar.close()),
    }
}

fn limit_fill_price(view: MarketView, side: OrderSide, limit: Decimal) -> Option<Decimal> {
    match view {
        MarketView::Quote(quote) => match side {
            OrderSide::Buy if quote.ask <= limit => Some(quote.ask),
            OrderSide::Sell if quote.bid >= limit => Some(quote.bid),
            _ => None,
        },
        MarketView::Bar(bar) => {
            let open = Decimal::from_f64_retain(bar.open())?;
            let high = Decimal::from_f64_retain(bar.high())?;
            let low = Decimal::from_f64_retain(bar.low())?;
            match side {
                OrderSide::Buy if low <= limit => Some(limit.min(open)),
                OrderSide::Sell if high >= limit => Some(limit.max(open)),
                _ => None,
            }
        }
        MarketView::Trade(price) => match side {
            OrderSide::Buy if price <= limit => Some(price),
            OrderSide::Sell if price >= limit => Some(price),
            _ => None,
        },
    }
}

fn trigger_price(view: MarketView, side: OrderSide) -> Decimal {
    match view {
        MarketView::Quote(quote) => match side {
            OrderSide::Buy => quote.ask,
            OrderSide::Sell => quote.bid,
        },
        MarketView::Trade(price) => price,
        MarketView::Bar(bar) => {
            let value = match side {
                OrderSide::Buy => bar.high(),
                OrderSide::Sell => bar.low(),
            };
            Decimal::from_f64_retain(value).unwrap_or(Decimal::ZERO)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{OrderType, TimeInForce};
    use chrono::DateTime;

    const DAY_MS: i64 = 86_400_000;

    fn timestamp(millis: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp_millis(millis).expect("timestamp")
    }

    fn request(id: &str, side: OrderSide, order_type: OrderType, tif: TimeInForce) -> OrderRequest {
        OrderRequest::new(
            ClientOrderId::new(id),
            "AAA",
            side,
            order_type,
            Decimal::from(10),
            tif,
        )
    }

    fn bar(open: f64, high: f64, low: f64, close: f64) -> ExecMarketEvent {
        ExecMarketEvent::Bar {
            symbol: "AAA".to_owned(),
            bar: OhlcvBar::new(0, open, high, low, close, 1_000.0, 1_000.0 * close).expect("bar"),
            ts: timestamp(0),
        }
    }

    #[test]
    fn limit_buy_fills_at_the_gapped_open() {
        let mut engine = PaperMatchingEngine::frictionless();
        engine
            .register_order(
                request(
                    "b-1",
                    OrderSide::Buy,
                    OrderType::Limit {
                        price: Decimal::from(100),
                    },
                    TimeInForce::Gtc,
                ),
                Decimal::from(100),
            )
            .expect("register");
        let reports = engine
            .on_market_event(&bar(90.0, 95.0, 85.0, 92.0))
            .expect("match");
        assert_eq!(reports.len(), 1);
        match reports.first() {
            Some(ExecutionReport::Fill {
                fill_price,
                fill_qty,
                ..
            }) => {
                assert_eq!(*fill_price, Decimal::from(90));
                assert_eq!(*fill_qty, Decimal::from(10));
            }
            other => panic!("unexpected report: {other:?}"),
        }
        assert!(!engine.is_working(&ClientOrderId::new("b-1")));
    }

    #[test]
    fn limit_sell_fills_at_the_gapped_open() {
        let mut engine = PaperMatchingEngine::frictionless();
        engine
            .register_order(
                request(
                    "s-1",
                    OrderSide::Sell,
                    OrderType::Limit {
                        price: Decimal::from(100),
                    },
                    TimeInForce::Gtc,
                ),
                Decimal::from(100),
            )
            .expect("register");
        let reports = engine
            .on_market_event(&bar(110.0, 115.0, 105.0, 112.0))
            .expect("match");
        match reports.first() {
            Some(ExecutionReport::Fill { fill_price, .. }) => {
                assert_eq!(*fill_price, Decimal::from(110));
            }
            other => panic!("unexpected report: {other:?}"),
        }
    }

    #[test]
    fn stop_order_activates_when_high_touches_the_stop() {
        let mut engine = PaperMatchingEngine::frictionless();
        engine
            .register_order(
                request(
                    "stop-1",
                    OrderSide::Buy,
                    OrderType::Stop {
                        stop_price: Decimal::from(105),
                    },
                    TimeInForce::Gtc,
                ),
                Decimal::from(100),
            )
            .expect("register");
        let no_trigger = engine
            .on_market_event(&bar(100.0, 104.0, 99.0, 101.0))
            .expect("no trigger");
        assert!(no_trigger.is_empty());
        assert!(engine.is_working(&ClientOrderId::new("stop-1")));
        let reports = engine
            .on_market_event(&bar(106.0, 110.0, 105.0, 108.0))
            .expect("trigger");
        assert_eq!(reports.len(), 1);
        assert!(!engine.is_working(&ClientOrderId::new("stop-1")));
    }

    #[test]
    fn ioc_limit_expires_without_a_cross() {
        let mut engine = PaperMatchingEngine::frictionless();
        engine
            .register_order(
                request(
                    "ioc-1",
                    OrderSide::Buy,
                    OrderType::Limit {
                        price: Decimal::from(90),
                    },
                    TimeInForce::Ioc,
                ),
                Decimal::from(90),
            )
            .expect("register");
        let reports = engine
            .on_market_event(&bar(95.0, 100.0, 94.0, 96.0))
            .expect("match");
        assert!(matches!(
            reports.first(),
            Some(ExecutionReport::Expired { .. })
        ));
        assert!(!engine.is_working(&ClientOrderId::new("ioc-1")));
    }

    #[test]
    fn day_orders_expire_on_the_next_session_date() {
        let mut engine = PaperMatchingEngine::frictionless();
        engine
            .register_order(
                request(
                    "day-1",
                    OrderSide::Buy,
                    OrderType::Limit {
                        price: Decimal::from(90),
                    },
                    TimeInForce::Day,
                ),
                Decimal::from(90),
            )
            .expect("register");
        let first = ExecMarketEvent::Bar {
            symbol: "AAA".to_owned(),
            bar: OhlcvBar::new(0, 95.0, 100.0, 94.0, 96.0, 1_000.0, 96_000.0).expect("bar"),
            ts: timestamp(0),
        };
        assert!(
            engine
                .on_market_event(&first)
                .expect("first day")
                .is_empty()
        );
        let next = ExecMarketEvent::Bar {
            symbol: "AAA".to_owned(),
            bar: OhlcvBar::new(DAY_MS, 95.0, 100.0, 94.0, 96.0, 1_000.0, 96_000.0).expect("bar"),
            ts: timestamp(DAY_MS),
        };
        let reports = engine.on_market_event(&next).expect("next day");
        assert!(matches!(
            reports.first(),
            Some(ExecutionReport::Expired { .. })
        ));
    }

    #[test]
    fn stop_limit_waits_for_the_limit_after_activation() {
        let mut engine = PaperMatchingEngine::frictionless();
        engine
            .register_order(
                request(
                    "sl-1",
                    OrderSide::Buy,
                    OrderType::StopLimit {
                        stop_price: Decimal::from(105),
                        limit_price: Decimal::from(106),
                    },
                    TimeInForce::Gtc,
                ),
                Decimal::from(103),
            )
            .expect("register");
        let activation = engine
            .on_market_event(&bar(106.6, 107.0, 106.5, 106.8))
            .expect("activation");
        assert!(activation.is_empty());
        assert!(engine.is_working(&ClientOrderId::new("sl-1")));
        let reports = engine
            .on_market_event(&bar(105.0, 106.0, 104.0, 105.5))
            .expect("fill");
        match reports.first() {
            Some(ExecutionReport::Fill { fill_price, .. }) => {
                assert_eq!(*fill_price, Decimal::from(105));
            }
            other => panic!("unexpected report: {other:?}"),
        }
    }

    #[test]
    fn fok_expires_when_the_limit_does_not_cross() {
        let mut engine = PaperMatchingEngine::frictionless();
        engine
            .register_order(
                request(
                    "fok-1",
                    OrderSide::Sell,
                    OrderType::Limit {
                        price: Decimal::from(110),
                    },
                    TimeInForce::Fok,
                ),
                Decimal::from(110),
            )
            .expect("register");
        let reports = engine
            .on_market_event(&bar(105.0, 108.0, 100.0, 104.0))
            .expect("match");
        assert!(matches!(
            reports.first(),
            Some(ExecutionReport::Expired { .. })
        ));
    }

    #[test]
    fn quotes_set_the_market_price_and_cross_limit_orders() {
        let mut engine = PaperMatchingEngine::frictionless();
        engine
            .register_order(
                request(
                    "b-1",
                    OrderSide::Buy,
                    OrderType::Limit {
                        price: Decimal::from(101),
                    },
                    TimeInForce::Gtc,
                ),
                Decimal::from(101),
            )
            .expect("register");
        let event = ExecMarketEvent::Quote {
            symbol: "AAA".to_owned(),
            bid_price: Decimal::from(99),
            bid_size: Decimal::from(100),
            ask_price: Decimal::from(100),
            ask_size: Decimal::from(100),
            ts: timestamp(0),
        };
        let reports = engine.on_market_event(&event).expect("match");
        match reports.first() {
            Some(ExecutionReport::Fill { fill_price, .. }) => {
                assert_eq!(*fill_price, Decimal::from(100));
            }
            other => panic!("unexpected report: {other:?}"),
        }
    }
}
