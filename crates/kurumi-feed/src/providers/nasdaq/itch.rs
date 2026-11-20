//! Native Nasdaq TotalView-ITCH 5.0 decoding and replay streaming.
//!
//! [`ItchDecoder`] decodes one length-delimited ITCH message at a time using
//! the official big-endian field offsets from the TotalView-ITCH 5.0
//! specification. Buffers are only bounds-checked and read in place; the
//! decoder never copies a message or allocates a per-field `Vec`. Prices are
//! decoded straight into [`Decimal`] with four implied decimal places and
//! timestamps are converted from the 48-bit nanosecond-since-midnight field to
//! a UTC [`DateTime`] using the documented US Eastern daylight-saving rule.
//!
//! # Supported messages
//!
//! | Type | Message | Produced event |
//! | --- | --- | --- |
//! | `S` | System Event | [`MarketEvent::Status`] |
//! | `R` | Stock Directory | none; updates the locate-to-symbol map |
//! | `A` / `F` | Add Order (no MPID / MPID) | [`MarketEvent::OrderBookDelta`] |
//! | `E` | Order Executed | [`MarketEvent::Trade`] |
//! | `C` | Order Executed With Price | [`MarketEvent::Trade`] when printable |
//! | `P` | Trade (Non-Cross) | [`MarketEvent::Trade`] |
//! | `X` | Order Cancel | [`MarketEvent::OrderBookDelta`] |
//! | `D` | Order Delete | [`MarketEvent::OrderBookDelta`] |
//! | `U` | Order Replace | two [`MarketEvent::OrderBookDelta`] values |
//!
//! Every other message type defined by the specification is skipped by its
//! official length. A type outside the specification, a short buffer, or an
//! inconsistent length is a [`FeedError::ProtocolError`]; a malformed frame can
//! therefore never panic a Tokio worker. Messages that reference an order not
//! seen earlier in the stream (for example a replay that starts mid-file) are
//! skipped when the execution or cancel price cannot be recovered.
//!
//! # Book semantics
//!
//! Add orders insert a tracked order and emit a delta with the displayed
//! quantity. Executions use the resting order's price, or the explicit
//! execution price for `C`. A cancel emits the order's remaining displayed
//! quantity (zero once fully cancelled); a delete emits zero to remove the
//! level; a replace emits a zero-size removal for the original reference
//! followed by an add for the new reference.

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::{mpsc, watch};

use super::time::timestamp_from_nanos_since_midnight;
use crate::connector::{
    BoxFuture, ConnectorCommand, MarketDataConnector, apply_command, event_matches,
};
use crate::error::FeedError;
use crate::model::{
    BookDelta, BookSide, ConnectionState, FeedStatusEvent, InstrumentId, MarketEvent, Subscription,
    TradeTick, Venue,
};
use crate::util::now_utc;

/// Provider name used by the decoder when no override is supplied.
const DEFAULT_PROVIDER: &str = "nasdaq-itch";

/// Number of bytes in a 48-bit ITCH timestamp.
const TIMESTAMP_LEN: usize = 6;

/// The official total length of each TotalView-ITCH 5.0 message type.
///
/// The table is used both to validate a decoded message and to skip auxiliary
/// messages that Kurumi does not project onto its event model.
fn message_length(message_type: u8) -> Option<usize> {
    match message_type {
        b'S' => Some(12),
        b'R' => Some(39),
        b'H' => Some(25),
        b'Y' => Some(20),
        b'L' => Some(26),
        b'V' => Some(35),
        b'W' => Some(12),
        b'K' => Some(28),
        b'J' => Some(35),
        b'h' => Some(21),
        b'A' => Some(36),
        b'F' => Some(40),
        b'E' => Some(31),
        b'C' => Some(36),
        b'X' => Some(23),
        b'D' => Some(19),
        b'U' => Some(35),
        b'P' => Some(44),
        b'Q' => Some(40),
        b'B' => Some(19),
        b'I' => Some(50),
        b'N' => Some(20),
        // Direct Listing with Capital Raise price discovery (ITCH 5.0, 2023).
        b'O' => Some(48),
        _ => None,
    }
}

/// A resting displayable order tracked so executions and cancels can be priced.
#[derive(Debug, Clone)]
struct RestingOrder {
    /// Ticker the order belongs to.
    symbol: String,
    /// Side of the book.
    side: BookSide,
    /// Display price of the order.
    price: Decimal,
    /// Remaining displayed quantity.
    remaining: Decimal,
}

/// A stateful TotalView-ITCH 5.0 binary decoder.
///
/// The decoder owns the per-day locate-to-ticker mapping established by Stock
/// Directory messages and the resting-order table needed to price executions
/// and cancels. A new trading day requires a new decoder with the new trading
/// date.
///
/// # Examples
///
/// ```
/// use chrono::NaiveDate;
/// use kurumi_feed::providers::nasdaq::ItchDecoder;
///
/// let date = NaiveDate::from_ymd_opt(2024, 11, 20).expect("valid date");
/// let mut decoder = ItchDecoder::new(date);
/// // Type `S`, locate 0, tracking 0, timestamp 0, event code `O`.
/// let frame = [b'S', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, b'O'];
/// let events = decoder.decode(&frame).expect("valid system event");
/// assert_eq!(events.len(), 1);
/// assert_eq!(decoder.trading_date(), date);
/// ```
#[derive(Debug)]
pub struct ItchDecoder {
    provider: String,
    trading_date: NaiveDate,
    stock_locate: HashMap<u16, String>,
    orders: HashMap<u64, RestingOrder>,
}

