//! The multi-asset portfolio backtest engine.
//!
//! [`PortfolioBacktestEngine`] walks a [`MultiAssetFeed`] one
//! [`MarketSnapshot`] at a time, asks a [`RebalanceTrigger`] whether to rebalance,
//! obtains target weights from a [`TargetWeightAllocator`], and turns them into
//! sell-before-buy executions with the existing fee and slippage models.
//!
//! # Execution model
//!
//! - [`ExecutionTiming::SignalBarClose`] fills on the close of the bar that
//!   produced the signal.
//! - [`ExecutionTiming::NextBarOpen`] queues the generated orders and fills them
//!   on the open of the next observation in the feed.
//!
//! Allocation always sees prices and returns up to the signal timestamp only.
//! Nothing after the signal is visible to the allocator, so `NextBarOpen` does
//! not introduce look-ahead bias.
//!
//! # Halt handling
//!
//! A symbol that has no bar in the signal snapshot is frozen: its current
//! position is not traded, and the remaining tradable target weights are
//! rescaled to fill the equity left after the frozen position and the cash
//! buffer. This keeps a suspended instrument in the book without inventing a
//! quote for it.
//!
//! # Warmup
//!
//! [`PortfolioBacktestEngine::run_on_slice`] accepts a warmup prefix. Bars
//! before `warmup_bars` update the account's last known prices and the rolling
//! price history only: no trigger is consulted, no order is generated, and no
//! equity or weight observation is recorded. The first evaluated bar can
//! therefore consume the whole warmup history for its initial allocation
//! without a cold start, and the reported curves begin at that bar.
//!
//! # Sell-before-buy
//!
//! Sells are settled first, then the buy basket is checked against the cash that
//! remains. When the basket needs more cash than is available, every buy is
//! scaled by `available_cash / required_cash` and floored to the configured
//! quantity precision before execution, so fees and slippage cannot push cash
//! negative or cause a valid rebalance to be rejected.
//!
//! # Examples
//!
//! ```
//! use kurumi_backtest::portfolio::{
//!     CalendarTrigger, ExecutionTiming, PortfolioBacktestConfig, PortfolioBacktestEngine,
//!     MarketSnapshot, MultiAssetFeed, StaticWeightAllocator,
//! };
//! use kurumi_backtest::{NoFee, NoSlippage};
//! use kurumi_ta::OhlcvBar;
//! use rust_decimal::Decimal;
//! use std::collections::BTreeMap;
//!
//! # fn main() -> Result<(), kurumi_backtest::KurumiError> {
//! let day = 86_400_000_i64;
//! let mut series = BTreeMap::new();
//! for (symbol, prices) in [
//!     ("STOCK", [100.0, 101.0, 102.0]),
//!     ("BOND", [50.0, 50.5, 51.0]),
//! ] {
//!     let bars = prices
//!         .iter()
//!         .enumerate()
//!         .map(|(index, price)| {
//!             OhlcvBar::new(
//!                 index as i64 * day,
//!                 *price,
//!                 price + 1.0,
//!                 price - 1.0,
//!                 *price,
//!                 1_000.0,
//!                 price * 1_000.0,
//!             )
//!         })
//!         .collect::<Result<Vec<_>, _>>()?;
//!     series.insert(symbol.to_owned(), bars);
//! }
//! let feed = MultiAssetFeed::new(series)?;
//! let mut config = PortfolioBacktestConfig::new(Decimal::from(10_000))?;
//! config.execution_timing = ExecutionTiming::SignalBarClose;
//! let mut engine = PortfolioBacktestEngine::new(
//!     config,
//!     Box::new(NoFee),
//!     Box::new(NoSlippage),
//! )?;
//! let mut allocator = StaticWeightAllocator::new(BTreeMap::from([
//!     ("STOCK".to_owned(), 0.6),
//!     ("BOND".to_owned(), 0.4),
//! ]))?;
//! let report = engine.run(feed, &CalendarTrigger::Monthly, &mut allocator)?;
//! assert_eq!(report.rebalance_events.len(), 1);
//! assert!(report.final_equity > Decimal::ZERO);
//! # Ok(())
//! # }
//! ```

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use kurumi_core::{KurumiError, Price, Quantity};
use kurumi_portfolio::nalgebra::DVector;
use kurumi_portfolio::{
    AlignmentPolicy, AssetSeries, Holding, PortfolioWeights, RebalanceSide, ReturnMatrix,
    ReturnMethod, generate_rebalance_actions,
};
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

use crate::fee::{FeeModel, Liquidity};
use crate::order::Side;
use crate::portfolio::account::{MultiAssetAccount, MultiAssetFill};
use crate::portfolio::allocator::{AllocationContext, TargetWeightAllocator};
use crate::portfolio::feed::{
    AlignedMarketData, MarketSlice, MarketSnapshot, MultiAssetFeed, decimal_from_f64,
};
use crate::portfolio::report::{PortfolioBacktestReport, RebalanceEvent};
use crate::portfolio::trigger::{RebalanceCause, RebalanceTrigger, TriggerContext};
use crate::slippage::SlippageModel;

/// The quantity precision used when fractional trading is enabled.
///
/// Buy quantities are rounded down to this many decimal places so that the
/// scaled basket can never require more cash than the account holds.
const FRACTIONAL_QUANTITY_SCALE: u32 = 12;

/// When a rebalance signal is converted into executions.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::ExecutionTiming;
///
/// assert_ne!(ExecutionTiming::SignalBarClose, ExecutionTiming::NextBarOpen);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionTiming {
    /// Fill on the close of the bar that produced the signal.
    #[default]
    SignalBarClose,
    /// Queue the orders and fill them on the open of the next observation.
    NextBarOpen,
}

/// Configuration of a multi-asset portfolio backtest.
///
/// The default is a fully configurable daily backtest with a 60-period lookback,
/// a 0.2% cash buffer, fractional trading disabled, and a zero minimum trade
/// value. Use [`PortfolioBacktestConfig::new`] to set the starting cash and then
/// override the public fields.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::{ExecutionTiming, PortfolioBacktestConfig};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let mut config = PortfolioBacktestConfig::new(Decimal::from(1_000_000))?;
/// config.execution_timing = ExecutionTiming::NextBarOpen;
/// config.cash_buffer_ratio = Decimal::new(5, 3);
/// config.validate()?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PortfolioBacktestConfig {
    /// Starting cash of the account; must be strictly positive.
    pub initial_cash: Decimal,
    /// When generated orders are executed.
    pub execution_timing: ExecutionTiming,
    /// Fraction of equity kept as cash to absorb fees and slippage; in
    /// `[0, 1)`.
    pub cash_buffer_ratio: Decimal,
    /// Per-order notional below which generated orders are suppressed as noise.
    pub min_trade_value: Decimal,
    /// Whether fractional units may be traded.
    pub allow_fractional: bool,
    /// Maximum number of return observations passed to the allocator.
    pub lookback_window: usize,
    /// Simple or log returns used to build the allocation history.
    pub return_method: ReturnMethod,
    /// Number of return periods in a year used by the performance report.
    pub periods_per_year: f64,
    /// Annual risk-free rate used by the Sharpe and Jensen's alpha statistics.
    pub risk_free_rate: f64,
}

