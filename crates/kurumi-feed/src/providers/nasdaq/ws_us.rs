//! US equity real-time WebSocket connector (Alpaca Market Data v2 protocol).
//!
//! The adapter implements the Alpaca Market Data v2 wire protocol, which is
//! also served by the free all-hours `wss://stream.data.alpaca.markets/v2/test`
//! stream. Point [`NasdaqWsConfig::endpoint_url`] at Alpaca SIP, IEX, the test
//! stream, an enterprise SIP endpoint, or a local mock server; the lifecycle is
//! the same.
//!
//! # Lifecycle
//!
//! 1. connect the WebSocket;
//! 2. read the `connected` welcome frame;
//! 3. send `{"action":"auth","key":...,"secret":...}` and wait for
//!    `authenticated`;
//! 4. send `subscribe` for the initial subscriptions and for every later
//!    [`ConnectorCommand`];
//! 5. parse batch arrays of `t` (trade), `q` (quote), and `b` (bar) records;
//! 6. send an empty ping every [`NasdaqWsConfig::heartbeat_interval`].
//!
//! A server-side close returns [`FeedError::Disconnected`], so
//! [`crate::ManagedConnector`] reconnects with backoff and replays the complete
//! active subscription set. Authentication failures and protocol errors are
//! surfaced as their dedicated [`FeedError`] variants.
//!
//! All prices are parsed from the JSON literal straight into [`Decimal`]; the
//! only `f64` boundary is the [`kurumi_ta::OhlcvBar`] carried by
//! [`MarketEvent::Bar`].

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use bytes::Bytes;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde_json::{Map, Value, json};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use super::json::{
    optional_decimal, protocol_error, required_decimal, required_rfc3339, required_string,
};
use crate::connector::{
    BoxFuture, ConnectorCommand, MarketDataConnector, apply_command, event_matches,
};
use crate::error::FeedError;
use crate::model::{
    BarEvent, ChannelKind, ConnectionState, FeedStatusEvent, InstrumentId, MarketEvent, QuoteTick,
    Subscription, TradeTick, Venue,
};
use crate::util::now_utc;
use kurumi_ta::OhlcvBar;

/// Provider name attached to WebSocket errors by default.
const DEFAULT_PROVIDER: &str = "nasdaq-ws";

/// Interval reported for Alpaca's default one-minute bar stream.
const ALPACA_BAR_INTERVAL_SECS: u64 = 60;

/// Active subscriptions grouped by Alpaca channel name.
type ChannelMap = BTreeMap<&'static str, BTreeSet<String>>;

/// The concrete WebSocket stream returned by `connect_async`.
type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// The write half of a split WebSocket stream.
type WsWrite = SplitSink<WsStream, Message>;

/// The read half of a split WebSocket stream.
type WsRead = SplitStream<WsStream>;

/// Configuration for the US equity WebSocket connector.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use kurumi_feed::providers::nasdaq::NasdaqWsConfig;
///
/// let config = NasdaqWsConfig::alpaca_test()
///     .with_credentials("key", "secret")
///     .with_heartbeat_interval(Duration::from_secs(10));
/// assert_eq!(
///     config.endpoint_url,
///     "wss://stream.data.alpaca.markets/v2/test"
/// );
/// assert!(config.validate().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NasdaqWsConfig {
    /// WebSocket endpoint, including the `ws://` or `wss://` scheme.
    pub endpoint_url: String,
    /// Alpaca API key.
    pub api_key: String,
    /// Alpaca API secret.
    pub api_secret: String,
    /// Interval between client heartbeat pings; zero disables pings.
    pub heartbeat_interval: Duration,
}

impl NasdaqWsConfig {
    /// Creates a configuration for the Alpaca SIP stream.
    #[must_use]
    pub fn alpaca_sip() -> Self {
        Self::alpaca("wss://stream.data.alpaca.markets/v2/sip")
    }

