//! Asynchronous broker abstraction and the local paper broker.
//!
//! This module is compiled only with the `live` feature. [`ExecutionBroker`] is
//! the boundary between the synchronous execution kernel and an asynchronous
//! venue: order submission, cancellation, startup account reconciliation, market
//! event fan-out, and an execution-report stream.
//!
//! [`PaperBroker`] wraps [`PaperMatchingEngine`]
//! behind a [`tokio::sync::Mutex`], so a live feed drives the same matching code
//! as the backtest runner with zero network latency.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex as StdMutex;

use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use rust_decimal::Decimal;
use tokio::sync::{Mutex, mpsc};

use crate::error::ExecError;
use crate::order::{ClientOrderId, ExecutionReport, OrderRequest};
use crate::paper::PaperMatchingEngine;
use crate::strategy::ExecMarketEvent;

/// The size of the report channel used by the bundled brokers.
///
/// A live feed can produce bursts of fills; the channel is deliberately large
/// enough to absorb a full rebalance without blocking the matching task.
pub const DEFAULT_REPORT_CHANNEL_CAPACITY: usize = 4096;

/// One position in a broker account snapshot.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{live::BrokerPosition, live::BrokerAccountSnapshot};
/// use rust_decimal::Decimal;
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct BrokerPosition {
    /// Instrument symbol.
    pub symbol: String,
    /// Signed quantity; negative is short.
    pub quantity: Decimal,
    /// Absolute average entry price.
    pub avg_cost: Decimal,
}

impl BrokerPosition {
    /// Creates a broker position.
    #[must_use]
    pub fn new(symbol: impl Into<String>, quantity: Decimal, avg_cost: Decimal) -> Self {
        Self {
            symbol: symbol.into(),
            quantity,
            avg_cost,
        }
    }
}

/// A point-in-time broker account snapshot used for startup reconciliation.
///
/// # Examples
///
/// ```
/// use kurumi_exec::live::BrokerAccountSnapshot;
/// use rust_decimal::Decimal;
/// use std::collections::BTreeMap;
///
/// let snapshot = BrokerAccountSnapshot {
///     cash: Decimal::from(1_000),
///     equity: Decimal::from(1_000),
///     buying_power: Decimal::from(2_000),
///     positions: BTreeMap::new(),
/// };
/// assert_eq!(snapshot.cash, Decimal::from(1_000));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct BrokerAccountSnapshot {
    /// Settled cash.
    pub cash: Decimal,
    /// Marked-to-market equity reported by the broker.
    pub equity: Decimal,
    /// Broker-reported buying power.
    pub buying_power: Decimal,
    /// Positions keyed by symbol.
    pub positions: BTreeMap<String, BrokerPosition>,
}

