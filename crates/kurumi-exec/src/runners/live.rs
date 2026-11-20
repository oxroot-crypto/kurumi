//! The asynchronous live execution runner.
//!
//! [`LiveRunner`] connects a `kurumi-feed` broadcast receiver to the synchronous
//! [`Strategy`], [`RiskGate`], [`ExecutionLedger`], and [`ExecutionBroker`] stack.
//! Market events are translated losslessly into [`ExecMarketEvent`]s, the ledger
//! is marked first, the broker is allowed to match resting orders, and only then
//! is the strategy invoked. Execution reports arrive through the broker's channel
//! and are applied exactly once.
//!
//! The runner is compiled only with the `live` feature. It uses Tokio for its
//! event loop, but the strategy, risk, ledger, and matching code it drives remain
//! the same synchronous types used by [`BacktestRunner`](crate::BacktestRunner).

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use chrono::{DateTime, Utc};
use kurumi_feed::{BarAggregator, BarEvent, MarketEvent};
use rust_decimal::Decimal;
use tokio::sync::{broadcast, mpsc, watch};

use crate::error::ExecError;
use crate::journal::{AuditJournal, JournalRecord};
use crate::ledger::{ExecutionLedger, LedgerPositionSeed};
use crate::order::{
    ClientOrderId, ExecutionReport, OrderCommand, OrderRequest, OrderSide, OrderStatus,
};
use crate::risk_gate::{RiskContext, RiskGate};
use crate::runners::backtest::{ExecEquityPoint, ExecFill, RiskRejection};
use crate::runners::broker::ExecutionBroker;
use crate::strategy::{ExecMarketEvent, Strategy, StrategyContext};
use crate::util::decimal_from_f64;

/// Configuration of a [`LiveRunner`].
///
/// # Examples
///
/// ```
/// use kurumi_exec::live::LiveConfig;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let config = LiveConfig {
///     initial_cash: Decimal::from(100_000),
///     ..LiveConfig::default()
/// };
/// config.validate()?;
/// assert!(config.cancel_on_shutdown);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveConfig {
    /// Starting cash; replaced by the broker snapshot during reconciliation.
    pub initial_cash: Decimal,
    /// Extra cash frozen above a buy order's reference notional, in basis points.
    pub fee_buffer_bps: Decimal,
    /// Optional trade-to-bar aggregation interval in seconds.
    pub bar_interval_secs: Option<u64>,
    /// Whether outstanding orders are cancelled during shutdown.
    pub cancel_on_shutdown: bool,
    /// Optional dropped-event threshold that pauses new exposure.
    ///
    /// When a broadcast receiver reports at least this many skipped events, the
    /// runner rejects orders that increase exposure until
    /// [`LiveRunner::resume_trading`] is called. The close/reduce direction stays
    /// available.
    pub lag_pause_threshold: Option<u64>,
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            initial_cash: Decimal::from(100_000),
            fee_buffer_bps: Decimal::from(25),
            bar_interval_secs: None,
            cancel_on_shutdown: true,
            lag_pause_threshold: None,
        }
    }
}

impl LiveConfig {
    /// Validates the configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::StrategyError`] for non-positive cash, a negative fee
    /// buffer, a zero aggregation interval, or a zero lag threshold.
    pub fn validate(&self) -> Result<(), ExecError> {
        if self.initial_cash <= Decimal::ZERO {
            return Err(ExecError::strategy(
                "live initial cash must be strictly positive",
            ));
        }
        if self.fee_buffer_bps < Decimal::ZERO {
            return Err(ExecError::strategy("live fee buffer must be non-negative"));
        }
        if self.bar_interval_secs == Some(0) {
            return Err(ExecError::strategy(
                "bar aggregation interval must be strictly positive",
            ));
        }
        if self.lag_pause_threshold == Some(0) {
            return Err(ExecError::strategy(
                "lag pause threshold must be strictly positive",
            ));
        }
        Ok(())
    }
}

