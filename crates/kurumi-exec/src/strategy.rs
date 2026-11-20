//! The synchronous strategy interface and the portfolio rebalance bridge.
//!
//! A [`Strategy`] receives immutable [`StrategyContext`] snapshots of the
//! ledger, deterministic timestamps, and deterministic client order identifiers.
//! It returns [`OrderCommand`]s; the runner owns risk checks, reservations,
//! matching, and accounting. The module has no runtime, clock, or network
//! dependency, so the same strategy implementation runs inside a backtest or a
//! replay feed.
//!
//! [`PortfolioRebalanceStrategy`] wraps an existing
//! [`RebalanceTrigger`] and
//! [`TargetWeightAllocator`] from
//! `kurumi-backtest`, maintaining its own price history and emitting
//! sell-before-buy market orders. HRP, ERC, and 60/40 allocations therefore run
//! unchanged on the execution kernel.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use kurumi_backtest::portfolio::{
    AllocationContext, MarketSnapshot, RebalanceCause, RebalanceTrigger, TargetWeightAllocator,
    TriggerContext,
};
use kurumi_portfolio::nalgebra::DVector;
use kurumi_portfolio::{
    AlignmentPolicy, AssetSeries, Holding, PortfolioWeights, ReturnMatrix, ReturnMethod,
    generate_rebalance_actions,
};
use kurumi_ta::OhlcvBar;
use rust_decimal::Decimal;

use crate::error::ExecError;
use crate::ledger::ExecutionLedger;
use crate::order::{
    ClientOrderId, ExecutionReport, OrderCommand, OrderRecord, OrderRequest, OrderSide, OrderType,
    TimeInForce,
};
use crate::util::{decimal_from_f64, f64_from_decimal};

/// A market event delivered to a strategy.
///
/// The variants mirror the synchronous market data the kernel accepts: trades,
/// top-of-book quotes, OHLCV bars, and full multi-asset snapshots. The type is
/// independent of `kurumi-feed`, keeping the core free of any asynchronous
/// dependency.
///
/// # Examples
///
/// ```
/// use kurumi_exec::ExecMarketEvent;
/// use kurumi_exec::chrono::{DateTime, Utc};
/// use rust_decimal::Decimal;
///
/// let ts = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
/// let event = ExecMarketEvent::Trade {
///     symbol: "AAA".to_owned(),
///     price: Decimal::from(100),
///     size: Decimal::ONE,
///     ts,
/// };
/// assert_eq!(event.timestamp(), ts);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum ExecMarketEvent {
    /// One executed trade.
    Trade {
        /// Instrument symbol.
        symbol: String,
        /// Trade price.
        price: Decimal,
        /// Trade size.
        size: Decimal,
        /// Event timestamp.
        ts: DateTime<Utc>,
    },
    /// A top-of-book quote.
    Quote {
        /// Instrument symbol.
        symbol: String,
        /// Best bid price.
        bid_price: Decimal,
        /// Best bid size.
        bid_size: Decimal,
        /// Best ask price.
        ask_price: Decimal,
        /// Best ask size.
        ask_size: Decimal,
        /// Event timestamp.
        ts: DateTime<Utc>,
    },
    /// A completed OHLCV bar.
    Bar {
        /// Instrument symbol.
        symbol: String,
        /// The bar.
        bar: OhlcvBar,
        /// Event timestamp.
        ts: DateTime<Utc>,
    },
    /// A full multi-asset snapshot.
    Snapshot(MarketSnapshot),
}

