//! Execution runners.
//!
//! [`backtest`] drives the strategy, risk gate, ledger, and paper matching
//! engine over an aligned market-data timeline.

pub mod backtest;

pub use backtest::{
    BacktestConfig, BacktestRunner, ExecBacktestReport, ExecEquityPoint, ExecFill, RiskRejection,
};
