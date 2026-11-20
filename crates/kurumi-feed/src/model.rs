//! The provider-neutral real-time market-data model.
//!
//! Every event shares one vocabulary: instruments are identified by symbol and
//! [`Venue`], timestamps are UTC [`DateTime`] values carrying both the exchange
//! time and the local receive time, and prices and sizes stay in
//! [`rust_decimal::Decimal`] from the wire all the way to the consumer. Nothing
//! in this module rounds money through `f64`.

use chrono::{DateTime, Utc};
use kurumi_ta::OhlcvBar;
use rust_decimal::Decimal;

/// A trading venue that publishes market data.
///
/// The three US equity venues and the consolidated SIP feed are named
/// explicitly so connectors can advertise them; providers with unlisted venues
/// use [`Venue::Custom`] with their own stable code.
///
/// # Examples
///
/// ```
/// use kurumi_feed::Venue;
///
/// assert_eq!(Venue::Nasdaq.to_string(), "NASDAQ");
/// assert_eq!(Venue::Custom("BATS".to_owned()).to_string(), "BATS");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Venue {
    /// Nasdaq.
    Nasdaq,
    /// New York Stock Exchange.
    Nyse,
    /// Investors Exchange.
    Iex,
    /// Consolidated tape (Securities Information Processor).
    Sip,
    /// A venue identified by a provider-specific code.
    Custom(String),
}

impl Venue {
    /// Returns the canonical venue code.
    ///
    /// The built-in venues use their conventional uppercase codes; custom
    /// venues return the stored code unchanged.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_feed::Venue;
    ///
    /// assert_eq!(Venue::Iex.as_str(), "IEX");
    /// assert_eq!(Venue::Custom("BATS".to_owned()).as_str(), "BATS");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Nasdaq => "NASDAQ",
            Self::Nyse => "NYSE",
            Self::Iex => "IEX",
            Self::Sip => "SIP",
            Self::Custom(code) => code.as_str(),
        }
    }
}

impl std::fmt::Display for Venue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The identity of a tradable instrument on a specific venue.
///
/// The same ticker can trade on several venues with different prices, so the
/// symbol alone is not a key: [`InstrumentId`] combines both and implements
/// [`Ord`] and [`Hash`] so it can be used in `BTreeMap`, `BTreeSet`, and
/// `HashMap`.
///
/// # Examples
///
/// ```
/// use kurumi_feed::{InstrumentId, Venue};
///
/// let instrument = InstrumentId::new("AAPL", Venue::Nasdaq);
/// assert_eq!(instrument.to_string(), "AAPL@NASDAQ");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstrumentId {
    /// Ticker or provider symbol.
    pub symbol: String,
    /// Venue on which the symbol trades.
    pub venue: Venue,
}

impl InstrumentId {
    /// Creates an instrument identifier from a symbol and a venue.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_feed::{InstrumentId, Venue};
    ///
    /// let instrument = InstrumentId::new("MSFT", Venue::Nyse);
    /// assert_eq!(instrument.symbol, "MSFT");
    /// assert_eq!(instrument.venue, Venue::Nyse);
    /// ```
    #[must_use]
    pub fn new(symbol: impl Into<String>, venue: Venue) -> Self {
        Self {
            symbol: symbol.into(),
            venue,
        }
    }
}

impl std::fmt::Display for InstrumentId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}@{}", self.symbol, self.venue)
    }
}

/// A stream of market data requested from a connector.
///
/// Channels are selected independently per instrument, which lets a consumer
/// subscribe to trades and quotes for one symbol while only needing order-book
/// deltas for another.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChannelKind {
    /// Executed trades (prints).
    Trades,
    /// Top-of-book bid/ask quotes.
    Quotes,
    /// Incremental order-book updates.
    OrderBook,
    /// Aggregated OHLCV bars at the given interval, in seconds.
    Bars {
        /// Bucket width in seconds. Must be strictly positive.
        interval_secs: u64,
    },
}

/// A complete subscription request for one instrument.
///
/// The aggregator and connectors compare subscriptions by value; a set of
/// [`Subscription`]s is kept in a `BTreeSet` by the resilience runtime so the
/// active set can be replayed deterministically after a reconnect.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Subscription {
    /// Instrument whose channels are requested.
    pub instrument: InstrumentId,
    /// Channels requested for the instrument.
    pub channels: Vec<ChannelKind>,
}

