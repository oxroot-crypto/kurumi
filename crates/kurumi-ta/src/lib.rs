//! Time-series structures and technical indicators.
//!
//! The crate owns candle/OHLCV structures and exposes each indicator in two
//! forms:
//!
//! - an O(1) streaming form for live data, updated one observation at a time;
//! - a vectorized batch form for research, historical analysis, and backtests.
//!
//! Both forms share one definition of the formula and must agree within the
//! documented tolerance. This crate sits above `kurumi-core` and `kurumi-math`,
//! and below `kurumi-risk`, `kurumi-backtest`, and the `kurumi` facade.
//!
//! The fixed-capacity [`RingBuffer`], [`RollingWindow`], and [`RollingVariance`]
//! primitives live in `kurumi-core` and are re-exported here, so the indicators
//! and the other crates share one implementation of their `O(1)` storage.
//!
//! # Streaming contract
//!
//! Every indicator implements [`StreamingIndicator`]. A call to
//! [`StreamingIndicator::next`] consumes exactly one observation, performs a
//! constant amount of arithmetic, and never scans the lookback window or
//! reallocates the indicator's internal storage. Until the warm-up period is
//! complete the method returns `Ok(None)`; afterwards it returns the current
//! value. [`StreamingIndicator::compute_batch`] feeds a slice through the same
//! state machine and collects the per-observation results, which makes a
//! batch run bit-for-bit identical to a fresh tick-by-tick run.
//!
//! ```
//! use kurumi_ta::{Sma, StreamingIndicator};
//!
//! # fn main() -> Result<(), kurumi_ta::KurumiError> {
//! let mut sma = Sma::new(3)?;
//! assert_eq!(sma.next(1.0)?, None);
//! assert_eq!(sma.next(2.0)?, None);
//! assert_eq!(sma.next(3.0)?, Some(2.0));
//! assert_eq!(sma.next(4.0)?, Some(3.0));
//! # Ok(())
//! # }
//! ```
//!
//! # Numerical conventions
//!
//! Indicators consume `f64` values and return `f64` values. Inputs are
//! validated at the public boundary: `NaN` and infinite values produce
//! [`KurumiError::InvalidParameter`], never a silently poisoned state. OHLCV
//! bars are validated by [`OhlcvBar::new`] before any indicator sees them.
//!
//! Technical-indicator results are descriptive statistics, not prices, so they
//! are returned as `f64` rather than `rust_decimal::Decimal`. Currency amounts
//! and order prices remain exact decimal values in `kurumi-core`.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

pub mod bar;
pub mod indicators;
pub mod traits;

mod util;

pub use bar::OhlcvBar;
pub use indicators::{
    Atr, BollingerBands, BollingerBandsOutput, Ema, Macd, MacdOutput, Roc, Rsi, Sma, Stochastic,
    StochasticOutput, Wma,
};
pub use kurumi_core::ring_buffer::{RingBuffer, RollingVariance, RollingWindow};
pub use traits::StreamingIndicator;

pub use kurumi_core::KurumiError;