impl Default for PortfolioBacktestConfig {
    fn default() -> Self {
        Self {
            initial_cash: Decimal::from(100_000),
            execution_timing: ExecutionTiming::SignalBarClose,
            cash_buffer_ratio: Decimal::new(2, 3),
            min_trade_value: Decimal::ZERO,
            allow_fractional: false,
            lookback_window: 60,
            return_method: ReturnMethod::Simple,
            periods_per_year: 252.0,
            risk_free_rate: 0.0,
        }
    }
}

impl PortfolioBacktestConfig {
    /// Creates a daily configuration with `initial_cash` and the documented
    /// defaults.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PortfolioBacktestConfig::validate`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::PortfolioBacktestConfig;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let config = PortfolioBacktestConfig::new(Decimal::from(10_000))?;
    /// assert_eq!(config.lookback_window, 60);
    /// assert!(PortfolioBacktestConfig::new(Decimal::ZERO).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(initial_cash: Decimal) -> Result<Self, KurumiError> {
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
    /// Returns [`KurumiError::InvalidParameter`] when `initial_cash` is not
    /// strictly positive, the cash buffer is negative or at least one, the
    /// minimum trade value is negative, the lookback window is zero, or the
    /// annualization values are not finite with a positive period count.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::PortfolioBacktestConfig;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let mut config = PortfolioBacktestConfig::new(Decimal::from(10_000))?;
    /// config.lookback_window = 0;
    /// assert!(config.validate().is_err());
    /// config.lookback_window = 20;
    /// config.periods_per_year = 0.0;
    /// assert!(config.validate().is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn validate(&self) -> Result<(), KurumiError> {
        if self.initial_cash <= Decimal::ZERO {
            return Err(KurumiError::InvalidParameter {
                parameter: "initial_cash",
                reason: "must be strictly positive".to_owned(),
            });
        }
        if self.cash_buffer_ratio < Decimal::ZERO || self.cash_buffer_ratio >= Decimal::ONE {
            return Err(KurumiError::InvalidParameter {
                parameter: "cash_buffer_ratio",
                reason: "must be in [0, 1)".to_owned(),
            });
        }
        if self.min_trade_value < Decimal::ZERO {
            return Err(KurumiError::InvalidParameter {
                parameter: "min_trade_value",
                reason: "must be non-negative".to_owned(),
            });
        }
        if self.lookback_window == 0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "lookback_window",
                reason: "must be at least one".to_owned(),
            });
        }
        if !self.periods_per_year.is_finite() || self.periods_per_year <= 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "periods_per_year",
                reason: "must be finite and strictly positive".to_owned(),
            });
        }
        if !self.risk_free_rate.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "risk_free_rate",
                reason: "must be finite".to_owned(),
            });
        }
        Ok(())
    }
}

/// A stateful multi-asset backtest executor.
///
/// The engine owns the fee and slippage models and the immutable configuration;
/// each [`PortfolioBacktestEngine::run`] creates a fresh account, so an engine
/// can be reused for several feeds.
///
/// # Examples
///
/// ```no_run
/// use kurumi_backtest::portfolio::{
///     MultiAssetFeed, PortfolioBacktestConfig, PortfolioBacktestEngine,
///     StaticWeightAllocator,
///     CalendarTrigger,
/// };
/// use kurumi_backtest::{NoFee, NoSlippage};
/// use rust_decimal::Decimal;
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// # let feed = MultiAssetFeed::new(std::collections::BTreeMap::new())?;
/// # let mut allocator = StaticWeightAllocator::new(std::collections::BTreeMap::from([
/// #     ("AAA".to_owned(), 1.0),
/// # ]))?;
/// let config = PortfolioBacktestConfig::new(Decimal::from(10_000))?;
/// let mut engine = PortfolioBacktestEngine::new(
///     config,
///     Box::new(NoFee),
///     Box::new(NoSlippage),
/// )?;
/// let report = engine.run(feed, &CalendarTrigger::Monthly, &mut allocator)?;
/// # let _ = report;
/// # Ok(())
/// # }
/// ```
pub struct PortfolioBacktestEngine {
    config: PortfolioBacktestConfig,
    fee_model: Box<dyn FeeModel>,
    slippage_model: Box<dyn SlippageModel>,
}