    /// Creates a configuration for the Alpaca IEX stream.
    #[must_use]
    pub fn alpaca_iex() -> Self {
        Self::alpaca("wss://stream.data.alpaca.markets/v2/iex")
    }

    /// Creates a configuration for the Alpaca all-hours test stream.
    #[must_use]
    pub fn alpaca_test() -> Self {
        Self::alpaca("wss://stream.data.alpaca.markets/v2/test")
    }

    /// Creates a configuration for a custom endpoint.
    #[must_use]
    pub fn custom(endpoint_url: impl Into<String>) -> Self {
        Self {
            endpoint_url: endpoint_url.into(),
            api_key: String::new(),
            api_secret: String::new(),
            heartbeat_interval: Duration::from_secs(30),
        }
    }

    /// Sets the API credentials.
    #[must_use]
    pub fn with_credentials(
        mut self,
        api_key: impl Into<String>,
        api_secret: impl Into<String>,
    ) -> Self {
        self.api_key = api_key.into();
        self.api_secret = api_secret.into();
        self
    }

    /// Sets the client heartbeat interval; zero disables pings.
    #[must_use]
    pub fn with_heartbeat_interval(mut self, interval: Duration) -> Self {
        self.heartbeat_interval = interval;
        self
    }

    /// Validates the endpoint and credentials.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] for a missing `ws://`/`wss://`
    /// scheme, an empty host, or empty credentials.
    pub fn validate(&self) -> Result<(), FeedError> {
        let parsed =
            reqwest::Url::parse(&self.endpoint_url).map_err(|error| FeedError::InvalidConfig {
                reason: format!("endpoint_url is not a valid URL: {error}"),
            })?;
        if !matches!(parsed.scheme(), "ws" | "wss") {
            return Err(FeedError::InvalidConfig {
                reason: "endpoint_url must use the ws:// or wss:// scheme".to_owned(),
            });
        }
        if parsed.host_str().is_none_or(str::is_empty) {
            return Err(FeedError::InvalidConfig {
                reason: "endpoint_url must include a host".to_owned(),
            });
        }
        if self.api_key.is_empty() || self.api_secret.is_empty() {
            return Err(FeedError::InvalidConfig {
                reason: "api_key and api_secret must not be empty".to_owned(),
            });
        }
        Ok(())
    }

    /// Creates a configuration with the shared Alpaca defaults.
    fn alpaca(endpoint_url: &str) -> Self {
        Self {
            endpoint_url: endpoint_url.to_owned(),
            api_key: String::new(),
            api_secret: String::new(),
            heartbeat_interval: Duration::from_secs(30),
        }
    }
}

/// An Alpaca Market Data v2 compatible WebSocket connector.
///
/// # Examples
///
/// ```
/// use kurumi_feed::MarketDataConnector;
/// use kurumi_feed::providers::nasdaq::{NasdaqWsConfig, NasdaqWsProvider};
///
/// let config = NasdaqWsConfig::alpaca_sip().with_credentials("key", "secret");
/// let provider = NasdaqWsProvider::new(config).expect("valid configuration");
/// assert_eq!(provider.name(), "nasdaq-ws");
/// ```
pub struct NasdaqWsProvider {
    name: String,
    config: NasdaqWsConfig,
    venues: Vec<Venue>,
}

impl std::fmt::Debug for NasdaqWsProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NasdaqWsProvider")
            .field("name", &self.name)
            .field("endpoint", &self.config.endpoint_url)
            .field("venues", &self.venues)
            .finish_non_exhaustive()
    }
}

impl NasdaqWsProvider {
    /// Creates a provider after validating `config`.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] when the configuration is invalid.
    pub fn new(config: NasdaqWsConfig) -> Result<Self, FeedError> {
        config.validate()?;
        let venues = if config.endpoint_url.contains("/sip") {
            vec![Venue::Sip]
        } else if config.endpoint_url.contains("/iex") {
            vec![Venue::Iex]
        } else {
            vec![Venue::Nasdaq]
        };
        Ok(Self {
            name: DEFAULT_PROVIDER.to_owned(),
            config,
            venues,
        })
    }