impl ExecMarketEvent {
    /// Returns the event timestamp.
    #[must_use]
    #[inline]
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Self::Trade { ts, .. } | Self::Quote { ts, .. } | Self::Bar { ts, .. } => *ts,
            Self::Snapshot(snapshot) => snapshot.timestamp(),
        }
    }

    /// Returns the event's single instrument, if it has one.
    #[must_use]
    #[inline]
    pub fn symbol(&self) -> Option<&str> {
        match self {
            Self::Trade { symbol, .. } | Self::Quote { symbol, .. } | Self::Bar { symbol, .. } => {
                Some(symbol)
            }
            Self::Snapshot(_) => None,
        }
    }

    /// Returns the reference price of `symbol` for `side`.
    ///
    /// A quote uses its ask for a buy and its bid for a sell; a trade or bar uses
    /// its price or close. Snapshots use the snapshot bar's close.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::StrategyError`] when a bar price is not a positive,
    /// representable decimal.
    pub fn reference_price(
        &self,
        symbol: &str,
        side: OrderSide,
    ) -> Result<Option<Decimal>, ExecError> {
        match self {
            Self::Trade {
                symbol: event_symbol,
                price,
                ..
            } => Ok((event_symbol == symbol).then_some(*price)),
            Self::Quote {
                symbol: event_symbol,
                bid_price,
                ask_price,
                ..
            } => {
                if event_symbol == symbol {
                    Ok(Some(match side {
                        OrderSide::Buy => *ask_price,
                        OrderSide::Sell => *bid_price,
                    }))
                } else {
                    Ok(None)
                }
            }
            Self::Bar {
                symbol: event_symbol,
                bar,
                ..
            } => {
                if event_symbol == symbol {
                    decimal_from_f64(bar.close(), symbol).map(Some)
                } else {
                    Ok(None)
                }
            }
            Self::Snapshot(snapshot) => match snapshot.bar(symbol) {
                Some(bar) => decimal_from_f64(bar.close(), symbol).map(Some),
                None => Ok(None),
            },
        }
    }

    /// Returns the latest price of `symbol` carried by the event.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::StrategyError`] when a bar price is invalid.
    pub fn last_price(&self, symbol: &str) -> Result<Option<Decimal>, ExecError> {
        self.reference_price(symbol, OrderSide::Buy)
    }

    /// Returns the event's bars keyed by symbol, or `None` for a trade or quote.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::StrategyError`] when a bar price is invalid.
    pub fn bars(&self) -> Result<Option<BTreeMap<String, Decimal>>, ExecError> {
        match self {
            Self::Snapshot(snapshot) => {
                let mut closes = BTreeMap::new();
                for (symbol, bar) in snapshot.bars() {
                    closes.insert(symbol.clone(), decimal_from_f64(bar.close(), symbol)?);
                }
                Ok(Some(closes))
            }
            Self::Bar { symbol, bar, .. } => {
                let mut closes = BTreeMap::new();
                closes.insert(symbol.clone(), decimal_from_f64(bar.close(), symbol)?);
                Ok(Some(closes))
            }
            Self::Trade { .. } | Self::Quote { .. } => Ok(None),
        }
    }
}

/// A read-only view of the ledger handed to a strategy callback.
///
/// The context is cheap to construct per callback and never grants mutation.
/// [`StrategyContext::next_order_id`] is the only stateful operation: it
/// increments a caller-owned sequence, guaranteeing deterministic identifiers
/// across identical runs.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{ExecutionLedger, StrategyContext};
/// use chrono::{DateTime, Utc};
/// use rust_decimal::Decimal;
/// use std::sync::atomic::AtomicU64;
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let ledger = ExecutionLedger::new(Decimal::from(1_000))?;
/// let sequence = AtomicU64::new(0);
/// let now = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
/// let context = StrategyContext::new(&ledger, now, &sequence);
/// assert_eq!(context.now(), now);
/// assert_eq!(context.next_order_id().as_str(), "ord-000000000000");
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct StrategyContext<'a> {
    ledger: &'a ExecutionLedger,
    now: DateTime<Utc>,
    order_sequence: &'a AtomicU64,
}

impl<'a> StrategyContext<'a> {
    /// Creates a context from the ledger, the event timestamp, and the shared
    /// order-id sequence.
    #[must_use]
    pub fn new(
        ledger: &'a ExecutionLedger,
        now: DateTime<Utc>,
        order_sequence: &'a AtomicU64,
    ) -> Self {
        Self {
            ledger,
            now,
            order_sequence,
        }
    }

