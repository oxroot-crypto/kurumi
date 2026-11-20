//! Nasdaq Cloud Data Service (NCDS) REST client and polling connector.
//!
//! The client implements the documented Nasdaq Data Link REST resources used by
//! Nasdaq Basic and Last Sale+:
//!
//! - `POST /v1/auth/token` for OAuth 2.0 client-credentials authentication;
//! - `GET /v1/<source>/<offset>/equities/lastsale/<symbols>`;
//! - `GET /v1/<source>/<offset>/equities/lastquote/<symbols>`;
//! - `GET /v1/<source>/<offset>/equities/snapshot/<symbols>`;
//! - `GET /v2/<source>/<offset>/equities/bars/<symbol>/<precision>/false/<range>`.
//!
//! See the [Nasdaq Data Link REST API reference](https://github.com/Nasdaq/NasdaqCloudDataService-REST-API).
//!
//! Every endpoint is derived from [`NcdsRestConfig::base_url`] and
//! [`NcdsRestConfig::token_url`], so the same client runs against the
//! enterprise-provided base URL or a local mock server. Prices and sizes are
//! parsed directly into [`Decimal`] from the retained JSON literal; they never
//! pass through `f64`. Timestamps are US Eastern local times without an offset
//! and are converted to UTC with the same US Eastern daylight-saving rule as the
//! ITCH decoder.
//!
//! [`NcdsPollingConnector`] polls Last Sale and Last Quote, suppresses unchanged
//! observations, and publishes the standard [`MarketEvent`] stream.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tokio::sync::{RwLock, mpsc, watch};
use tokio::time::{Instant, MissedTickBehavior};

use super::json::{
    optional_string, protocol_error, required_decimal, required_string, required_u64,
};
use super::time::eastern_naive_to_utc;
use crate::connector::{
    BoxFuture, ConnectorCommand, MarketDataConnector, apply_command, event_matches,
};
use crate::error::FeedError;
use crate::model::{
    ChannelKind, InstrumentId, MarketEvent, QuoteTick, Subscription, TradeTick, Venue,
};
use crate::util::now_utc;

/// Provider name attached to REST and polling errors.
const REST_PROVIDER: &str = "nasdaq-ncds-rest";

/// Accepted NCDS data sources.
const SOURCES: [&str; 4] = ["Nasdaq", "BX", "PSX", "CQT"];

/// Accepted NCDS latency offsets.
const OFFSETS: [&str; 2] = ["realtime", "delayed"];

/// Accepted NCDS bar date ranges.
const BAR_RANGES: [&str; 9] = ["1d", "5d", "1m", "3m", "6m", "1y", "5y", "max", "ytd"];

/// Configuration for [`NcdsRestClient`].
///
/// `source` and `offset` are not part of the five required transport settings
/// but select the NCDS data source and latency tier; [`NcdsRestConfig::new`]
/// defaults them to `Nasdaq` and `realtime`.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use kurumi_feed::providers::nasdaq::NcdsRestConfig;
///
/// let config = NcdsRestConfig::new(
///     "http://127.0.0.1:8080",
///     "http://127.0.0.1:8080/v1/auth/token",
///     "client",
///     "secret",
///     Duration::from_secs(1),
/// );
/// assert!(config.validate().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NcdsRestConfig {
    /// Base URL of the NCDS deployment, without the API version suffix.
    pub base_url: String,
    /// Full URL of the `POST /v1/auth/token` endpoint.
    pub token_url: String,
    /// NCDS data source: `Nasdaq`, `BX`, `PSX`, or `CQT`.
    pub source: String,
    /// Data latency tier: `realtime` or `delayed`.
    pub offset: String,
    /// OAuth 2.0 client identifier.
    pub client_id: String,
    /// OAuth 2.0 client secret.
    pub client_secret: String,
    /// Delay between Last Sale and Last Quote polls.
    pub poll_interval: Duration,
}