/// An asynchronous execution venue.
///
/// Implementations must be safe to share across Tokio workers and must deliver
/// every execution report through [`ExecutionBroker::subscribe_reports`]. The
/// returned future of [`ExecutionBroker::on_market_event`] is also available for
/// direct matching, but the live runner reads the report channel as the single
/// source of truth.
///
/// # Examples
///
/// ```
/// use kurumi_exec::live::{BrokerAccountSnapshot, ExecutionBroker};
/// use kurumi_exec::{ClientOrderId, ExecMarketEvent, ExecutionReport, OrderRequest};
/// use futures_util::future::BoxFuture;
/// use tokio::sync::mpsc;
/// use rust_decimal::Decimal;
/// use std::collections::BTreeMap;
///
/// struct NullBroker {
///     tx: mpsc::Sender<ExecutionReport>,
///     rx: std::sync::Mutex<Option<mpsc::Receiver<ExecutionReport>>>,
/// }
///
/// impl ExecutionBroker for NullBroker {
///     fn name(&self) -> &str { "null" }
///     fn submit_order(&self, _request: OrderRequest) -> BoxFuture<'_, Result<(), kurumi_exec::ExecError>> {
///         Box::pin(async { Ok(()) })
///     }
///     fn cancel_order<'a>(&'a self, _id: &'a ClientOrderId) -> BoxFuture<'a, Result<(), kurumi_exec::ExecError>> {
///         Box::pin(async { Ok(()) })
///     }
///     fn cancel_all<'a>(&'a self, _symbol: Option<&'a str>) -> BoxFuture<'a, Result<(), kurumi_exec::ExecError>> {
///         Box::pin(async { Ok(()) })
///     }
///     fn sync_account(&self) -> BoxFuture<'_, Result<BrokerAccountSnapshot, kurumi_exec::ExecError>> {
///         Box::pin(async {
///             Ok(BrokerAccountSnapshot {
///                 cash: Decimal::from(1_000),
///                 equity: Decimal::from(1_000),
///                 buying_power: Decimal::from(1_000),
///                 positions: BTreeMap::new(),
///             })
///         })
///     }
///     fn on_market_event<'a>(&'a self, _event: &'a ExecMarketEvent) -> BoxFuture<'a, Result<Vec<ExecutionReport>, kurumi_exec::ExecError>> {
///         Box::pin(async { Ok(Vec::new()) })
///     }
///     fn subscribe_reports(&self) -> mpsc::Receiver<ExecutionReport> {
///         if let Some(receiver) = self.rx.lock().ok().and_then(|mut guard| guard.take()) {
///             receiver
///         } else {
///             let (sender, receiver) = mpsc::channel(1);
///             drop(sender);
///             receiver
///         }
///     }
/// }
/// # let (tx, rx) = mpsc::channel(1);
/// # let broker = NullBroker { tx, rx: std::sync::Mutex::new(Some(rx)) };
/// assert_eq!(broker.name(), "null");
/// ```
pub trait ExecutionBroker: Send + Sync {
    /// Returns the stable broker name used in journals.
    fn name(&self) -> &str;

    /// Submits one order.
    ///
    /// # Errors
    ///
    /// Returns a typed [`ExecError`] when the venue rejects or cannot reach the
    /// order.
    fn submit_order(&self, request: OrderRequest) -> BoxFuture<'_, Result<(), ExecError>>;

    /// Cancels one order by client identifier.
    ///
    /// # Errors
    ///
    /// Returns a typed [`ExecError`] when the venue rejects or cannot reach the
    /// cancellation.
    fn cancel_order<'a>(
        &'a self,
        client_order_id: &'a ClientOrderId,
    ) -> BoxFuture<'a, Result<(), ExecError>>;

    /// Cancels every open order, optionally restricted to one symbol.
    ///
    /// # Errors
    ///
    /// Returns a typed [`ExecError`] when the venue rejects or cannot reach the
    /// cancellation.
    fn cancel_all<'a>(&'a self, symbol: Option<&'a str>) -> BoxFuture<'a, Result<(), ExecError>>;

    /// Downloads the starting account state for reconciliation.
    ///
    /// # Errors
    ///
    /// Returns a typed [`ExecError`] when the venue cannot be reached or returns
    /// malformed state.
    fn sync_account(&self) -> BoxFuture<'_, Result<BrokerAccountSnapshot, ExecError>>;

    /// Drives venue-side matching with one market event.
    ///
    /// A real broker normally returns an empty vector and delivers fills through
    /// [`ExecutionBroker::subscribe_reports`]; the paper broker returns the
    /// reports and also pushes them to the channel.
    ///
    /// # Errors
    ///
    /// Returns a typed [`ExecError`] when matching fails.
    fn on_market_event<'a>(
        &'a self,
        event: &'a ExecMarketEvent,
    ) -> BoxFuture<'a, Result<Vec<ExecutionReport>, ExecError>>;

    /// Returns the execution-report stream.
    ///
    /// The method is normally called once; a second call returns a closed
    /// receiver rather than panicking.
    fn subscribe_reports(&self) -> mpsc::Receiver<ExecutionReport>;
}

struct PaperBrokerState {
    engine: PaperMatchingEngine,
    clock: DateTime<Utc>,
}

/// An asynchronous local paper broker.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{PaperMatchingEngine, live::{ExecutionBroker, PaperBroker}};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let broker = PaperBroker::new(PaperMatchingEngine::frictionless(), Decimal::from(10_000))?;
/// assert_eq!(broker.name(), "paper");
/// # Ok(())
/// # }
/// ```
pub struct PaperBroker {
    state: Mutex<PaperBrokerState>,
    initial_cash: Decimal,
    report_tx: mpsc::Sender<ExecutionReport>,
    report_rx: StdMutex<Option<mpsc::Receiver<ExecutionReport>>>,
}