    /// Overrides the provider name used for routing and diagnostics.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Overrides the venues advertised by the connector.
    #[must_use]
    pub fn with_venues(mut self, venues: Vec<Venue>) -> Self {
        self.venues = venues;
        self
    }

    /// Returns the configuration in use.
    #[must_use]
    pub fn config(&self) -> &NasdaqWsConfig {
        &self.config
    }
}

impl MarketDataConnector for NasdaqWsProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn supported_venues(&self) -> &[Venue] {
        &self.venues
    }

    fn connect(
        &self,
        initial_subs: Vec<Subscription>,
        tx: mpsc::Sender<MarketEvent>,
        cmd_rx: mpsc::Receiver<ConnectorCommand>,
        shutdown: watch::Receiver<bool>,
    ) -> BoxFuture<'_, Result<(), FeedError>> {
        Box::pin(self.run_session(initial_subs, tx, cmd_rx, shutdown))
    }
}

impl NasdaqWsProvider {
    /// Runs one authenticated WebSocket session.
    async fn run_session(
        &self,
        initial_subs: Vec<Subscription>,
        tx: mpsc::Sender<MarketEvent>,
        mut cmd_rx: mpsc::Receiver<ConnectorCommand>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), FeedError> {
        let (socket, _response) = connect_async(self.config.endpoint_url.as_str())
            .await
            .map_err(|error| FeedError::ConnectionFailed {
                provider: self.name.clone(),
                reason: error.to_string(),
            })?;
        let (mut write, mut read) = socket.split();

        let welcome = next_json(&mut read, &mut shutdown, &self.name).await?;
        expect_success(&welcome, "connected", &self.name)?;

        let auth = json!({
            "action": "auth",
            "key": self.config.api_key.clone(),
            "secret": self.config.api_secret.clone(),
        });
        send_json(&mut write, &auth, &self.name).await?;
        loop {
            let frame = next_json(&mut read, &mut shutdown, &self.name).await?;
            let mut authenticated = false;
            for item in frame_items(&frame) {
                match message_kind(item) {
                    Some("error") => return Err(ws_error(item, &self.name)),
                    Some("success")
                        if item.get("msg").and_then(Value::as_str) == Some("authenticated") =>
                    {
                        authenticated = true;
                    }
                    _ => {}
                }
            }
            if authenticated {
                break;
            }
        }
        if !send_event(
            &tx,
            MarketEvent::Status(FeedStatusEvent {
                provider: self.name.clone(),
                state: ConnectionState::Authenticated,
                message: Some("Alpaca session authenticated".to_owned()),
                ts: now_utc(),
            }),
            &mut shutdown,
        )
        .await
        {
            return Ok(());
        }

        let mut active: BTreeSet<Subscription> = initial_subs.into_iter().collect();
        let mut sent_channels = channel_map(&active);
        let initial_frame = subscription_frame("subscribe", &sent_channels);
        if let Some(frame) = initial_frame {
            send_json(&mut write, &frame, &self.name).await?;
        }

        let heartbeat_interval = self.config.heartbeat_interval;
        let heartbeat_enabled = !heartbeat_interval.is_zero();
        let heartbeat = tokio::time::sleep(heartbeat_interval);
        tokio::pin!(heartbeat);

        loop {
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Ok(());
                    }
                }
                command = cmd_rx.recv() => {
                    let Some(command) = command else {
                        return Ok(());
                    };
                    apply_command(&mut active, &command);
                    let requested = channel_map(&active);
                    let (unsubscribe, subscribe) = subscription_diff(&sent_channels, &requested);
                    if let Some(frame) = subscription_frame("unsubscribe", &unsubscribe) {
                        send_json(&mut write, &frame, &self.name).await?;
                    }
                    if let Some(frame) = subscription_frame("subscribe", &subscribe) {
                        send_json(&mut write, &frame, &self.name).await?;
                    }
                    sent_channels = requested;
                }
                message = read.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            let frame: Value = serde_json::from_str(text.as_str()).map_err(|error| {
                                protocol_error(&self.name, format!("invalid JSON frame: {error}"))
                            })?;
                            for item in frame_items(&frame) {
                                match parse_event(item, self.venues.first(), &self.name)? {
                                    Some(event) if event_matches(&active, &event) => {
                                        if !send_event(&tx, event, &mut shutdown).await {
                                            return Ok(());
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Some(Ok(Message::Binary(_))) => {
                            return Err(protocol_error(
                                &self.name,
                                "expected a JSON text frame, received binary data",
                            ));
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            write.send(Message::Pong(payload)).await.map_err(|error| {
                                FeedError::Disconnected {
                                    provider: self.name.clone(),
                                    reason: error.to_string(),
                                }
                            })?;
                        }
                        Some(Ok(Message::Close(_))) | None => {
                            return Err(FeedError::Disconnected {
                                provider: self.name.clone(),
                                reason: "WebSocket closed by the server".to_owned(),
                            });
                        }
                        Some(Ok(_)) => {}
                        Some(Err(error)) => {
                            return Err(FeedError::Disconnected {
                                provider: self.name.clone(),
                                reason: error.to_string(),
                            });
                        }
                    }
                }
                () = &mut heartbeat, if heartbeat_enabled => {
                    heartbeat.as_mut().reset(tokio::time::Instant::now() + heartbeat_interval);
                    write
                        .send(Message::Ping(Bytes::new()))
                        .await
                        .map_err(|error| FeedError::Disconnected {
                            provider: self.name.clone(),
                            reason: error.to_string(),
                        })?;
                }
            }
        }
    }
}

