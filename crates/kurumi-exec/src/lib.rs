//! Sans-I/O execution kernel with an optional asynchronous live layer.
//!
//! `kurumi-exec` is the synchronous, deterministic execution layer above
//! `kurumi-backtest`:
//!
//! - [`order`] defines order identifiers, requests, commands, execution
//!   reports, and the [`OrderRecord`] finite-state machine with
//!   execution-id-based fill deduplication;
//! - [`ledger`] provides [`ExecutionLedger`], the `Decimal` cash and position
//!   book with buy-cash and sell-quantity reservations;
//! - [`strategy`] defines [`Strategy`], [`StrategyContext`], the market event
//!   enum, and [`PortfolioRebalanceStrategy`], which bridges the existing
//!   `kurumi-backtest` rebalance triggers and allocators;
//! - [`risk_gate`] provides the ordered pre-trade [`RiskGate`] and its six
//!   built-in rules, including the daily-loss kill switch;
//! - [`paper`] provides the [`PaperMatchingEngine`], which fills market, limit,
//!   stop, and stop-limit orders against quotes, trades, bars, or snapshots
//!   using the existing fee and slippage models;
//! - [`journal`] provides the append-only [`AuditJournal`], with in-memory and
//!   JSONL sinks;
//! - [`runners`] provides the synchronous [`BacktestRunner`]. With the
//!   additive `live` feature, [`runners::broker`], [`runners::broker_alpaca`],
//!   and [`runners::live`] add the async broker abstraction, `PaperBroker`,
//!   Alpaca REST adapter, and `LiveRunner`, re-exported under [`live`].
//!
//! # Invariants
//!
//! The strategy, risk, ledger, matching, and runner APIs are 100% synchronous.
//! They never read `Utc::now`, never generate random identifiers, and never
//! perform network I/O. Timestamps always arrive with reports or market events,
//! and client order identifiers come from
//! [`StrategyContext::next_order_id`]. The same strategy and matching engine can
//! therefore be replayed through any synchronous event source with identical
//! fills, fees, and final `Decimal` equity.
//!
//! The additive `live` feature is off by default. It adds the `kurumi-feed`
//! market-data bridge, Tokio/`reqwest` broker adapters, and `LiveRunner`, which
//! drives exactly the same synchronous strategy, risk, ledger, and paper
//! matching types as the backtest. The default `kurumi-exec` build contains no
//! runtime, network, or blocking I/O code.
//!
//! # Examples
//!
//! ```
//! use kurumi_exec::{
//!     ClientOrderId, ExecId, ExecutionReport, OrderRecord, OrderRequest, OrderSide, OrderStatus,
//!     OrderType, TimeInForce,
//! };
//! use chrono::{DateTime, Utc};
//! use rust_decimal::Decimal;
//!
//! # fn main() -> Result<(), kurumi_exec::ExecError> {
//! let ts = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
//! let mut record = OrderRecord::new(OrderRequest::new(
//!     ClientOrderId::new("o-1"),
//!     "AAA",
//!     OrderSide::Buy,
//!     OrderType::Market,
//!     Decimal::from(10),
//!     TimeInForce::Day,
//! ));
//! record.apply_report(&ExecutionReport::Accepted {
//!     client_order_id: ClientOrderId::new("o-1"),
//!     ts,
//! })?;
//! record.apply_report(&ExecutionReport::Fill {
//!     client_order_id: ClientOrderId::new("o-1"),
//!     exec_id: ExecId::new("e-1"),
//!     symbol: "AAA".to_owned(),
//!     side: OrderSide::Buy,
//!     fill_price: Decimal::from(100),
//!     fill_qty: Decimal::from(10),
//!     fee: Decimal::ZERO,
//!     slippage: Decimal::ZERO,
//!     ts,
//! })?;
//! assert_eq!(record.status(), OrderStatus::Filled);
//! # Ok(())
//! # }
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

pub mod error;
pub mod journal;
pub mod ledger;
pub mod order;
pub mod paper;
pub mod risk_gate;
pub mod runners;
pub mod strategy;
mod util;

pub use error::ExecError;
pub use journal::{AuditJournal, JournalRecord, JsonlFileJournal, MemoryJournal};
pub use ledger::{ExecutionLedger, LedgerPosition, LedgerPositionSeed};
pub use order::{
    BrokerOrderId, ClientOrderId, ExecId, ExecutionReport, OrderCommand, OrderRecord, OrderRequest,
    OrderSide, OrderStatus, OrderType, TimeInForce,
};
pub use paper::PaperMatchingEngine;
pub use risk_gate::{
    DuplicateOrderIdRule, MaxDailyLossRule, MaxOrderNotionalRule, MaxPositionWeightRule,
    OrderRateLimitRule, RiskContext, RiskGate, RiskRule, SymbolFilterRule,
};
pub use runners::{
    BacktestConfig, BacktestRunner, ExecBacktestReport, ExecEquityPoint, ExecFill, RiskRejection,
};
pub use strategy::{
    ExecMarketEvent, PortfolioRebalanceStrategy, RebalanceStrategyConfig, Strategy, StrategyContext,
};

/// Asynchronous live execution types, available with the `live` feature.
///
/// This namespace re-exports the broker abstraction, the local paper broker,
/// the Alpaca REST adapter, and the `LiveRunner` event loop. It is absent from
/// the default Sans-I/O build.
#[cfg(feature = "live")]
pub mod live {
    pub use crate::runners::broker::{
        BrokerAccountSnapshot, BrokerPosition, DEFAULT_REPORT_CHANNEL_CAPACITY, ExecutionBroker,
        PaperBroker,
    };
    pub use crate::runners::broker_alpaca::{
        ALPACA_LIVE_URL, ALPACA_PAPER_URL, AlpacaTradingBroker,
    };
    pub use crate::runners::live::{
        LiveConfig, LiveRunner, LiveSessionSummary, bar_event_to_exec, feed_event_to_exec,
    };
}

/// The `chrono` crate, re-exported so callers can build event timestamps without
/// adding a separate dependency.
pub use chrono;

/// The shared Kurumi error type.
pub use kurumi_core::KurumiError;