impl NcdsRestConfig {
    /// Creates a configuration with the `Nasdaq` realtime source.
    #[must_use]
    pub fn new(
        base_url: impl Into<String>,
        token_url: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        poll_interval: Duration,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            token_url: token_url.into(),
            source: "Nasdaq".to_owned(),
            offset: "realtime".to_owned(),
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            poll_interval,
        }
    }

    /// Returns the venue that corresponds to the configured data source.
    ///
    /// `Nasdaq` maps to [`Venue::Nasdaq`], `CQT` to [`Venue::Sip`], and the
    /// remaining sources to [`Venue::Custom`] with their two-letter code.
    #[must_use]
    pub fn venue(&self) -> Venue {
        match self.source.as_str() {
            "Nasdaq" => Venue::Nasdaq,
            "CQT" => Venue::Sip,
            source => Venue::Custom(source.to_owned()),
        }
    }

    /// Validates the configuration.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] for an empty or non-HTTP URL, an
    /// empty credential, an unsupported source/offset, or a zero poll interval.
    pub fn validate(&self) -> Result<(), FeedError> {
        for (name, url) in [("base_url", &self.base_url), ("token_url", &self.token_url)] {
            if url.trim().is_empty() {
                return Err(invalid_config(format!("{name} must not be empty")));
            }
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err(invalid_config(format!(
                    "{name} must be an http:// or https:// URL"
                )));
            }
        }
        if self.client_id.is_empty() || self.client_secret.is_empty() {
            return Err(invalid_config(
                "client_id and client_secret must not be empty",
            ));
        }
        if !SOURCES.contains(&self.source.as_str()) {
            return Err(invalid_config(format!(
                "source `{}` is not one of {SOURCES:?}",
                self.source
            )));
        }
        if !OFFSETS.contains(&self.offset.as_str()) {
            return Err(invalid_config(format!(
                "offset `{}` is not one of {OFFSETS:?}",
                self.offset
            )));
        }
        if self.poll_interval.is_zero() {
            return Err(invalid_config("poll_interval must be strictly positive"));
        }
        Ok(())
    }
}

/// One observation from the Last Sale resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastSale {
    /// Ticker the sale belongs to.
    pub symbol: String,
    /// Sale price in the quote currency.
    pub price: Decimal,
    /// Number of shares sold.
    pub size: Decimal,
    /// Sale condition codes reported by Nasdaq.
    pub conditions: String,
    /// Venue code reported by Nasdaq.
    pub exchange: String,
    /// Exchange timestamp converted to UTC.
    pub exchange_ts: DateTime<Utc>,
}

/// One observation from the Last Quote resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastQuote {
    /// Ticker the quote belongs to.
    pub symbol: String,
    /// Best bid price.
    pub bid_price: Decimal,
    /// Size available at the best bid.
    pub bid_size: Decimal,
    /// Best ask price.
    pub ask_price: Decimal,
    /// Size available at the best ask.
    pub ask_size: Decimal,
    /// Exchange timestamp converted to UTC.
    pub exchange_ts: DateTime<Utc>,
}

/// One observation from the Snapshot resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NcdsSnapshot {
    /// Ticker the snapshot describes.
    pub symbol: String,
    /// Session open.
    pub open: Decimal,
    /// Session high.
    pub high: Decimal,
    /// Session low.
    pub low: Decimal,
    /// Session close so far.
    pub close: Decimal,
    /// Most recent last-sale price.
    pub last_sale: Decimal,
    /// Most recent trade price.
    pub last_trade: Decimal,
    /// Previous session close.
    pub previous_close: Decimal,
    /// Consolidated session volume.
    pub volume: Decimal,
    /// Consolidated session dollar volume.
    pub dollar_volume: Decimal,
    /// Net change from the previous close.
    pub net_change: Decimal,
    /// Percent change from the previous close.
    pub percent_change: Decimal,
    /// Exchange timestamp converted to UTC.
    pub exchange_ts: DateTime<Utc>,
}

/// One OHLCV bar from the Bars resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NcdsBar {
    /// Bar start timestamp converted to UTC.
    pub exchange_ts: DateTime<Utc>,
    /// Bar open.
    pub open: Decimal,
    /// Bar high.
    pub high: Decimal,
    /// Bar low.
    pub low: Decimal,
    /// Bar close.
    pub close: Decimal,
    /// Bar volume.
    pub volume: Decimal,
}

/// A bar precision accepted by the NCDS Bars resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NcdsBarPrecision {
    /// One-minute bars.
    OneMinute,
    /// Five-minute bars.
    FiveMinute,
    /// Ten-minute bars.
    TenMinute,
    /// Fifteen-minute bars.
    FifteenMinute,
    /// Thirty-minute bars.
    ThirtyMinute,
    /// Daily bars.
    OneDay,
    /// Weekly bars.
    OneWeek,
    /// Monthly bars.
    OneMonth,
}