/// Reads the next JSON text frame, ignoring control frames.
async fn next_json(
    read: &mut WsRead,
    shutdown: &mut watch::Receiver<bool>,
    provider: &str,
) -> Result<Value, FeedError> {
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Err(FeedError::Disconnected {
                        provider: provider.to_owned(),
                        reason: "shutdown requested during the WebSocket handshake".to_owned(),
                    });
                }
            }
            message = read.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        return serde_json::from_str(text.as_str()).map_err(|error| {
                            protocol_error(provider, format!("invalid JSON frame: {error}"))
                        });
                    }
                    Some(Ok(Message::Binary(_))) => {
                        return Err(protocol_error(
                            provider,
                            "expected a JSON text frame, received binary data",
                        ));
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        return Err(FeedError::Disconnected {
                            provider: provider.to_owned(),
                            reason: "WebSocket closed during the handshake".to_owned(),
                        });
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        return Err(FeedError::Disconnected {
                            provider: provider.to_owned(),
                            reason: error.to_string(),
                        });
                    }
                }
            }
        }
    }
}

/// Serialises and sends one JSON control frame.
async fn send_json(write: &mut WsWrite, value: &Value, provider: &str) -> Result<(), FeedError> {
    let text = serde_json::to_string(value)
        .map_err(|error| protocol_error(provider, format!("failed to encode a frame: {error}")))?;
    write
        .send(Message::Text(text.into()))
        .await
        .map_err(|error| FeedError::Disconnected {
            provider: provider.to_owned(),
            reason: error.to_string(),
        })
}

/// Returns the `T` discriminator of one frame element.
fn message_kind(item: &Value) -> Option<&str> {
    item.get("T").and_then(Value::as_str)
}

/// Returns the elements of a batch array or the single frame object.
fn frame_items(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().collect(),
        item => vec![item],
    }
}

/// Requires a `success` frame with the expected message.
fn expect_success(value: &Value, expected: &str, provider: &str) -> Result<(), FeedError> {
    for item in frame_items(value) {
        if message_kind(item) == Some("error") {
            return Err(ws_error(item, provider));
        }
        if message_kind(item) == Some("success")
            && item.get("msg").and_then(Value::as_str) == Some(expected)
        {
            return Ok(());
        }
    }
    Err(protocol_error(
        provider,
        format!("expected a `{expected}` success frame"),
    ))
}

