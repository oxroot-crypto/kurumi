//! Nasdaq market-data adapters.
//!
//! Three transports share one provider-neutral event model:
//!
//! - [`itch`] decodes native Nasdaq TotalView-ITCH 5.0 binary messages and
//!   replays length-prefixed streams from any [`tokio::io::AsyncRead`];
//! - [`ncds_rest`] implements the Nasdaq Cloud Data Service REST resources and
//!   a deduplicating Last Sale / Last Quote polling connector;
//! - [`ws_us`] implements the Alpaca Market Data v2 WebSocket protocol used for
//!   US equity SIP, IEX, and free test streams.
//!
//! All three decode prices directly into [`rust_decimal::Decimal`] and convert
//! US Eastern timestamps to UTC without a time-zone database.

pub mod itch;
pub mod ncds_rest;
pub mod ws_us;

mod json;
mod time;

pub use itch::{ItchDecoder, ItchStreamProvider};
pub use ncds_rest::{
    LastQuote, LastSale, NcdsBar, NcdsBarPrecision, NcdsPollingConnector, NcdsRestClient,
    NcdsRestConfig, NcdsSnapshot,
};
pub use ws_us::{NasdaqWsConfig, NasdaqWsProvider};
