//! The synchronous backtest driver for the execution kernel.
//!
//! [`BacktestRunner`] closes the loop between an [`AlignedMarketData`] timeline,
//! any [`Strategy`], the [`RiskGate`], the [`ExecutionLedger`], and the
//! [`PaperMatchingEngine`]:
//!
//! 1. mark the ledger to the observation's close prices;
//! 2. match orders that were left working by earlier observations;
//! 3. call the strategy and process its commands, submitting sells before
//!    matching them and then submitting buys, so sale proceeds can fund buys;
//! 4. match the new orders and record an equity snapshot.
//!
//! The driver has no runtime and no clock: all timestamps come from the market
//! data. The same strategy and matching engine can therefore be driven by a
//! replay adapter without changing fills, fees, or final `Decimal` equity.
//!
//! # Examples
//!
//! See the `portfolio_rebalance` integration test for a complete four-asset
//! monthly rebalance run.

use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use kurumi_backtest::portfolio::{AlignedMarketData, MarketSlice, MarketSnapshot};
use kurumi_risk::{MetricsConfig, PerformanceMetrics};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::error::ExecError;
use crate::journal::{AuditJournal, JournalRecord};
use crate::ledger::ExecutionLedger;
use crate::order::{ClientOrderId, ExecutionReport, OrderCommand, OrderRequest, OrderSide};
use crate::paper::PaperMatchingEngine;
use crate::risk_gate::{RiskContext, RiskGate};
use crate::strategy::{ExecMarketEvent, Strategy, StrategyContext};

/// Configuration of a [`BacktestRunner`].
///
/// # Examples
///
/// ```
/// use kurumi_exec::BacktestConfig;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let config = BacktestConfig::new(Decimal::from(100_000))?;
/// assert_eq!(config.periods_per_year, 252.0);
/// assert!(config.validate().is_ok());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BacktestConfig {
    /// Starting cash; must be strictly positive.
    pub initial_cash: Decimal,
    /// Extra cash frozen above the reference notional of a buy order, in basis
    /// points, to absorb fees and adverse slippage.
    ///
    /// A fill whose cost exceeds the released reservation plus unreserved cash
    /// is rejected, so this buffer must cover the fee rate and the worst
    /// expected adverse slippage for market orders.
    pub fee_buffer_bps: Decimal,
    /// Number of return periods in a year used by the performance report.
    pub periods_per_year: f64,
    /// Annual risk-free rate used by Sharpe, Sortino, and Jensen statistics.
    pub risk_free_rate: f64,
}

impl Default for BacktestConfig {
    fn default() -> Self {
        Self {
            initial_cash: Decimal::from(100_000),
            fee_buffer_bps: Decimal::from(25),
            periods_per_year: 252.0,
            risk_free_rate: 0.0,
        }
    }
}

impl BacktestConfig {
    /// Creates a daily configuration with defaults for the other fields.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`BacktestConfig::validate`].
    pub fn new(initial_cash: Decimal) -> Result<Self, ExecError> {
        let config = Self {
            initial_cash,
            ..Self::default()
        };
        config.validate()?;
        Ok(config)
    }

    /// Validates the configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::StrategyError`] when cash is not positive, the fee
    /// buffer is negative, or the annualization settings are invalid.
    pub fn validate(&self) -> Result<(), ExecError> {
        if self.initial_cash <= Decimal::ZERO {
            return Err(ExecError::strategy(
                "initial cash must be strictly positive",
            ));
        }
        if self.fee_buffer_bps < Decimal::ZERO {
            return Err(ExecError::strategy("fee buffer must be non-negative"));
        }
        if !self.periods_per_year.is_finite() || self.periods_per_year <= 0.0 {
            return Err(ExecError::strategy(
                "periods_per_year must be finite and strictly positive",
            ));
        }
        if !self.risk_free_rate.is_finite() {
            return Err(ExecError::strategy("risk_free_rate must be finite"));
        }
        Ok(())
    }
}

