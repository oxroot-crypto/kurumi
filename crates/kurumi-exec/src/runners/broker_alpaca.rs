//! Alpaca Trading API v2 broker adapter.
//!
//! The adapter maps [`OrderRequest`] values to Alpaca's REST order payload,
//! cancels by client order id, reconciles the account and positions at startup,
//! and turns open-order polling into standard [`ExecutionReport`]s. Every price
//! and quantity is carried as a decimal string in JSON and parsed directly into
//! [`rust_decimal::Decimal`]; no price ever passes through `f64`.
//!
//! The default endpoints are `https://paper-api.alpaca.markets` and
//! `https://api.alpaca.markets`, but a local mock server URL can be supplied for
//! offline end-to-end tests.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use reqwest::Client;
use rust_decimal::Decimal;
use serde_json::{Map, Value};
use tokio::sync::{Mutex, mpsc, watch};
use tokio::task::JoinHandle;

use crate::error::ExecError;
use crate::order::{
    ClientOrderId, ExecId, ExecutionReport, OrderRequest, OrderSide, OrderType, TimeInForce,
};
use crate::runners::broker::{
    BrokerAccountSnapshot, BrokerPosition, DEFAULT_REPORT_CHANNEL_CAPACITY, ExecutionBroker,
};
use crate::strategy::ExecMarketEvent;

/// The Alpaca paper-trading endpoint.
pub const ALPACA_PAPER_URL: &str = "https://paper-api.alpaca.markets";
/// The Alpaca live-trading endpoint.
pub const ALPACA_LIVE_URL: &str = "https://api.alpaca.markets";

#[derive(Debug, Clone, PartialEq)]
struct TrackedOrder {
    symbol: String,
    status: String,
    filled_qty: Decimal,
    filled_avg_price: Decimal,
}

struct AlpacaState {
    clock: DateTime<Utc>,
    orders: BTreeMap<String, TrackedOrder>,
}

/// A REST broker for Alpaca Trading API v2.
///
/// # Examples
///
/// ```
/// use kurumi_exec::live::{AlpacaTradingBroker, ExecutionBroker};
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let broker = AlpacaTradingBroker::paper("key", "secret")?;
/// assert_eq!(broker.name(), "alpaca");
/// # Ok(())
/// # }
/// ```
pub struct AlpacaTradingBroker {
    client: Client,
    base_url: String,
    api_key: String,
    api_secret: String,
    state: Mutex<AlpacaState>,
    report_tx: mpsc::Sender<ExecutionReport>,
    report_rx: StdMutex<Option<mpsc::Receiver<ExecutionReport>>>,
}

