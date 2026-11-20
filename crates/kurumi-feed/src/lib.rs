//! Asynchronous real-time market-data infrastructure for Kurumi.
//!
//! `kurumi-feed` is the workspace's only asynchronous crate. It owns the
//! network runtime, the connector abstraction, the resilience runtime, the
//! multi-provider fan-out hub, and the streaming OHLCV aggregator; the rest of
//! Kurumi stays synchronous and runtime-free.
//!
//! # Layers
//!
//! ```text
//! MarketDataConnector      one provider's wire protocol
//!         |
//! ManagedConnector         backoff, heartbeat, auto-resubscribe
//!         |
//! MarketDataHub            multi-provider routing + broadcast fan-out
//!         |
//! BarAggregator            trades -> aligned OHLCV bars -> kurumi-ta
//! ```
//!
//! # Data model
//!
//! Prices and quantities are [`rust_decimal::Decimal`] from the wire to the
//! consumer. The only `f64` boundary is [`BarEvent`], whose [`OhlcvBar`] payload
//! is the validated input type of `kurumi-ta` streaming indicators.
//!
//! # Async isolation
//!
//! `tokio`, `tokio-tungstenite`, and `reqwest` are confined to this crate.
//! Network TLS is pure Rust (`rustls`) so portable builds never require
//! OpenSSL. See the `kurumi-feed` invariants in `AGENTS.md`.
//!
//! # Built-in providers
//!
//! [`MockReplayProvider`] is the deterministic reference connector. The
//! [`providers::nasdaq`] module adds native TotalView-ITCH 5.0 binary decoding
//! and replay ([`ItchDecoder`], [`ItchStreamProvider`]), the Nasdaq Cloud Data
//! Service REST resources and deduplicating polling connector
//! ([`NcdsRestClient`], [`NcdsPollingConnector`]), and an Alpaca Market Data v2
//! compatible US equity WebSocket connector ([`NasdaqWsProvider`]).
//!
//! # Offline testing
//!
//! The test suite never contacts a live endpoint. [`MockReplayProvider`],
//! embedded ITCH binary fixtures, and loopback WebSocket and HTTP mock servers
//! cover the default tests. The live Alpaca template is `#[ignore]`d and reads
//! its credentials from environment variables.
//!
//! # Example
//!
//! ```
//! use std::sync::Arc;
//! use kurumi_feed::{
//!     ChannelKind, InstrumentId, MarketDataConnector, MarketDataHub,
//!     MockReplayProvider, Subscription, Venue,
//! };
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), kurumi_feed::FeedError> {
//! let mut hub = MarketDataHub::new(256)?;
//! let provider: Arc<dyn MarketDataConnector> = Arc::new(MockReplayProvider::new(
//!     "mock",
//!     vec![Venue::Nasdaq],
//!     Vec::new(),
//! ));
//! hub.register(provider)?;
//! let _events = hub.subscribe_events();
//! hub.start()?;
//! hub.subscribe(
//!     "mock",
//!     vec![Subscription::new(
//!         InstrumentId::new("AAPL", Venue::Nasdaq),
//!         vec![ChannelKind::Trades],
//!     )],
//! )
//! .await?;
//! hub.shutdown().await?;
//! # Ok(())
//! # }
//! ```
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

pub mod aggregator;
pub mod connector;
pub mod error;
pub mod hub;
pub mod model;
pub mod providers;
pub mod runtime;

mod util;

pub use aggregator::BarAggregator;
pub use connector::{BoxFuture, ConnectorCommand, MarketDataConnector};
pub use error::FeedError;
pub use hub::MarketDataHub;
pub use model::{
    BarEvent, BookDelta, BookSide, ChannelKind, ConnectionState, FeedStatusEvent, InstrumentId,
    MarketEvent, QuoteTick, Subscription, TradeTick, Venue,
};
pub use providers::MockReplayProvider;
pub use providers::nasdaq::{
    ItchDecoder, ItchStreamProvider, LastQuote, LastSale, NasdaqWsConfig, NasdaqWsProvider,
    NcdsBar, NcdsBarPrecision, NcdsPollingConnector, NcdsRestClient, NcdsRestConfig, NcdsSnapshot,
};
pub use runtime::{ManagedConnector, ReconnectPolicy};

pub use kurumi_ta::OhlcvBar;
