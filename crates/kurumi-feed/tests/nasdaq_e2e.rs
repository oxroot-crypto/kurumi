//! Offline end-to-end tests for the Nasdaq market-data suite.
//!
//! Every default test runs against loopback sockets or in-memory fixtures. The
//! single live-endpoint template is `#[ignore]`d and reads credentials from the
//! environment.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use futures_util::{SinkExt, StreamExt};
use kurumi_feed::{
    BarAggregator, ChannelKind, FeedError, InstrumentId, ItchStreamProvider, ManagedConnector,
    MarketDataConnector, MarketEvent, NasdaqWsConfig, NasdaqWsProvider, NcdsPollingConnector,
    NcdsRestClient, NcdsRestConfig, ReconnectPolicy, Subscription, TradeTick, Venue,
};
use kurumi_ta::{Ema, StreamingIndicator};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{WebSocketStream, accept_async};

/// The US Eastern trading date used by every ITCH fixture.
fn trading_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 11, 20).expect("valid date")
}

/// Builds a UTC timestamp from whole Unix seconds.
fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(seconds, 0).expect("valid timestamp")
}

// ---------------------------------------------------------------------------
// TotalView-ITCH 5.0 fixture builders, following the official byte offsets.
// ---------------------------------------------------------------------------

/// Builds the common 11-byte ITCH header.
fn itch_header(message_type: u8, stock_locate: u16, tracking: u16, nanos: u64) -> Vec<u8> {
    let mut frame = vec![message_type];
    frame.extend_from_slice(&stock_locate.to_be_bytes());
    frame.extend_from_slice(&tracking.to_be_bytes());
    let timestamp = nanos.to_be_bytes();
    frame.extend_from_slice(&timestamp[2..]);
    frame
}

/// Builds a Stock Directory message for one ticker.
fn stock_directory(stock_locate: u16, symbol: &str) -> Vec<u8> {
    let mut frame = itch_header(b'R', stock_locate, 1, 0);
    let mut ticker = [b' '; 8];
    for (slot, byte) in ticker.iter_mut().zip(symbol.bytes()) {
        *slot = byte;
    }
    frame.extend_from_slice(&ticker);
    frame.extend_from_slice(b"QN");
    frame.extend_from_slice(&100_u32.to_be_bytes());
    frame.extend_from_slice(b"NC  PN 1N");
    frame.extend_from_slice(&1_u32.to_be_bytes());
    frame.push(b'N');
    assert_eq!(frame.len(), 39);
    frame
}