impl NcdsBarPrecision {
    /// Returns the wire spelling of the precision.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OneMinute => "1minute",
            Self::FiveMinute => "5minute",
            Self::TenMinute => "10minute",
            Self::FifteenMinute => "15minute",
            Self::ThirtyMinute => "30minute",
            Self::OneDay => "1day",
            Self::OneWeek => "1week",
            Self::OneMonth => "1month",
        }
    }
}

/// A cached OAuth 2.0 access token.
#[derive(Debug)]
struct CachedToken {
    access_token: String,
    expires_at: Instant,
    refresh_margin: Duration,
}

impl CachedToken {
    /// Returns `true` when the token stays valid for the refresh margin.
    fn is_valid(&self) -> bool {
        Instant::now() + self.refresh_margin < self.expires_at
    }
}

/// An authenticated NCDS REST client with automatic token refresh.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use kurumi_feed::providers::nasdaq::{NcdsRestClient, NcdsRestConfig};
///
/// let config = NcdsRestConfig::new(
///     "https://example.com",
///     "https://example.com/v1/auth/token",
///     "client",
///     "secret",
///     Duration::from_secs(1),
/// );
/// let client = NcdsRestClient::new(config).expect("valid configuration");
/// assert_eq!(client.config().source, "Nasdaq");
/// ```
pub struct NcdsRestClient {
    config: NcdsRestConfig,
    http: reqwest::Client,
    token: RwLock<Option<CachedToken>>,
}

impl std::fmt::Debug for NcdsRestClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NcdsRestClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl NcdsRestClient {
    /// Creates a client after validating `config`.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] when the configuration is invalid or
    /// the HTTP client cannot be built.
    pub fn new(config: NcdsRestConfig) -> Result<Self, FeedError> {
        config.validate()?;
        let http = reqwest::Client::builder()
            .build()
            .map_err(|error| invalid_config(format!("failed to build the HTTP client: {error}")))?;
        Ok(Self {
            config,
            http,
            token: RwLock::new(None),
        })
    }

    /// Returns the client configuration.
    #[must_use]
    pub fn config(&self) -> &NcdsRestConfig {
        &self.config
    }

    /// Fetches the latest eligible sale for one or more symbols.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::RateLimited`] on HTTP 429, an authentication error
    /// on HTTP 401/403, and [`FeedError::ProtocolError`] for a malformed body.
    pub async fn fetch_last_sale(&self, symbols: &[String]) -> Result<Vec<LastSale>, FeedError> {
        let symbols = self.symbols_path(symbols)?;
        let url = self.endpoint(&format!(
            "/v1/{}/{}/equities/lastsale/{symbols}",
            self.config.source, self.config.offset
        ))?;
        let value = self.get_json(url).await?;
        let items = value.as_array().ok_or_else(|| {
            protocol_error(REST_PROVIDER, "Last Sale response must be a JSON array")
        })?;
        items
            .iter()
            .map(parse_last_sale)
            .collect::<Result<Vec<_>, _>>()
    }

    /// Fetches the latest eligible quote for one or more symbols.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`NcdsRestClient::fetch_last_sale`].
    pub async fn fetch_last_quote(&self, symbols: &[String]) -> Result<Vec<LastQuote>, FeedError> {
        let symbols = self.symbols_path(symbols)?;
        let url = self.endpoint(&format!(
            "/v1/{}/{}/equities/lastquote/{symbols}",
            self.config.source, self.config.offset
        ))?;
        let value = self.get_json(url).await?;
        let items = value.as_array().ok_or_else(|| {
            protocol_error(REST_PROVIDER, "Last Quote response must be a JSON array")
        })?;
        items
            .iter()
            .map(parse_last_quote)
            .collect::<Result<Vec<_>, _>>()
    }

    /// Fetches a consolidated session snapshot for one or more symbols.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`NcdsRestClient::fetch_last_sale`].
    pub async fn fetch_snapshot(&self, symbols: &[String]) -> Result<Vec<NcdsSnapshot>, FeedError> {
        let symbols = self.symbols_path(symbols)?;
        let url = self.endpoint(&format!(
            "/v1/{}/{}/equities/snapshot/{symbols}",
            self.config.source, self.config.offset
        ))?;
        let value = self.get_json(url).await?;
        let items = value.as_array().ok_or_else(|| {
            protocol_error(REST_PROVIDER, "Snapshot response must be a JSON array")
        })?;
        items
            .iter()
            .map(parse_snapshot)
            .collect::<Result<Vec<_>, _>>()
    }