/// The result of one live session.
///
/// # Examples
///
/// ```
/// use kurumi_exec::live::LiveSessionSummary;
/// use rust_decimal::Decimal;
///
/// let summary = LiveSessionSummary {
///     broker_name: "paper".to_owned(),
///     initial_cash: Decimal::ONE,
///     final_equity: Decimal::ONE,
///     equity_curve: Vec::new(),
///     fills: Vec::new(),
///     risk_rejections: Vec::new(),
///     accepted_orders: 0,
///     rejected_orders: 0,
///     canceled_orders: 0,
///     expired_orders: 0,
///     open_orders: 0,
///     events_processed: 0,
///     lagged_events: 0,
///     paused_on_lag: false,
/// };
/// assert_eq!(summary.broker_name, "paper");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct LiveSessionSummary {
    /// Broker name.
    pub broker_name: String,
    /// Reconciled starting cash.
    pub initial_cash: Decimal,
    /// Final marked-to-market equity.
    pub final_equity: Decimal,
    /// Equity curve.
    pub equity_curve: Vec<ExecEquityPoint>,
    /// Applied fills.
    pub fills: Vec<ExecFill>,
    /// Risk and broker rejections.
    pub risk_rejections: Vec<RiskRejection>,
    /// Accepted order count.
    pub accepted_orders: usize,
    /// Rejected order count.
    pub rejected_orders: usize,
    /// Cancelled order count.
    pub canceled_orders: usize,
    /// Expired order count.
    pub expired_orders: usize,
    /// Orders left open when the session ended.
    pub open_orders: usize,
    /// Number of market events processed.
    pub events_processed: usize,
    /// Number of market events skipped by the broadcast receiver.
    pub lagged_events: u64,
    /// Whether new exposure was paused because of data loss.
    pub paused_on_lag: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct LiveCounters {
    accepted: usize,
    rejected: usize,
    canceled: usize,
    expired: usize,
}

/// The asynchronous live execution driver.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use kurumi_exec::{PaperMatchingEngine, RiskGate, live::{LiveConfig, LiveRunner, PaperBroker}};
/// use kurumi_exec::MemoryJournal;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let broker = Arc::new(PaperBroker::new(
///     PaperMatchingEngine::frictionless(),
///     Decimal::from(10_000),
/// )?);
/// let runner = LiveRunner::new(
///     LiveConfig::default(),
///     RiskGate::empty(),
///     broker,
///     Box::new(MemoryJournal::new()),
/// )?;
/// assert!(runner.ledger().cash() > Decimal::ZERO);
/// # Ok(())
/// # }
/// ```
pub struct LiveRunner {
    config: LiveConfig,
    ledger: ExecutionLedger,
    risk_gate: RiskGate,
    broker: Arc<dyn ExecutionBroker>,
    journal: Box<dyn AuditJournal>,
    order_sequence: AtomicU64,
    report_rx: Option<mpsc::Receiver<ExecutionReport>>,
    aggregator: Option<BarAggregator>,
    used: bool,
    started: bool,
    paused_open_new: bool,
    fills: Vec<ExecFill>,
    risk_rejections: Vec<RiskRejection>,
    counters: LiveCounters,
    equity_curve: Vec<ExecEquityPoint>,
    events_processed: usize,
    lagged_events: u64,
    last_event: Option<ExecMarketEvent>,
    last_ts: DateTime<Utc>,
}

impl fmt::Debug for LiveRunner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveRunner")
            .field("config", &self.config)
            .field("broker", &self.broker.name())
            .field("events_processed", &self.events_processed)
            .finish_non_exhaustive()
    }
}

impl LiveRunner {
    /// Creates a live runner and takes the broker's report stream.
    ///
    /// # Errors
    ///
    /// Returns configuration errors, a feed error for an invalid aggregation
    /// interval, or a ledger construction error.
    pub fn new(
        config: LiveConfig,
        risk_gate: RiskGate,
        broker: Arc<dyn ExecutionBroker>,
        journal: Box<dyn AuditJournal>,
    ) -> Result<Self, ExecError> {
        config.validate()?;
        let ledger = ExecutionLedger::new(config.initial_cash)?;
        let aggregator = match config.bar_interval_secs {
            Some(interval) => Some(BarAggregator::new(interval)?),
            None => None,
        };
        let report_rx = broker.subscribe_reports();
        Ok(Self {
            config,
            ledger,
            risk_gate,
            broker,
            journal,
            order_sequence: AtomicU64::new(0),
            report_rx: Some(report_rx),
            aggregator,
            used: false,
            started: false,
            paused_open_new: false,
            fills: Vec::new(),
            risk_rejections: Vec::new(),
            counters: LiveCounters::default(),
            equity_curve: Vec::new(),
            events_processed: 0,
            lagged_events: 0,
            last_event: None,
            last_ts: DateTime::<Utc>::UNIX_EPOCH,
        })
    }