/// Maps an Alpaca error frame to a typed error.
fn ws_error(item: &Value, provider: &str) -> FeedError {
    let code = item.get("code").and_then(Value::as_i64).unwrap_or(0);
    let message = item
        .get("msg")
        .and_then(Value::as_str)
        .unwrap_or("unknown provider error");
    match code {
        401 | 403 => FeedError::AuthenticationFailed {
            provider: provider.to_owned(),
            reason: format!("code {code}: {message}"),
        },
        406 => FeedError::ConnectionFailed {
            provider: provider.to_owned(),
            reason: format!("connection limit reached (code 406): {message}"),
        },
        429 => FeedError::RateLimited {
            provider: provider.to_owned(),
            retry_after_ms: 1_000,
        },
        _ => protocol_error(provider, format!("code {code}: {message}")),
    }
}

/// Converts one Alpaca frame element into a market event.
fn parse_event(
    item: &Value,
    venue: Option<&Venue>,
    provider: &str,
) -> Result<Option<MarketEvent>, FeedError> {
    let venue = venue.cloned().unwrap_or(Venue::Nasdaq);
    match message_kind(item) {
        Some("t") => Ok(Some(MarketEvent::Trade(parse_trade(
            item, &venue, provider,
        )?))),
        Some("q") => Ok(Some(MarketEvent::Quote(parse_quote(
            item, &venue, provider,
        )?))),
        Some("b") => Ok(Some(MarketEvent::Bar(parse_bar(item, &venue, provider)?))),
        Some("error") => Err(ws_error(item, provider)),
        _ => Ok(None),
    }
}

/// Parses an Alpaca trade record.
fn parse_trade(item: &Value, venue: &Venue, provider: &str) -> Result<TradeTick, FeedError> {
    let symbol = required_string(item, "S", provider)?;
    let conditions = item
        .get("c")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let trade_id = item
        .get("i")
        .and_then(Value::as_u64)
        .map(|id| id.to_string());
    Ok(TradeTick {
        instrument: InstrumentId::new(symbol, venue.clone()),
        price: required_decimal(item, "p", provider)?,
        size: required_decimal(item, "s", provider)?,
        trade_id,
        conditions,
        exchange_ts: required_rfc3339(item, "t", provider)?,
        received_ts: now_utc(),
    })
}

/// Parses an Alpaca quote record.
fn parse_quote(item: &Value, venue: &Venue, provider: &str) -> Result<QuoteTick, FeedError> {
    Ok(QuoteTick {
        instrument: InstrumentId::new(required_string(item, "S", provider)?, venue.clone()),
        bid_price: required_decimal(item, "bp", provider)?,
        bid_size: required_decimal(item, "bs", provider)?,
        ask_price: required_decimal(item, "ap", provider)?,
        ask_size: required_decimal(item, "as", provider)?,
        exchange_ts: required_rfc3339(item, "t", provider)?,
        received_ts: now_utc(),
    })
}

/// Parses an Alpaca bar record into a finalized bar event.
fn parse_bar(item: &Value, venue: &Venue, provider: &str) -> Result<BarEvent, FeedError> {
    let symbol = required_string(item, "S", provider)?;
    let exchange_ts = required_rfc3339(item, "t", provider)?;
    let open = required_decimal(item, "o", provider)?;
    let high = required_decimal(item, "h", provider)?;
    let low = required_decimal(item, "l", provider)?;
    let close = required_decimal(item, "c", provider)?;
    let volume = required_decimal(item, "v", provider)?;
    let turnover = match optional_decimal(item, "vw", provider)? {
        Some(vwap) => volume
            .checked_mul(vwap)
            .ok_or_else(|| protocol_error(provider, "bar turnover overflows Decimal"))?,
        None => Decimal::ZERO,
    };
    let bar = OhlcvBar::new(
        exchange_ts.timestamp_millis(),
        decimal_to_f64(open, "open", provider)?,
        decimal_to_f64(high, "high", provider)?,
        decimal_to_f64(low, "low", provider)?,
        decimal_to_f64(close, "close", provider)?,
        decimal_to_f64(volume, "volume", provider)?,
        decimal_to_f64(turnover, "turnover", provider)?,
    )
    .map_err(|error| protocol_error(provider, format!("invalid bar: {error}")))?;
    Ok(BarEvent {
        instrument: InstrumentId::new(symbol, venue.clone()),
        interval_secs: ALPACA_BAR_INTERVAL_SECS,
        bar,
        is_final: true,
    })
}