impl ItchDecoder {
    /// Creates a decoder for the given US Eastern trading date.
    ///
    /// Stock Directory messages are expected to establish the locate mapping
    /// before order messages refer to a locate code.
    #[must_use]
    pub fn new(trading_date: NaiveDate) -> Self {
        Self {
            provider: DEFAULT_PROVIDER.to_owned(),
            trading_date,
            stock_locate: HashMap::new(),
            orders: HashMap::new(),
        }
    }

    /// Overrides the provider name attached to lifecycle events and errors.
    #[must_use]
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = provider.into();
        self
    }

    /// Returns the trading date used to anchor ITCH timestamps.
    #[must_use]
    pub fn trading_date(&self) -> NaiveDate {
        self.trading_date
    }

    /// Returns the locate-to-ticker mapping built from Stock Directory messages.
    #[must_use]
    pub fn stock_locate(&self) -> &HashMap<u16, String> {
        &self.stock_locate
    }

    /// Returns the ticker assigned to a locate code, if a directory message has
    /// been seen for it.
    #[must_use]
    pub fn symbol_for(&self, stock_locate: u16) -> Option<&str> {
        self.stock_locate.get(&stock_locate).map(String::as_str)
    }

    /// Decodes exactly one ITCH message using the local receive time.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::ProtocolError`] for a truncated buffer, a message
    /// type outside the ITCH 5.0 specification, a length inconsistency, or a
    /// malformed timestamp, symbol, side, or price.
    pub fn decode(&mut self, payload: &[u8]) -> Result<Vec<MarketEvent>, FeedError> {
        let mut events = Vec::new();
        self.decode_into_at(payload, now_utc(), &mut events)?;
        Ok(events)
    }

    /// Decodes exactly one ITCH message with an explicit receive timestamp.
    ///
    /// This is the deterministic entry point used by tests and by replay
    /// adapters that need to preserve a recorded arrival time.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`ItchDecoder::decode`].
    pub fn decode_at(
        &mut self,
        payload: &[u8],
        received_ts: DateTime<Utc>,
    ) -> Result<Vec<MarketEvent>, FeedError> {
        let mut events = Vec::new();
        self.decode_into_at(payload, received_ts, &mut events)?;
        Ok(events)
    }

    /// Decodes one message into an existing event buffer.
    ///
    /// Reusing the caller's buffer avoids a per-frame allocation in a hot
    /// replay loop. On error, events already appended for this frame remain in
    /// `out`; callers that retry a frame should clear it first.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`ItchDecoder::decode`].
    pub fn decode_into_at(
        &mut self,
        payload: &[u8],
        received_ts: DateTime<Utc>,
        out: &mut Vec<MarketEvent>,
    ) -> Result<(), FeedError> {
        let Some(message_type) = payload.first().copied() else {
            return Err(self.protocol_error("empty ITCH message"));
        };
        let expected = message_length(message_type).ok_or_else(|| {
            self.protocol_error(format!("unknown ITCH message type 0x{message_type:02X}"))
        })?;
        if payload.len() != expected {
            return Err(self.protocol_error(format!(
                "ITCH message type `{}` has length {} but the specification requires {expected}",
                char::from(message_type),
                payload.len()
            )));
        }

        let stock_locate = read_u16(payload, 1).map_err(|reason| self.protocol_error(reason))?;
        let timestamp = read_u48(payload, 5).map_err(|reason| self.protocol_error(reason))?;
        let exchange_ts = timestamp_from_nanos_since_midnight(self.trading_date, timestamp)
            .map_err(|error| self.protocol_error(error.to_string()))?;

        match message_type {
            b'S' => self.on_system_event(payload, received_ts, out),
            b'R' => self.on_stock_directory(payload, stock_locate),
            b'A' | b'F' => self.on_add_order(payload, stock_locate, exchange_ts, received_ts, out),
            b'E' => self.on_order_executed(payload, exchange_ts, received_ts, out),
            b'C' => self.on_order_executed_with_price(
                payload,
                stock_locate,
                exchange_ts,
                received_ts,
                out,
            ),
            b'P' => self.on_non_cross_trade(payload, stock_locate, exchange_ts, received_ts, out),
            b'X' => self.on_order_cancel(payload, exchange_ts, received_ts, out),
            b'D' => self.on_order_delete(payload, exchange_ts, received_ts, out),
            b'U' => self.on_order_replace(payload, stock_locate, exchange_ts, received_ts, out),
            _ => Ok(()),
        }
    }

    /// Projects a System Event onto a lifecycle status.
    fn on_system_event(
        &self,
        payload: &[u8],
        received_ts: DateTime<Utc>,
        out: &mut Vec<MarketEvent>,
    ) -> Result<(), FeedError> {
        let code = read_u8(payload, 11).map_err(|reason| self.protocol_error(reason))?;
        let (state, description) = match code {
            b'O' => (ConnectionState::Connected, "start of messages"),
            b'S' => (ConnectionState::Authenticated, "start of system hours"),
            b'Q' => (ConnectionState::Authenticated, "start of market hours"),
            b'M' => (ConnectionState::Connected, "end of market hours"),
            b'E' => (ConnectionState::Disconnected, "end of system hours"),
            b'C' => (ConnectionState::Disconnected, "end of messages"),
            _ => {
                return Err(self.protocol_error(format!("unknown system event code 0x{code:02X}")));
            }
        };
        out.push(MarketEvent::Status(FeedStatusEvent {
            provider: self.provider.clone(),
            state,
            message: Some(description.to_owned()),
            ts: received_ts,
        }));
        Ok(())
    }

    /// Records a Stock Directory entry in the locate map.
    fn on_stock_directory(&mut self, payload: &[u8], stock_locate: u16) -> Result<(), FeedError> {
        let symbol = read_symbol(payload, 11).map_err(|reason| self.protocol_error(reason))?;
        if symbol.is_empty() {
            return Err(self.protocol_error("Stock Directory message carries an empty ticker"));
        }
        self.stock_locate.insert(stock_locate, symbol);
        Ok(())
    }

    /// Emits a book delta for an Add Order message and tracks the order.
    fn on_add_order(
        &mut self,
        payload: &[u8],
        stock_locate: u16,
        exchange_ts: DateTime<Utc>,
        received_ts: DateTime<Utc>,
        out: &mut Vec<MarketEvent>,
    ) -> Result<(), FeedError> {
        let order_ref = read_u64(payload, 11).map_err(|reason| self.protocol_error(reason))?;
        let side = read_side(payload, 19).map_err(|reason| self.protocol_error(reason))?;
        let shares = read_u32(payload, 20).map_err(|reason| self.protocol_error(reason))?;
        let price = read_price(payload, 32).map_err(|reason| self.protocol_error(reason))?;
        let symbol = match self.symbol_for(stock_locate) {
            Some(symbol) => symbol.to_owned(),
            None => read_symbol(payload, 24).map_err(|reason| self.protocol_error(reason))?,
        };
        if symbol.is_empty() {
            return Err(self.protocol_error("Add Order message carries an empty ticker"));
        }
        let size = Decimal::new(i64::from(shares), 0);
        self.orders.insert(
            order_ref,
            RestingOrder {
                symbol: symbol.clone(),
                side,
                price,
                remaining: size,
            },
        );
        out.push(MarketEvent::OrderBookDelta(BookDelta {
            instrument: InstrumentId::new(symbol, Venue::Nasdaq),
            side,
            price,
            size,
            order_ref: Some(order_ref),
            exchange_ts,
            received_ts,
        }));
        Ok(())
    }

    /// Emits a trade for an Order Executed message using the resting price.
    fn on_order_executed(
        &mut self,
        payload: &[u8],
        exchange_ts: DateTime<Utc>,
        received_ts: DateTime<Utc>,
        out: &mut Vec<MarketEvent>,
    ) -> Result<(), FeedError> {
        let order_ref = read_u64(payload, 11).map_err(|reason| self.protocol_error(reason))?;
        let executed = read_u32(payload, 19).map_err(|reason| self.protocol_error(reason))?;
        let match_number = read_u64(payload, 23).map_err(|reason| self.protocol_error(reason))?;
        let Some(order) = self.orders.get(&order_ref).cloned() else {
            return Ok(());
        };
        let quantity = Decimal::new(i64::from(executed), 0);
        self.reduce_order(order_ref, quantity)?;
        out.push(MarketEvent::Trade(TradeTick {
            instrument: InstrumentId::new(order.symbol, Venue::Nasdaq),
            price: order.price,
            size: quantity,
            trade_id: Some(match_number.to_string()),
            conditions: Vec::new(),
            exchange_ts,
            received_ts,
        }));
        Ok(())
    }

    /// Emits a trade for a printable Order Executed With Price message.
    fn on_order_executed_with_price(
        &mut self,
        payload: &[u8],
        stock_locate: u16,
        exchange_ts: DateTime<Utc>,
        received_ts: DateTime<Utc>,
        out: &mut Vec<MarketEvent>,
    ) -> Result<(), FeedError> {
        let order_ref = read_u64(payload, 11).map_err(|reason| self.protocol_error(reason))?;
        let executed = read_u32(payload, 19).map_err(|reason| self.protocol_error(reason))?;
        let match_number = read_u64(payload, 23).map_err(|reason| self.protocol_error(reason))?;
        let printable = read_u8(payload, 31).map_err(|reason| self.protocol_error(reason))?;
        if !matches!(printable, b'Y' | b'N') {
            return Err(self.protocol_error(format!(
                "Order Executed With Price has invalid printable flag 0x{printable:02X}"
            )));
        }
        let price = read_price(payload, 32).map_err(|reason| self.protocol_error(reason))?;
        let quantity = Decimal::new(i64::from(executed), 0);
        self.reduce_order(order_ref, quantity)?;
        if printable == b'N' {
            return Ok(());
        }
        let Some(symbol) = self.symbol_for(stock_locate).map(str::to_owned) else {
            return Ok(());
        };
        out.push(MarketEvent::Trade(TradeTick {
            instrument: InstrumentId::new(symbol, Venue::Nasdaq),
            price,
            size: quantity,
            trade_id: Some(match_number.to_string()),
            conditions: Vec::new(),
            exchange_ts,
            received_ts,
        }));
        Ok(())
    }

    /// Emits a book delta for a partial Order Cancel message.
    fn on_order_cancel(
        &mut self,
        payload: &[u8],
        exchange_ts: DateTime<Utc>,
        received_ts: DateTime<Utc>,
        out: &mut Vec<MarketEvent>,
    ) -> Result<(), FeedError> {
        let order_ref = read_u64(payload, 11).map_err(|reason| self.protocol_error(reason))?;
        let cancelled = read_u32(payload, 19).map_err(|reason| self.protocol_error(reason))?;
        let Some(order) = self.orders.get(&order_ref).cloned() else {
            return Ok(());
        };
        let quantity = Decimal::new(i64::from(cancelled), 0);
        self.reduce_order(order_ref, quantity)?;
        let remaining = self
            .orders
            .get(&order_ref)
            .map_or(Decimal::ZERO, |tracked| tracked.remaining);
        out.push(MarketEvent::OrderBookDelta(BookDelta {
            instrument: InstrumentId::new(order.symbol, Venue::Nasdaq),
            side: order.side,
            price: order.price,
            size: remaining,
            order_ref: Some(order_ref),
            exchange_ts,
            received_ts,
        }));
        Ok(())
    }

    /// Emits a zero-size book delta for an Order Delete message.
    fn on_order_delete(
        &mut self,
        payload: &[u8],
        exchange_ts: DateTime<Utc>,
        received_ts: DateTime<Utc>,
        out: &mut Vec<MarketEvent>,
    ) -> Result<(), FeedError> {
        let order_ref = read_u64(payload, 11).map_err(|reason| self.protocol_error(reason))?;
        let Some(order) = self.orders.remove(&order_ref) else {
            return Ok(());
        };
        out.push(MarketEvent::OrderBookDelta(BookDelta {
            instrument: InstrumentId::new(order.symbol, Venue::Nasdaq),
            side: order.side,
            price: order.price,
            size: Decimal::ZERO,
            order_ref: Some(order_ref),
            exchange_ts,
            received_ts,
        }));
        Ok(())
    }

    /// Emits the removal and re-add that model an Order Replace message.
    fn on_order_replace(
        &mut self,
        payload: &[u8],
        stock_locate: u16,
        exchange_ts: DateTime<Utc>,
        received_ts: DateTime<Utc>,
        out: &mut Vec<MarketEvent>,
    ) -> Result<(), FeedError> {
        let original_ref = read_u64(payload, 11).map_err(|reason| self.protocol_error(reason))?;
        let new_ref = read_u64(payload, 19).map_err(|reason| self.protocol_error(reason))?;
        let shares = read_u32(payload, 27).map_err(|reason| self.protocol_error(reason))?;
        let price = read_price(payload, 31).map_err(|reason| self.protocol_error(reason))?;
        let Some(original) = self.orders.remove(&original_ref) else {
            return Ok(());
        };
        let symbol = match self.symbol_for(stock_locate) {
            Some(symbol) => symbol.to_owned(),
            None => original.symbol.clone(),
        };
        let size = Decimal::new(i64::from(shares), 0);
        self.orders.insert(
            new_ref,
            RestingOrder {
                symbol: symbol.clone(),
                side: original.side,
                price,
                remaining: size,
            },
        );
        let instrument = InstrumentId::new(symbol, Venue::Nasdaq);
        out.push(MarketEvent::OrderBookDelta(BookDelta {
            instrument: instrument.clone(),
            side: original.side,
            price: original.price,
            size: Decimal::ZERO,
            order_ref: Some(original_ref),
            exchange_ts,
            received_ts,
        }));
        out.push(MarketEvent::OrderBookDelta(BookDelta {
            instrument,
            side: original.side,
            price,
            size,
            order_ref: Some(new_ref),
            exchange_ts,
            received_ts,
        }));
        Ok(())
    }

    /// Emits a trade for a Non-Cross Trade message.
    fn on_non_cross_trade(
        &self,
        payload: &[u8],
        stock_locate: u16,
        exchange_ts: DateTime<Utc>,
        received_ts: DateTime<Utc>,
        out: &mut Vec<MarketEvent>,
    ) -> Result<(), FeedError> {
        let shares = read_u32(payload, 20).map_err(|reason| self.protocol_error(reason))?;
        let price = read_price(payload, 32).map_err(|reason| self.protocol_error(reason))?;
        let match_number = read_u64(payload, 36).map_err(|reason| self.protocol_error(reason))?;
        let symbol = match self.symbol_for(stock_locate) {
            Some(symbol) => symbol.to_owned(),
            None => read_symbol(payload, 24).map_err(|reason| self.protocol_error(reason))?,
        };
        if symbol.is_empty() {
            return Err(self.protocol_error("Trade message carries an empty ticker"));
        }
        out.push(MarketEvent::Trade(TradeTick {
            instrument: InstrumentId::new(symbol, Venue::Nasdaq),
            price,
            size: Decimal::new(i64::from(shares), 0),
            trade_id: Some(match_number.to_string()),
            conditions: Vec::new(),
            exchange_ts,
            received_ts,
        }));
        Ok(())
    }

    /// Deducts `quantity` from a tracked order, removing it at zero.
    fn reduce_order(&mut self, order_ref: u64, quantity: Decimal) -> Result<(), FeedError> {
        let provider = self.provider.clone();
        let remaining =
            {
                let Some(order) = self.orders.get_mut(&order_ref) else {
                    return Ok(());
                };
                let remaining = order.remaining.checked_sub(quantity).ok_or_else(|| {
                    FeedError::ProtocolError {
                        provider: provider.clone(),
                        reason: "execution or cancel exceeds the tracked order quantity".to_owned(),
                    }
                })?;
                if remaining > Decimal::ZERO {
                    order.remaining = remaining;
                }
                remaining
            };
        if remaining <= Decimal::ZERO {
            self.orders.remove(&order_ref);
        }
        Ok(())
    }

    /// Builds a protocol error attributed to the decoder.
    fn protocol_error(&self, reason: impl Into<String>) -> FeedError {
        FeedError::ProtocolError {
            provider: self.provider.clone(),
            reason: reason.into(),
        }
    }
}