impl std::fmt::Debug for PortfolioBacktestEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PortfolioBacktestEngine")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl PortfolioBacktestEngine {
    /// Creates an engine from a validated configuration and cost models.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PortfolioBacktestConfig::validate`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::{PortfolioBacktestConfig, PortfolioBacktestEngine};
    /// use kurumi_backtest::{NoFee, NoSlippage};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let config = PortfolioBacktestConfig::new(Decimal::from(10_000))?;
    /// let engine = PortfolioBacktestEngine::new(config, Box::new(NoFee), Box::new(NoSlippage))?;
    /// assert_eq!(engine.config().initial_cash, Decimal::from(10_000));
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        config: PortfolioBacktestConfig,
        fee_model: Box<dyn FeeModel>,
        slippage_model: Box<dyn SlippageModel>,
    ) -> Result<Self, KurumiError> {
        config.validate()?;
        Ok(Self {
            config,
            fee_model,
            slippage_model,
        })
    }

    /// Returns the configuration.
    #[must_use]
    #[inline]
    pub fn config(&self) -> &PortfolioBacktestConfig {
        &self.config
    }

    /// Runs the backtest over `feed` with no warmup prefix.
    ///
    /// The method is deterministic: every price, position, and weight container
    /// is a [`BTreeMap`], so repeated runs over the same inputs produce identical
    /// reports. Internally the feed is moved into an [`AlignedMarketData`] and
    /// evaluated by [`PortfolioBacktestEngine::run_on_slice`] with
    /// `warmup_bars = 0`, so both entry points share one execution path.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InsufficientData`] for an empty feed or a feed too
    /// short to compute performance metrics, [`KurumiError::InvalidParameter`]
    /// for malformed quotes or allocations, and propagates account, fee,
    /// slippage, optimizer, and risk errors.
    ///
    /// # Examples
    ///
    /// See the [module documentation](self) for a complete runnable example.
    pub fn run(
        &mut self,
        feed: MultiAssetFeed,
        trigger: &dyn RebalanceTrigger,
        allocator: &mut dyn TargetWeightAllocator,
    ) -> Result<PortfolioBacktestReport, KurumiError> {
        let data = AlignedMarketData::from_feed(feed);
        self.run_on_slice(data.slice(), 0, trigger, allocator)
    }

    /// Runs the backtest over a borrowed window of aligned market data.
    ///
    /// `data` is a zero-copy [`MarketSlice`], so several engines can evaluate
    /// different strategies or parameter sets against the same immutable
    /// timeline without cloning the underlying snapshots.
    ///
    /// # Warmup
    ///
    /// The first `warmup_bars` observations are a warmup prefix. For them the
    /// engine only marks the account to market and appends the bars to the
    /// rolling price history: no trigger is consulted, no order is generated,
    /// and neither [`PortfolioBacktestReport::equity_curve`] nor
    /// [`PortfolioBacktestReport::weight_history`] receives an entry. Evaluation
    /// starts at index `warmup_bars`, whose initial allocation can use the whole
    /// warmup history, and every reported curve therefore has length
    /// `data.len() - warmup_bars`. Trigger bar counters
    /// ([`TriggerContext::bars_advanced`] and
    /// [`TriggerContext::last_rebalance_bar`]) are relative to that first
    /// evaluated bar, so the evaluated segment behaves like a standalone
    /// backtest with a pre-populated history.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InsufficientData`] for an empty slice or fewer
    /// than three evaluated observations, [`KurumiError::InvalidParameter`] when
    /// `warmup_bars` is not smaller than the slice length or a quote or
    /// allocation is malformed, and propagates account, fee, slippage,
    /// optimizer, and risk errors.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::{
    ///     AlignedMarketData, CalendarTrigger, MarketSnapshot, MultiAssetFeed,
    ///     PortfolioBacktestConfig, PortfolioBacktestEngine, StaticWeightAllocator,
    /// };
    /// use kurumi_backtest::{NoFee, NoSlippage};
    /// use kurumi_ta::OhlcvBar;
    /// use rust_decimal::Decimal;
    /// use std::collections::BTreeMap;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let day = 86_400_000_i64;
    /// let mut series = BTreeMap::new();
    /// let bars = (0..8)
    ///     .map(|index| {
    ///         OhlcvBar::new(
    ///             index * day,
    ///             100.0 + index as f64,
    ///             101.0 + index as f64,
    ///             99.0 + index as f64,
    ///             100.0 + index as f64,
    ///             1_000.0,
    ///             100_000.0,
    ///         )
    ///     })
    ///     .collect::<Result<Vec<_>, _>>()?;
    /// series.insert("AAA".to_owned(), bars);
    /// let data = AlignedMarketData::from_feed(MultiAssetFeed::new(series)?);
    ///
    /// let config = PortfolioBacktestConfig::new(Decimal::from(10_000))?;
    /// let mut engine = PortfolioBacktestEngine::new(config, Box::new(NoFee), Box::new(NoSlippage))?;
    /// let mut allocator = StaticWeightAllocator::new(BTreeMap::from([("AAA".to_owned(), 1.0)]))?;
    /// let report = engine.run_on_slice(data.slice(), 3, &CalendarTrigger::Monthly, &mut allocator)?;
    /// assert_eq!(report.equity_curve.len(), 5);
    /// # Ok(())
    /// # }
    /// ```
    pub fn run_on_slice(
        &mut self,
        data: MarketSlice<'_>,
        warmup_bars: usize,
        trigger: &dyn RebalanceTrigger,
        allocator: &mut dyn TargetWeightAllocator,
    ) -> Result<PortfolioBacktestReport, KurumiError> {
        if data.is_empty() {
            return Err(KurumiError::InsufficientData {
                required: 1,
                available: 0,
            });
        }
        if warmup_bars >= data.len() {
            return Err(KurumiError::InvalidParameter {
                parameter: "warmup_bars",
                reason: format!(
                    "must be smaller than the {} observations in the market slice",
                    data.len()
                ),
            });
        }
        self.config.validate()?;

        let evaluated_bars = data.len() - warmup_bars;
        let mut account = MultiAssetAccount::new(self.config.initial_cash)?;
        let mut price_history: BTreeMap<String, Vec<(DateTime<Utc>, f64)>> = BTreeMap::new();
        let mut active_symbols: BTreeSet<String> = BTreeSet::new();
        let mut equity_curve = Vec::with_capacity(evaluated_bars);
        let mut weight_history = Vec::with_capacity(evaluated_bars);
        let mut rebalance_events: Vec<RebalanceEvent> = Vec::new();
        let mut pending: Option<PendingRebalance> = None;
        let mut last_target_weights = BTreeMap::new();
        let mut last_rebalance_timestamp: Option<DateTime<Utc>> = None;
        let mut last_rebalance_bar: Option<usize> = None;
        let mut total_turnover = 0.0;
        let mut min_cash = self.config.initial_cash;

        for (index, snapshot) in data.iter().enumerate() {
            if index < warmup_bars {
                account.update_market_prices(snapshot)?;
                for (symbol, bar) in snapshot.bars() {
                    active_symbols.insert(symbol.clone());
                    price_history
                        .entry(symbol.clone())
                        .or_default()
                        .push((snapshot.timestamp(), bar.close()));
                }
                continue;
            }

            let evaluation_index = index - warmup_bars;
            let mut executed_pending: Option<(PendingRebalance, ExecutionSummary)> = None;
            if let Some(pending_rebalance) = pending.take() {
                let summary = execute_orders(
                    &mut account,
                    snapshot,
                    &pending_rebalance.orders,
                    PriceSource::Open,
                    self.fee_model.as_ref(),
                    self.slippage_model.as_ref(),
                    self.config.allow_fractional,
                )?;
                executed_pending = Some((pending_rebalance, summary));
            }

            account.update_market_prices(snapshot)?;
            for (symbol, bar) in snapshot.bars() {
                active_symbols.insert(symbol.clone());
                price_history
                    .entry(symbol.clone())
                    .or_default()
                    .push((snapshot.timestamp(), bar.close()));
            }
            let universe: Vec<String> = active_symbols.iter().cloned().collect();

            if let Some((pending_rebalance, summary)) = executed_pending {
                let after_weights = account.current_weights()?;
                let event =
                    pending_rebalance.into_event(snapshot.timestamp(), summary, after_weights);
                total_turnover += event.turnover;
                rebalance_events.push(event);
            }

            let current_weights = account.current_weights()?;
            let context = TriggerContext {
                current_timestamp: snapshot.timestamp(),
                last_rebalance_timestamp,
                bars_advanced: evaluation_index,
                last_rebalance_bar,
                current_weights,
                target_weights: last_target_weights.clone(),
            };

            if trigger.should_rebalance(&context) {
                let cause = if last_rebalance_timestamp.is_none() {
                    RebalanceCause::Initial
                } else {
                    trigger.cause(&context)
                };
                let returns = build_return_matrix(
                    &price_history,
                    &universe,
                    self.config.lookback_window,
                    self.config.return_method,
                )?;
                let allocation_context =
                    AllocationContext::new(snapshot.timestamp(), universe.clone(), returns);
                let allocation = allocator.allocate(&allocation_context)?;
                let current_weights = account.current_weights()?;
                let plan = build_plan(
                    &allocation,
                    &universe,
                    &current_weights,
                    &account,
                    snapshot,
                    self.config.cash_buffer_ratio,
                )?;
                let orders = build_orders(&plan, &account, snapshot, &self.config)?;

                last_target_weights = plan.target_weights.clone();
                last_rebalance_timestamp = Some(snapshot.timestamp());
                last_rebalance_bar = Some(evaluation_index);

                if orders.is_empty() {
                    let summary = ExecutionSummary::default();
                    let after_weights = account.current_weights()?;
                    let pending_rebalance = PendingRebalance {
                        cause,
                        before_weights: current_weights,
                        target_weights: plan.target_weights,
                        turnover: plan.turnover,
                        frozen_symbols: plan.frozen_symbols,
                        orders,
                    };
                    let event =
                        pending_rebalance.into_event(snapshot.timestamp(), summary, after_weights);
                    total_turnover += event.turnover;
                    rebalance_events.push(event);
                } else {
                    let pending_rebalance = PendingRebalance {
                        cause,
                        before_weights: current_weights,
                        target_weights: plan.target_weights,
                        turnover: plan.turnover,
                        frozen_symbols: plan.frozen_symbols,
                        orders,
                    };
                    match self.config.execution_timing {
                        ExecutionTiming::SignalBarClose => {
                            let summary = execute_orders(
                                &mut account,
                                snapshot,
                                &pending_rebalance.orders,
                                PriceSource::Close,
                                self.fee_model.as_ref(),
                                self.slippage_model.as_ref(),
                                self.config.allow_fractional,
                            )?;
                            account.update_market_prices(snapshot)?;
                            let after_weights = account.current_weights()?;
                            let event = pending_rebalance.into_event(
                                snapshot.timestamp(),
                                summary,
                                after_weights,
                            );
                            total_turnover += event.turnover;
                            rebalance_events.push(event);
                        }
                        ExecutionTiming::NextBarOpen => pending = Some(pending_rebalance),
                    }
                }
            }

            account.update_market_prices(snapshot)?;
            min_cash = min_cash.min(account.cash());
            let equity = account.equity()?;
            equity_curve.push((snapshot.timestamp(), equity));
            weight_history.push((snapshot.timestamp(), account.current_weights()?));
        }

        let final_equity = equity_curve.last().map(|(_, equity)| *equity).ok_or(
            KurumiError::InsufficientData {
                required: 1,
                available: 0,
            },
        )?;
        PortfolioBacktestReport::from_backtest(
            self.config.initial_cash,
            final_equity,
            equity_curve,
            weight_history,
            rebalance_events,
            account.total_fees(),
            account.total_slippage(),
            min_cash,
            total_turnover,
            self.config.periods_per_year,
            self.config.risk_free_rate,
        )
    }
}