impl fmt::Debug for PaperBroker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PaperBroker")
            .field("initial_cash", &self.initial_cash)
            .finish_non_exhaustive()
    }
}

impl PaperBroker {
    /// Wraps one matching engine with the default report channel capacity.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::InvalidOrder`] when `initial_cash` is not strictly
    /// positive.
    pub fn new(engine: PaperMatchingEngine, initial_cash: Decimal) -> Result<Self, ExecError> {
        Self::with_channel_capacity(engine, initial_cash, DEFAULT_REPORT_CHANNEL_CAPACITY)
    }

    /// Wraps one matching engine with an explicit report channel capacity.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::InvalidOrder`] for non-positive cash or a zero
    /// channel capacity.
    pub fn with_channel_capacity(
        engine: PaperMatchingEngine,
        initial_cash: Decimal,
        capacity: usize,
    ) -> Result<Self, ExecError> {
        if initial_cash <= Decimal::ZERO {
            return Err(ExecError::invalid_order(
                "paper broker initial cash must be strictly positive",
            ));
        }
        if capacity == 0 {
            return Err(ExecError::invalid_order(
                "report channel capacity must be at least one",
            ));
        }
        let (report_tx, report_rx) = mpsc::channel(capacity);
        Ok(Self {
            state: Mutex::new(PaperBrokerState {
                engine,
                clock: DateTime::<Utc>::UNIX_EPOCH,
            }),
            initial_cash,
            report_tx,
            report_rx: StdMutex::new(Some(report_rx)),
        })
    }

    async fn emit(&self, reports: &[ExecutionReport]) -> Result<(), ExecError> {
        for report in reports {
            self.report_tx
                .send(report.clone())
                .await
                .map_err(|_| ExecError::BrokerError {
                    message: "paper broker report receiver was dropped".to_owned(),
                })?;
        }
        Ok(())
    }
}

impl ExecutionBroker for PaperBroker {
    fn name(&self) -> &str {
        "paper"
    }