    /// Fetches unadjusted bars for one symbol.
    ///
    /// `range` is one of `1d`, `5d`, `1m`, `3m`, `6m`, `1y`, `5y`, `max`, or
    /// `ytd`. The Bars resource is version 2 and currently only supports
    /// unadjusted prices, which this method always requests.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] for an invalid symbol or range and
    /// the same transport errors as [`NcdsRestClient::fetch_last_sale`].
    pub async fn fetch_bars(
        &self,
        symbol: &str,
        precision: NcdsBarPrecision,
        range: &str,
    ) -> Result<Vec<NcdsBar>, FeedError> {
        validate_symbol(symbol)?;
        if !BAR_RANGES.contains(&range) {
            return Err(invalid_config(format!(
                "bar range `{range}` is not one of {BAR_RANGES:?}"
            )));
        }
        let url = self.endpoint(&format!(
            "/v2/{}/{}/equities/bars/{symbol}/{}/false/{range}",
            self.config.source,
            self.config.offset,
            precision.as_str()
        ))?;
        let value = self.get_json(url).await?;
        let items = value.get(symbol).and_then(Value::as_array).ok_or_else(|| {
            protocol_error(
                REST_PROVIDER,
                format!("Bars response is missing the `{symbol}` array"),
            )
        })?;
        items.iter().map(parse_bar).collect::<Result<Vec<_>, _>>()
    }

    /// Returns a cached access token, refreshing it before expiry.
    async fn access_token(&self) -> Result<String, FeedError> {
        {
            let guard = self.token.read().await;
            if let Some(token) = guard.as_ref().filter(|token| token.is_valid()) {
                return Ok(token.access_token.clone());
            }
        }
        let mut guard = self.token.write().await;
        if let Some(token) = guard.as_ref().filter(|token| token.is_valid()) {
            return Ok(token.access_token.clone());
        }
        let token = self.request_token().await?;
        let access_token = token.access_token.clone();
        *guard = Some(token);
        Ok(access_token)
    }

    /// Requests a fresh access token with the client-credentials grant.
    async fn request_token(&self) -> Result<CachedToken, FeedError> {
        let url = parse_url(&self.config.token_url, "token_url")?;
        let response = self
            .http
            .post(url)
            .json(&json!({
                "client_id": self.config.client_id,
                "client_secret": self.config.client_secret,
            }))
            .send()
            .await
            .map_err(|error| FeedError::ConnectionFailed {
                provider: REST_PROVIDER.to_owned(),
                reason: error.to_string(),
            })?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(FeedError::AuthenticationFailed {
                provider: REST_PROVIDER.to_owned(),
                reason: format!("token endpoint returned HTTP {status}"),
            });
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(FeedError::RateLimited {
                provider: REST_PROVIDER.to_owned(),
                retry_after_ms: retry_after_ms(&response),
            });
        }
        if !status.is_success() {
            return Err(FeedError::ConnectionFailed {
                provider: REST_PROVIDER.to_owned(),
                reason: format!("token endpoint returned HTTP {status}"),
            });
        }
        let value: Value = response.json().await.map_err(|error| {
            protocol_error(REST_PROVIDER, format!("invalid token response: {error}"))
        })?;
        let access_token = required_string(&value, "access_token", REST_PROVIDER)?;
        let expires_in = required_u64(&value, "expires_in", REST_PROVIDER)?;
        let refresh_margin = Duration::from_secs((expires_in / 10).min(30));
        // Cap the advertised lifetime so a malformed server response cannot
        // overflow the monotonic clock when the expiry is computed.
        let lifetime = Duration::from_secs(expires_in.min(7 * 86_400));
        let expires_at = Instant::now() + lifetime;
        Ok(CachedToken {
            access_token,
            expires_at,
            refresh_margin,
        })
    }

    /// Performs an authenticated GET and decodes the JSON response.
    async fn get_json(&self, url: reqwest::Url) -> Result<Value, FeedError> {
        let token = self.access_token().await?;
        let response = self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| FeedError::ConnectionFailed {
                provider: REST_PROVIDER.to_owned(),
                reason: error.to_string(),
            })?;
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(FeedError::RateLimited {
                provider: REST_PROVIDER.to_owned(),
                retry_after_ms: retry_after_ms(&response),
            });
        }
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(FeedError::AuthenticationFailed {
                provider: REST_PROVIDER.to_owned(),
                reason: format!("REST endpoint returned HTTP {status}"),
            });
        }
        if !status.is_success() {
            return Err(protocol_error(
                REST_PROVIDER,
                format!("REST endpoint returned HTTP {status}"),
            ));
        }
        response.json().await.map_err(|error| {
            protocol_error(REST_PROVIDER, format!("invalid JSON response: {error}"))
        })
    }

    /// Builds a URL below the configured base URL.
    fn endpoint(&self, path: &str) -> Result<reqwest::Url, FeedError> {
        let base = self.config.base_url.trim_end_matches('/');
        parse_url(&format!("{base}{path}"), "base_url")
    }

    /// Validates and joins a symbol list into a path segment.
    fn symbols_path(&self, symbols: &[String]) -> Result<String, FeedError> {
        if symbols.is_empty() {
            return Err(invalid_config("at least one symbol is required"));
        }
        for symbol in symbols {
            validate_symbol(symbol)?;
        }
        Ok(symbols.join(","))
    }
}