/// One marked-to-market equity observation.
///
/// # Examples
///
/// ```
/// use kurumi_exec::ExecEquityPoint;
/// use chrono::{DateTime, Utc};
/// use rust_decimal::Decimal;
///
/// let point = ExecEquityPoint {
///     timestamp: DateTime::<Utc>::from_timestamp_millis(0).expect("epoch"),
///     cash: Decimal::from(100),
///     reserved_cash: Decimal::ZERO,
///     equity: Decimal::from(100),
/// };
/// assert_eq!(point.equity, Decimal::from(100));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExecEquityPoint {
    /// Observation timestamp.
    pub timestamp: DateTime<Utc>,
    /// Cash balance.
    pub cash: Decimal,
    /// Cash reserved by open buy orders.
    pub reserved_cash: Decimal,
    /// Marked-to-market equity.
    pub equity: Decimal,
}

/// One executed fill in the report.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{ExecFill, ExecId};
/// use chrono::{DateTime, Utc};
/// use rust_decimal::Decimal;
///
/// let fill = ExecFill {
///     timestamp: DateTime::<Utc>::from_timestamp_millis(0).expect("epoch"),
///     client_order_id: "o-1".to_owned(),
///     exec_id: ExecId::new("e-1"),
///     symbol: "AAA".to_owned(),
///     side: kurumi_exec::OrderSide::Buy,
///     fill_price: Decimal::from(100),
///     fill_qty: Decimal::ONE,
///     fee: Decimal::ZERO,
///     slippage: Decimal::ZERO,
/// };
/// assert_eq!(fill.symbol, "AAA");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ExecFill {
    /// Fill timestamp.
    pub timestamp: DateTime<Utc>,
    /// Client order identifier.
    pub client_order_id: String,
    /// Broker execution identifier.
    pub exec_id: crate::ExecId,
    /// Instrument symbol.
    pub symbol: String,
    /// Fill direction.
    pub side: OrderSide,
    /// Execution price.
    pub fill_price: Decimal,
    /// Filled quantity.
    pub fill_qty: Decimal,
    /// Fee charged.
    pub fee: Decimal,
    /// Slippage embedded in the price.
    pub slippage: Decimal,
}

/// One order rejected by the risk gate or by reservation accounting.
///
/// # Examples
///
/// ```
/// use kurumi_exec::RiskRejection;
/// use chrono::{DateTime, Utc};
///
/// let rejection = RiskRejection {
///     timestamp: DateTime::<Utc>::from_timestamp_millis(0).expect("epoch"),
///     client_order_id: "o-1".to_owned(),
///     rule: "MaxOrderNotional".to_owned(),
///     reason: "too large".to_owned(),
/// };
/// assert_eq!(rejection.rule, "MaxOrderNotional");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RiskRejection {
    /// Rejection timestamp.
    pub timestamp: DateTime<Utc>,
    /// Client order identifier.
    pub client_order_id: String,
    /// Rule or accounting stage that rejected the order.
    pub rule: String,
    /// Human-readable reason.
    pub reason: String,
}