    fn submit_order(&self, request: OrderRequest) -> BoxFuture<'_, Result<(), ExecError>> {
        Box::pin(async move {
            let timestamp = {
                let mut state = self.state.lock().await;
                let reference_price = state
                    .engine
                    .last_price(&request.symbol)
                    .or_else(|| request.reference_price())
                    .ok_or_else(|| ExecError::BrokerError {
                        message: format!(
                            "paper broker has no reference price for `{}`",
                            request.symbol
                        ),
                    })?;
                state
                    .engine
                    .register_order(request.clone(), reference_price)?;
                state.clock
            };
            self.report_tx
                .send(ExecutionReport::Accepted {
                    client_order_id: request.client_order_id,
                    ts: timestamp,
                })
                .await
                .map_err(|_| ExecError::BrokerError {
                    message: "paper broker report receiver was dropped".to_owned(),
                })?;
            Ok(())
        })
    }

    fn cancel_order<'a>(
        &'a self,
        client_order_id: &'a ClientOrderId,
    ) -> BoxFuture<'a, Result<(), ExecError>> {
        Box::pin(async move {
            let timestamp = {
                let mut state = self.state.lock().await;
                state.engine.cancel(client_order_id)?;
                state.clock
            };
            self.report_tx
                .send(ExecutionReport::Canceled {
                    client_order_id: client_order_id.clone(),
                    ts: timestamp,
                })
                .await
                .map_err(|_| ExecError::BrokerError {
                    message: "paper broker report receiver was dropped".to_owned(),
                })?;
            Ok(())
        })
    }

    fn cancel_all<'a>(&'a self, symbol: Option<&'a str>) -> BoxFuture<'a, Result<(), ExecError>> {
        Box::pin(async move {
            let reports = {
                let mut state = self.state.lock().await;
                let identifiers = state
                    .engine
                    .working_order_ids()
                    .filter(|identifier| {
                        symbol.is_none_or(|filter| {
                            state
                                .engine
                                .working_request(identifier)
                                .is_some_and(|request| request.symbol == filter)
                        })
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let timestamp = state.clock;
                let mut reports = Vec::with_capacity(identifiers.len());
                for identifier in identifiers {
                    state.engine.cancel(&identifier)?;
                    reports.push(ExecutionReport::Canceled {
                        client_order_id: identifier,
                        ts: timestamp,
                    });
                }
                reports
            };
            self.emit(&reports).await
        })
    }

    fn sync_account(&self) -> BoxFuture<'_, Result<BrokerAccountSnapshot, ExecError>> {
        Box::pin(async move {
            Ok(BrokerAccountSnapshot {
                cash: self.initial_cash,
                equity: self.initial_cash,
                buying_power: self.initial_cash,
                positions: BTreeMap::new(),
            })
        })
    }

    fn on_market_event<'a>(
        &'a self,
        event: &'a ExecMarketEvent,
    ) -> BoxFuture<'a, Result<Vec<ExecutionReport>, ExecError>> {
        Box::pin(async move {
            let reports = {
                let mut state = self.state.lock().await;
                state.clock = event.timestamp();
                state.engine.on_market_event(event)?
            };
            self.emit(&reports).await?;
            Ok(reports)
        })
    }

    fn subscribe_reports(&self) -> mpsc::Receiver<ExecutionReport> {
        if let Ok(mut guard) = self.report_rx.lock() {
            if let Some(receiver) = guard.take() {
                return receiver;
            }
        }
        let (sender, receiver) = mpsc::channel(1);
        drop(sender);
        receiver
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{OrderSide, OrderType, TimeInForce};
    use crate::strategy::ExecMarketEvent;
    use chrono::DateTime;
    use kurumi_ta::OhlcvBar;

    #[tokio::test]
    async fn paper_broker_accepts_and_fills_a_market_order() {
        let broker = PaperBroker::new(PaperMatchingEngine::frictionless(), Decimal::from(10_000))
            .expect("broker");
        let mut reports = broker.subscribe_reports();
        let request = OrderRequest::new(
            ClientOrderId::new("o-1"),
            "AAA",
            OrderSide::Buy,
            OrderType::Market,
            Decimal::from(10),
            TimeInForce::Day,
        );
        let timestamp = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
        broker
            .on_market_event(&ExecMarketEvent::Bar {
                symbol: "AAA".to_owned(),
                bar: OhlcvBar::new(0, 100.0, 101.0, 99.0, 100.0, 1_000.0, 100_000.0).expect("bar"),
                ts: timestamp,
            })
            .await
            .expect("market event");
        broker.submit_order(request).await.expect("submit");
        let accepted = reports.recv().await.expect("accepted");
        assert!(matches!(accepted, ExecutionReport::Accepted { .. }));
        let fills = broker
            .on_market_event(&ExecMarketEvent::Bar {
                symbol: "AAA".to_owned(),
                bar: OhlcvBar::new(0, 100.0, 101.0, 99.0, 100.0, 1_000.0, 100_000.0).expect("bar"),
                ts: timestamp,
            })
            .await
            .expect("match");
        assert_eq!(fills.len(), 1);
        let fill = reports.recv().await.expect("fill report");
        assert!(matches!(fill, ExecutionReport::Fill { .. }));
    }

    #[tokio::test]
    async fn paper_broker_cancels_resting_orders() {
        let broker = PaperBroker::new(PaperMatchingEngine::frictionless(), Decimal::from(10_000))
            .expect("broker");
        let mut reports = broker.subscribe_reports();
        let request = OrderRequest::new(
            ClientOrderId::new("o-2"),
            "AAA",
            OrderSide::Buy,
            OrderType::Limit {
                price: Decimal::from(1),
            },
            Decimal::from(1),
            TimeInForce::Gtc,
        );
        broker
            .on_market_event(&ExecMarketEvent::Trade {
                symbol: "AAA".to_owned(),
                price: Decimal::from(100),
                size: Decimal::ONE,
                ts: DateTime::<Utc>::from_timestamp_millis(0).expect("epoch"),
            })
            .await
            .expect("market event");
        broker.submit_order(request).await.expect("submit");
        let _accepted = reports.recv().await.expect("accepted");
        broker.cancel_all(None).await.expect("cancel all");
        let canceled = reports.recv().await.expect("canceled");
        assert!(matches!(canceled, ExecutionReport::Canceled { .. }));
    }
}