impl Subscription {
    /// Creates a subscription for the given instrument and channels.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_feed::{ChannelKind, InstrumentId, Subscription, Venue};
    ///
    /// let subscription = Subscription::new(
    ///     InstrumentId::new("AAPL", Venue::Nasdaq),
    ///     vec![ChannelKind::Trades],
    /// );
    /// assert_eq!(subscription.channels.len(), 1);
    /// ```
    #[must_use]
    pub fn new(instrument: InstrumentId, channels: Vec<ChannelKind>) -> Self {
        Self {
            instrument,
            channels,
        }
    }
}

/// The side of an order book affected by a [`BookDelta`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BookSide {
    /// Bid side (buyers).
    Bid,
    /// Ask side (sellers).
    Ask,
}

/// One executed trade.
///
/// `price` and `size` are exact decimal values decoded directly from the
/// exchange frame; they are never routed through `f64`. `exchange_ts` is the
/// exchange's declared time and `received_ts` the local receipt time, which
/// lets consumers measure latency without losing the authoritative timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeTick {
    /// Instrument that traded.
    pub instrument: InstrumentId,
    /// Execution price in the quote currency.
    pub price: Decimal,
    /// Executed base-unit size.
    pub size: Decimal,
    /// Venue trade identifier, when the feed supplies one.
    pub trade_id: Option<String>,
    /// Sale conditions reported by the venue.
    pub conditions: Vec<String>,
    /// Exchange timestamp of the execution.
    pub exchange_ts: DateTime<Utc>,
    /// Local time at which the event was received.
    pub received_ts: DateTime<Utc>,
}

/// A top-of-book quote update.
///
/// A zero size on one side means that side is empty. Prices and sizes remain
/// exact decimals decoded from the provider message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteTick {
    /// Instrument quoted.
    pub instrument: InstrumentId,
    /// Best bid price.
    pub bid_price: Decimal,
    /// Size available at the best bid.
    pub bid_size: Decimal,
    /// Best ask price.
    pub ask_price: Decimal,
    /// Size available at the best ask.
    pub ask_size: Decimal,
    /// Exchange timestamp of the quote.
    pub exchange_ts: DateTime<Utc>,
    /// Local time at which the event was received.
    pub received_ts: DateTime<Utc>,
}

/// An incremental order-book update.
///
/// A delta with `size == Decimal::ZERO` removes the price level. `order_ref`
/// carries the provider's order reference when the protocol exposes one; a
/// level-based feed leaves it `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookDelta {
    /// Instrument whose book changed.
    pub instrument: InstrumentId,
    /// Side of the book that changed.
    pub side: BookSide,
    /// Price level affected by the update.
    pub price: Decimal,
    /// New size at the level; zero removes the level.
    pub size: Decimal,
    /// Provider order reference, when available.
    pub order_ref: Option<u64>,
    /// Exchange timestamp of the update.
    pub exchange_ts: DateTime<Utc>,
    /// Local time at which the event was received.
    pub received_ts: DateTime<Utc>,
}

/// A closed or in-progress OHLCV bar.
///
/// `bar` is a [`kurumi_ta::OhlcvBar`], so its prices are `f64` values validated
/// by `kurumi-ta`. The bar timestamp is the aligned bucket start in Unix
/// milliseconds. `is_final` is `true` only once the bucket has closed; the
/// streaming aggregator in [`crate::BarAggregator`] emits final bars.
#[derive(Debug, Clone, PartialEq)]
pub struct BarEvent {
    /// Instrument the bar describes.
    pub instrument: InstrumentId,
    /// Bucket width in seconds.
    pub interval_secs: u64,
    /// Aggregated candle.
    pub bar: OhlcvBar,
    /// Whether the bucket has closed and the candle will not change again.
    pub is_final: bool,
}

/// Lifecycle state advertised by a provider or the resilience runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionState {
    /// A connection attempt is in progress.
    Connecting,
    /// The transport is connected.
    Connected,
    /// The provider accepted the session credentials.
    Authenticated,
    /// A previous connection was lost and the runtime is retrying.
    Reconnecting,
    /// The connection is closed.
    Disconnected,
}