/// Deduplication keys for polling.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SaleKey {
    exchange_ts: DateTime<Utc>,
    price: Decimal,
    size: Decimal,
}

impl From<&LastSale> for SaleKey {
    fn from(sale: &LastSale) -> Self {
        Self {
            exchange_ts: sale.exchange_ts,
            price: sale.price,
            size: sale.size,
        }
    }
}

/// Deduplication key for a quote.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct QuoteKey {
    exchange_ts: DateTime<Utc>,
    bid_price: Decimal,
    bid_size: Decimal,
    ask_price: Decimal,
    ask_size: Decimal,
}

impl From<&LastQuote> for QuoteKey {
    fn from(quote: &LastQuote) -> Self {
        Self {
            exchange_ts: quote.exchange_ts,
            bid_price: quote.bid_price,
            bid_size: quote.bid_size,
            ask_price: quote.ask_price,
            ask_size: quote.ask_size,
        }
    }
}

/// Per-symbol observations already published downstream.
#[derive(Debug, Default)]
struct LastSeen {
    sales: BTreeMap<String, SaleKey>,
    quotes: BTreeMap<String, QuoteKey>,
}

/// Polls NCDS Last Sale and Last Quote into the standard event stream.
///
/// Only observations whose timestamp, price, or size changed since the previous
/// poll are emitted. Quote changes compare both sides and both sizes. Bars
/// subscriptions are ignored because the polling connector covers Last Sale and
/// Last Quote only; use [`NcdsRestClient::fetch_bars`] for historical bars.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use kurumi_feed::MarketDataConnector;
/// use kurumi_feed::providers::nasdaq::{NcdsPollingConnector, NcdsRestConfig};
///
/// let config = NcdsRestConfig::new(
///     "https://example.com",
///     "https://example.com/v1/auth/token",
///     "client",
///     "secret",
///     Duration::from_secs(1),
/// );
/// let connector = NcdsPollingConnector::new("ncds", config).expect("valid configuration");
/// assert_eq!(connector.name(), "ncds");
/// ```
pub struct NcdsPollingConnector {
    name: String,
    client: NcdsRestClient,
    venues: Vec<Venue>,
    seen: Mutex<LastSeen>,
}

impl std::fmt::Debug for NcdsPollingConnector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NcdsPollingConnector")
            .field("name", &self.name)
            .field("venues", &self.venues)
            .finish_non_exhaustive()
    }
}

impl NcdsPollingConnector {
    /// Creates a polling connector after validating `config`.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] for an empty name or invalid config.
    pub fn new(name: impl Into<String>, config: NcdsRestConfig) -> Result<Self, FeedError> {
        let name = name.into();
        if name.is_empty() {
            return Err(invalid_config("NCDS connector name must not be empty"));
        }
        let venues = vec![config.venue()];
        Ok(Self {
            name,
            client: NcdsRestClient::new(config)?,
            venues,
            seen: Mutex::new(LastSeen::default()),
        })
    }

    /// Overrides the venue attached to emitted events.
    ///
    /// The first entry is used for instruments and is also advertised through
    /// [`MarketDataConnector::supported_venues`].
    #[must_use]
    pub fn with_venues(mut self, venues: Vec<Venue>) -> Self {
        self.venues = venues;
        self
    }

    /// Returns the underlying REST client.
    #[must_use]
    pub fn client(&self) -> &NcdsRestClient {
        &self.client
    }

