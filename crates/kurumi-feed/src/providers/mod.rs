//! Built-in market-data connector implementations.
//!
//! The mock provider in [`mock`] is the reference implementation used by the
//! workspace test suite. The [`nasdaq`] adapters cover the native TotalView-ITCH
//! binary protocol, the Nasdaq Cloud Data Service REST resources, and the
//! Alpaca-compatible US equity WebSocket stream. Live providers depend on the
//! same [`crate::MarketDataConnector`] contract.

pub mod mock;
pub mod nasdaq;

pub use mock::MockReplayProvider;
pub use nasdaq::{
    ItchDecoder, ItchStreamProvider, LastQuote, LastSale, NasdaqWsConfig, NasdaqWsProvider,
    NcdsBar, NcdsBarPrecision, NcdsPollingConnector, NcdsRestClient, NcdsRestConfig, NcdsSnapshot,
};