    /// Returns the configuration.
    #[must_use]
    #[inline]
    pub fn config(&self) -> &LiveConfig {
        &self.config
    }

    /// Returns the ledger.
    #[must_use]
    #[inline]
    pub fn ledger(&self) -> &ExecutionLedger {
        &self.ledger
    }

    /// Returns the broker.
    #[must_use]
    #[inline]
    pub fn broker(&self) -> &Arc<dyn ExecutionBroker> {
        &self.broker
    }

    /// Returns `true` when new exposure is paused after data loss.
    #[must_use]
    #[inline]
    pub fn paused_on_lag(&self) -> bool {
        self.paused_open_new
    }

    /// Resumes new exposure after an operator has verified the data feed.
    pub fn resume_trading(&mut self) {
        self.paused_open_new = false;
    }

    /// Runs the live event loop until `shutdown` becomes `true` or the market
    /// stream closes.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::StrategyError`] for a second run on the same runner,
    /// and propagates reconciliation, broker, ledger, journal, and strategy
    /// errors.
    pub async fn run<S: Strategy>(
        &mut self,
        strategy: &mut S,
        mut market_events: broadcast::Receiver<MarketEvent>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<LiveSessionSummary, ExecError> {
        if self.used {
            return Err(ExecError::strategy("a LiveRunner can only be run once"));
        }
        self.used = true;
        let mut report_rx = self
            .report_rx
            .take()
            .ok_or_else(|| ExecError::strategy("the broker report stream was already taken"))?;

        let snapshot = self.broker.sync_account().await?;
        let day_start_equity = if snapshot.equity > Decimal::ZERO {
            snapshot.equity
        } else {
            snapshot.cash
        };
        let seeds = snapshot
            .positions
            .into_values()
            .map(|position| {
                LedgerPositionSeed::new(position.symbol, position.quantity, position.avg_cost)
            })
            .collect::<Vec<_>>();
        self.ledger
            .reconcile_starting_state(snapshot.cash, day_start_equity, seeds)?;

        let mut reports_open = true;
        loop {
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
                report = report_rx.recv(), if reports_open => {
                    match report {
                        Some(report) => self.apply_report(report, strategy)?,
                        None => reports_open = false,
                    }
                }
                received = market_events.recv() => {
                    match received {
                        Ok(event) => {
                            self.events_processed = self.events_processed.saturating_add(1);
                            self.handle_feed_event(event, strategy, &mut report_rx).await?;
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            self.on_lagged(skipped);
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }

        self.drain_reports(&mut report_rx, strategy)?;
        if let Some(aggregator) = self.aggregator.as_mut() {
            for bar in aggregator.flush()? {
                let event = bar_event_to_exec(bar)?;
                self.handle_exec_event(event, strategy, &mut report_rx)
                    .await?;
            }
        }

        let stop_context = StrategyContext::new(&self.ledger, self.last_ts, &self.order_sequence);
        let stop_commands = strategy.on_stop(&stop_context)?;
        if let Some(event) = self.last_event.clone() {
            self.process_commands(stop_commands, &event, strategy, &mut report_rx)
                .await?;
        }

        if self.config.cancel_on_shutdown {
            let identifiers = self
                .ledger
                .open_orders()
                .map(|record| record.client_order_id().clone())
                .collect::<Vec<_>>();
            if let Err(error) = self.broker.cancel_all(None).await {
                tracing::warn!(%error, "live shutdown cancel-all failed");
            }
            for identifier in identifiers {
                if let Err(error) = self.ledger.request_cancel(&identifier, self.last_ts) {
                    tracing::warn!(%error, %identifier, "live shutdown reservation release failed");
                }
            }
            self.drain_reports(&mut report_rx, strategy)?;
        }
        self.journal.flush()?;

        let final_equity = self.ledger.equity()?;
        if let Some(last) = self.equity_curve.last_mut() {
            last.equity = final_equity;
            last.cash = self.ledger.cash();
            last.reserved_cash = self.ledger.reserved_cash();
        }
        let open_orders = self.ledger.open_orders().count();
        Ok(LiveSessionSummary {
            broker_name: self.broker.name().to_owned(),
            initial_cash: self.ledger.initial_cash(),
            final_equity,
            equity_curve: std::mem::take(&mut self.equity_curve),
            fills: std::mem::take(&mut self.fills),
            risk_rejections: std::mem::take(&mut self.risk_rejections),
            accepted_orders: self.counters.accepted,
            rejected_orders: self.counters.rejected,
            canceled_orders: self.counters.canceled,
            expired_orders: self.counters.expired,
            open_orders,
            events_processed: self.events_processed,
            lagged_events: self.lagged_events,
            paused_on_lag: self.paused_open_new,
        })
    }

    async fn handle_feed_event<S: Strategy>(
        &mut self,
        event: MarketEvent,
        strategy: &mut S,
        report_rx: &mut mpsc::Receiver<ExecutionReport>,
    ) -> Result<(), ExecError> {
        match event {
            MarketEvent::Trade(tick) => {
                self.mark_to_market(&ExecMarketEvent::Trade {
                    symbol: tick.instrument.symbol.clone(),
                    price: tick.price,
                    size: tick.size,
                    ts: tick.exchange_ts,
                })?;
                if self.aggregator.is_some() {
                    self.broker_match(
                        &ExecMarketEvent::Trade {
                            symbol: tick.instrument.symbol.clone(),
                            price: tick.price,
                            size: tick.size,
                            ts: tick.exchange_ts,
                        },
                        strategy,
                        report_rx,
                    )
                    .await?;
                    let bar = {
                        let aggregator = self
                            .aggregator
                            .as_mut()
                            .ok_or_else(|| ExecError::strategy("bar aggregator disappeared"))?;
                        aggregator.on_trade(&tick)?
                    };
                    if let Some(bar) = bar {
                        let exec = bar_event_to_exec(bar)?;
                        self.handle_exec_event(exec, strategy, report_rx).await?;
                    }
                } else {
                    let exec = ExecMarketEvent::Trade {
                        symbol: tick.instrument.symbol,
                        price: tick.price,
                        size: tick.size,
                        ts: tick.exchange_ts,
                    };
                    self.handle_exec_event(exec, strategy, report_rx).await?;
                }
            }
            MarketEvent::Quote(tick) => {
                let mid = tick
                    .bid_price
                    .checked_add(tick.ask_price)
                    .ok_or(kurumi_core::KurumiError::NumericalOverflow {
                        operation: "live quote mid",
                    })?
                    .checked_div(Decimal::from(2))
                    .ok_or(kurumi_core::KurumiError::NumericalOverflow {
                        operation: "live quote mid",
                    })?;
                if mid > Decimal::ZERO {
                    self.ledger.mark_to_market(&tick.instrument.symbol, mid)?;
                }
                let exec = ExecMarketEvent::Quote {
                    symbol: tick.instrument.symbol,
                    bid_price: tick.bid_price,
                    bid_size: tick.bid_size,
                    ask_price: tick.ask_price,
                    ask_size: tick.ask_size,
                    ts: tick.exchange_ts,
                };
                self.handle_exec_event(exec, strategy, report_rx).await?;
            }
            MarketEvent::Bar(bar) => {
                if bar.is_final {
                    let exec = bar_event_to_exec(bar)?;
                    self.handle_exec_event(exec, strategy, report_rx).await?;
                }
            }
            MarketEvent::Lagged { skipped, provider } => {
                tracing::warn!(provider, skipped, "feed connector reported dropped events");
                self.on_lagged(skipped);
            }
            MarketEvent::Status(status) => {
                tracing::debug!(provider = status.provider, state = ?status.state, "feed status");
            }
            MarketEvent::OrderBookDelta(_) => {}
        }
        Ok(())
    }

    async fn handle_exec_event<S: Strategy>(
        &mut self,
        event: ExecMarketEvent,
        strategy: &mut S,
        report_rx: &mut mpsc::Receiver<ExecutionReport>,
    ) -> Result<(), ExecError> {
        let timestamp = event.timestamp();
        self.last_event = Some(event.clone());
        self.last_ts = timestamp;
        let start_commands = if self.started {
            Vec::new()
        } else {
            self.started = true;
            let context = StrategyContext::new(&self.ledger, timestamp, &self.order_sequence);
            strategy.on_start(&context)?
        };
        self.mark_to_market(&event)?;
        if !start_commands.is_empty() {
            // Prime the broker with this observation so an on-start market order
            // has a reference price, then submit and match the start commands.
            self.broker_match(&event, strategy, report_rx).await?;
            self.process_commands(start_commands, &event, strategy, report_rx)
                .await?;
        }
        self.broker_match(&event, strategy, report_rx).await?;
        let commands = {
            let context = StrategyContext::new(&self.ledger, timestamp, &self.order_sequence);
            strategy.on_market_event(&event, &context)?
        };
        self.process_commands(commands, &event, strategy, report_rx)
            .await?;

        let equity = self.ledger.equity()?;
        self.equity_curve.push(ExecEquityPoint {
            timestamp,
            cash: self.ledger.cash(),
            reserved_cash: self.ledger.reserved_cash(),
            equity,
        });
        self.journal.record(&JournalRecord::equity(
            timestamp,
            self.ledger.cash(),
            self.ledger.reserved_cash(),
            equity,
        ))?;
        Ok(())
    }

    async fn broker_match<S: Strategy>(
        &mut self,
        event: &ExecMarketEvent,
        strategy: &mut S,
        report_rx: &mut mpsc::Receiver<ExecutionReport>,
    ) -> Result<(), ExecError> {
        let _ = self.broker.on_market_event(event).await?;
        self.drain_reports(report_rx, strategy)
    }

    async fn process_commands<S: Strategy>(
        &mut self,
        commands: Vec<OrderCommand>,
        event: &ExecMarketEvent,
        strategy: &mut S,
        report_rx: &mut mpsc::Receiver<ExecutionReport>,
    ) -> Result<(), ExecError> {
        let timestamp = event.timestamp();
        let mut immediate = Vec::new();
        let mut sells = Vec::new();
        let mut buys = Vec::new();
        for command in commands {
            match command {
                OrderCommand::Submit(request) if request.side == OrderSide::Sell => {
                    sells.push(request);
                }
                OrderCommand::Submit(request) => buys.push(request),
                other => immediate.push(other),
            }
        }
        for command in immediate {
            match command {
                OrderCommand::Cancel { client_order_id } => {
                    self.cancel_order(&client_order_id, timestamp, strategy, report_rx)
                        .await?;
                }
                OrderCommand::CancelAll { symbol } => {
                    let identifiers = self
                        .ledger
                        .open_orders()
                        .filter(|record| {
                            symbol
                                .as_deref()
                                .is_none_or(|filter| record.symbol() == filter)
                        })
                        .map(|record| record.client_order_id().clone())
                        .collect::<Vec<_>>();
                    self.broker.cancel_all(symbol.as_deref()).await?;
                    for identifier in identifiers {
                        let _ = self.ledger.request_cancel(&identifier, timestamp);
                    }
                    self.drain_reports(report_rx, strategy)?;
                }
                OrderCommand::Submit(_) => {}
            }
        }
        for request in sells {
            self.submit_order(request, event, strategy, report_rx)
                .await?;
        }
        self.broker_match(event, strategy, report_rx).await?;
        for request in buys {
            self.submit_order(request, event, strategy, report_rx)
                .await?;
        }
        self.broker_match(event, strategy, report_rx).await?;
        Ok(())
    }

    async fn submit_order<S: Strategy>(
        &mut self,
        request: OrderRequest,
        event: &ExecMarketEvent,
        strategy: &mut S,
        report_rx: &mut mpsc::Receiver<ExecutionReport>,
    ) -> Result<(), ExecError> {
        request.validate()?;
        let timestamp = event.timestamp();
        let reference_price = event
            .reference_price(&request.symbol, request.side)?
            .or_else(|| self.ledger.last_price(&request.symbol));
        let Some(reference_price) = reference_price else {
            return self.reject_unreserved(
                &request,
                "Execution",
                format!("no reference price for `{}`", request.symbol),
                timestamp,
                strategy,
            );
        };
        let risk_context = RiskContext {
            now: timestamp,
            equity: self.ledger.equity()?,
            day_start_equity: self.ledger.day_start_equity(),
            position_qty: self.ledger.position_qty(&request.symbol),
            reference_price: Some(reference_price),
            available_cash: self.ledger.available_cash(),
        };
        if self.paused_open_new && risk_context.increases_exposure(&request) {
            return self.reject_unreserved(
                &request,
                "LaggedData",
                "new exposure is paused after market-data loss".to_owned(),
                timestamp,
                strategy,
            );
        }
        match self.risk_gate.check(&request, &risk_context) {
            Ok(()) => {}
            Err(ExecError::RiskRejected { rule, reason }) => {
                return self.reject_unreserved(&request, &rule, reason, timestamp, strategy);
            }
            Err(ExecError::DuplicateClientOrderId { .. }) => {
                return self.reject_unreserved(
                    &request,
                    "DuplicateOrderId",
                    "client order id has already been used".to_owned(),
                    timestamp,
                    strategy,
                );
            }
            Err(error) => return Err(error),
        }
        match self
            .ledger
            .reserve_for_order(&request, reference_price, self.config.fee_buffer_bps)
        {
            Ok(()) => {}
            Err(ExecError::InsufficientFunds {
                required,
                available,
            }) => {
                return self.reject_unreserved(
                    &request,
                    "InsufficientFunds",
                    format!("required {required}, available {available}"),
                    timestamp,
                    strategy,
                );
            }
            Err(ExecError::InsufficientPosition {
                symbol,
                required,
                available,
            }) => {
                return self.reject_unreserved(
                    &request,
                    "InsufficientPosition",
                    format!("`{symbol}` required {required}, available {available}"),
                    timestamp,
                    strategy,
                );
            }
            Err(ExecError::DuplicateClientOrderId { .. }) => {
                return self.reject_unreserved(
                    &request,
                    "DuplicateOrderId",
                    "client order id has already been used".to_owned(),
                    timestamp,
                    strategy,
                );
            }
            Err(error) => return Err(error),
        }
        self.journal
            .record(&JournalRecord::submit(timestamp, &request))?;
        let client_order_id = request.client_order_id.clone();
        let accepted_request = request.clone();
        match self.broker.submit_order(request).await {
            Ok(()) => {
                self.risk_gate.accept(&accepted_request, &risk_context);
                self.apply_report(
                    ExecutionReport::Accepted {
                        client_order_id,
                        ts: timestamp,
                    },
                    strategy,
                )
            }
            Err(error) => {
                if let Err(release_error) = self.ledger.request_cancel(&client_order_id, timestamp)
                {
                    tracing::warn!(%release_error, %client_order_id, "live reservation release failed");
                }
                self.reject_reserved(
                    &client_order_id,
                    "Broker",
                    format!("broker rejected the order: {error}"),
                    timestamp,
                    strategy,
                )?;
                self.drain_reports(report_rx, strategy)?;
                Ok(())
            }
        }
    }

    async fn cancel_order<S: Strategy>(
        &mut self,
        client_order_id: &ClientOrderId,
        timestamp: DateTime<Utc>,
        strategy: &mut S,
        report_rx: &mut mpsc::Receiver<ExecutionReport>,
    ) -> Result<(), ExecError> {
        self.broker.cancel_order(client_order_id).await?;
        self.ledger.request_cancel(client_order_id, timestamp)?;
        self.drain_reports(report_rx, strategy)
    }

    fn reject_unreserved<S: Strategy>(
        &mut self,
        request: &OrderRequest,
        rule: &str,
        reason: String,
        timestamp: DateTime<Utc>,
        strategy: &mut S,
    ) -> Result<(), ExecError> {
        self.risk_rejections.push(RiskRejection {
            timestamp,
            client_order_id: request.client_order_id.to_string(),
            rule: rule.to_owned(),
            reason: reason.clone(),
        });
        self.counters.rejected = self.counters.rejected.saturating_add(1);
        self.journal.record(&JournalRecord::risk_rejected(
            timestamp,
            &request.client_order_id,
            rule,
            &reason,
        ))?;
        self.deliver(
            &ExecutionReport::Rejected {
                client_order_id: request.client_order_id.clone(),
                reason,
                ts: timestamp,
            },
            strategy,
            timestamp,
        )
    }

    fn reject_reserved<S: Strategy>(
        &mut self,
        client_order_id: &ClientOrderId,
        rule: &str,
        reason: String,
        timestamp: DateTime<Utc>,
        strategy: &mut S,
    ) -> Result<(), ExecError> {
        self.risk_rejections.push(RiskRejection {
            timestamp,
            client_order_id: client_order_id.to_string(),
            rule: rule.to_owned(),
            reason: reason.clone(),
        });
        self.counters.rejected = self.counters.rejected.saturating_add(1);
        self.journal.record(&JournalRecord::risk_rejected(
            timestamp,
            client_order_id,
            rule,
            &reason,
        ))?;
        let report = ExecutionReport::Rejected {
            client_order_id: client_order_id.clone(),
            reason,
            ts: timestamp,
        };
        self.ledger.apply_execution_report(&report)?;
        self.deliver(&report, strategy, timestamp)
    }

    fn drain_reports<S: Strategy>(
        &mut self,
        report_rx: &mut mpsc::Receiver<ExecutionReport>,
        strategy: &mut S,
    ) -> Result<(), ExecError> {
        while let Ok(report) = report_rx.try_recv() {
            self.apply_report(report, strategy)?;
        }
        Ok(())
    }

    fn apply_report<S: Strategy>(
        &mut self,
        report: ExecutionReport,
        strategy: &mut S,
    ) -> Result<(), ExecError> {
        let client_order_id = report.client_order_id().clone();
        let Some(record) = self.ledger.order(&client_order_id).cloned() else {
            tracing::warn!(%client_order_id, "ignoring execution report for an unknown order");
            return Ok(());
        };
        match &report {
            ExecutionReport::Accepted { .. } => {
                if record.status() != OrderStatus::PendingSubmit {
                    return Ok(());
                }
            }
            ExecutionReport::Fill { exec_id, .. } => {
                if record.applied_exec_ids().contains(exec_id) {
                    return Ok(());
                }
                if record.status().is_terminal() {
                    return Ok(());
                }
            }
            ExecutionReport::Canceled { .. } => {
                if record.status() == OrderStatus::Canceled {
                    return Ok(());
                }
            }
            ExecutionReport::Expired { .. } => {
                if record.status() == OrderStatus::Expired {
                    return Ok(());
                }
            }
            ExecutionReport::Rejected { .. } => {
                if record.status() == OrderStatus::Rejected {
                    return Ok(());
                }
            }
        }
        self.ledger.apply_execution_report(&report)?;
        match &report {
            ExecutionReport::Accepted { .. } => {
                self.counters.accepted = self.counters.accepted.saturating_add(1);
                self.journal
                    .record(&JournalRecord::accepted(report.ts(), &client_order_id))?;
            }
            ExecutionReport::Fill {
                exec_id,
                symbol,
                side,
                fill_price,
                fill_qty,
                fee,
                slippage,
                ts,
                ..
            } => {
                self.journal.record(&JournalRecord::fill(&report)?)?;
                self.fills.push(ExecFill {
                    timestamp: *ts,
                    client_order_id: client_order_id.to_string(),
                    exec_id: exec_id.clone(),
                    symbol: symbol.clone(),
                    side: *side,
                    fill_price: *fill_price,
                    fill_qty: *fill_qty,
                    fee: *fee,
                    slippage: *slippage,
                });
            }
            ExecutionReport::Canceled { .. } => {
                self.counters.canceled = self.counters.canceled.saturating_add(1);
                self.journal
                    .record(&JournalRecord::canceled(report.ts(), &client_order_id))?;
            }
            ExecutionReport::Expired { .. } => {
                self.counters.expired = self.counters.expired.saturating_add(1);
            }
            ExecutionReport::Rejected { reason, .. } => {
                self.counters.rejected = self.counters.rejected.saturating_add(1);
                self.risk_rejections.push(RiskRejection {
                    timestamp: report.ts(),
                    client_order_id: client_order_id.to_string(),
                    rule: "Broker".to_owned(),
                    reason: reason.clone(),
                });
                self.journal.record(&JournalRecord::risk_rejected(
                    report.ts(),
                    &client_order_id,
                    "Broker",
                    reason,
                ))?;
            }
        }
        self.deliver(&report, strategy, report.ts())
    }

    fn deliver<S: Strategy>(
        &self,
        report: &ExecutionReport,
        strategy: &mut S,
        timestamp: DateTime<Utc>,
    ) -> Result<(), ExecError> {
        let context = StrategyContext::new(&self.ledger, timestamp, &self.order_sequence);
        strategy.on_execution_report(report, &context)
    }

    fn on_lagged(&mut self, skipped: u64) {
        self.lagged_events = self.lagged_events.saturating_add(skipped);
        tracing::warn!(skipped, "live market-data events were dropped");
        if self
            .config
            .lag_pause_threshold
            .is_some_and(|threshold| skipped >= threshold)
        {
            self.paused_open_new = true;
            tracing::warn!("pausing new exposure after market-data loss");
        }
    }

    fn mark_to_market(&mut self, event: &ExecMarketEvent) -> Result<(), ExecError> {
        match event {
            ExecMarketEvent::Trade { symbol, price, .. } => {
                if *price > Decimal::ZERO {
                    self.ledger.mark_to_market(symbol, *price)?;
                }
            }
            ExecMarketEvent::Quote {
                symbol,
                bid_price,
                ask_price,
                ..
            } => {
                let mid = bid_price
                    .checked_add(*ask_price)
                    .ok_or(kurumi_core::KurumiError::NumericalOverflow {
                        operation: "live quote mid",
                    })?
                    .checked_div(Decimal::from(2))
                    .ok_or(kurumi_core::KurumiError::NumericalOverflow {
                        operation: "live quote mid",
                    })?;
                if mid > Decimal::ZERO {
                    self.ledger.mark_to_market(symbol, mid)?;
                }
            }
            ExecMarketEvent::Bar { symbol, bar, .. } => {
                let close = decimal_from_f64(bar.close(), symbol)?;
                self.ledger.mark_to_market(symbol, close)?;
            }
            ExecMarketEvent::Snapshot(snapshot) => {
                for (symbol, bar) in snapshot.bars() {
                    let close = decimal_from_f64(bar.close(), symbol)?;
                    self.ledger.mark_to_market(symbol, close)?;
                }
            }
        }
        Ok(())
    }
}

/// Converts one `kurumi-feed` market event into the execution event type.
///
/// Non-final bars and control events return `Ok(None)`. Trades, quotes, final
/// bars, status, and order-book deltas are mapped without changing their exact
/// `Decimal` payload.
///
/// # Errors
///
/// Returns [`ExecError::StrategyError`] when a bar timestamp cannot be
/// represented as a UTC timestamp.
pub fn feed_event_to_exec(event: &MarketEvent) -> Result<Option<ExecMarketEvent>, ExecError> {
    match event {
        MarketEvent::Trade(tick) => Ok(Some(ExecMarketEvent::Trade {
            symbol: tick.instrument.symbol.clone(),
            price: tick.price,
            size: tick.size,
            ts: tick.exchange_ts,
        })),
        MarketEvent::Quote(tick) => Ok(Some(ExecMarketEvent::Quote {
            symbol: tick.instrument.symbol.clone(),
            bid_price: tick.bid_price,
            bid_size: tick.bid_size,
            ask_price: tick.ask_price,
            ask_size: tick.ask_size,
            ts: tick.exchange_ts,
        })),
        MarketEvent::Bar(bar) if bar.is_final => bar_event_to_exec(bar.clone()).map(Some),
        MarketEvent::Bar(_) | MarketEvent::Status(_) | MarketEvent::Lagged { .. } => Ok(None),
        MarketEvent::OrderBookDelta(_) => Ok(None),
    }
}

/// Converts one finalized feed bar into an execution bar event.
///
/// # Errors
///
/// Returns [`ExecError::StrategyError`] when the bar timestamp is out of range.
pub fn bar_event_to_exec(bar: BarEvent) -> Result<ExecMarketEvent, ExecError> {
    let timestamp = DateTime::<Utc>::from_timestamp_millis(bar.bar.timestamp())
        .ok_or_else(|| ExecError::strategy("feed bar timestamp is out of range"))?;
    Ok(ExecMarketEvent::Bar {
        symbol: bar.instrument.symbol,
        bar: bar.bar,
        ts: timestamp,
    })
}

impl From<kurumi_feed::FeedError> for ExecError {
    fn from(error: kurumi_feed::FeedError) -> Self {
        ExecError::BrokerError {
            message: error.to_string(),
        }
    }
}
