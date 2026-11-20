//! Execution runners.
//!
//! [`backtest`] drives the strategy, risk gate, ledger, and paper matching
//! engine over an aligned market-data timeline.

pub mod backtest;
#[cfg(feature = "live")]
pub mod broker;
#[cfg(feature = "live")]
pub mod broker_alpaca;
#[cfg(feature = "live")]
pub mod live;

pub use backtest::{
    BacktestConfig, BacktestRunner, ExecBacktestReport, ExecEquityPoint, ExecFill, RiskRejection,
};
#[cfg(feature = "live")]
pub use broker::{
    BrokerAccountSnapshot, BrokerPosition, DEFAULT_REPORT_CHANNEL_CAPACITY, ExecutionBroker,
    PaperBroker,
};
#[cfg(feature = "live")]
pub use broker_alpaca::{ALPACA_LIVE_URL, ALPACA_PAPER_URL, AlpacaTradingBroker};
#[cfg(feature = "live")]
pub use live::{LiveConfig, LiveRunner, LiveSessionSummary, bar_event_to_exec, feed_event_to_exec};