fn build_return_matrix(
    history: &BTreeMap<String, Vec<(DateTime<Utc>, f64)>>,
    universe: &[String],
    lookback_window: usize,
    method: ReturnMethod,
) -> Result<Option<ReturnMatrix>, KurumiError> {
    let mut series = Vec::new();
    for symbol in universe {
        let Some(observations) = history.get(symbol) else {
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
    let matrix = match ReturnMatrix::from_price_series(&series, AlignmentPolicy::InnerJoin, method)
    {
        Ok(matrix) => matrix,
        Err(KurumiError::InsufficientData { .. }) => return Ok(None),
        Err(error) => return Err(error),
    };
    let (rows, columns) = matrix.dimensions();
    if rows <= lookback_window {
        return Ok(Some(matrix));
    }
    let start = rows - lookback_window;
    let timestamps = matrix
        .timestamps()
        .get(start..)
        .ok_or(KurumiError::InsufficientData {
            required: lookback_window,
            available: rows,
        })?
        .to_vec();
    let data = matrix
        .data()
        .view((start, 0), (lookback_window, columns))
        .into_owned();
    Ok(Some(ReturnMatrix::new(
        matrix.symbols().to_vec(),
        timestamps,
        data,
    )?))
}

/// The normalized target and its audit metadata, before order generation.
struct Plan {
    target_weights: BTreeMap<String, f64>,
    frozen_symbols: Vec<String>,
    turnover: f64,
}

fn build_plan(
    allocation: &PortfolioWeights,
    universe: &[String],
    current_weights: &BTreeMap<String, f64>,
    account: &MultiAssetAccount,
    snapshot: &MarketSnapshot,
    cash_buffer_ratio: Decimal,
) -> Result<Plan, KurumiError> {
    let mut raw = BTreeMap::new();
    for (symbol, weight) in allocation.symbols().iter().zip(allocation.weights().iter()) {
        if !universe.iter().any(|candidate| candidate == symbol) {
            return Err(KurumiError::InvalidParameter {
                parameter: "allocation",
                reason: format!("allocator returned unknown symbol `{symbol}`"),
            });
        }
        if !weight.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "allocation",
                reason: format!("weight of `{symbol}` is not finite"),
            });
        }
        if *weight < -1e-9 {
            return Err(KurumiError::InvalidParameter {
                parameter: "allocation",
                reason: format!("weight of `{symbol}` is negative; short targets are unsupported"),
            });
        }
        raw.insert(symbol.clone(), weight.max(0.0));
    }

    let mut frozen_symbols = Vec::new();
    for symbol in universe {
        let allocated = raw.get(symbol).copied().unwrap_or(0.0) > 0.0;
        let held = account
            .position(symbol)
            .is_some_and(|position| !position.is_flat());
        if snapshot.bar(symbol).is_none() && (allocated || held) {
            frozen_symbols.push(symbol.clone());
        }
    }
    let frozen: BTreeSet<&str> = frozen_symbols.iter().map(String::as_str).collect();

    let frozen_weight = frozen_symbols
        .iter()
        .map(|symbol| current_weights.get(symbol).copied().unwrap_or(0.0))
        .sum::<f64>();
    let buffer = f64::try_from(cash_buffer_ratio).map_err(|_| KurumiError::InvalidParameter {
        parameter: "cash_buffer_ratio",
        reason: "cannot be represented as f64".to_owned(),
    })?;
    let available = (1.0 - buffer - frozen_weight).max(0.0);
    let tradable_sum = raw
        .iter()
        .filter(|(symbol, weight)| !frozen.contains(symbol.as_str()) && **weight > 0.0)
        .map(|(_, weight)| *weight)
        .sum::<f64>();
    let scale = if tradable_sum > 0.0 {
        if frozen_symbols.is_empty() {
            // Preserve an allocator's intentional cash residual; only cap the
            // target when it would exceed the investable fraction.
            (available / tradable_sum).min(1.0)
        } else {
            // A frozen position removes part of the universe, so the remaining
            // target weights are redistributed over the available equity.
            available / tradable_sum
        }
    } else {
        0.0
    };

    let mut target_weights = BTreeMap::new();
    for symbol in universe {
        let weight = if frozen.contains(symbol.as_str()) {
            current_weights.get(symbol).copied().unwrap_or(0.0)
        } else {
            raw.get(symbol).copied().unwrap_or(0.0) * scale
        };
        if !weight.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "target_weights",
                reason: format!("normalized weight of `{symbol}` is not finite"),
            });
        }
        target_weights.insert(symbol.clone(), weight);
    }

    let mut turnover = 0.0;
    let mut symbols = BTreeSet::new();
    symbols.extend(current_weights.keys().cloned());
    symbols.extend(target_weights.keys().cloned());
    for symbol in symbols {
        let before = current_weights.get(&symbol).copied().unwrap_or(0.0);
        let after = target_weights.get(&symbol).copied().unwrap_or(0.0);
        turnover += (after - before).abs();
    }
    let turnover = 0.5 * turnover;
    if !turnover.is_finite() {
        return Err(KurumiError::NumericalOverflow {
            operation: "portfolio_turnover",
        });
    }

    Ok(Plan {
        target_weights,
        frozen_symbols,
        turnover,
    })
}