/// Converts a decimal to a finite `f64` for the `kurumi-ta` OHLCV boundary.
fn decimal_to_f64(value: Decimal, field: &str, provider: &str) -> Result<f64, FeedError> {
    value
        .to_f64()
        .filter(|converted| converted.is_finite())
        .ok_or_else(|| {
            protocol_error(
                provider,
                format!("bar {field} cannot be represented as f64"),
            )
        })
}

/// Groups active subscriptions by Alpaca channel name.
fn channel_map(active: &BTreeSet<Subscription>) -> ChannelMap {
    let mut channels: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    for subscription in active {
        for channel in &subscription.channels {
            let name = match channel {
                ChannelKind::Trades => "trades",
                ChannelKind::Quotes => "quotes",
                ChannelKind::Bars { .. } => "bars",
                ChannelKind::OrderBook => continue,
            };
            channels
                .entry(name)
                .or_default()
                .insert(subscription.instrument.symbol.clone());
        }
    }
    channels
}

/// Builds a subscribe/unsubscribe frame, or `None` when no channel is present.
fn subscription_frame(action: &str, channels: &ChannelMap) -> Option<Value> {
    let mut object = Map::new();
    object.insert("action".to_owned(), Value::String(action.to_owned()));
    for (channel, symbols) in channels {
        if symbols.is_empty() {
            continue;
        }
        let symbols = symbols
            .iter()
            .map(|symbol| Value::String(symbol.clone()))
            .collect();
        object.insert((*channel).to_owned(), Value::Array(symbols));
    }
    if object.len() == 1 {
        None
    } else {
        Some(Value::Object(object))
    }
}

/// Computes the unsubscribe and subscribe deltas between two channel maps.
fn subscription_diff(previous: &ChannelMap, requested: &ChannelMap) -> (ChannelMap, ChannelMap) {
    let empty = BTreeSet::new();
    let mut unsubscribe = BTreeMap::new();
    let mut subscribe = BTreeMap::new();
    let names: BTreeSet<&str> = previous.keys().chain(requested.keys()).copied().collect();
    for name in names {
        let old = previous.get(name).unwrap_or(&empty);
        let new = requested.get(name).unwrap_or(&empty);
        let removed: BTreeSet<String> = old.difference(new).cloned().collect();
        let added: BTreeSet<String> = new.difference(old).cloned().collect();
        if !removed.is_empty() {
            unsubscribe.insert(name, removed);
        }
        if !added.is_empty() {
            subscribe.insert(name, added);
        }
    }
    (unsubscribe, subscribe)
}