    /// Returns the event timestamp supplied by the runner.
    ///
    /// Strategies must use this instead of reading the wall clock so replay is
    /// deterministic.
    #[must_use]
    #[inline]
    pub fn now(&self) -> DateTime<Utc> {
        self.now
    }

    /// Returns current equity.
    ///
    /// # Errors
    ///
    /// Propagates [`ExecutionLedger::equity`].
    #[inline]
    pub fn equity(&self) -> Result<Decimal, ExecError> {
        self.ledger.equity()
    }

    /// Returns unreserved cash.
    #[must_use]
    #[inline]
    pub fn available_cash(&self) -> Decimal {
        self.ledger.available_cash()
    }

    /// Returns the signed position quantity of `symbol`.
    #[must_use]
    #[inline]
    pub fn position_qty(&self, symbol: &str) -> Decimal {
        self.ledger.position_qty(symbol)
    }

    /// Returns marked-to-market position weights.
    ///
    /// # Errors
    ///
    /// Propagates [`ExecutionLedger::current_weights`].
    #[inline]
    pub fn current_weights(&self) -> Result<BTreeMap<String, f64>, ExecError> {
        self.ledger.current_weights()
    }

    /// Returns the last mark price of `symbol`.
    #[must_use]
    #[inline]
    pub fn last_price(&self, symbol: &str) -> Option<Decimal> {
        self.ledger.last_price(symbol)
    }

    /// Iterates over the orders that are not terminal.
    pub fn open_orders(&self) -> impl Iterator<Item = &'a OrderRecord> {
        self.ledger.open_orders()
    }

    /// Generates the next deterministic client order identifier.
    ///
    /// The identifier is `ord-` followed by the zero-padded sequence number, so
    /// two identical runs produce identical identifiers.
    #[must_use]
    pub fn next_order_id(&self) -> ClientOrderId {
        let sequence = self.order_sequence.fetch_add(1, Ordering::Relaxed);
        ClientOrderId::new(format!("ord-{sequence:012}"))
    }
}

/// A fully synchronous strategy.
///
/// Implementations must not read the wall clock, generate random identifiers,
/// or perform I/O. They derive every decision from the event and the
/// [`StrategyContext`].
///
/// # Examples
///
/// ```
/// use kurumi_exec::{
///     ExecError, ExecMarketEvent, OrderCommand, Strategy, StrategyContext,
/// };
///
/// struct BuyAndHold;
///
/// impl Strategy for BuyAndHold {
///     fn id(&self) -> &str {
///         "buy-and-hold"
///     }
///
///     fn on_market_event(
///         &mut self,
///         _event: &ExecMarketEvent,
///         _ctx: &StrategyContext<'_>,
///     ) -> Result<Vec<OrderCommand>, ExecError> {
///         Ok(Vec::new())
///     }
/// }
///
/// let strategy = BuyAndHold;
/// assert_eq!(strategy.id(), "buy-and-hold");
/// ```
pub trait Strategy {
    /// Returns the stable strategy identifier used by journals.
    fn id(&self) -> &str;

    /// Called once before the first market event.
    ///
    /// The default implementation emits no orders.
    ///
    /// # Errors
    ///
    /// A strategy may return any [`ExecError`]; the runner aborts the run.
    fn on_start(&mut self, _ctx: &StrategyContext<'_>) -> Result<Vec<OrderCommand>, ExecError> {
        Ok(Vec::new())
    }

    /// Called once per market event.
    ///
    /// # Errors
    ///
    /// A strategy may return any [`ExecError`]; the runner aborts the run.
    fn on_market_event(
        &mut self,
        event: &ExecMarketEvent,
        ctx: &StrategyContext<'_>,
    ) -> Result<Vec<OrderCommand>, ExecError>;