/// A provider lifecycle notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedStatusEvent {
    /// Provider that emitted the notification.
    pub provider: String,
    /// Current lifecycle state.
    pub state: ConnectionState,
    /// Optional human-readable detail.
    pub message: Option<String>,
    /// Local time at which the state changed.
    pub ts: DateTime<Utc>,
}

/// Everything a market-data connector can deliver to a consumer.
///
/// Prices and quantities are exact [`Decimal`] values; only [`BarEvent`] wraps
/// `f64` candles because that is the input type of `kurumi-ta` indicators. The
/// [`MarketEvent::Status`] and [`MarketEvent::Lagged`] variants carry control
/// information alongside the data stream.
#[derive(Debug, Clone, PartialEq)]
pub enum MarketEvent {
    /// An executed trade.
    Trade(TradeTick),
    /// A top-of-book quote update.
    Quote(QuoteTick),
    /// An incremental order-book update.
    OrderBookDelta(BookDelta),
    /// A bar emitted by a provider or by [`crate::BarAggregator`].
    Bar(BarEvent),
    /// A provider lifecycle notification.
    Status(FeedStatusEvent),
    /// A provider skipped `skipped` events because an internal queue was
    /// saturated.
    ///
    /// This is a lossy-provider signal. The broadcast-backed
    /// [`crate::MarketDataHub`] never drops an event for one consumer to keep
    /// another alive; instead a slow broadcast subscriber observes
    /// `tokio::sync::broadcast::error::RecvError::Lagged` on its own receiver.
    Lagged {
        /// Provider that skipped events.
        provider: String,
        /// Number of events that were skipped.
        skipped: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instrument_ordering_is_deterministic() {
        let aapl_nasdaq = InstrumentId::new("AAPL", Venue::Nasdaq);
        let aapl_nyse = InstrumentId::new("AAPL", Venue::Nyse);
        let msft_nyse = InstrumentId::new("MSFT", Venue::Nyse);
        assert!(aapl_nasdaq < aapl_nyse);
        assert!(aapl_nyse < msft_nyse);
    }

    #[test]
    fn venue_display_uses_canonical_codes() {
        assert_eq!(Venue::Nasdaq.to_string(), "NASDAQ");
        assert_eq!(Venue::Nyse.to_string(), "NYSE");
        assert_eq!(Venue::Iex.to_string(), "IEX");
        assert_eq!(Venue::Sip.to_string(), "SIP");
        assert_eq!(Venue::Custom("BATS".to_owned()).to_string(), "BATS");
    }

    #[test]
    fn instrument_display_names_symbol_and_venue() {
        let instrument = InstrumentId::new("AAPL", Venue::Nasdaq);
        assert_eq!(instrument.to_string(), "AAPL@NASDAQ");
    }

    #[test]
    fn event_variants_preserve_decimal_payloads_and_timestamps() {
        let instrument = InstrumentId::new("AAPL", Venue::Nasdaq);
        let ts = DateTime::<Utc>::UNIX_EPOCH;
        let quote = QuoteTick {
            instrument: instrument.clone(),
            bid_price: Decimal::new(10_000, 2),
            bid_size: Decimal::new(3, 0),
            ask_price: Decimal::new(10_100, 2),
            ask_size: Decimal::new(4, 0),
            exchange_ts: ts,
            received_ts: ts,
        };
        assert_eq!(quote.bid_price, Decimal::new(10_000, 2));
        assert_eq!(quote.ask_size, Decimal::new(4, 0));

        let delta = BookDelta {
            instrument: instrument.clone(),
            side: BookSide::Ask,
            price: Decimal::new(10_100, 2),
            size: Decimal::ZERO,
            order_ref: Some(7),
            exchange_ts: ts,
            received_ts: ts,
        };
        assert_eq!(delta.side, BookSide::Ask);
        assert_eq!(delta.size, Decimal::ZERO);
        assert_eq!(delta.order_ref, Some(7));

        let status = FeedStatusEvent {
            provider: "mock".to_owned(),
            state: ConnectionState::Authenticated,
            message: Some("session accepted".to_owned()),
            ts,
        };
        assert_eq!(status.state, ConnectionState::Authenticated);

        let lagged = MarketEvent::Lagged {
            provider: "mock".to_owned(),
            skipped: 12,
        };
        match lagged {
            MarketEvent::Lagged { skipped, .. } => assert_eq!(skipped, 12),
            _ => panic!("expected a lagged event"),
        }
    }
}