/// Sends one event unless shutdown wins the race.
async fn send_event(
    tx: &mpsc::Sender<MarketEvent>,
    event: MarketEvent,
    shutdown: &mut watch::Receiver<bool>,
) -> bool {
    if *shutdown.borrow() {
        return false;
    }
    tokio::select! {
        biased;
        _ = shutdown.changed() => false,
        result = tx.send(event) => result.is_ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    fn config() -> NasdaqWsConfig {
        NasdaqWsConfig::alpaca_test().with_credentials("key", "secret")
    }

    #[test]
    fn alpaca_endpoints_are_available_as_constructors() {
        assert_eq!(
            NasdaqWsConfig::alpaca_sip().endpoint_url,
            "wss://stream.data.alpaca.markets/v2/sip"
        );
        assert_eq!(
            NasdaqWsConfig::alpaca_iex().endpoint_url,
            "wss://stream.data.alpaca.markets/v2/iex"
        );
        assert_eq!(
            NasdaqWsConfig::alpaca_test().endpoint_url,
            "wss://stream.data.alpaca.markets/v2/test"
        );
    }

    #[test]
    fn config_rejects_bad_schemes_and_missing_credentials() {
        let mut invalid = config();
        invalid.endpoint_url = "https://example.com".to_owned();
        assert!(invalid.validate().is_err());
        let mut missing_secret = NasdaqWsConfig::alpaca_test();
        missing_secret.api_key = "key".to_owned();
        assert!(missing_secret.validate().is_err());
    }

    #[test]
    fn trade_frame_decodes_decimal_price_and_rfc3339_time() {
        let item = json!({
            "T": "t",
            "S": "AAPL",
            "i": 42,
            "p": 187.5123,
            "s": 10,
            "c": ["@", "T"],
            "t": "2024-11-20T14:30:01.123456789Z"
        });
        let event = parse_event(&item, Some(&Venue::Sip), DEFAULT_PROVIDER)
            .expect("valid event")
            .expect("a trade");
        let trade = match event {
            MarketEvent::Trade(trade) => trade,
            other => panic!("expected a trade, got {other:?}"),
        };
        assert_eq!(trade.instrument.venue, Venue::Sip);
        assert_eq!(trade.price, Decimal::new(1_875_123, 4));
        assert_eq!(trade.trade_id.as_deref(), Some("42"));
        assert_eq!(trade.exchange_ts.timestamp_subsec_nanos(), 123_456_789);
        assert_eq!(trade.exchange_ts.hour(), 14);
    }

    #[test]
    fn error_406_is_a_connection_limit_error() {
        let item = json!({"T": "error", "code": 406, "msg": "connection limit exceeded"});
        assert!(matches!(
            parse_event(&item, None, DEFAULT_PROVIDER),
            Err(FeedError::ConnectionFailed { .. })
        ));
    }

    #[test]
    fn bar_frame_becomes_a_final_one_minute_bar() {
        let item = json!({
            "T": "b",
            "S": "AAPL",
            "o": 187.50,
            "h": 188.00,
            "l": 187.40,
            "c": 187.75,
            "v": 1000,
            "vw": 187.65,
            "t": "2024-11-20T14:31:00Z"
        });
        let event = parse_event(&item, None, DEFAULT_PROVIDER)
            .expect("valid event")
            .expect("a bar");
        let bar = match event {
            MarketEvent::Bar(bar) => bar,
            other => panic!("expected a bar, got {other:?}"),
        };
        assert!(bar.is_final);
        assert_eq!(bar.interval_secs, 60);
        assert_eq!(bar.instrument.symbol, "AAPL");
        approx::assert_relative_eq!(bar.bar.close(), 187.75, epsilon = 1e-12);
    }

    #[test]
    fn subscription_diff_tracks_adds_and_removals_per_channel() {
        let previous = BTreeMap::from([("trades", BTreeSet::from(["AAPL".to_owned()]))]);
        let requested = BTreeMap::from([
            ("quotes", BTreeSet::from(["AAPL".to_owned()])),
            ("trades", BTreeSet::from(["MSFT".to_owned()])),
        ]);
        let (unsubscribe, subscribe) = subscription_diff(&previous, &requested);
        assert_eq!(
            unsubscribe.get("trades").cloned(),
            Some(BTreeSet::from(["AAPL".to_owned()]))
        );
        assert_eq!(
            subscribe.get("trades").cloned(),
            Some(BTreeSet::from(["MSFT".to_owned()]))
        );
        assert_eq!(
            subscribe.get("quotes").cloned(),
            Some(BTreeSet::from(["AAPL".to_owned()]))
        );
    }
}