#[derive(Debug, Clone)]
struct OrderIntent {
    symbol: String,
    side: Side,
    quantity: Decimal,
}

fn build_orders(
    plan: &Plan,
    account: &MultiAssetAccount,
    snapshot: &MarketSnapshot,
    config: &PortfolioBacktestConfig,
) -> Result<Vec<OrderIntent>, KurumiError> {
    let frozen: BTreeSet<&str> = plan.frozen_symbols.iter().map(String::as_str).collect();
    let mut symbols = BTreeSet::new();
    for (symbol, position) in account.positions() {
        if !position.is_flat() {
            symbols.insert(symbol.clone());
        }
    }
    for (symbol, weight) in &plan.target_weights {
        if *weight > 0.0 {
            symbols.insert(symbol.clone());
        }
    }
    if symbols.is_empty() {
        return Ok(Vec::new());
    }

    let mut holdings = Vec::with_capacity(symbols.len());
    let mut target_symbols = Vec::with_capacity(symbols.len());
    let mut target_values = Vec::with_capacity(symbols.len());
    for symbol in &symbols {
        let quantity = account
            .position(symbol)
            .map_or(Decimal::ZERO, |position| position.quantity());
        let price = match snapshot.bar(symbol) {
            Some(bar) => decimal_from_f64(bar.close(), symbol)?,
            None => match account.last_known_price(symbol) {
                Some(price) => price,
                None => account
                    .position(symbol)
                    .map(|position| position.avg_cost())
                    .filter(|price| *price > Decimal::ZERO)
                    .ok_or_else(|| KurumiError::InvalidParameter {
                        parameter: "holdings",
                        reason: format!("no price is known for `{symbol}`"),
                    })?,
            },
        };
        holdings.push(Holding::new(symbol.clone(), quantity, price)?);
        target_symbols.push(symbol.clone());
        target_values.push(plan.target_weights.get(symbol).copied().unwrap_or(0.0));
    }
    let target = PortfolioWeights::new(target_symbols, DVector::from_vec(target_values))?;
    let actions = generate_rebalance_actions(
        account.equity()?,
        &holdings,
        &target,
        config.min_trade_value,
        config.allow_fractional,
    )?;
    Ok(actions
        .into_iter()
        .filter(|action| !frozen.contains(action.symbol.as_str()))
        .filter_map(|action| {
            let side = match action.side {
                RebalanceSide::Buy => Side::Buy,
                RebalanceSide::Sell => Side::Sell,
            };
            if action.quantity <= Decimal::ZERO {
                return None;
            }
            Some(OrderIntent {
                symbol: action.symbol,
                side,
                quantity: action.quantity,
            })
        })
        .collect())
}

#[derive(Debug, Clone, Copy)]
enum PriceSource {
    Open,
    Close,
}

fn reference_price(
    snapshot: &MarketSnapshot,
    symbol: &str,
    source: PriceSource,
) -> Result<Option<Price>, KurumiError> {
    let Some(bar) = snapshot.bar(symbol) else {
        return Ok(None);
    };
    let value = match source {
        PriceSource::Open => bar.open(),
        PriceSource::Close => bar.close(),
    };
    Ok(Some(Price::new(decimal_from_f64(value, symbol)?)?))
}

#[derive(Debug, Clone, Copy, Default)]
struct ExecutionSummary {
    fees: Decimal,
    slippage: Decimal,
    sell_count: usize,
    buy_count: usize,
}

#[derive(Debug, Clone)]
struct PendingRebalance {
    cause: RebalanceCause,
    before_weights: BTreeMap<String, f64>,
    target_weights: BTreeMap<String, f64>,
    turnover: f64,
    frozen_symbols: Vec<String>,
    orders: Vec<OrderIntent>,
}