/// Builds an Add Order message.
fn add_order(
    stock_locate: u16,
    nanos: u64,
    order_ref: u64,
    side: u8,
    shares: u32,
    price: u32,
    symbol: &str,
) -> Vec<u8> {
    let mut frame = itch_header(b'A', stock_locate, 2, nanos);
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

/// Builds an Order Executed message.
fn order_executed(
    stock_locate: u16,
    nanos: u64,
    order_ref: u64,
    shares: u32,
    match_number: u64,
) -> Vec<u8> {
    let mut frame = itch_header(b'E', stock_locate, 3, nanos);
    frame.extend_from_slice(&order_ref.to_be_bytes());
    frame.extend_from_slice(&shares.to_be_bytes());
    frame.extend_from_slice(&match_number.to_be_bytes());
    assert_eq!(frame.len(), 31);
    frame
}

/// Builds a Non-Cross Trade message.
fn non_cross_trade(
    stock_locate: u16,
    nanos: u64,
    shares: u32,
    price: u32,
    symbol: &str,
    match_number: u64,
) -> Vec<u8> {
    let mut frame = itch_header(b'P', stock_locate, 4, nanos);
    frame.extend_from_slice(&0_u64.to_be_bytes());
    frame.push(b'B');
    frame.extend_from_slice(&shares.to_be_bytes());
    let mut ticker = [b' '; 8];
    for (slot, byte) in ticker.iter_mut().zip(symbol.bytes()) {
        *slot = byte;
    }
    frame.extend_from_slice(&ticker);
    frame.extend_from_slice(&price.to_be_bytes());
    frame.extend_from_slice(&match_number.to_be_bytes());
    assert_eq!(frame.len(), 44);
    frame
}

/// Builds an Order Delete message.
fn order_delete(stock_locate: u16, nanos: u64, order_ref: u64) -> Vec<u8> {
    let mut frame = itch_header(b'D', stock_locate, 5, nanos);
    frame.extend_from_slice(&order_ref.to_be_bytes());
    assert_eq!(frame.len(), 19);
    frame
}

/// Builds a System Event message.
fn system_event(nanos: u64, code: u8) -> Vec<u8> {
    let mut frame = itch_header(b'S', 0, 0, nanos);
    frame.push(code);
    assert_eq!(frame.len(), 12);
    frame
}

/// Prefixes an ITCH payload with its two-byte big-endian length.
fn length_frame(payload: &[u8]) -> Vec<u8> {
    let mut frame = u16::try_from(payload.len())
        .expect("ITCH payload fits u16")
        .to_be_bytes()
        .to_vec();
    frame.extend_from_slice(payload);
    frame
}

/// Collects every event until the provider closes its event channel.
async fn collect_events(
    receiver: &mut mpsc::Receiver<MarketEvent>,
    limit: Duration,
) -> Vec<MarketEvent> {
    let mut events = Vec::new();
    loop {
        match timeout(limit, receiver.recv()).await {
            Ok(Some(event)) => events.push(event),
            Ok(None) => return events,
            Err(_) => panic!("timed out waiting for a market event"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn itch_stream_decodes_official_layouts_and_filters_subscriptions() {
    let aapl = InstrumentId::new("AAPL", Venue::Nasdaq);
    let base_nanos = 34_200_u64 * 1_000_000_000;
    let payloads = [
        system_event(base_nanos, b'Q'),
        stock_directory(1, "AAPL"),
        stock_directory(2, "NVDA"),
        add_order(
            1,
            base_nanos + 1_000_000_000,
            100,
            b'B',
            500,
            1_875_000,
            "AAPL",
        ),
        add_order(
            2,
            base_nanos + 1_500_000_000,
            200,
            b'S',
            300,
            500_000,
            "NVDA",
        ),
        order_executed(1, base_nanos + 2_123_456_789, 100, 100, 9_001),
        non_cross_trade(1, base_nanos + 3_000_000_000, 50, 1_875_123, "AAPL", 9_002),
        order_delete(1, base_nanos + 4_000_000_000, 100),
    ];
    let mut wire = Vec::new();
    for payload in &payloads {
        wire.extend_from_slice(&length_frame(payload));
    }

    let (mut writer, reader): (DuplexStream, DuplexStream) = tokio::io::duplex(64 * 1024);
    let writer_task = tokio::spawn(async move {
        writer.write_all(&wire).await.expect("fixture write");
        writer.shutdown().await.expect("fixture shutdown");
    });
    let provider =
        ItchStreamProvider::new("itch-e2e", reader, trading_date()).expect("valid provider");
    let (tx, mut rx) = mpsc::channel(64);
    let (_command_tx, command_rx) = mpsc::channel(8);
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let initial = vec![Subscription::new(
        aapl.clone(),
        vec![ChannelKind::Trades, ChannelKind::OrderBook],
    )];
    let run =
        tokio::spawn(async move { provider.connect(initial, tx, command_rx, shutdown_rx).await });

    let events = collect_events(&mut rx, Duration::from_secs(10)).await;
    writer_task.await.expect("writer task");
    run.await
        .expect("provider task joined")
        .expect("ITCH replay completed");

    assert!(events.iter().any(
        |event| matches!(event, MarketEvent::Status(status) if status.provider == "itch-e2e")
    ));
    let trades: Vec<&TradeTick> = events
        .iter()
        .filter_map(|event| match event {
            MarketEvent::Trade(trade) => Some(trade),
            _ => None,
        })
        .collect();
    assert_eq!(trades.len(), 2);

    let executed = trades
        .iter()
        .find(|trade| trade.trade_id.as_deref() == Some("9001"))
        .expect("executed trade");
    assert_eq!(executed.instrument, aapl);
    assert_eq!(executed.price, Decimal::new(1_875_000, 4));
    assert_eq!(executed.size, Decimal::new(100, 0));
    let expected_executed = trading_date()
        .and_hms_nano_opt(14, 30, 2, 123_456_789)
        .expect("valid UTC time")
        .and_utc();
    assert_eq!(executed.exchange_ts, expected_executed);
    assert_eq!(executed.exchange_ts.timestamp_subsec_nanos(), 123_456_789);

    let print = trades
        .iter()
        .find(|trade| trade.trade_id.as_deref() == Some("9002"))
        .expect("non-cross trade");
    assert_eq!(print.price, Decimal::new(1_875_123, 4));
    assert_eq!(print.size, Decimal::new(50, 0));

    let deltas: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            MarketEvent::OrderBookDelta(delta) => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(deltas.len(), 2);
    let added = deltas.first().expect("add delta");
    assert_eq!(added.instrument, aapl);
    assert_eq!(added.price, Decimal::new(1_875_000, 4));
    assert_eq!(added.size, Decimal::new(500, 0));
    assert_eq!(added.order_ref, Some(100));
    let deleted = deltas.get(1).expect("delete delta");
    assert_eq!(deleted.size, Decimal::ZERO);
    assert_eq!(deleted.order_ref, Some(100));

    for event in &events {
        match event {
            MarketEvent::Trade(trade) => assert_eq!(trade.instrument, aapl),
            MarketEvent::OrderBookDelta(delta) => assert_eq!(delta.instrument, aapl),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Alpaca Market Data v2 WebSocket mock server.
// ---------------------------------------------------------------------------

/// Reads the next JSON text frame from a mock WebSocket server.
async fn next_client_json(ws: &mut WebSocketStream<TcpStream>) -> Value {
    loop {
        match ws.next().await.expect("client frame").expect("valid frame") {
            Message::Text(text) => {
                return serde_json::from_str(text.as_str()).expect("client JSON frame");
            }
            Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
            Message::Close(_) => panic!("client closed during the handshake"),
        }
    }
}

/// Sends one JSON frame from the mock server.
async fn send_server_json(ws: &mut WebSocketStream<TcpStream>, value: &Value) {
    let text = serde_json::to_string(value).expect("server JSON");
    ws.send(Message::Text(text.into()))
        .await
        .expect("server frame");
}

/// Builds an Alpaca trade frame using decimal strings.
fn alpaca_trade(symbol: &str, timestamp: &str, price: &str, size: &str, id: u64) -> Value {
    json!({
        "T": "t",
        "S": symbol,
        "i": id,
        "p": price,
        "s": size,
        "t": timestamp,
    })
}

/// Runs a two-connection Alpaca mock server that forces one reconnect.
async fn run_mock_alpaca(
    listener: TcpListener,
    subscribe_tx: mpsc::UnboundedSender<Value>,
    mut stop: watch::Receiver<bool>,
) {
    let start = 1_700_000_040_i64;
    for connection in 0..2 {
        let (stream, _) = listener.accept().await.expect("mock accept");
        let mut ws = accept_async(stream).await.expect("mock handshake");
        send_server_json(&mut ws, &json!([{"T": "success", "msg": "connected"}])).await;
        let auth = next_client_json(&mut ws).await;
        assert_eq!(auth.get("action").and_then(Value::as_str), Some("auth"));
        assert_eq!(auth.get("key").and_then(Value::as_str), Some("key"));
        assert_eq!(auth.get("secret").and_then(Value::as_str), Some("secret"));
        send_server_json(&mut ws, &json!([{"T": "success", "msg": "authenticated"}])).await;
        let subscription = next_client_json(&mut ws).await;
        subscribe_tx
            .send(subscription)
            .expect("record subscription");

        let batch = if connection == 0 {
            json!([
                alpaca_trade("AAPL", &at(start).to_rfc3339(), "100.0000", "1", 1),
                alpaca_trade("AAPL", &at(start + 30).to_rfc3339(), "101.0000", "1", 2),
                alpaca_trade("AAPL", &at(start + 70).to_rfc3339(), "102.0000", "1", 3),
                {
                    "T": "q",
                    "S": "AAPL",
                    "bp": "100.0000",
                    "bs": "10",
                    "ap": "100.1000",
                    "as": "12",
                    "t": at(start + 70).to_rfc3339(),
                }
            ])
        } else {
            json!([
                alpaca_trade("AAPL", &at(start + 130).to_rfc3339(), "103.0000", "1", 4),
                alpaca_trade("AAPL", &at(start + 190).to_rfc3339(), "104.0000", "1", 5),
            ])
        };
        send_server_json(&mut ws, &batch).await;
        if connection == 0 {
            ws.send(Message::Close(None)).await.expect("mock close");
        } else {
            loop {
                tokio::select! {
                    changed = stop.changed() => {
                        if changed.is_err() || *stop.borrow() {
                            break;
                        }
                    }
                    message = ws.next() => {
                        if message.is_none() {
                            break;
                        }
                    }
                }
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn websocket_reconnects_resubscribes_and_drives_bars_and_ema() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("local address");
    let (subscribe_tx, mut subscribe_rx) = mpsc::unbounded_channel();
    let (stop_tx, stop_rx) = watch::channel(false);
    let server = tokio::spawn(run_mock_alpaca(listener, subscribe_tx, stop_rx));

    let config = NasdaqWsConfig::custom(format!("ws://{address}"))
        .with_credentials("key", "secret")
        .with_heartbeat_interval(Duration::from_secs(30));
    let provider = NasdaqWsProvider::new(config).expect("valid provider");
    let policy = ReconnectPolicy {
        initial_backoff: Duration::from_millis(5),
        max_backoff: Duration::from_millis(20),
        backoff_multiplier: 2.0,
        max_retries: Some(5),
        heartbeat_timeout: Duration::from_secs(5),
    };
    let managed =
        ManagedConnector::new(Arc::new(provider), policy).expect("valid reconnect policy");
    let (tx, mut rx) = mpsc::channel(64);
    let (_command_tx, command_rx) = mpsc::channel(8);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let aapl = InstrumentId::new("AAPL", Venue::Nasdaq);
    let initial = vec![Subscription::new(
        aapl,
        vec![ChannelKind::Trades, ChannelKind::Quotes],
    )];
    let run = tokio::spawn(async move { managed.run(initial, tx, command_rx, shutdown_rx).await });

    let mut aggregator = BarAggregator::new(60).expect("valid interval");
    let mut ema = Ema::new(2).expect("valid period");
    let mut trades = 0_usize;
    let mut quotes = 0_usize;
    let mut bars = 0_usize;
    let mut last_ema = None;
    let mut last_close = None;
    while trades < 5 || quotes == 0 {
        let event = timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("event timeout")
            .expect("event channel closed early");
        match event {
            MarketEvent::Trade(tick) => {
                trades += 1;
                if let Some(bar) = aggregator.on_trade(&tick).expect("aggregate trade") {
                    bars += 1;
                    last_close = Some(bar.bar.close());
                    last_ema = ema.next(bar.bar.close()).expect("ema");
                }
            }
            MarketEvent::Quote(_) => quotes += 1,
            _ => {}
        }
    }
    for bar in aggregator.flush().expect("flush bars") {
        bars += 1;
        last_close = Some(bar.bar.close());
        last_ema = ema.next(bar.bar.close()).expect("ema");
    }
    assert_eq!(trades, 5);
    assert!(quotes >= 1);
    assert!(bars >= 3);
    approx::assert_relative_eq!(last_close.expect("last close"), 104.0, epsilon = 1e-12);
    let final_ema = last_ema.expect("ema output");
    approx::assert_relative_eq!(final_ema, 103.5, epsilon = 1e-12);

    let first = timeout(Duration::from_secs(5), subscribe_rx.recv())
        .await
        .expect("first subscription timed out")
        .expect("first subscription");
    let second = timeout(Duration::from_secs(5), subscribe_rx.recv())
        .await
        .expect("second subscription timed out")
        .expect("second subscription");
    for frame in [&first, &second] {
        assert_eq!(
            frame.get("action").and_then(Value::as_str),
            Some("subscribe")
        );
        assert_eq!(
            frame
                .get("trades")
                .and_then(Value::as_array)
                .and_then(|symbols| symbols.first())
                .and_then(Value::as_str),
            Some("AAPL")
        );
        assert_eq!(
            frame
                .get("quotes")
                .and_then(Value::as_array)
                .and_then(|symbols| symbols.first())
                .and_then(Value::as_str),
            Some("AAPL")
        );
    }

    shutdown_tx.send(true).expect("request shutdown");
    stop_tx.send(true).expect("stop server");
    let outcome = timeout(Duration::from_secs(5), run)
        .await
        .expect("client did not stop")
        .expect("client task panicked");
    assert!(outcome.is_ok(), "client stopped with {outcome:?}");
    timeout(Duration::from_secs(5), server)
        .await
        .expect("server did not stop")
        .expect("server task panicked");
}

// ---------------------------------------------------------------------------
// NCDS REST mock server and polling tests.
// ---------------------------------------------------------------------------

/// One parsed HTTP request from the mock server.
struct MockRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl MockRequest {
    /// Returns a header value by lowercase name.
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Returns the byte index just past the HTTP header terminator.
fn http_header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

/// Reads one complete HTTP/1.1 request.
async fn read_http_request(stream: &mut TcpStream) -> MockRequest {
    let mut buffer = Vec::new();
    let header_end = loop {
        if let Some(end) = http_header_end(&buffer) {
            break end;
        }
        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk).await.expect("request read");
        assert!(read > 0, "connection closed before the HTTP headers");
        buffer.extend_from_slice(&chunk[..read]);
    };
    let header_text = std::str::from_utf8(&buffer[..header_end]).expect("UTF-8 headers");
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().expect("request line");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().expect("method").to_owned();
    let path = parts.next().expect("path").to_owned();
    let mut headers = Vec::new();
    let mut content_length = 0_usize;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_owned();
            if name == "content-length" {
                content_length = value.parse().expect("content length");
            }
            headers.push((name, value));
        }
    }
    while buffer.len() < header_end + content_length {
        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk).await.expect("body read");
        assert!(read > 0, "connection closed inside the body");
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = buffer[header_end..header_end + content_length].to_vec();
    MockRequest {
        method,
        path,
        headers,
        body,
    }
}

/// Writes one HTTP JSON response and closes the connection.
async fn respond_json(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    extra_headers: &[(&str, &str)],
    body: &str,
) {
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in extra_headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    response.push_str(body);
    stream
        .write_all(response.as_bytes())
        .await
        .expect("response write");
    stream.shutdown().await.expect("response shutdown");
}

/// Runs the loopback NCDS mock server until the listener is dropped.
async fn run_mock_ncds(
    listener: TcpListener,
    token_requests: Arc<AtomicUsize>,
    sale_requests: Arc<AtomicUsize>,
) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let token_requests = Arc::clone(&token_requests);
        let sale_requests = Arc::clone(&sale_requests);
        tokio::spawn(async move {
            let request = read_http_request(&mut stream).await;
            let path = request.path.as_str();
            if request.method == "POST" && path == "/v1/auth/token" {
                token_requests.fetch_add(1, Ordering::SeqCst);
                assert_eq!(request.header("content-type"), Some("application/json"));
                let body = std::str::from_utf8(&request.body).expect("UTF-8 token body");
                assert!(body.contains("\"client_id\":\"client\""));
                assert!(body.contains("\"client_secret\":\"secret\""));
                respond_json(
                    &mut stream,
                    200,
                    "OK",
                    &[],
                    r#"{"access_token":"test-token","expires_in":3600}"#,
                )
                .await;
                return;
            }
            assert_eq!(
                request.header("authorization"),
                Some("Bearer test-token"),
                "REST calls must carry the bearer token"
            );
            if path.starts_with("/v1/Nasdaq/realtime/equities/lastsale/") {
                let count = sale_requests.fetch_add(1, Ordering::SeqCst);
                let price = if count < 2 { "187.5123" } else { "188.0000" };
                let body = format!(
                    r#"[{{"symbol":"AAPL","timestamp":"2024-11-20T09:30:01.383","price":{price},"size":10,"conditions":"@","exchange":"Q"}}]"#
                );
                respond_json(&mut stream, 200, "OK", &[], &body).await;
                return;
            }
            if path.starts_with("/v1/Nasdaq/realtime/equities/lastquote/BAD") {
                respond_json(
                    &mut stream,
                    429,
                    "Too Many Requests",
                    &[("Retry-After", "2")],
                    r#"{"error":"rate limited"}"#,
                )
                .await;
                return;
            }
            if path.starts_with("/v1/Nasdaq/realtime/equities/lastquote/") {
                respond_json(
                    &mut stream,
                    200,
                    "OK",
                    &[],
                    r#"[{"symbol":"AAPL","timestamp":"2024-11-20T09:30:01.383","bidPrice":187.5000,"bidSize":3,"askPrice":187.5100,"askSize":4}]"#,
                )
                .await;
                return;
            }
            if path.starts_with("/v1/Nasdaq/realtime/equities/snapshot/") {
                respond_json(
                    &mut stream,
                    200,
                    "OK",
                    &[],
                    r#"[{"symbol":"AAPL","timestamp":"2024-11-20T09:30:01.383","open":187.50,"high":188.00,"low":187.40,"close":187.75,"lastTrade":187.75,"volume":1000,"lastSale":187.75,"previousClose":187.00,"dollarVolume":187750.0,"netChange":0.75,"percentChange":0.4011}]"#,
                )
                .await;
                return;
            }
            if path.starts_with("/v2/Nasdaq/realtime/equities/bars/AAPL/1minute/false/1d") {
                respond_json(
                    &mut stream,
                    200,
                    "OK",
                    &[],
                    r#"{"AAPL":[{"t":"2024-11-20T09:30:00.000","o":187.5000,"h":188.0000,"l":187.4000,"c":187.7500,"v":1000}]}"#,
                )
                .await;
                return;
            }
            respond_json(
                &mut stream,
                404,
                "Not Found",
                &[],
                r#"{"error":"not found"}"#,
            )
            .await;
        });
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ncds_oauth_and_rest_polling_run_against_a_loopback_server() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("local address");
    let token_requests = Arc::new(AtomicUsize::new(0));
    let sale_requests = Arc::new(AtomicUsize::new(0));
    let server = tokio::spawn(run_mock_ncds(
        listener,
        Arc::clone(&token_requests),
        Arc::clone(&sale_requests),
    ));

    let config = NcdsRestConfig::new(
        format!("http://{address}"),
        format!("http://{address}/v1/auth/token"),
        "client",
        "secret",
        Duration::from_millis(10),
    );
    let client = NcdsRestClient::new(config.clone()).expect("valid client");
    let sales = client
        .fetch_last_sale(&["AAPL".to_owned()])
        .await
        .expect("last sale");
    assert_eq!(sales.len(), 1);
    let sale = sales.first().expect("one sale");
    assert_eq!(sale.price, Decimal::new(1_875_123, 4));
    assert_eq!(sale.size, Decimal::new(10, 0));
    assert_eq!(sale.exchange_ts.to_string(), "2024-11-20 14:30:01.383 UTC");

    let quotes = client
        .fetch_last_quote(&["AAPL".to_owned()])
        .await
        .expect("last quote");
    let quote = quotes.first().expect("one quote");
    assert_eq!(quote.bid_price, Decimal::new(1_875_000, 4));
    assert_eq!(quote.ask_size, Decimal::new(4, 0));

    let snapshots = client
        .fetch_snapshot(&["AAPL".to_owned()])
        .await
        .expect("snapshot");
    let snapshot = snapshots.first().expect("one snapshot");
    assert_eq!(snapshot.last_sale, Decimal::new(1_877_500, 4));
    assert_eq!(snapshot.volume, Decimal::new(1_000, 0));

    let bars = client
        .fetch_bars("AAPL", kurumi_feed::NcdsBarPrecision::OneMinute, "1d")
        .await
        .expect("bars");
    let bar = bars.first().expect("one bar");
    assert_eq!(bar.close, Decimal::new(1_877_500, 4));
    assert_eq!(bar.volume, Decimal::new(1_000, 0));

    let limited = client.fetch_last_quote(&["BAD".to_owned()]).await;
    assert!(matches!(
        limited,
        Err(FeedError::RateLimited {
            retry_after_ms: 2_000,
            ..
        })
    ));
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);

    sale_requests.store(0, Ordering::SeqCst);
    let connector = NcdsPollingConnector::new("ncds", config.clone()).expect("valid connector");
    let active = std::collections::BTreeSet::from([Subscription::new(
        InstrumentId::new("AAPL", Venue::Nasdaq),
        vec![ChannelKind::Trades],
    )]);
    let first = connector.poll_once(&active).await.expect("first poll");
    let second = connector.poll_once(&active).await.expect("second poll");
    let third = connector.poll_once(&active).await.expect("third poll");
    assert_eq!(first.len(), 1);
    assert!(
        second.is_empty(),
        "an unchanged observation must be deduplicated"
    );
    assert_eq!(third.len(), 1);
    let changed = third.first().expect("changed sale");
    match changed {
        MarketEvent::Trade(tick) => {
            assert_eq!(tick.price, Decimal::new(1_880_000, 4));
        }
        other => panic!("expected a trade, got {other:?}"),
    }

    // Drive the polling connector through its real `MarketDataConnector` loop.
    sale_requests.store(0, Ordering::SeqCst);
    let streaming = NcdsPollingConnector::new("ncds-stream", config).expect("valid connector");
    let (tx, mut rx) = mpsc::channel(16);
    let (_command_tx, command_rx) = mpsc::channel(8);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let initial: Vec<Subscription> = active.into_iter().collect();
    let run = tokio::spawn(async move {
        streaming
            .connect(initial, tx, command_rx, shutdown_rx)
            .await
    });
    let first_streamed = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("first polled event timed out")
        .expect("polling connector channel closed");
    let second_streamed = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("second polled event timed out")
        .expect("polling connector channel closed");
    for (event, expected) in [
        (first_streamed, Decimal::new(1_875_123, 4)),
        (second_streamed, Decimal::new(1_880_000, 4)),
    ] {
        match event {
            MarketEvent::Trade(tick) => assert_eq!(tick.price, expected),
            other => panic!("expected a trade, got {other:?}"),
        }
    }
    shutdown_tx.send(true).expect("request polling shutdown");
    let outcome = timeout(Duration::from_secs(5), run)
        .await
        .expect("polling connector did not stop")
        .expect("polling connector task panicked");
    assert!(
        outcome.is_ok(),
        "polling connector stopped with {outcome:?}"
    );

    // Each client (the direct REST client and the two polling connectors)
    // obtains one token and reuses it for the whole run.
    assert_eq!(token_requests.load(Ordering::SeqCst), 3);

    server.abort();
}

// ---------------------------------------------------------------------------
// Ignored live-endpoint template.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires ALPACA_API_KEY and ALPACA_API_SECRET and outbound network access"]
async fn live_alpaca_test_stream_reports_the_first_five_events() {
    let api_key = std::env::var("ALPACA_API_KEY").expect("ALPACA_API_KEY is required");
    let api_secret = std::env::var("ALPACA_API_SECRET").expect("ALPACA_API_SECRET is required");
    let config = NasdaqWsConfig::alpaca_test()
        .with_credentials(api_key, api_secret)
        .with_heartbeat_interval(Duration::from_secs(15));
    let provider = NasdaqWsProvider::new(config).expect("valid provider");
    let (tx, mut rx) = mpsc::channel(32);
    let (_command_tx, command_rx) = mpsc::channel(8);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let subscription = Subscription::new(
        InstrumentId::new("FAKEPACA", Venue::Nasdaq),
        vec![ChannelKind::Trades],
    );
    let run = tokio::spawn(async move {
        provider
            .connect(vec![subscription], tx, command_rx, shutdown_rx)
            .await
    });

    let mut printed = 0_usize;
    while printed < 5 {
        match timeout(Duration::from_secs(60), rx.recv())
            .await
            .expect("live stream timed out")
        {
            Some(event) => {
                println!("{event:?}");
                printed += 1;
            }
            None => break,
        }
    }
    assert_eq!(printed, 5);
    shutdown_tx.send(true).expect("request shutdown");
    let outcome = timeout(Duration::from_secs(10), run)
        .await
        .expect("live provider did not stop")
        .expect("live provider task panicked");
    assert!(outcome.is_ok(), "live provider stopped with {outcome:?}");
}