    /// Called for every execution report produced for the strategy's orders.
    ///
    /// The default implementation ignores reports.
    ///
    /// # Errors
    ///
    /// A strategy may return any [`ExecError`]; the runner aborts the run.
    fn on_execution_report(
        &mut self,
        _report: &ExecutionReport,
        _ctx: &StrategyContext<'_>,
    ) -> Result<(), ExecError> {
        Ok(())
    }

    /// Called once after the last market event.
    ///
    /// The default implementation emits no orders. Market orders returned here
    /// execute against the final observation; limit and stop orders are
    /// registered but wait for a later event, so they may remain open when the
    /// run ends.
    ///
    /// # Errors
    ///
    /// A strategy may return any [`ExecError`]; the runner aborts the run.
    fn on_stop(&mut self, _ctx: &StrategyContext<'_>) -> Result<Vec<OrderCommand>, ExecError> {
        Ok(Vec::new())
    }
}

/// Configuration of [`PortfolioRebalanceStrategy`].
///
/// The defaults mirror the multi-asset backtest engine: a 60-observation
/// lookback, simple returns, a 0.2% cash buffer, no minimum trade value, and no
/// fractional trading.
///
/// # Examples
///
/// ```
/// use kurumi_exec::RebalanceStrategyConfig;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let mut config = RebalanceStrategyConfig::default();
/// config.lookback_window = 20;
/// config.validate()?;
/// assert_eq!(config.lookback_window, 20);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RebalanceStrategyConfig {
    /// Maximum number of return observations passed to the allocator.
    pub lookback_window: usize,
    /// Simple or log returns used to build the allocation history.
    pub return_method: ReturnMethod,
    /// Fraction of equity kept in cash to absorb fees and slippage; in `[0, 1)`.
    pub cash_buffer_ratio: Decimal,
    /// Per-order notional below which generated orders are suppressed.
    pub min_trade_value: Decimal,
    /// Whether fractional units may be traded.
    pub allow_fractional: bool,
}

impl Default for RebalanceStrategyConfig {
    fn default() -> Self {
        Self {
            lookback_window: 60,
            return_method: ReturnMethod::Simple,
            cash_buffer_ratio: Decimal::new(2, 3),
            min_trade_value: Decimal::ZERO,
            allow_fractional: false,
        }
    }
}

impl RebalanceStrategyConfig {
    /// Validates the configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::StrategyError`] when the lookback window is zero,
    /// the cash buffer is outside `[0, 1)`, or the minimum trade value is
    /// negative.
    pub fn validate(&self) -> Result<(), ExecError> {
        if self.lookback_window == 0 {
            return Err(ExecError::strategy("lookback_window must be at least one"));
        }
        if self.cash_buffer_ratio < Decimal::ZERO || self.cash_buffer_ratio >= Decimal::ONE {
            return Err(ExecError::strategy("cash_buffer_ratio must be in [0, 1)"));
        }
        if self.min_trade_value < Decimal::ZERO {
            return Err(ExecError::strategy("min_trade_value must be non-negative"));
        }
        Ok(())
    }
}