impl fmt::Debug for AlpacaTradingBroker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AlpacaTradingBroker")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl AlpacaTradingBroker {
    /// Creates a broker for an explicit base URL.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::BrokerError`] for a non-HTTP URL, empty credentials,
    /// or an HTTP client that cannot be built.
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        api_secret: impl Into<String>,
    ) -> Result<Self, ExecError> {
        let base_url = base_url.into();
        let api_key = api_key.into();
        let api_secret = api_secret.into();
        if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
            return Err(ExecError::BrokerError {
                message: "Alpaca base URL must start with http:// or https://".to_owned(),
            });
        }
        if api_key.is_empty() || api_secret.is_empty() {
            return Err(ExecError::BrokerError {
                message: "Alpaca credentials must not be empty".to_owned(),
            });
        }
        let client = Client::builder()
            .build()
            .map_err(|error| ExecError::BrokerError {
                message: error.to_string(),
            })?;
        let (report_tx, report_rx) = mpsc::channel(DEFAULT_REPORT_CHANNEL_CAPACITY);
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
            api_secret,
            state: Mutex::new(AlpacaState {
                clock: DateTime::<Utc>::UNIX_EPOCH,
                orders: BTreeMap::new(),
            }),
            report_tx,
            report_rx: StdMutex::new(Some(report_rx)),
        })
    }

    /// Creates a broker for Alpaca paper trading.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`AlpacaTradingBroker::new`].
    pub fn paper(
        api_key: impl Into<String>,
        api_secret: impl Into<String>,
    ) -> Result<Self, ExecError> {
        Self::new(ALPACA_PAPER_URL, api_key, api_secret)
    }

    /// Creates a broker for Alpaca live trading.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`AlpacaTradingBroker::new`].
    pub fn live(
        api_key: impl Into<String>,
        api_secret: impl Into<String>,
    ) -> Result<Self, ExecError> {
        Self::new(ALPACA_LIVE_URL, api_key, api_secret)
    }

    /// Returns the configured base URL.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Polls orders and returns the execution reports implied by changes
    /// since the previous poll.
    ///
    /// The broker deduplicates by order status and cumulative filled quantity,
    /// so polling the same snapshot repeatedly never emits the same fill twice.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::BrokerError`] for transport, HTTP, or malformed JSON
    /// failures.
    pub async fn poll_orders(&self) -> Result<Vec<ExecutionReport>, ExecError> {
        let response = self
            .request(
                self.client
                    .get(self.url("/v2/orders"))
                    .query(&[("status", "open"), ("limit", "500")]),
            )
            .send()
            .await
            .map_err(broker_error)?;
        let value = self.parse_response(response).await?;
        let orders = value.as_array().ok_or_else(|| ExecError::BrokerError {
            message: "Alpaca open-order response is not an array".to_owned(),
        })?;
        let mut reports = Vec::new();
        let mut state = self.state.lock().await;
        let timestamp = state.clock;
        for item in orders {
            let client_order_id = required_string(item.get("client_order_id"), "client_order_id")?;
            let order_id = required_string(item.get("id"), "id")?;
            let status = item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("new")
                .to_owned();
            let filled_qty = json_decimal(item.get("filled_qty")).unwrap_or(Decimal::ZERO);
            let avg_fill_price =
                json_decimal(item.get("filled_avg_price")).unwrap_or(Decimal::ZERO);
            let previous = state.orders.get(&client_order_id).cloned();
            let previous_filled = previous
                .as_ref()
                .map_or(Decimal::ZERO, |order| order.filled_qty);
            if filled_qty > previous_filled {
                let delta = filled_qty.checked_sub(previous_filled).ok_or_else(|| {
                    ExecError::BrokerError {
                        message: "Alpaca cumulative fill quantity underflowed".to_owned(),
                    }
                })?;
                let fill_price = if avg_fill_price > Decimal::ZERO {
                    avg_fill_price
                } else {
                    previous
                        .as_ref()
                        .map_or(Decimal::ZERO, |order| order.filled_avg_price)
                };
                if delta > Decimal::ZERO && fill_price > Decimal::ZERO {
                    reports.push(ExecutionReport::Fill {
                        client_order_id: ClientOrderId::new(client_order_id.clone()),
                        exec_id: ExecId::new(format!("alpaca-{order_id}-{filled_qty}")),
                        symbol: required_string(item.get("symbol"), "symbol")?,
                        side: match required_string(item.get("side"), "side")?.as_str() {
                            "sell" => OrderSide::Sell,
                            _ => OrderSide::Buy,
                        },
                        fill_price,
                        fill_qty: delta,
                        fee: Decimal::ZERO,
                        slippage: Decimal::ZERO,
                        ts: timestamp,
                    });
                }
            }
            let previous_status = previous.as_ref().map(|order| order.status.as_str());
            match status.as_str() {
                "canceled" if previous_status != Some("canceled") => {
                    reports.push(ExecutionReport::Canceled {
                        client_order_id: ClientOrderId::new(client_order_id.clone()),
                        ts: timestamp,
                    });
                }
                "expired" if previous_status != Some("expired") => {
                    reports.push(ExecutionReport::Expired {
                        client_order_id: ClientOrderId::new(client_order_id.clone()),
                        ts: timestamp,
                    });
                }
                "rejected" if previous_status != Some("rejected") => {
                    reports.push(ExecutionReport::Rejected {
                        client_order_id: ClientOrderId::new(client_order_id.clone()),
                        reason: "Alpaca rejected the order".to_owned(),
                        ts: timestamp,
                    });
                }
                _ => {}
            }
            state.orders.insert(
                client_order_id,
                TrackedOrder {
                    symbol: required_string(item.get("symbol"), "symbol")?,
                    status,
                    filled_qty,
                    filled_avg_price: avg_fill_price,
                },
            );
        }
        Ok(reports)
    }

    /// Spawns a task that polls open orders every `interval` and pushes reports
    /// into the broker's report channel.
    ///
    /// Polling errors are logged with `tracing`; the task stops when `shutdown`
    /// becomes `true` or the sender is dropped.
    pub fn spawn_polling(
        self: &Arc<Self>,
        interval: Duration,
        mut shutdown: watch::Receiver<bool>,
    ) -> JoinHandle<()> {
        let broker = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval.max(Duration::from_millis(1)));
            loop {
                tokio::select! {
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            break;
                        }
                    }
                    _ = ticker.tick() => {
                        match broker.poll_orders().await {
                            Ok(reports) => {
                                for report in reports {
                                    if broker.report_tx.send(report).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            Err(error) => {
                                tracing::warn!(%error, "Alpaca order polling failed");
                            }
                        }
                    }
                }
            }
        })
    }

    /// Sends one execution report to the report channel.
    async fn emit(&self, report: ExecutionReport) -> Result<(), ExecError> {
        self.report_tx
            .send(report)
            .await
            .map_err(|_| ExecError::BrokerError {
                message: "Alpaca report receiver was dropped".to_owned(),
            })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    fn request(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.api_secret)
            .header("Content-Type", "application/json")
    }

    async fn parse_response(&self, response: reqwest::Response) -> Result<Value, ExecError> {
        let status = response.status();
        let text = response.text().await.map_err(broker_error)?;
        if !status.is_success() {
            return Err(ExecError::BrokerError {
                message: format!("Alpaca HTTP {}: {text}", status.as_u16()),
            });
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(broker_error)
    }

    fn order_payload(request: &OrderRequest) -> Value {
        let mut body = Map::new();
        body.insert(
            "client_order_id".to_owned(),
            Value::String(request.client_order_id.to_string()),
        );
        body.insert("symbol".to_owned(), Value::String(request.symbol.clone()));
        body.insert(
            "qty".to_owned(),
            Value::String(request.quantity.to_string()),
        );
        body.insert(
            "side".to_owned(),
            Value::String(
                match request.side {
                    OrderSide::Buy => "buy",
                    OrderSide::Sell => "sell",
                }
                .to_owned(),
            ),
        );
        body.insert(
            "type".to_owned(),
            Value::String(
                match request.order_type {
                    OrderType::Market => "market",
                    OrderType::Limit { .. } => "limit",
                    OrderType::Stop { .. } => "stop",
                    OrderType::StopLimit { .. } => "stop_limit",
                }
                .to_owned(),
            ),
        );
        body.insert(
            "time_in_force".to_owned(),
            Value::String(
                match request.tif {
                    TimeInForce::Day => "day",
                    TimeInForce::Gtc => "gtc",
                    TimeInForce::Ioc => "ioc",
                    TimeInForce::Fok => "fok",
                }
                .to_owned(),
            ),
        );
        if let Some(limit_price) = request.order_type.limit_price() {
            body.insert(
                "limit_price".to_owned(),
                Value::String(limit_price.to_string()),
            );
        }
        if let Some(stop_price) = request.order_type.stop_price() {
            body.insert(
                "stop_price".to_owned(),
                Value::String(stop_price.to_string()),
            );
        }
        Value::Object(body)
    }

    fn initial_reports(
        request: &OrderRequest,
        value: &Value,
        timestamp: DateTime<Utc>,
    ) -> Result<Vec<ExecutionReport>, ExecError> {
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("accepted");
        if status == "rejected" {
            let reason = value
                .get("reject_reason")
                .and_then(Value::as_str)
                .unwrap_or("Alpaca rejected the order");
            return Err(ExecError::BrokerError {
                message: reason.to_owned(),
            });
        }
        let mut reports = vec![ExecutionReport::Accepted {
            client_order_id: request.client_order_id.clone(),
            ts: timestamp,
        }];
        let filled_qty = json_decimal(value.get("filled_qty")).unwrap_or(Decimal::ZERO);
        let avg_price = json_decimal(value.get("filled_avg_price")).unwrap_or(Decimal::ZERO);
        if filled_qty > Decimal::ZERO && avg_price > Decimal::ZERO {
            let order_id = value
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(request.client_order_id.as_str());
            reports.push(ExecutionReport::Fill {
                client_order_id: request.client_order_id.clone(),
                exec_id: ExecId::new(format!("alpaca-{order_id}-{filled_qty}")),
                symbol: request.symbol.clone(),
                side: request.side,
                fill_price: avg_price,
                fill_qty: filled_qty,
                fee: Decimal::ZERO,
                slippage: Decimal::ZERO,
                ts: timestamp,
            });
        }
        Ok(reports)
    }
}