    /// Polls once and returns only the events that changed since the last call.
    ///
    /// # Errors
    ///
    /// Propagates the REST errors from [`NcdsRestClient::fetch_last_sale`] and
    /// [`NcdsRestClient::fetch_last_quote`].
    pub async fn poll_once(
        &self,
        active: &BTreeSet<Subscription>,
    ) -> Result<Vec<MarketEvent>, FeedError> {
        let mut trade_symbols = BTreeSet::new();
        let mut quote_symbols = BTreeSet::new();
        for subscription in active {
            for channel in &subscription.channels {
                match channel {
                    ChannelKind::Trades => {
                        trade_symbols.insert(subscription.instrument.symbol.clone());
                    }
                    ChannelKind::Quotes => {
                        quote_symbols.insert(subscription.instrument.symbol.clone());
                    }
                    ChannelKind::OrderBook | ChannelKind::Bars { .. } => {}
                }
            }
        }

        let venue = self.venues.first().cloned().unwrap_or(Venue::Nasdaq);
        let mut events = Vec::new();
        if !trade_symbols.is_empty() {
            let symbols: Vec<String> = trade_symbols.into_iter().collect();
            let mut sales = self.client.fetch_last_sale(&symbols).await?;
            sales.sort_by(|left, right| left.symbol.cmp(&right.symbol));
            let mut seen = self.lock_seen()?;
            for sale in sales {
                let key = SaleKey::from(&sale);
                if seen.sales.get(&sale.symbol) == Some(&key) {
                    continue;
                }
                seen.sales.insert(sale.symbol.clone(), key);
                events.push(MarketEvent::Trade(TradeTick {
                    instrument: InstrumentId::new(sale.symbol, venue.clone()),
                    price: sale.price,
                    size: sale.size,
                    trade_id: None,
                    conditions: if sale.conditions.is_empty() {
                        Vec::new()
                    } else {
                        vec![sale.conditions]
                    },
                    exchange_ts: sale.exchange_ts,
                    received_ts: now_utc(),
                }));
            }
        }
        if !quote_symbols.is_empty() {
            let symbols: Vec<String> = quote_symbols.into_iter().collect();
            let mut quotes = self.client.fetch_last_quote(&symbols).await?;
            quotes.sort_by(|left, right| left.symbol.cmp(&right.symbol));
            let mut seen = self.lock_seen()?;
            for quote in quotes {
                let key = QuoteKey::from(&quote);
                if seen.quotes.get(&quote.symbol) == Some(&key) {
                    continue;
                }
                seen.quotes.insert(quote.symbol.clone(), key);
                events.push(MarketEvent::Quote(QuoteTick {
                    instrument: InstrumentId::new(quote.symbol, venue.clone()),
                    bid_price: quote.bid_price,
                    bid_size: quote.bid_size,
                    ask_price: quote.ask_price,
                    ask_size: quote.ask_size,
                    exchange_ts: quote.exchange_ts,
                    received_ts: now_utc(),
                }));
            }
        }
        Ok(events)
    }

    /// Locks the deduplication state, translating poisoning into a typed error.
    fn lock_seen(&self) -> Result<MutexGuard<'_, LastSeen>, FeedError> {
        self.seen.lock().map_err(|_| FeedError::ProtocolError {
            provider: self.name.clone(),
            reason: "NCDS deduplication state mutex is poisoned".to_owned(),
        })
    }
}

impl MarketDataConnector for NcdsPollingConnector {
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
        Box::pin(self.run(initial_subs, tx, cmd_rx, shutdown))
    }
}