/// Reads an unsigned 8-bit field.
fn read_u8(bytes: &[u8], offset: usize) -> Result<u8, String> {
    bytes
        .get(offset)
        .copied()
        .ok_or_else(|| format!("truncated ITCH message at offset {offset}"))
}

/// Reads a big-endian unsigned 16-bit field.
fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| "field offset overflow".to_owned())?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| format!("truncated ITCH message at offset {offset}"))?;
    let array: [u8; 2] = slice
        .try_into()
        .map_err(|_| format!("truncated ITCH message at offset {offset}"))?;
    Ok(u16::from_be_bytes(array))
}

/// Reads a big-endian unsigned 32-bit field.
fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| "field offset overflow".to_owned())?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| format!("truncated ITCH message at offset {offset}"))?;
    let array: [u8; 4] = slice
        .try_into()
        .map_err(|_| format!("truncated ITCH message at offset {offset}"))?;
    Ok(u32::from_be_bytes(array))
}

/// Reads a big-endian unsigned 48-bit timestamp field.
fn read_u48(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let end = offset
        .checked_add(TIMESTAMP_LEN)
        .ok_or_else(|| "field offset overflow".to_owned())?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| format!("truncated ITCH message at offset {offset}"))?;
    let mut value = 0_u64;
    for byte in slice {
        value = (value << 8) | u64::from(*byte);
    }
    Ok(value)
}