impl PendingRebalance {
    fn into_event(
        self,
        timestamp: DateTime<Utc>,
        summary: ExecutionSummary,
        after_weights: BTreeMap<String, f64>,
    ) -> RebalanceEvent {
        RebalanceEvent {
            timestamp,
            cause: self.cause,
            before_weights: self.before_weights,
            target_weights: self.target_weights,
            after_weights,
            turnover: self.turnover,
            fees: summary.fees,
            slippage: summary.slippage,
            sell_count: summary.sell_count,
            buy_count: summary.buy_count,
            frozen_symbols: self.frozen_symbols,
        }
    }
}

#[derive(Debug, Clone)]
struct Quote {
    symbol: String,
    quantity: Decimal,
    fill_price: Price,
    reference_price: Price,
    fee: Decimal,
}

impl Quote {
    fn required_cash(&self) -> Result<Decimal, KurumiError> {
        self.fill_price
            .value()
            .checked_mul(self.quantity)
            .and_then(|notional| notional.checked_add(self.fee))
            .ok_or(KurumiError::NumericalOverflow {
                operation: "PortfolioBacktestEngine::required_cash",
            })
    }

    fn slippage(&self) -> Result<Decimal, KurumiError> {
        self.fill_price
            .value()
            .checked_sub(self.reference_price.value())
            .map(|difference| difference.abs())
            .and_then(|difference| difference.checked_mul(self.quantity))
            .ok_or(KurumiError::NumericalOverflow {
                operation: "PortfolioBacktestEngine::slippage",
            })
    }
}

fn quote_order(
    snapshot: &MarketSnapshot,
    symbol: &str,
    quantity: Decimal,
    source: PriceSource,
    fee_model: &dyn FeeModel,
    slippage_model: &dyn SlippageModel,
) -> Result<Option<Quote>, KurumiError> {
    if quantity <= Decimal::ZERO {
        return Ok(None);
    }
    let Some(reference) = reference_price(snapshot, symbol, source)? else {
        return Ok(None);
    };
    let fill_price = slippage_model.fill_price(Side::Buy, reference)?;
    let units = Quantity::new(quantity)?;
    let fee = fee_model.fee(fill_price, units, Side::Buy, Liquidity::Taker)?;
    Ok(Some(Quote {
        symbol: symbol.to_owned(),
        quantity,
        fill_price,
        reference_price: reference,
        fee,
    }))
}

fn floor_quantity(quantity: Decimal, allow_fractional: bool) -> Decimal {
    if allow_fractional {
        quantity.round_dp_with_strategy(FRACTIONAL_QUANTITY_SCALE, RoundingStrategy::ToZero)
    } else {
        quantity.floor()
    }
}

