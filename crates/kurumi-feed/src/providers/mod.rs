//! Built-in market-data connector implementations.
//!
//! The mock provider in [`mock`] is the reference implementation used by the
//! workspace test suite. Live providers live in their own modules and depend on
//! the same [`crate::MarketDataConnector`] contract.

pub mod mock;

pub use mock::MockReplayProvider;