impl ExecutionBroker for AlpacaTradingBroker {
    fn name(&self) -> &str {
        "alpaca"
    }

    fn submit_order(&self, request: OrderRequest) -> BoxFuture<'_, Result<(), ExecError>> {
        Box::pin(async move {
            request.validate()?;
            let response = self
                .request(self.client.post(self.url("/v2/orders")))
                .json(&Self::order_payload(&request))
                .send()
                .await
                .map_err(broker_error)?;
            let value = self.parse_response(response).await?;
            let status = value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("accepted")
                .to_owned();
            let filled_qty = json_decimal(value.get("filled_qty")).unwrap_or(Decimal::ZERO);
            let filled_avg_price =
                json_decimal(value.get("filled_avg_price")).unwrap_or(Decimal::ZERO);
            let timestamp = {
                let mut state = self.state.lock().await;
                let timestamp = state.clock;
                state.orders.insert(
                    request.client_order_id.to_string(),
                    TrackedOrder {
                        symbol: request.symbol.clone(),
                        status,
                        filled_qty,
                        filled_avg_price,
                    },
                );
                timestamp
            };
            let reports = Self::initial_reports(&request, &value, timestamp)?;
            for report in reports {
                self.emit(report).await?;
            }
            Ok(())
        })
    }

    fn cancel_order<'a>(
        &'a self,
        client_order_id: &'a ClientOrderId,
    ) -> BoxFuture<'a, Result<(), ExecError>> {
        Box::pin(async move {
            let path = format!("/v2/orders/{client_order_id}");
            let response = self
                .request(self.client.delete(self.url(&path)))
                .send()
                .await
                .map_err(broker_error)?;
            let _ = self.parse_response(response).await?;
            let timestamp = {
                let mut state = self.state.lock().await;
                if let Some(order) = state.orders.get_mut(client_order_id.as_str()) {
                    order.status = "canceled".to_owned();
                }
                state.clock
            };
            self.emit(ExecutionReport::Canceled {
                client_order_id: client_order_id.clone(),
                ts: timestamp,
            })
            .await
        })
    }

    fn cancel_all<'a>(&'a self, symbol: Option<&'a str>) -> BoxFuture<'a, Result<(), ExecError>> {
        Box::pin(async move {
            let mut query = vec![("status", "open")];
            if let Some(symbol) = symbol {
                query.push(("symbol", symbol));
            }
            let response = self
                .request(self.client.delete(self.url("/v2/orders")).query(&query))
                .send()
                .await
                .map_err(broker_error)?;
            let _ = self.parse_response(response).await?;
            let (timestamp, canceled) = {
                let mut state = self.state.lock().await;
                let identifiers = state
                    .orders
                    .iter()
                    .filter(|(_, order)| {
                        !matches!(
                            order.status.as_str(),
                            "filled" | "canceled" | "expired" | "rejected"
                        )
                    })
                    .map(|(identifier, _)| identifier.clone())
                    .collect::<Vec<_>>();
                let timestamp = state.clock;
                let mut canceled = Vec::with_capacity(identifiers.len());
                for identifier in identifiers {
                    if let Some(order) = state.orders.get_mut(&identifier) {
                        order.status = "canceled".to_owned();
                        canceled.push(identifier);
                    }
                }
                (timestamp, canceled)
            };
            for identifier in canceled {
                self.emit(ExecutionReport::Canceled {
                    client_order_id: ClientOrderId::new(identifier),
                    ts: timestamp,
                })
                .await?;
            }
            Ok(())
        })
    }

    fn sync_account(&self) -> BoxFuture<'_, Result<BrokerAccountSnapshot, ExecError>> {
        Box::pin(async move {
            let account_response = self
                .request(self.client.get(self.url("/v2/account")))
                .send()
                .await
                .map_err(broker_error)?;
            let account = self.parse_response(account_response).await?;
            let positions_response = self
                .request(self.client.get(self.url("/v2/positions")))
                .send()
                .await
                .map_err(broker_error)?;
            let positions_value = self.parse_response(positions_response).await?;
            let cash = required_decimal(account.get("cash"), "cash")?;
            let equity = required_decimal(account.get("equity"), "equity")?;
            let buying_power = json_decimal(account.get("buying_power")).unwrap_or(equity);
            let mut positions = BTreeMap::new();
            for item in positions_value
                .as_array()
                .ok_or_else(|| ExecError::BrokerError {
                    message: "Alpaca positions response is not an array".to_owned(),
                })?
            {
                let symbol = required_string(item.get("symbol"), "symbol")?;
                let quantity = required_decimal(item.get("qty"), "qty")?;
                let avg_cost = json_decimal(item.get("avg_entry_price")).unwrap_or(Decimal::ZERO);
                positions.insert(
                    symbol.clone(),
                    BrokerPosition::new(symbol, quantity, avg_cost),
                );
            }
            Ok(BrokerAccountSnapshot {
                cash,
                equity,
                buying_power,
                positions,
            })
        })
    }

    fn on_market_event<'a>(
        &'a self,
        event: &'a ExecMarketEvent,
    ) -> BoxFuture<'a, Result<Vec<ExecutionReport>, ExecError>> {
        Box::pin(async move {
            self.state.lock().await.clock = event.timestamp();
            Ok(Vec::new())
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

fn broker_error(error: impl fmt::Display) -> ExecError {
    ExecError::BrokerError {
        message: error.to_string(),
    }
}

fn json_decimal(value: Option<&Value>) -> Option<Decimal> {
    match value? {
        Value::String(text) => text.parse().ok(),
        Value::Number(number) => number.to_string().parse().ok(),
        _ => None,
    }
}

fn required_decimal(value: Option<&Value>, field: &str) -> Result<Decimal, ExecError> {
    json_decimal(value).ok_or_else(|| ExecError::BrokerError {
        message: format!("Alpaca response is missing a decimal `{field}`"),
    })
}

fn required_string(value: Option<&Value>, field: &str) -> Result<String, ExecError> {
    value
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| ExecError::BrokerError {
            message: format!("Alpaca response is missing a string `{field}`"),
        })
}