/// Reads a big-endian unsigned 64-bit field.
fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let end = offset
        .checked_add(8)
        .ok_or_else(|| "field offset overflow".to_owned())?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| format!("truncated ITCH message at offset {offset}"))?;
    let array: [u8; 8] = slice
        .try_into()
        .map_err(|_| format!("truncated ITCH message at offset {offset}"))?;
    Ok(u64::from_be_bytes(array))
}

/// Reads a right-space-padded ASCII ticker field.
fn read_symbol(bytes: &[u8], offset: usize) -> Result<String, String> {
    let end = offset
        .checked_add(8)
        .ok_or_else(|| "ticker offset overflow".to_owned())?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| format!("truncated ITCH ticker at offset {offset}"))?;
    let text = std::str::from_utf8(slice)
        .map_err(|_| format!("ITCH ticker at offset {offset} is not valid UTF-8"))?;
    Ok(text.trim_end_matches(' ').to_owned())
}

/// Reads a 4-decimal fixed-point ITCH price.
fn read_price(bytes: &[u8], offset: usize) -> Result<Decimal, String> {
    let raw = read_u32(bytes, offset)?;
    Ok(Decimal::new(i64::from(raw), 4))
}

/// Reads a buy/sell indicator into a book side.
fn read_side(bytes: &[u8], offset: usize) -> Result<BookSide, String> {
    match read_u8(bytes, offset)? {
        b'B' => Ok(BookSide::Bid),
        b'S' => Ok(BookSide::Ask),
        other => Err(format!(
            "ITCH buy/sell indicator 0x{other:02X} is neither `B` nor `S`"
        )),
    }
}