/// The result of one execution backtest.
///
/// # Examples
///
/// ```
/// use kurumi_exec::ExecBacktestReport;
/// use rust_decimal::Decimal;
///
/// let report = ExecBacktestReport {
///     initial_cash: Decimal::ONE,
///     final_equity: Decimal::ONE,
///     equity_curve: Vec::new(),
///     fills: Vec::new(),
///     risk_rejections: Vec::new(),
///     returns: Vec::new(),
///     metrics: None,
///     total_fees: Decimal::ZERO,
///     total_slippage: Decimal::ZERO,
///     accepted_orders: 0,
///     rejected_orders: 0,
///     canceled_orders: 0,
///     expired_orders: 0,
///     open_orders: 0,
/// };
/// assert_eq!(report.final_equity, Decimal::ONE);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ExecBacktestReport {
    /// Starting cash.
    pub initial_cash: Decimal,
    /// Final marked-to-market equity.
    pub final_equity: Decimal,
    /// Equity curve.
    pub equity_curve: Vec<ExecEquityPoint>,
    /// Executed fills in application order.
    pub fills: Vec<ExecFill>,
    /// Risk and reservation rejections in application order.
    pub risk_rejections: Vec<RiskRejection>,
    /// Per-observation simple returns.
    pub returns: Vec<f64>,
    /// Full performance statistics, or `None` when fewer than two returns exist.
    pub metrics: Option<PerformanceMetrics>,
    /// Total fees charged.
    pub total_fees: Decimal,
    /// Total slippage embedded in fills.
    pub total_slippage: Decimal,
    /// Accepted order count.
    pub accepted_orders: usize,
    /// Rejected order count.
    pub rejected_orders: usize,
    /// Cancelled order count.
    pub canceled_orders: usize,
    /// Expired order count.
    pub expired_orders: usize,
    /// Orders still open when the run ended.
    pub open_orders: usize,
}

/// The synchronized execution backtest driver.
///
/// A runner is single-use: it owns one ledger and one matching engine, and
/// [`BacktestRunner::run`] rejects a second run instead of silently carrying
/// state across runs.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{
///     BacktestConfig, BacktestRunner, MemoryJournal, PaperMatchingEngine, RiskGate,
/// };
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let runner = BacktestRunner::new(
///     BacktestConfig::new(Decimal::from(10_000))?,
///     RiskGate::empty(),
///     PaperMatchingEngine::frictionless(),
///     Box::new(MemoryJournal::new()),
/// )?;
/// assert_eq!(runner.ledger().cash(), Decimal::from(10_000));
/// # Ok(())
/// # }
/// ```
pub struct BacktestRunner {
    config: BacktestConfig,
    ledger: ExecutionLedger,
    risk_gate: RiskGate,
    matching: PaperMatchingEngine,
    journal: Box<dyn AuditJournal>,
    order_sequence: std::sync::atomic::AtomicU64,
    used: bool,
}

impl fmt::Debug for BacktestRunner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BacktestRunner")
            .field("config", &self.config)
            .field("ledger", &self.ledger)
            .finish_non_exhaustive()
    }
}

/// Mutable run traces shared by the command and report handlers.
struct RunTrace<'a> {
    fills: &'a mut Vec<ExecFill>,
    rejections: &'a mut Vec<RiskRejection>,
    counters: &'a mut RunCounters,
}

#[derive(Debug, Clone, Copy, Default)]
struct RunCounters {
    accepted: usize,
    rejected: usize,
    canceled: usize,
    expired: usize,
}

impl BacktestRunner {
    /// Creates a runner.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`BacktestConfig::validate`] and ledger creation.
    pub fn new(
        config: BacktestConfig,
        risk_gate: RiskGate,
        matching: PaperMatchingEngine,
        journal: Box<dyn AuditJournal>,
    ) -> Result<Self, ExecError> {
        config.validate()?;
        let ledger = ExecutionLedger::new(config.initial_cash)?;
        Ok(Self {
            config,
            ledger,
            risk_gate,
            matching,
            journal,
            order_sequence: std::sync::atomic::AtomicU64::new(0),
            used: false,
        })
    }

    /// Creates a frictionless runner with no risk rules and an in-memory
    /// journal.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`BacktestConfig::validate`] and ledger creation.
    pub fn frictionless(initial_cash: Decimal) -> Result<Self, ExecError> {
        Self::new(
            BacktestConfig::new(initial_cash)?,
            RiskGate::empty(),
            PaperMatchingEngine::frictionless(),
            Box::new(crate::journal::MemoryJournal::new()),
        )
    }

    /// Returns the configuration.
    #[must_use]
    #[inline]
    pub fn config(&self) -> &BacktestConfig {
        &self.config
    }

    /// Returns the ledger.
    #[must_use]
    #[inline]
    pub fn ledger(&self) -> &ExecutionLedger {
        &self.ledger
    }

