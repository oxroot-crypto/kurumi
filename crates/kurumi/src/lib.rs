//! Kurumi: the all-in-one facade for the workspace.
//!
//! Each workspace member is re-exported behind an additive Cargo feature:
//!
//! | Feature | Re-export | Member crate |
//! | --- | --- | --- |
//! | `core` | `kurumi::core` | `kurumi-core` |
//! | `math` | `kurumi::math` | `kurumi-math` |
//! | `ta` | `kurumi::ta` | `kurumi-ta` |
//! | `pricing` | `kurumi::pricing` | `kurumi-pricing` |
//! | `risk` | `kurumi::risk` | `kurumi-risk` |
//! | `backtest` | `kurumi::backtest` | `kurumi-backtest` |
//!
//! Default features enable every module. Lower-layer features are implied by
//! the features above them, so enabling `risk` also enables `core`, `math`,
//! `ta`, and `pricing`.
//!
//! Python and WebAssembly bindings are planned as additional opt-in features of
//! this crate. Keep that binding surface thin: it must delegate to the Rust API
//! rather than reimplement financial logic.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

#[cfg(feature = "core")]
pub use kurumi_core as core;

#[cfg(feature = "math")]
pub use kurumi_math as math;

#[cfg(feature = "ta")]
pub use kurumi_ta as ta;

#[cfg(feature = "pricing")]
pub use kurumi_pricing as pricing;

#[cfg(feature = "risk")]
pub use kurumi_risk as risk;

#[cfg(feature = "backtest")]
pub use kurumi_backtest as backtest;