/// A reader backed by any `AsyncRead` transport, framed as 2-byte length + ITCH.
///
/// Each frame is a big-endian `u16` payload length followed by exactly one
/// TotalView-ITCH 5.0 message. That framing matches Nasdaq's historical ITCH
/// files and a simple length-prefixed TCP stream, so a local replay file, an
/// in-memory fixture, or a TCP socket can all be driven through the same
/// [`MarketDataConnector`] implementation.
///
/// The provider is single-shot: the reader is consumed by the first
/// [`MarketDataConnector::connect`] call. A later call reports a graceful end of
/// stream so the resilience runtime stops instead of reconnecting a file that
/// has already been read. Use [`crate::MarketDataHub`] with a fresh provider per
/// replay, or a live WebSocket provider for reconnecting sessions.
///
/// # Examples
///
/// ```
/// use chrono::NaiveDate;
/// use kurumi_feed::MarketDataConnector;
/// use kurumi_feed::providers::nasdaq::ItchStreamProvider;
///
/// let date = NaiveDate::from_ymd_opt(2024, 11, 20).expect("valid date");
/// let provider = ItchStreamProvider::new("itch-replay", &b""[..], date)
///     .expect("valid provider");
/// assert_eq!(provider.name(), "itch-replay");
/// ```
pub struct ItchStreamProvider<R> {
    name: String,
    reader: Mutex<Option<R>>,
    trading_date: NaiveDate,
    venues: Vec<Venue>,
}

impl<R> ItchStreamProvider<R> {
    /// Creates a provider that reads from `reader` for the given trading date.
    ///
    /// # Errors
    ///
    /// Returns [`FeedError::InvalidConfig`] when `name` is empty.
    pub fn new(
        name: impl Into<String>,
        reader: R,
        trading_date: NaiveDate,
    ) -> Result<Self, FeedError> {
        let name = name.into();
        if name.is_empty() {
            return Err(FeedError::InvalidConfig {
                reason: "ITCH provider name must not be empty".to_owned(),
            });
        }
        Ok(Self {
            name,
            reader: Mutex::new(Some(reader)),
            trading_date,
            venues: vec![Venue::Nasdaq],
        })
    }

    /// Returns the trading date used for the replay.
    #[must_use]
    pub fn trading_date(&self) -> NaiveDate {
        self.trading_date
    }
}

impl<R> std::fmt::Debug for ItchStreamProvider<R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ItchStreamProvider")
            .field("name", &self.name)
            .field("trading_date", &self.trading_date)
            .field("venues", &self.venues)
            .finish_non_exhaustive()
    }
}

impl<R> MarketDataConnector for ItchStreamProvider<R>
where
    R: AsyncRead + Unpin + Send,
{
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
        Box::pin(async move {
            let reader = match self.reader.lock() {
                Ok(mut reader) => reader.take(),
                Err(_) => {
                    return Err(FeedError::ProtocolError {
                        provider: self.name.clone(),
                        reason: "ITCH reader mutex is poisoned".to_owned(),
                    });
                }
            };
            let Some(mut reader) = reader else {
                return Ok(());
            };
            self.stream(&mut reader, initial_subs, tx, cmd_rx, shutdown)
                .await
        })
    }
}