/// Bridges a [`RebalanceTrigger`] and a [`TargetWeightAllocator`] into the
/// execution kernel.
///
/// The strategy caches the close-price history of the configured universe,
/// asks the trigger whether the observation is a rebalance date, builds a
/// `(T, N)` return matrix up to and including the observation, runs the
/// allocator, and emits deterministic market orders with sells first.
///
/// The strategy never observes a later bar: history is appended only for the
/// event currently being processed, so the allocation is look-ahead free.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::{CalendarTrigger, StaticWeightAllocator};
/// use kurumi_exec::{PortfolioRebalanceStrategy, Strategy};
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let allocator = StaticWeightAllocator::new(
///     [("AAA".to_owned(), 0.5), ("BBB".to_owned(), 0.5)].into(),
/// )?;
/// let strategy = PortfolioRebalanceStrategy::new(
///     vec!["AAA".to_owned(), "BBB".to_owned()],
///     Box::new(CalendarTrigger::Monthly),
///     Box::new(allocator),
/// )?;
/// assert_eq!(strategy.id(), "portfolio-rebalance");
/// # Ok(())
/// # }
/// ```
pub struct PortfolioRebalanceStrategy {
    id: String,
    symbols: Vec<String>,
    trigger: Box<dyn RebalanceTrigger>,
    allocator: Box<dyn TargetWeightAllocator>,
    config: RebalanceStrategyConfig,
    history: BTreeMap<String, Vec<(DateTime<Utc>, f64)>>,
    bars_advanced: usize,
    last_rebalance_timestamp: Option<DateTime<Utc>>,
    last_rebalance_bar: Option<usize>,
    last_target_weights: BTreeMap<String, f64>,
    last_cause: Option<RebalanceCause>,
}

impl PortfolioRebalanceStrategy {
    /// Creates a strategy with the default configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::StrategyError`] when the universe is empty, contains
    /// an empty or duplicate symbol, or when the default configuration is
    /// invalid.
    pub fn new(
        symbols: Vec<String>,
        trigger: Box<dyn RebalanceTrigger>,
        allocator: Box<dyn TargetWeightAllocator>,
    ) -> Result<Self, ExecError> {
        Self::with_config(
            symbols,
            trigger,
            allocator,
            RebalanceStrategyConfig::default(),
        )
    }

    /// Creates a strategy with an explicit configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::StrategyError`] when the universe is empty, contains
    /// an empty or duplicate symbol, or when the configuration is invalid.
    pub fn with_config(
        symbols: Vec<String>,
        trigger: Box<dyn RebalanceTrigger>,
        allocator: Box<dyn TargetWeightAllocator>,
        config: RebalanceStrategyConfig,
    ) -> Result<Self, ExecError> {
        if symbols.is_empty() {
            return Err(ExecError::strategy("rebalance universe must not be empty"));
        }
        let mut seen = BTreeSet::new();
        for symbol in &symbols {
            if symbol.trim().is_empty() {
                return Err(ExecError::strategy("rebalance symbol must not be empty"));
            }
            if !seen.insert(symbol.clone()) {
                return Err(ExecError::strategy(format!(
                    "duplicate rebalance symbol `{symbol}`"
                )));
            }
        }
        config.validate()?;
        let history = symbols
            .iter()
            .map(|symbol| (symbol.clone(), Vec::new()))
            .collect();
        Ok(Self {
            id: "portfolio-rebalance".to_owned(),
            symbols,
            trigger,
            allocator,
            config,
            history,
            bars_advanced: 0,
            last_rebalance_timestamp: None,
            last_rebalance_bar: None,
            last_target_weights: BTreeMap::new(),
            last_cause: None,
        })
    }

    /// Overrides the strategy identifier.
    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    /// Returns the configured universe.
    #[must_use]
    #[inline]
    pub fn symbols(&self) -> &[String] {
        &self.symbols
    }

    /// Returns the configuration.
    #[must_use]
    #[inline]
    pub fn config(&self) -> &RebalanceStrategyConfig {
        &self.config
    }

    /// Returns the last normalized target weights.
    #[must_use]
    #[inline]
    pub fn last_target_weights(&self) -> &BTreeMap<String, f64> {
        &self.last_target_weights
    }

    /// Returns the cause of the last rebalance, if one has happened.
    #[must_use]
    #[inline]
    pub fn last_cause(&self) -> Option<RebalanceCause> {
        self.last_cause
    }

    /// Returns the number of events processed so far.
    #[must_use]
    #[inline]
    pub fn bars_advanced(&self) -> usize {
        self.bars_advanced
    }

    /// Records the close prices carried by the event in the rolling history.
    fn record_history(&mut self, event: &ExecMarketEvent) -> Result<(), ExecError> {
        let Some(bars) = event.bars()? else {
            return Ok(());
        };
        let timestamp = event.timestamp();
        for (symbol, close) in bars {
            let Some(observations) = self.history.get_mut(&symbol) else {
                continue;
            };
            observations.push((timestamp, f64_from_decimal(close)));
        }
        Ok(())
    }

    fn build_returns(&self) -> Result<Option<ReturnMatrix>, ExecError> {
        let mut series = Vec::new();
        for symbol in &self.symbols {
            let Some(observations) = self.history.get(symbol) else {
                continue;
            };
            if observations.len() < 2 {
                continue;
            }
            series.push(AssetSeries::new(symbol.clone(), observations.clone())?);
        }
        if series.is_empty() {
            return Ok(None);
        }
        let matrix = match ReturnMatrix::from_price_series(
            &series,
            AlignmentPolicy::InnerJoin,
            self.config.return_method,
        ) {
            Ok(matrix) => matrix,
            Err(kurumi_core::KurumiError::InsufficientData { .. }) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let (rows, columns) = matrix.dimensions();
        if rows <= self.config.lookback_window {
            return Ok(Some(matrix));
        }
        let start = rows - self.config.lookback_window;
        let Some(timestamps) = matrix.timestamps().get(start..) else {
            return Err(ExecError::strategy(
                "return history is shorter than the lookback",
            ));
        };
        let data = matrix
            .data()
            .view((start, 0), (self.config.lookback_window, columns))
            .into_owned();
        let trimmed = ReturnMatrix::new(matrix.symbols().to_vec(), timestamps.to_vec(), data)?;
        Ok(Some(trimmed))
    }

    fn build_orders(
        &mut self,
        ctx: &StrategyContext<'_>,
        allocation: &PortfolioWeights,
        timestamp: DateTime<Utc>,
    ) -> Result<Vec<OrderCommand>, ExecError> {
        let mut raw = BTreeMap::new();
        for (symbol, weight) in allocation.symbols().iter().zip(allocation.weights().iter()) {
            if !self.symbols.iter().any(|candidate| candidate == symbol) {
                return Err(ExecError::strategy(format!(
                    "allocator returned unknown symbol `{symbol}`"
                )));
            }
            if !weight.is_finite() {
                return Err(ExecError::strategy(format!(
                    "weight of `{symbol}` is not finite"
                )));
            }
            if *weight < 0.0 {
                return Err(ExecError::strategy(format!(
                    "weight of `{symbol}` is negative; short targets are unsupported"
                )));
            }
            raw.insert(symbol.clone(), *weight);
        }

        let buffer = f64_from_decimal(self.config.cash_buffer_ratio);
        let investable = (1.0 - buffer).max(0.0);
        let total: f64 = raw.values().sum();
        let scale = if total > investable && total > 0.0 {
            investable / total
        } else {
            1.0
        };

        let mut holdings = Vec::new();
        let mut target_symbols = Vec::new();
        let mut target_values = Vec::new();
        let mut target_weights = BTreeMap::new();
        for symbol in &self.symbols {
            let quantity = ctx.position_qty(symbol);
            let target_weight = raw.get(symbol).copied().unwrap_or(0.0) * scale;
            target_weights.insert(symbol.clone(), target_weight);
            if quantity.is_zero() && target_weight <= 0.0 {
                continue;
            }
            let price = ctx.last_price(symbol).ok_or_else(|| {
                ExecError::strategy(format!("no mark price is known for `{symbol}`"))
            })?;
            holdings.push(Holding::new(symbol.clone(), quantity, price)?);
            target_symbols.push(symbol.clone());
            target_values.push(target_weight);
        }
        if holdings.is_empty() {
            self.last_target_weights = target_weights;
            self.last_rebalance_timestamp = Some(timestamp);
            self.last_rebalance_bar = Some(self.bars_advanced);
            return Ok(Vec::new());
        }
        let target = PortfolioWeights::new(target_symbols, DVector::from_vec(target_values))?;
        let equity = ctx.equity()?;
        let actions = generate_rebalance_actions(
            equity,
            &holdings,
            &target,
            self.config.min_trade_value,
            self.config.allow_fractional,
        )?;

        let mut actions = actions
            .into_iter()
            .filter(|action| action.quantity > Decimal::ZERO)
            .collect::<Vec<_>>();
        actions.sort_by(|left, right| {
            let rank = |side: kurumi_portfolio::RebalanceSide| match side {
                kurumi_portfolio::RebalanceSide::Sell => 0_u8,
                kurumi_portfolio::RebalanceSide::Buy => 1_u8,
            };
            rank(left.side)
                .cmp(&rank(right.side))
                .then_with(|| left.symbol.cmp(&right.symbol))
        });

        let mut commands = Vec::with_capacity(actions.len());
        for action in actions {
            let side = match action.side {
                kurumi_portfolio::RebalanceSide::Buy => OrderSide::Buy,
                kurumi_portfolio::RebalanceSide::Sell => OrderSide::Sell,
            };
            let mut request = OrderRequest::new(
                ctx.next_order_id(),
                action.symbol.clone(),
                side,
                OrderType::Market,
                action.quantity,
                TimeInForce::Day,
            )
            .with_tag("rebalance");
            if side == OrderSide::Sell {
                request = request.reduce_only();
            }
            commands.push(OrderCommand::Submit(request));
        }

        self.last_target_weights = target_weights;
        self.last_rebalance_timestamp = Some(timestamp);
        self.last_rebalance_bar = Some(self.bars_advanced);
        Ok(commands)
    }
}

impl Strategy for PortfolioRebalanceStrategy {
    fn id(&self) -> &str {
        &self.id
    }

    fn on_market_event(
        &mut self,
        event: &ExecMarketEvent,
        ctx: &StrategyContext<'_>,
    ) -> Result<Vec<OrderCommand>, ExecError> {
        self.record_history(event)?;
        self.bars_advanced = self.bars_advanced.saturating_add(1);
        let current_weights = ctx.current_weights()?;
        let trigger_context = TriggerContext {
            current_timestamp: event.timestamp(),
            last_rebalance_timestamp: self.last_rebalance_timestamp,
            bars_advanced: self.bars_advanced,
            last_rebalance_bar: self.last_rebalance_bar,
            current_weights,
            target_weights: self.last_target_weights.clone(),
        };
        if !self.trigger.should_rebalance(&trigger_context) {
            return Ok(Vec::new());
        }
        let cause = if self.last_rebalance_timestamp.is_none() {
            RebalanceCause::Initial
        } else {
            self.trigger.cause(&trigger_context)
        };
        let returns = self.build_returns()?;
        let allocation_context =
            AllocationContext::new(event.timestamp(), self.symbols.clone(), returns);
        let allocation = self.allocator.allocate(&allocation_context)?;
        let commands = self.build_orders(ctx, &allocation, event.timestamp())?;
        self.last_cause = Some(cause);
        Ok(commands)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurumi_backtest::StaticWeightAllocator;
    use kurumi_backtest::portfolio::CalendarTrigger;

    #[test]
    fn context_ids_are_deterministic() {
        let ledger = ExecutionLedger::new(Decimal::from(1_000)).expect("ledger");
        let sequence = AtomicU64::new(7);
        let now = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
        let context = StrategyContext::new(&ledger, now, &sequence);
        assert_eq!(context.next_order_id().as_str(), "ord-000000000007");
        assert_eq!(context.next_order_id().as_str(), "ord-000000000008");
    }

    #[test]
    fn rebalance_strategy_validates_its_universe() {
        let allocator =
            StaticWeightAllocator::new(BTreeMap::from([("AAA".to_owned(), 1.0)])).expect("weights");
        assert!(
            PortfolioRebalanceStrategy::new(
                Vec::new(),
                Box::new(CalendarTrigger::Monthly),
                Box::new(allocator),
            )
            .is_err()
        );
    }
}
