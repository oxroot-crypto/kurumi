//! Streaming technical indicators.
//!
//! Each indicator is a small state machine implementing
//! [`StreamingIndicator`](crate::StreamingIndicator). The modules are grouped by
//! category:
//!
//! - trend: [`Sma`], [`Ema`], [`Wma`], [`Macd`];
//! - momentum and oscillation: [`Rsi`], [`Stochastic`], [`Roc`];
//! - volatility and channels: [`Atr`], [`BollingerBands`].
//!
//! Indicators never scan their lookback window inside
//! [`StreamingIndicator::next`](crate::StreamingIndicator::next): the window
//! aggregates are maintained incrementally by the types in
//! [`ring_buffer`](crate::ring_buffer). Warm-up returns `Ok(None)`; the first
//! complete value is produced by the observation that finishes the window.

mod atr;
mod bollinger;
mod ema;
mod macd;
mod roc;
mod rsi;
mod sma;
mod stochastic;
mod wma;

pub use atr::Atr;
pub use bollinger::{BollingerBands, BollingerBandsOutput};
pub use ema::Ema;
pub use macd::{Macd, MacdOutput};
pub use roc::Roc;
pub use rsi::Rsi;
pub use sma::Sma;
pub use stochastic::{Stochastic, StochasticOutput};
pub use wma::Wma;