impl<R> ItchStreamProvider<R>
where
    R: AsyncRead + Unpin + Send,
{
    /// Reads and dispatches length-prefixed frames until EOF or shutdown.
    async fn stream(
        &self,
        reader: &mut R,
        initial_subs: Vec<Subscription>,
        tx: mpsc::Sender<MarketEvent>,
        mut cmd_rx: mpsc::Receiver<ConnectorCommand>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), FeedError> {
        let mut active: BTreeSet<Subscription> = initial_subs.into_iter().collect();
        let mut decoder = ItchDecoder::new(self.trading_date).with_provider(self.name.clone());
        let mut header = [0_u8; 2];
        let mut payload: Vec<u8> = Vec::new();
        let mut events: Vec<MarketEvent> = Vec::new();

        loop {
            match read_exact_with_commands(
                reader,
                &mut header,
                &mut active,
                &mut cmd_rx,
                &mut shutdown,
                &self.name,
            )
            .await?
            {
                ReadOutcome::Filled => {}
                ReadOutcome::EndOfStream | ReadOutcome::Shutdown => return Ok(()),
            }
            let length = usize::from(u16::from_be_bytes(header));
            if length == 0 {
                return Err(FeedError::ProtocolError {
                    provider: self.name.clone(),
                    reason: "ITCH frame length must be strictly positive".to_owned(),
                });
            }
            payload.resize(length, 0);
            match read_exact_with_commands(
                reader,
                &mut payload,
                &mut active,
                &mut cmd_rx,
                &mut shutdown,
                &self.name,
            )
            .await?
            {
                ReadOutcome::Filled => {}
                ReadOutcome::EndOfStream => {
                    return Err(FeedError::ProtocolError {
                        provider: self.name.clone(),
                        reason: "ITCH stream ended inside a frame payload".to_owned(),
                    });
                }
                ReadOutcome::Shutdown => return Ok(()),
            }

            events.clear();
            decoder.decode_into_at(&payload, now_utc(), &mut events)?;
            for event in events.drain(..) {
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

/// Outcome of an interruptible exact read.
enum ReadOutcome {
    /// The buffer was filled.
    Filled,
    /// The transport reached end of stream before filling the buffer.
    EndOfStream,
    /// A shutdown signal or a closed command channel stopped the session.
    Shutdown,
}

/// Fills `buffer` while still applying connector commands and shutdown.
///
/// [`AsyncReadExt::read`] is cancellation safe, so a command arriving between
/// two partial reads never loses already-received bytes.
async fn read_exact_with_commands<R>(
    reader: &mut R,
    buffer: &mut [u8],
    active: &mut BTreeSet<Subscription>,
    cmd_rx: &mut mpsc::Receiver<ConnectorCommand>,
    shutdown: &mut watch::Receiver<bool>,
    provider: &str,
) -> Result<ReadOutcome, FeedError>
where
    R: AsyncRead + Unpin,
{
    let mut filled = 0_usize;
    while filled < buffer.len() {
        if *shutdown.borrow() {
            return Ok(ReadOutcome::Shutdown);
        }
        let Some(target) = buffer.get_mut(filled..) else {
            return Err(FeedError::ProtocolError {
                provider: provider.to_owned(),
                reason: "ITCH read offset exceeds the frame buffer".to_owned(),
            });
        };
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(ReadOutcome::Shutdown);
                }
            }
            command = cmd_rx.recv() => {
                match command {
                    Some(command) => apply_command(active, &command),
                    None => return Ok(ReadOutcome::Shutdown),
                }
            }
            result = reader.read(target) => {
                match result {
                    Ok(0) => return Ok(ReadOutcome::EndOfStream),
                    Ok(read) => filled = filled.saturating_add(read),
                    Err(error) => {
                        return Err(FeedError::Disconnected {
                            provider: provider.to_owned(),
                            reason: error.to_string(),
                        });
                    }
                }
            }
        }
    }
    Ok(ReadOutcome::Filled)
}

/// Sends one event unless shutdown wins the race.
///
/// Returns `false` when the session should stop.
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
    use chrono::Datelike;

    fn trading_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 11, 20).expect("valid date")
    }

    fn header(message_type: u8, stock_locate: u16, timestamp: u64) -> Vec<u8> {
        let mut frame = vec![message_type];
        frame.extend_from_slice(&stock_locate.to_be_bytes());
        frame.extend_from_slice(&0_u16.to_be_bytes());
        let timestamp_bytes = timestamp.to_be_bytes();
        frame.extend_from_slice(&timestamp_bytes[2..]);
        frame
    }

    fn stock_directory(stock_locate: u16, symbol: &str) -> Vec<u8> {
        let mut frame = header(b'R', stock_locate, 0);
        let mut ticker = [b' '; 8];
        for (slot, byte) in ticker.iter_mut().zip(symbol.bytes()) {
            *slot = byte;
        }
        frame.extend_from_slice(&ticker);
        frame.extend_from_slice(b"QN");
        frame.extend_from_slice(&100_u32.to_be_bytes());
        frame.extend_from_slice(b"NC  PN 1N");
        frame.extend_from_slice(&0_u32.to_be_bytes());
        frame.push(b'N');
        assert_eq!(frame.len(), 39);
        frame
    }

    fn add_order(
        stock_locate: u16,
        timestamp: u64,
        order_ref: u64,
        side: u8,
        shares: u32,
        price: u32,
        symbol: &str,
    ) -> Vec<u8> {
        let mut frame = header(b'A', stock_locate, timestamp);
        frame.extend_from_slice(&order_ref.to_be_bytes());
        frame.push(side);
        frame.extend_from_slice(&shares.to_be_bytes());
        let mut ticker = [b' '; 8];
        for (slot, byte) in ticker.iter_mut().zip(symbol.bytes()) {
            *slot = byte;
        }
        frame.extend_from_slice(&ticker);
        frame.extend_from_slice(&price.to_be_bytes());
        assert_eq!(frame.len(), 36);
        frame
    }

    #[test]
    fn directory_maps_locate_and_trims_padding() {
        let mut decoder = ItchDecoder::new(trading_date());
        let events = decoder
            .decode_at(&stock_directory(7, "AAPL"), DateTime::<Utc>::UNIX_EPOCH)
            .expect("valid directory");
        assert!(events.is_empty());
        assert_eq!(decoder.symbol_for(7), Some("AAPL"));
        assert_eq!(decoder.stock_locate().len(), 1);
    }

    #[test]
    fn add_and_delete_round_trip_through_the_book() {
        let mut decoder = ItchDecoder::new(trading_date());
        decoder
            .decode(&stock_directory(7, "AAPL"))
            .expect("valid directory");
        let timestamp = 34_200_123_456_789_u64;
        let add = add_order(7, timestamp, 42, b'B', 500, 1_875_123, "AAPL");
        let events = decoder
            .decode_at(&add, DateTime::<Utc>::UNIX_EPOCH)
            .expect("valid add");
        let delta = match events.first() {
            Some(MarketEvent::OrderBookDelta(delta)) => delta,
            other => panic!("expected a book delta, got {other:?}"),
        };
        assert_eq!(delta.instrument.symbol, "AAPL");
        assert_eq!(delta.side, BookSide::Bid);
        assert_eq!(delta.price, Decimal::new(1_875_123, 4));
        assert_eq!(delta.size, Decimal::new(500, 0));
        assert_eq!(delta.order_ref, Some(42));
        let expected = NaiveDate::from_ymd_opt(2024, 11, 20)
            .expect("valid date")
            .and_hms_nano_opt(14, 30, 0, 123_456_789)
            .expect("valid UTC time")
            .and_utc();
        assert_eq!(delta.exchange_ts, expected);

        let mut delete = header(b'D', 7, timestamp);
        delete.extend_from_slice(&42_u64.to_be_bytes());
        let events = decoder.decode(&delete).expect("valid delete");
        let delta = match events.first() {
            Some(MarketEvent::OrderBookDelta(delta)) => delta,
            other => panic!("expected a book delta, got {other:?}"),
        };
        assert_eq!(delta.size, Decimal::ZERO);
        assert_eq!(delta.price, Decimal::new(1_875_123, 4));
    }

    #[test]
    fn execution_uses_the_tracked_price() {
        let mut decoder = ItchDecoder::new(trading_date());
        decoder
            .decode(&stock_directory(7, "AAPL"))
            .expect("valid directory");
        decoder
            .decode(&add_order(7, 1_000, 42, b'S', 500, 2_000_000, "AAPL"))
            .expect("valid add");
        let mut execute = header(b'E', 7, 2_000);
        execute.extend_from_slice(&42_u64.to_be_bytes());
        execute.extend_from_slice(&100_u32.to_be_bytes());
        execute.extend_from_slice(&9_u64.to_be_bytes());
        let events = decoder.decode(&execute).expect("valid execution");
        let trade = match events.first() {
            Some(MarketEvent::Trade(trade)) => trade,
            other => panic!("expected a trade, got {other:?}"),
        };
        assert_eq!(trade.price, Decimal::new(2_000_000, 4));
        assert_eq!(trade.size, Decimal::new(100, 0));
        assert_eq!(trade.trade_id.as_deref(), Some("9"));
    }

    #[test]
    fn auxiliary_messages_are_skipped_by_length() {
        let mut decoder = ItchDecoder::new(trading_date());
        let mut status = header(b'W', 0, 0);
        status.push(b'1');
        assert_eq!(status.len(), 12);
        assert!(
            decoder
                .decode(&status)
                .expect("skip MWCB status")
                .is_empty()
        );
    }

    #[test]
    fn truncated_and_unknown_frames_are_protocol_errors() {
        let mut decoder = ItchDecoder::new(trading_date());
        assert!(matches!(
            decoder.decode(b"S"),
            Err(FeedError::ProtocolError { .. })
        ));
        assert!(matches!(
            decoder.decode(&[b'z'; 12]),
            Err(FeedError::ProtocolError { .. })
        ));
    }

    #[test]
    fn system_event_maps_to_lifecycle_status() {
        let mut decoder = ItchDecoder::new(trading_date());
        let mut frame = header(b'S', 0, 0);
        frame.push(b'Q');
        let events = decoder
            .decode_at(&frame, DateTime::<Utc>::UNIX_EPOCH)
            .expect("valid system event");
        let status = match events.first() {
            Some(MarketEvent::Status(status)) => status,
            other => panic!("expected a status, got {other:?}"),
        };
        assert_eq!(status.state, ConnectionState::Authenticated);
        assert_eq!(status.provider, DEFAULT_PROVIDER);
    }

    #[test]
    fn timestamp_conversion_preserves_nanoseconds() {
        let mut decoder = ItchDecoder::new(trading_date());
        decoder
            .decode(&stock_directory(7, "AAPL"))
            .expect("valid directory");
        let events = decoder
            .decode(&add_order(7, 34_200_987_654_321, 1, b'B', 1, 1, "AAPL"))
            .expect("valid add");
        let delta = match events.first() {
            Some(MarketEvent::OrderBookDelta(delta)) => delta,
            other => panic!("expected a book delta, got {other:?}"),
        };
        assert_eq!(delta.exchange_ts.timestamp_subsec_nanos(), 987_654_321);
        assert_eq!(delta.exchange_ts.year(), 2024);
        assert_eq!(delta.exchange_ts.hour(), 14);
    }

    #[test]
    fn mpid_add_replace_and_cancel_update_the_book() {
        let mut decoder = ItchDecoder::new(trading_date());
        decoder
            .decode(&stock_directory(7, "AAPL"))
            .expect("valid directory");

        let mut add = add_order(7, 1_000, 10, b'B', 500, 1_000_000, "AAPL");
        *add.first_mut().expect("message type") = b'F';
        add.extend_from_slice(b"NSDQ");
        assert_eq!(add.len(), 40);
        assert_eq!(decoder.decode(&add).expect("valid MPID add").len(), 1);

        let mut replace = header(b'U', 7, 2_000);
        replace.extend_from_slice(&10_u64.to_be_bytes());
        replace.extend_from_slice(&11_u64.to_be_bytes());
        replace.extend_from_slice(&200_u32.to_be_bytes());
        replace.extend_from_slice(&1_010_000_u32.to_be_bytes());
        assert_eq!(replace.len(), 35);
        let events = decoder.decode(&replace).expect("valid replace");
        assert_eq!(events.len(), 2);
        let removal = match events.first() {
            Some(MarketEvent::OrderBookDelta(delta)) => delta,
            other => panic!("expected a removal delta, got {other:?}"),
        };
        assert_eq!(removal.price, Decimal::new(1_000_000, 4));
        assert_eq!(removal.size, Decimal::ZERO);
        assert_eq!(removal.order_ref, Some(10));
        let addition = match events.get(1) {
            Some(MarketEvent::OrderBookDelta(delta)) => delta,
            other => panic!("expected an add delta, got {other:?}"),
        };
        assert_eq!(addition.price, Decimal::new(1_010_000, 4));
        assert_eq!(addition.size, Decimal::new(200, 0));
        assert_eq!(addition.order_ref, Some(11));

        let mut cancel = header(b'X', 7, 3_000);
        cancel.extend_from_slice(&11_u64.to_be_bytes());
        cancel.extend_from_slice(&50_u32.to_be_bytes());
        assert_eq!(cancel.len(), 23);
        let events = decoder.decode(&cancel).expect("valid cancel");
        let delta = match events.first() {
            Some(MarketEvent::OrderBookDelta(delta)) => delta,
            other => panic!("expected a cancel delta, got {other:?}"),
        };
        assert_eq!(delta.price, Decimal::new(1_010_000, 4));
        assert_eq!(delta.size, Decimal::new(150, 0));
        assert_eq!(delta.order_ref, Some(11));
    }

    #[test]
    fn executed_with_price_honours_the_printable_flag() {
        let mut decoder = ItchDecoder::new(trading_date());
        decoder
            .decode(&stock_directory(7, "AAPL"))
            .expect("valid directory");
        decoder
            .decode(&add_order(7, 1_000, 20, b'S', 100, 1_000_000, "AAPL"))
            .expect("valid add");

        let mut non_printable = header(b'C', 7, 2_000);
        non_printable.extend_from_slice(&20_u64.to_be_bytes());
        non_printable.extend_from_slice(&10_u32.to_be_bytes());
        non_printable.extend_from_slice(&77_u64.to_be_bytes());
        non_printable.push(b'N');
        non_printable.extend_from_slice(&990_000_u32.to_be_bytes());
        assert_eq!(non_printable.len(), 36);
        assert!(
            decoder
                .decode(&non_printable)
                .expect("valid non-printable execution")
                .is_empty()
        );

        let mut printable = non_printable.clone();
        if let Some(flag) = printable.get_mut(31) {
            *flag = b'Y';
        }
        printable[19..23].copy_from_slice(&20_u32.to_be_bytes());
        printable[32..].copy_from_slice(&995_000_u32.to_be_bytes());
        let events = decoder
            .decode(&printable)
            .expect("valid printable execution");
        let trade = match events.first() {
            Some(MarketEvent::Trade(trade)) => trade,
            other => panic!("expected a trade, got {other:?}"),
        };
        assert_eq!(trade.price, Decimal::new(995_000, 4));
        assert_eq!(trade.size, Decimal::new(20, 0));

        let mut cancel = header(b'X', 7, 3_000);
        cancel.extend_from_slice(&20_u64.to_be_bytes());
        cancel.extend_from_slice(&70_u32.to_be_bytes());
        let events = decoder.decode(&cancel).expect("valid cancel");
        let delta = match events.first() {
            Some(MarketEvent::OrderBookDelta(delta)) => delta,
            other => panic!("expected a cancel delta, got {other:?}"),
        };
        assert_eq!(delta.size, Decimal::ZERO);
    }

    #[test]
    fn unknown_order_references_are_skipped() {
        let mut decoder = ItchDecoder::new(trading_date());
        decoder
            .decode(&stock_directory(7, "AAPL"))
            .expect("valid directory");
        let mut execute = header(b'E', 7, 1_000);
        execute.extend_from_slice(&999_u64.to_be_bytes());
        execute.extend_from_slice(&1_u32.to_be_bytes());
        execute.extend_from_slice(&1_u64.to_be_bytes());
        assert!(
            decoder
                .decode(&execute)
                .expect("skip unknown execution")
                .is_empty()
        );
    }

    use chrono::Timelike;
}