impl NcdsPollingConnector {
    /// Runs the polling session until shutdown or a terminal error.
    async fn run(
        &self,
        initial_subs: Vec<Subscription>,
        tx: mpsc::Sender<MarketEvent>,
        mut cmd_rx: mpsc::Receiver<ConnectorCommand>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), FeedError> {
        let mut active: BTreeSet<Subscription> = initial_subs.into_iter().collect();
        let mut ticker = tokio::time::interval(self.client.config().poll_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Ok(());
                    }
                }
                command = cmd_rx.recv() => {
                    match command {
                        Some(command) => apply_command(&mut active, &command),
                        None => return Ok(()),
                    }
                }
                _ = ticker.tick() => {
                    let events = self.poll_once(&active).await?;
                    for event in events {
                        if !event_matches(&active, &event) {
                            continue;
                        }
                        if !send_event(&tx, event, &mut shutdown).await {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
}

/// Parses one Last Sale object.
fn parse_last_sale(value: &Value) -> Result<LastSale, FeedError> {
    Ok(LastSale {
        symbol: required_string(value, "symbol", REST_PROVIDER)?,
        price: required_decimal(value, "price", REST_PROVIDER)?,
        size: required_decimal(value, "size", REST_PROVIDER)?,
        conditions: optional_string(value, "conditions").unwrap_or_default(),
        exchange: optional_string(value, "exchange").unwrap_or_default(),
        exchange_ts: parse_eastern_timestamp(value, "timestamp")?,
    })
}

/// Parses one Last Quote object.
fn parse_last_quote(value: &Value) -> Result<LastQuote, FeedError> {
    Ok(LastQuote {
        symbol: required_string(value, "symbol", REST_PROVIDER)?,
        bid_price: required_decimal(value, "bidPrice", REST_PROVIDER)?,
        bid_size: required_decimal(value, "bidSize", REST_PROVIDER)?,
        ask_price: required_decimal(value, "askPrice", REST_PROVIDER)?,
        ask_size: required_decimal(value, "askSize", REST_PROVIDER)?,
        exchange_ts: parse_eastern_timestamp(value, "timestamp")?,
    })
}

/// Parses one Snapshot object.
fn parse_snapshot(value: &Value) -> Result<NcdsSnapshot, FeedError> {
    Ok(NcdsSnapshot {
        symbol: required_string(value, "symbol", REST_PROVIDER)?,
        open: required_decimal(value, "open", REST_PROVIDER)?,
        high: required_decimal(value, "high", REST_PROVIDER)?,
        low: required_decimal(value, "low", REST_PROVIDER)?,
        close: required_decimal(value, "close", REST_PROVIDER)?,
        last_sale: required_decimal(value, "lastSale", REST_PROVIDER)?,
        last_trade: required_decimal(value, "lastTrade", REST_PROVIDER)?,
        previous_close: required_decimal(value, "previousClose", REST_PROVIDER)?,
        volume: required_decimal(value, "volume", REST_PROVIDER)?,
        dollar_volume: required_decimal(value, "dollarVolume", REST_PROVIDER)?,
        net_change: required_decimal(value, "netChange", REST_PROVIDER)?,
        percent_change: required_decimal(value, "percentChange", REST_PROVIDER)?,
        exchange_ts: parse_eastern_timestamp(value, "timestamp")?,
    })
}

/// Parses one bar object from the Bars response.
fn parse_bar(value: &Value) -> Result<NcdsBar, FeedError> {
    Ok(NcdsBar {
        exchange_ts: parse_eastern_timestamp(value, "t")?,
        open: required_decimal(value, "o", REST_PROVIDER)?,
        high: required_decimal(value, "h", REST_PROVIDER)?,
        low: required_decimal(value, "l", REST_PROVIDER)?,
        close: required_decimal(value, "c", REST_PROVIDER)?,
        volume: required_decimal(value, "v", REST_PROVIDER)?,
    })
}

/// Parses an NCDS US Eastern timestamp or, defensively, an RFC 3339 offset.
fn parse_eastern_timestamp(value: &Value, name: &str) -> Result<DateTime<Utc>, FeedError> {
    let text = required_string(value, name, REST_PROVIDER)?;
    if let Ok(parsed) = DateTime::parse_from_rfc3339(&text) {
        return Ok(parsed.with_timezone(&Utc));
    }
    let naive = NaiveDateTime::parse_from_str(&text, "%Y-%m-%dT%H:%M:%S%.f").map_err(|error| {
        protocol_error(
            REST_PROVIDER,
            format!("field `{name}` is not a valid NCDS timestamp: {error}"),
        )
    })?;
    eastern_naive_to_utc(naive).map_err(|error| protocol_error(REST_PROVIDER, error.to_string()))
}

/// Validates a ticker accepted in a URL path segment.
fn validate_symbol(symbol: &str) -> Result<(), FeedError> {
    if symbol.is_empty() {
        return Err(invalid_config("symbol must not be empty"));
    }
    if !symbol
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'^'))
    {
        return Err(invalid_config(format!(
            "symbol `{symbol}` contains characters that are not valid in an NCDS path"
        )));
    }
    Ok(())
}

/// Parses an absolute HTTP URL with a contextual error message.
fn parse_url(text: &str, field: &str) -> Result<reqwest::Url, FeedError> {
    reqwest::Url::parse(text)
        .map_err(|error| invalid_config(format!("{field} is not a valid URL: {error}")))
}

/// Reads a `Retry-After` header in milliseconds, defaulting to one second.
fn retry_after_ms(response: &reqwest::Response) -> u64 {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map_or(1_000, |seconds| seconds.saturating_mul(1_000))
}

/// Builds an invalid-configuration error.
fn invalid_config(reason: impl Into<String>) -> FeedError {
    FeedError::InvalidConfig {
        reason: reason.into(),
    }
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

    #[test]
    fn config_requires_http_urls_and_non_empty_credentials() {
        let mut config = NcdsRestConfig::new(
            "https://example.com",
            "https://example.com/v1/auth/token",
            "client",
            "secret",
            Duration::from_secs(1),
        );
        assert!(config.validate().is_ok());
        config.base_url = "ftp://example.com".to_owned();
        assert!(config.validate().is_err());
    }

    #[test]
    fn source_maps_to_a_venue() {
        let mut config = NcdsRestConfig::new(
            "https://example.com",
            "https://example.com/v1/auth/token",
            "client",
            "secret",
            Duration::from_secs(1),
        );
        assert_eq!(config.venue(), Venue::Nasdaq);
        config.source = "CQT".to_owned();
        assert_eq!(config.venue(), Venue::Sip);
        config.source = "BX".to_owned();
        assert_eq!(config.venue(), Venue::Custom("BX".to_owned()));
    }

    #[test]
    fn config_rejects_unknown_sources_and_offsets() {
        let mut config = NcdsRestConfig::new(
            "https://example.com",
            "https://example.com/v1/auth/token",
            "client",
            "secret",
            Duration::from_secs(1),
        );
        config.source = "Unknown".to_owned();
        assert!(config.validate().is_err());
        config.source = "Nasdaq".to_owned();
        config.offset = "nearline".to_owned();
        assert!(config.validate().is_err());
    }

    #[test]
    fn cached_tokens_refresh_inside_the_margin() {
        let healthy = CachedToken {
            access_token: "healthy".to_owned(),
            expires_at: Instant::now() + Duration::from_secs(60),
            refresh_margin: Duration::from_secs(30),
        };
        assert!(healthy.is_valid());
        let expiring = CachedToken {
            access_token: "expiring".to_owned(),
            expires_at: Instant::now() + Duration::from_secs(10),
            refresh_margin: Duration::from_secs(30),
        };
        assert!(!expiring.is_valid());
    }

    #[test]
    fn last_sale_decodes_decimal_prices_and_eastern_timestamps() {
        let value = serde_json::json!({
            "symbol": "AAPL",
            "timestamp": "2024-11-20T09:30:01.383",
            "price": 187.5123,
            "size": 10,
            "conditions": "@ To",
            "exchange": "Q"
        });
        let sale = parse_last_sale(&value).expect("valid sale");
        assert_eq!(sale.price, Decimal::new(1_875_123, 4));
        assert_eq!(sale.size, Decimal::new(10, 0));
        assert_eq!(sale.exchange_ts.to_string(), "2024-11-20 14:30:01.383 UTC");
    }

    #[test]
    fn last_quote_decodes_both_sides() {
        let value = serde_json::json!({
            "symbol": "AAPL",
            "timestamp": "2024-11-20T09:30:01.383",
            "bidPrice": 187.5000,
            "bidSize": 3,
            "askPrice": 187.5100,
            "askSize": 4
        });
        let quote = parse_last_quote(&value).expect("valid quote");
        assert_eq!(quote.bid_price, Decimal::new(1_875_000, 4));
        assert_eq!(quote.ask_price, Decimal::new(1_875_100, 4));
    }

    #[tokio::test]
    async fn bars_require_a_known_range() {
        let config = NcdsRestConfig::new(
            "https://example.com",
            "https://example.com/v1/auth/token",
            "client",
            "secret",
            Duration::from_secs(1),
        );
        let client = NcdsRestClient::new(config).expect("valid client");
        let result = client
            .fetch_bars("AAPL", NcdsBarPrecision::OneMinute, "2d")
            .await;
        assert!(matches!(result, Err(FeedError::InvalidConfig { .. })));
    }

    #[test]
    fn symbols_reject_path_injection() {
        assert!(validate_symbol("AAPL").is_ok());
        assert!(validate_symbol("BRK.B").is_ok());
        assert!(validate_symbol("../etc").is_err());
    }
}