    /// Returns the risk gate.
    #[must_use]
    #[inline]
    pub fn risk_gate(&self) -> &RiskGate {
        &self.risk_gate
    }

    /// Returns the risk gate mutably, for example to reset the kill switch
    /// between runs.
    #[must_use]
    #[inline]
    pub fn risk_gate_mut(&mut self) -> &mut RiskGate {
        &mut self.risk_gate
    }

    /// Returns the matching engine.
    #[must_use]
    #[inline]
    pub fn matching(&self) -> &PaperMatchingEngine {
        &self.matching
    }

    /// Returns the journal.
    #[must_use]
    pub fn journal(&self) -> &dyn AuditJournal {
        self.journal.as_ref()
    }

    /// Runs the backtest over an owned aligned timeline, borrowing its slice.
    ///
    /// # Errors
    ///
    /// Propagates the errors of [`BacktestRunner::run`].
    pub fn run_aligned<S: Strategy>(
        &mut self,
        data: &AlignedMarketData,
        strategy: &mut S,
    ) -> Result<ExecBacktestReport, ExecError> {
        self.run(data.slice(), strategy)
    }

    /// Runs the event loop over `data`.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::StrategyError`] for an empty timeline or an already
    /// used runner, and propagates strategy, risk, matching, ledger, journal, and
    /// performance-metric errors.
    pub fn run<S: Strategy>(
        &mut self,
        data: MarketSlice<'_>,
        strategy: &mut S,
    ) -> Result<ExecBacktestReport, ExecError> {
        if data.is_empty() {
            return Err(ExecError::StrategyError {
                message: "cannot run a backtest over an empty market slice".to_owned(),
            });
        }
        if self.used || !self.ledger.orders().is_empty() || !self.ledger.positions().is_empty() {
            return Err(ExecError::strategy("a BacktestRunner can only be run once"));
        }
        self.used = true;

        let mut fills = Vec::new();
        let mut rejections = Vec::new();
        let mut counters = RunCounters::default();
        let mut equity_curve = Vec::new();
        let mut trace = RunTrace {
            fills: &mut fills,
            rejections: &mut rejections,
            counters: &mut counters,
        };
        let mut last_equity = self.config.initial_cash;
        let mut last_date = None;
        let mut last_prices = BTreeMap::new();
        let mut last_snapshot: Option<MarketSnapshot> = None;

        if let Some(first) = data.snapshots().first() {
            let prices = first.close_prices()?;
            let timestamp = first.timestamp();
            let commands = {
                let context = StrategyContext::new(&self.ledger, timestamp, &self.order_sequence);
                strategy.on_start(&context)?
            };
            self.process_commands(commands, timestamp, &prices, strategy, &mut trace)?;
        }

        for snapshot in data.snapshots() {
            let timestamp = snapshot.timestamp();
            let prices = snapshot.close_prices()?;
            last_prices = prices.clone();
            last_snapshot = Some(snapshot.clone());
            let date = timestamp.date_naive();
            if last_date.is_some_and(|previous| date > previous) {
                self.ledger.set_day_start_equity(last_equity)?;
            }
            last_date = Some(date);
            for (symbol, price) in &prices {
                self.ledger.mark_to_market(symbol, *price)?;
            }
            let event = ExecMarketEvent::Snapshot(snapshot.clone());

            // Match orders left working by earlier observations.
            let reports = self.matching.on_market_event(&event)?;
            self.apply_reports(reports, strategy, timestamp, &mut trace)?;

            let commands = {
                let context = StrategyContext::new(&self.ledger, timestamp, &self.order_sequence);
                strategy.on_market_event(&event, &context)?
            };
            self.run_commands(commands, &event, &prices, strategy, &mut trace)?;

            let equity = self.ledger.equity()?;
            last_equity = equity;
            equity_curve.push(ExecEquityPoint {
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
        }

        if let Some(snapshot) = last_snapshot {
            let timestamp = snapshot.timestamp();
            let event = ExecMarketEvent::Snapshot(snapshot);
            let commands = {
                let context = StrategyContext::new(&self.ledger, timestamp, &self.order_sequence);
                strategy.on_stop(&context)?
            };
            self.run_commands(commands, &event, &last_prices, strategy, &mut trace)?;
            let reports = self.matching.expire_day_orders(timestamp)?;
            self.apply_reports(reports, strategy, timestamp, &mut trace)?;
        }
        self.journal.flush()?;

        let final_equity = self.ledger.equity()?;
        // `on_stop` commands execute against the final observation, so fold
        // their effect into the last equity point instead of leaving the curve
        // one state behind the final ledger.
        if let Some(last) = equity_curve.last_mut() {
            last.equity = final_equity;
            last.cash = self.ledger.cash();
            last.reserved_cash = self.ledger.reserved_cash();
        }
        let returns = returns_from_curve(&equity_curve)?;
        let metrics = if returns.len() >= 2 {
            let config =
                MetricsConfig::new(self.config.periods_per_year, self.config.risk_free_rate)?;
            Some(PerformanceMetrics::from_returns(&returns, config)?)
        } else {
            None
        };
        let open_orders = self.ledger.open_orders().count();
        Ok(ExecBacktestReport {
            initial_cash: self.config.initial_cash,
            final_equity,
            equity_curve,
            fills,
            risk_rejections: rejections,
            returns,
            metrics,
            total_fees: self.ledger.total_fees(),
            total_slippage: self.ledger.total_slippage(),
            accepted_orders: counters.accepted,
            rejected_orders: counters.rejected,
            canceled_orders: counters.canceled,
            expired_orders: counters.expired,
            open_orders,
        })
    }

    /// Processes one batch of commands in sells-before-buys order.
    fn run_commands(
        &mut self,
        commands: Vec<OrderCommand>,
        event: &ExecMarketEvent,
        prices: &BTreeMap<String, Decimal>,
        strategy: &mut dyn Strategy,
        trace: &mut RunTrace<'_>,
    ) -> Result<(), ExecError> {
        let timestamp = event.timestamp();
        let mut immediate = Vec::new();
        let mut sells = Vec::new();
        let mut buys = Vec::new();
        for command in commands {
            match command {
                OrderCommand::Submit(request) if request.side == OrderSide::Sell => {
                    sells.push(OrderCommand::Submit(request));
                }
                OrderCommand::Submit(request) => buys.push(OrderCommand::Submit(request)),
                other => immediate.push(other),
            }
        }
        self.process_commands(immediate, timestamp, prices, strategy, trace)?;
        self.process_commands(sells, timestamp, prices, strategy, trace)?;
        let reports = self.matching.on_market_event(event)?;
        self.apply_reports(reports, strategy, timestamp, trace)?;
        self.process_commands(buys, timestamp, prices, strategy, trace)?;
        let reports = self.matching.on_market_event(event)?;
        self.apply_reports(reports, strategy, timestamp, trace)
    }

    /// Processes submissions and cancellations at one timestamp.
    fn process_commands(
        &mut self,
        commands: Vec<OrderCommand>,
        timestamp: DateTime<Utc>,
        prices: &BTreeMap<String, Decimal>,
        strategy: &mut dyn Strategy,
        trace: &mut RunTrace<'_>,
    ) -> Result<(), ExecError> {
        for command in commands {
            match command {
                OrderCommand::Submit(request) => {
                    self.process_submit(request, timestamp, prices, strategy, trace)?;
                }
                OrderCommand::Cancel { client_order_id } => {
                    self.cancel_order(&client_order_id, timestamp, strategy, trace)?;
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
                    for identifier in identifiers {
                        self.cancel_order(&identifier, timestamp, strategy, trace)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn process_submit(
        &mut self,
        request: OrderRequest,
        timestamp: DateTime<Utc>,
        prices: &BTreeMap<String, Decimal>,
        strategy: &mut dyn Strategy,
        trace: &mut RunTrace<'_>,
    ) -> Result<(), ExecError> {
        request.validate()?;
        let reference_price = request
            .reference_price()
            .or_else(|| prices.get(&request.symbol).copied())
            .or_else(|| self.matching.last_price(&request.symbol))
            .or_else(|| self.ledger.last_price(&request.symbol));
        let Some(reference_price) = reference_price else {
            return self.reject_order(
                request.client_order_id.clone(),
                "Execution",
                format!("no reference price for `{}`", request.symbol),
                timestamp,
                strategy,
                trace,
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
        match self.risk_gate.check(&request, &risk_context) {
            Ok(()) => {}
            Err(ExecError::RiskRejected { rule, reason }) => {
                return self.reject_order(
                    request.client_order_id.clone(),
                    &rule,
                    reason,
                    timestamp,
                    strategy,
                    trace,
                );
            }
            Err(ExecError::DuplicateClientOrderId { .. }) => {
                return self.reject_order(
                    request.client_order_id.clone(),
                    "DuplicateOrderId",
                    "client order id has already been used".to_owned(),
                    timestamp,
                    strategy,
                    trace,
                );
            }
            Err(error) => return Err(error),
        }
        let reserve =
            self.ledger
                .reserve_for_order(&request, reference_price, self.config.fee_buffer_bps);
        match reserve {
            Ok(()) => {}
            Err(ExecError::InsufficientFunds {
                required,
                available,
            }) => {
                return self.reject_order(
                    request.client_order_id.clone(),
                    "InsufficientFunds",
                    format!("required {required}, available {available}"),
                    timestamp,
                    strategy,
                    trace,
                );
            }
            Err(ExecError::InsufficientPosition {
                symbol,
                required,
                available,
            }) => {
                return self.reject_order(
                    request.client_order_id.clone(),
                    "InsufficientPosition",
                    format!("`{symbol}` required {required}, available {available}"),
                    timestamp,
                    strategy,
                    trace,
                );
            }
            Err(ExecError::DuplicateClientOrderId { .. }) => {
                return self.reject_order(
                    request.client_order_id.clone(),
                    "DuplicateOrderId",
                    "client order id has already been used".to_owned(),
                    timestamp,
                    strategy,
                    trace,
                );
            }
            Err(error) => return Err(error),
        }
        self.risk_gate.accept(&request, &risk_context);

        let accepted = ExecutionReport::Accepted {
            client_order_id: request.client_order_id.clone(),
            ts: timestamp,
        };
        self.ledger.apply_execution_report(&accepted)?;
        self.matching
            .register_order(request.clone(), reference_price)?;
        self.journal
            .record(&JournalRecord::submit(timestamp, &request))?;
        self.journal.record(&JournalRecord::accepted(
            timestamp,
            &request.client_order_id,
        ))?;
        trace.counters.accepted = trace.counters.accepted.saturating_add(1);
        self.deliver(&accepted, strategy, timestamp)
    }

    fn cancel_order(
        &mut self,
        client_order_id: &ClientOrderId,
        timestamp: DateTime<Utc>,
        strategy: &mut dyn Strategy,
        trace: &mut RunTrace<'_>,
    ) -> Result<(), ExecError> {
        self.ledger.request_cancel(client_order_id, timestamp)?;
        match self.matching.cancel(client_order_id) {
            Ok(_) | Err(ExecError::OrderNotFound { .. }) => {}
            Err(error) => return Err(error),
        }
        let report = ExecutionReport::Canceled {
            client_order_id: client_order_id.clone(),
            ts: timestamp,
        };
        self.ledger.apply_execution_report(&report)?;
        self.journal
            .record(&JournalRecord::canceled(timestamp, client_order_id))?;
        trace.counters.canceled = trace.counters.canceled.saturating_add(1);
        self.deliver(&report, strategy, timestamp)
    }

    fn reject_order(
        &mut self,
        client_order_id: ClientOrderId,
        rule: &str,
        reason: String,
        timestamp: DateTime<Utc>,
        strategy: &mut dyn Strategy,
        trace: &mut RunTrace<'_>,
    ) -> Result<(), ExecError> {
        let report = ExecutionReport::Rejected {
            client_order_id: client_order_id.clone(),
            reason: reason.clone(),
            ts: timestamp,
        };
        self.journal.record(&JournalRecord::risk_rejected(
            timestamp,
            &client_order_id,
            rule,
            &reason,
        ))?;
        trace.rejections.push(RiskRejection {
            timestamp,
            client_order_id: client_order_id.to_string(),
            rule: rule.to_owned(),
            reason,
        });
        trace.counters.rejected = trace.counters.rejected.saturating_add(1);
        self.deliver(&report, strategy, timestamp)
    }

    fn apply_reports(
        &mut self,
        reports: Vec<ExecutionReport>,
        strategy: &mut dyn Strategy,
        timestamp: DateTime<Utc>,
        trace: &mut RunTrace<'_>,
    ) -> Result<(), ExecError> {
        for report in reports {
            match &report {
                ExecutionReport::Fill {
                    client_order_id,
                    exec_id,
                    symbol,
                    side,
                    fill_price,
                    fill_qty,
                    fee,
                    slippage,
                    ts,
                } => {
                    self.ledger.apply_execution_report(&report)?;
                    self.journal.record(&JournalRecord::fill(&report)?)?;
                    trace.fills.push(ExecFill {
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
                ExecutionReport::Canceled {
                    client_order_id,
                    ts,
                } => {
                    self.ledger.apply_execution_report(&report)?;
                    self.journal
                        .record(&JournalRecord::canceled(*ts, client_order_id))?;
                    trace.counters.canceled = trace.counters.canceled.saturating_add(1);
                }
                ExecutionReport::Expired { .. } => {
                    self.ledger.apply_execution_report(&report)?;
                    trace.counters.expired = trace.counters.expired.saturating_add(1);
                }
                ExecutionReport::Rejected { .. } => {
                    self.ledger.apply_execution_report(&report)?;
                    trace.counters.rejected = trace.counters.rejected.saturating_add(1);
                }
                ExecutionReport::Accepted { .. } => {
                    self.ledger.apply_execution_report(&report)?;
                }
            }
            self.deliver(&report, strategy, timestamp)?;
        }
        Ok(())
    }

    fn deliver(
        &self,
        report: &ExecutionReport,
        strategy: &mut dyn Strategy,
        timestamp: DateTime<Utc>,
    ) -> Result<(), ExecError> {
        let context = StrategyContext::new(&self.ledger, timestamp, &self.order_sequence);
        strategy.on_execution_report(report, &context)
    }
}

fn returns_from_curve(curve: &[ExecEquityPoint]) -> Result<Vec<f64>, ExecError> {
    let mut returns = Vec::new();
    for pair in curve.windows(2) {
        let Some(previous) = pair.first() else {
            continue;
        };
        let Some(current) = pair.get(1) else {
            continue;
        };
        if previous.equity <= Decimal::ZERO || current.equity <= Decimal::ZERO {
            return Err(ExecError::strategy(
                "equity must be strictly positive at every observation",
            ));
        }
        let change = current.equity.checked_sub(previous.equity).ok_or(
            kurumi_core::KurumiError::NumericalOverflow {
                operation: "backtest returns",
            },
        )?;
        let ratio = change.checked_div(previous.equity).ok_or(
            kurumi_core::KurumiError::NumericalOverflow {
                operation: "backtest returns",
            },
        )?;
        let value = ratio
            .to_f64()
            .ok_or(kurumi_core::KurumiError::InvalidParameter {
                parameter: "equity",
                reason: "return is not representable".to_owned(),
            })?;
        if !value.is_finite() {
            return Err(ExecError::strategy("equity return is not finite"));
        }
        returns.push(value);
    }
    Ok(returns)
}