fn execute_orders(
    account: &mut MultiAssetAccount,
    snapshot: &MarketSnapshot,
    orders: &[OrderIntent],
    source: PriceSource,
    fee_model: &dyn FeeModel,
    slippage_model: &dyn SlippageModel,
    allow_fractional: bool,
) -> Result<ExecutionSummary, KurumiError> {
    let mut summary = ExecutionSummary::default();
    let mut buys = Vec::new();
    for order in orders {
        match order.side {
            Side::Sell => {
                let Some(reference) = reference_price(snapshot, &order.symbol, source)? else {
                    continue;
                };
                let fill_price = slippage_model.fill_price(Side::Sell, reference)?;
                let units = Quantity::new(order.quantity)?;
                let fee = fee_model.fee(fill_price, units, Side::Sell, Liquidity::Taker)?;
                let slippage = fill_price
                    .value()
                    .checked_sub(reference.value())
                    .map(|difference| difference.abs())
                    .and_then(|difference| difference.checked_mul(order.quantity))
                    .ok_or(KurumiError::NumericalOverflow {
                        operation: "PortfolioBacktestEngine::sell_slippage",
                    })?;
                let fill = MultiAssetFill::new(
                    order.symbol.clone(),
                    Side::Sell,
                    fill_price,
                    units,
                    fee,
                    slippage,
                )?;
                account.apply_fill(&fill)?;
                summary.fees =
                    summary
                        .fees
                        .checked_add(fee)
                        .ok_or(KurumiError::NumericalOverflow {
                            operation: "PortfolioBacktestEngine::sell_fees",
                        })?;
                summary.slippage = summary.slippage.checked_add(slippage).ok_or(
                    KurumiError::NumericalOverflow {
                        operation: "PortfolioBacktestEngine::sell_slippage",
                    },
                )?;
                summary.sell_count += 1;
            }
            Side::Buy => buys.push(OrderIntent {
                symbol: order.symbol.clone(),
                side: Side::Buy,
                quantity: order.quantity,
            }),
        }
    }

    let mut quotes = Vec::with_capacity(buys.len());
    for order in &buys {
        if let Some(quote) = quote_order(
            snapshot,
            &order.symbol,
            order.quantity,
            source,
            fee_model,
            slippage_model,
        )? {
            quotes.push(quote);
        }
    }
    let mut total = quotes.iter().try_fold(Decimal::ZERO, |total, quote| {
        quote.required_cash().and_then(|required| {
            total
                .checked_add(required)
                .ok_or(KurumiError::NumericalOverflow {
                    operation: "PortfolioBacktestEngine::buy_basket",
                })
        })
    })?;
    let mut iteration = 0;
    while total > account.cash() && iteration < 64 {
        if account.cash() <= Decimal::ZERO || total <= Decimal::ZERO {
            quotes.clear();
            break;
        }
        let factor = account
            .cash()
            .checked_div(total)
            .ok_or(KurumiError::NumericalOverflow {
                operation: "PortfolioBacktestEngine::buy_scaling",
            })?;
        let mut scaled = Vec::with_capacity(quotes.len());
        for quote in &quotes {
            let quantity = floor_quantity(quote.quantity * factor, allow_fractional);
            if let Some(quote) = quote_order(
                snapshot,
                &quote.symbol,
                quantity,
                source,
                fee_model,
                slippage_model,
            )? {
                scaled.push(quote);
            }
        }
        quotes = scaled;
        total = quotes.iter().try_fold(Decimal::ZERO, |total, quote| {
            quote.required_cash().and_then(|required| {
                total
                    .checked_add(required)
                    .ok_or(KurumiError::NumericalOverflow {
                        operation: "PortfolioBacktestEngine::buy_basket",
                    })
            })
        })?;
        iteration += 1;
    }

    for mut quote in quotes {
        let mut attempt = 0;
        loop {
            let required = quote.required_cash()?;
            if required <= account.cash() {
                let units = Quantity::new(quote.quantity)?;
                let slippage = quote.slippage()?;
                let fill = MultiAssetFill::new(
                    quote.symbol.clone(),
                    Side::Buy,
                    quote.fill_price,
                    units,
                    quote.fee,
                    slippage,
                )?;
                account.apply_fill(&fill)?;
                summary.fees =
                    summary
                        .fees
                        .checked_add(quote.fee)
                        .ok_or(KurumiError::NumericalOverflow {
                            operation: "PortfolioBacktestEngine::buy_fees",
                        })?;
                summary.slippage = summary.slippage.checked_add(slippage).ok_or(
                    KurumiError::NumericalOverflow {
                        operation: "PortfolioBacktestEngine::buy_slippage",
                    },
                )?;
                summary.buy_count += 1;
                break;
            }
            if attempt >= 64 || account.cash() <= Decimal::ZERO {
                break;
            }
            let factor =
                account
                    .cash()
                    .checked_div(required)
                    .ok_or(KurumiError::NumericalOverflow {
                        operation: "PortfolioBacktestEngine::buy_scaling",
                    })?;
            let quantity = floor_quantity(quote.quantity * factor, allow_fractional);
            match quote_order(
                snapshot,
                &quote.symbol,
                quantity,
                source,
                fee_model,
                slippage_model,
            )? {
                Some(scaled) => quote = scaled,
                None => break,
            }
            attempt += 1;
        }
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fee::{FixedRateFee, NoFee};
    use crate::portfolio::allocator::{RollingOptimizerAllocator, StaticWeightAllocator};
    use crate::portfolio::trigger::CalendarTrigger;
    use crate::slippage::NoSlippage;
    use kurumi_math::SampleCovariance;
    use kurumi_portfolio::InverseVolatility;
    use kurumi_ta::OhlcvBar;
    use std::cell::RefCell;
    use std::rc::Rc;

    const DAY: i64 = 86_400_000;

    fn bar(day: i64, close: f64) -> OhlcvBar {
        OhlcvBar::new(
            day * DAY,
            close,
            close + 1.0,
            close - 1.0,
            close,
            1_000.0,
            close * 1_000.0,
        )
        .expect("valid bar")
    }

    fn feed(values: &[(&str, &[f64])]) -> MultiAssetFeed {
        let mut series = BTreeMap::new();
        for (symbol, closes) in values {
            series.insert(
                (*symbol).to_owned(),
                closes
                    .iter()
                    .enumerate()
                    .map(|(day, close)| bar(day as i64, *close))
                    .collect(),
            );
        }
        MultiAssetFeed::new(series).expect("valid feed")
    }

    fn engine(fee: Box<dyn FeeModel>) -> PortfolioBacktestEngine {
        PortfolioBacktestEngine::new(
            PortfolioBacktestConfig::new(Decimal::from(100_000)).expect("valid config"),
            fee,
            Box::new(NoSlippage),
        )
        .expect("valid engine")
    }

    fn allocator() -> StaticWeightAllocator {
        StaticWeightAllocator::new(BTreeMap::from([
            ("AAA".to_owned(), 0.6),
            ("BBB".to_owned(), 0.4),
        ]))
        .expect("valid allocator")
    }

    #[test]
    fn signal_close_runs_a_full_rebalance_with_audit_trail() {
        let feed = feed(&[
            ("AAA", &[100.0, 101.0, 102.0, 103.0]),
            ("BBB", &[50.0, 50.0, 51.0, 52.0]),
        ]);
        let mut engine = engine(Box::new(NoFee));
        let mut allocator = allocator();
        let report = engine
            .run(feed, &CalendarTrigger::Monthly, &mut allocator)
            .expect("backtest");
        assert_eq!(report.rebalance_events.len(), 1);
        assert_eq!(report.rebalance_events[0].cause, RebalanceCause::Initial);
        assert_eq!(report.weight_history.len(), 4);
        assert!(report.final_equity > Decimal::ZERO);
    }

    #[test]
    fn next_bar_open_queues_and_fills_on_the_next_observation() {
        let feed = feed(&[
            ("AAA", &[100.0, 110.0, 120.0, 130.0]),
            ("BBB", &[50.0, 50.0, 50.0, 50.0]),
        ]);
        let mut config =
            PortfolioBacktestConfig::new(Decimal::from(100_000)).expect("valid config");
        config.execution_timing = ExecutionTiming::NextBarOpen;
        let mut engine =
            PortfolioBacktestEngine::new(config, Box::new(NoFee), Box::new(NoSlippage))
                .expect("valid engine");
        let mut allocator = allocator();
        let report = engine
            .run(feed, &CalendarTrigger::Monthly, &mut allocator)
            .expect("backtest");
        assert_eq!(report.rebalance_events.len(), 1);
        assert_eq!(report.rebalance_events[0].buy_count, 2);
        assert_eq!(report.rebalance_events[0].timestamp.timestamp_millis(), DAY);
    }

    #[test]
    fn missing_bars_freeze_positions_instead_of_selling_them() {
        let mut series = BTreeMap::new();
        series.insert(
            "AAA".to_owned(),
            vec![bar(0, 100.0), bar(1, 101.0), bar(2, 102.0)],
        );
        series.insert("BBB".to_owned(), vec![bar(0, 50.0)]);
        let feed = MultiAssetFeed::new(series).expect("valid feed");
        let trigger = CalendarTrigger::fixed_interval(1).expect("valid interval");
        let mut engine = engine(Box::new(NoFee));
        let mut allocator = allocator();
        let report = engine
            .run(feed, &trigger, &mut allocator)
            .expect("backtest");
        assert!(
            report
                .rebalance_events
                .iter()
                .any(|event| event.frozen_symbols.iter().any(|symbol| symbol == "BBB"))
        );
        assert!(report.min_cash >= Decimal::ZERO);
    }

    #[test]
    fn fee_scaling_keeps_cash_non_negative_when_the_basket_exceeds_cash() {
        let feed = feed(&[
            ("AAA", &[100.0, 100.0, 100.0, 100.0]),
            ("BBB", &[50.0, 50.0, 50.0, 50.0]),
        ]);
        let fee = FixedRateFee::new(Decimal::new(15, 3)).expect("valid fee");
        let mut engine = engine(Box::new(fee));
        let mut allocator = allocator();
        let report = engine
            .run(feed, &CalendarTrigger::Monthly, &mut allocator)
            .expect("backtest");
        assert!(report.min_cash >= Decimal::ZERO);
        assert!(report.total_fees > Decimal::ZERO);
    }

    #[test]
    fn fractional_and_whole_unit_precision_are_both_supported() {
        let feed = feed(&[
            ("AAA", &[100.0, 101.0, 102.0, 103.0]),
            ("BBB", &[50.0, 51.0, 52.0, 53.0]),
        ]);
        let mut config = PortfolioBacktestConfig::new(Decimal::from(10_000)).expect("valid config");
        config.allow_fractional = true;
        let mut engine =
            PortfolioBacktestEngine::new(config, Box::new(NoFee), Box::new(NoSlippage))
                .expect("valid engine");
        let mut allocator = allocator();
        let report = engine
            .run(feed, &CalendarTrigger::Monthly, &mut allocator)
            .expect("backtest");
        assert!(report.final_equity > Decimal::ZERO);
        assert!(report.min_cash >= Decimal::ZERO);
    }

    #[test]
    fn intentional_cash_residuals_are_not_scaled_up() {
        let feed = feed(&[
            ("AAA", &[100.0, 101.0, 102.0]),
            ("BBB", &[50.0, 50.5, 51.0]),
        ]);
        let mut engine = engine(Box::new(NoFee));
        let mut allocator = StaticWeightAllocator::new(BTreeMap::from([
            ("AAA".to_owned(), 0.3),
            ("BBB".to_owned(), 0.2),
        ]))
        .expect("valid allocator");
        let report = engine
            .run(feed, &CalendarTrigger::Monthly, &mut allocator)
            .expect("backtest");
        let event = report.rebalance_events.first().expect("initial event");
        let invested = event.target_weights.values().sum::<f64>();
        assert!(
            (invested - 0.5).abs() < 1e-9,
            "a 50% target must keep its intentional cash residual instead of scaling to fully invested"
        );
    }

    #[test]
    fn invalid_configurations_are_rejected() {
        assert!(PortfolioBacktestConfig::new(Decimal::ZERO).is_err());
        let mut config = PortfolioBacktestConfig::new(Decimal::from(1_000)).expect("valid config");
        config.cash_buffer_ratio = Decimal::ONE;
        assert!(config.validate().is_err());
        config.cash_buffer_ratio = Decimal::ZERO;
        config.lookback_window = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn warmup_prefix_populates_history_without_trading_then_optimizes_immediately() {
        const TOTAL_BARS: usize = 120;
        const WARMUP_BARS: usize = 60;

        let mut series = BTreeMap::new();
        series.insert(
            "AAA".to_owned(),
            (0..TOTAL_BARS)
                .map(|day| bar(day as i64, 100.0 + day as f64 * 0.5))
                .collect::<Vec<_>>(),
        );
        series.insert(
            "BBB".to_owned(),
            (0..TOTAL_BARS)
                .map(|day| bar(day as i64, if day % 2 == 0 { 50.0 } else { 60.0 }))
                .collect::<Vec<_>>(),
        );
        let data = AlignedMarketData::from_feed(MultiAssetFeed::new(series).expect("valid feed"));

        let mut config =
            PortfolioBacktestConfig::new(Decimal::from(100_000)).expect("valid config");
        config.lookback_window = WARMUP_BARS;
        let mut engine =
            PortfolioBacktestEngine::new(config, Box::new(NoFee), Box::new(NoSlippage))
                .expect("valid engine");

        let allocations: Rc<RefCell<Vec<(usize, kurumi_portfolio::PortfolioWeights)>>> =
            Rc::new(RefCell::new(Vec::new()));
        let mut allocator = RecordingAllocator {
            inner: RollingOptimizerAllocator::new(SampleCovariance, InverseVolatility::new())
                .with_min_lookback_periods(WARMUP_BARS)
                .expect("valid lookback"),
            allocations: Rc::clone(&allocations),
        };
        let trigger = CalendarTrigger::fixed_interval(30).expect("valid interval");
        let report = engine
            .run_on_slice(data.slice(), WARMUP_BARS, &trigger, &mut allocator)
            .expect("backtest");

        assert_eq!(report.equity_curve.len(), TOTAL_BARS - WARMUP_BARS);
        assert_eq!(report.weight_history.len(), TOTAL_BARS - WARMUP_BARS);
        assert_eq!(
            report
                .equity_curve
                .first()
                .map(|(timestamp, _)| timestamp.timestamp_millis()),
            Some(WARMUP_BARS as i64 * DAY),
            "the reported curve must start on the first evaluated bar"
        );
        assert!(
            report
                .rebalance_events
                .iter()
                .all(|event| event.timestamp.timestamp_millis() >= WARMUP_BARS as i64 * DAY),
            "no rebalance may happen during warmup"
        );
        assert_eq!(
            report.rebalance_events.first().map(|event| event.cause),
            Some(RebalanceCause::Initial)
        );

        let allocations = allocations.borrow();
        let (first_count, first_weights) = allocations.first().expect("initial allocation");
        assert_eq!(
            *first_count, WARMUP_BARS,
            "the initial allocation must consume the whole warmup return history"
        );
        let aaa = first_weights.weight("AAA").expect("AAA weight");
        assert!(
            (aaa - 0.5).abs() > 0.05,
            "the rolling optimizer must run instead of the equal-weight fallback, got {aaa}"
        );
    }

    #[test]
    fn run_on_slice_rejects_warmup_that_consumes_the_whole_slice() {
        let feed = feed(&[("AAA", &[100.0, 101.0, 102.0, 103.0])]);
        let data = AlignedMarketData::from_feed(feed);
        let mut engine = engine(Box::new(NoFee));
        let mut allocator = StaticWeightAllocator::new(BTreeMap::from([("AAA".to_owned(), 1.0)]))
            .expect("valid allocator");
        assert!(
            engine
                .run_on_slice(
                    data.slice(),
                    data.len(),
                    &CalendarTrigger::Monthly,
                    &mut allocator
                )
                .is_err()
        );
        let empty =
            AlignedMarketData::from_feed(MultiAssetFeed::new(BTreeMap::new()).expect("empty feed"));
        assert!(
            engine
                .run_on_slice(empty.slice(), 0, &CalendarTrigger::Monthly, &mut allocator)
                .is_err()
        );
    }

    struct RecordingAllocator {
        inner: RollingOptimizerAllocator<SampleCovariance, InverseVolatility>,
        allocations: Rc<RefCell<Vec<(usize, kurumi_portfolio::PortfolioWeights)>>>,
    }

    impl TargetWeightAllocator for RecordingAllocator {
        fn allocate(
            &mut self,
            ctx: &AllocationContext,
        ) -> Result<kurumi_portfolio::PortfolioWeights, KurumiError> {
            let weights = self.inner.allocate(ctx)?;
            self.allocations
                .borrow_mut()
                .push((ctx.return_count(), weights.clone()));
            Ok(weights)
        }
    }
}
