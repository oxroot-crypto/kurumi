//! End-to-end execution tests for the live layer.
//!
//! Test A proves the "one codebase, two modes" invariant: the same
//! [`EmaCrossStrategy`] runs through the synchronous `BacktestRunner` and through
//! `LiveRunner` + `PaperBroker`, and both paths must produce bit-identical fills
//! and equity.

#![cfg(feature = "live")]

use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use kurumi_backtest::portfolio::{AlignedMarketData, MultiAssetFeed};
use kurumi_backtest::{FixedRateFee, NoSlippage};
use kurumi_exec::live::{
    AlpacaTradingBroker, BrokerAccountSnapshot, ExecutionBroker, LiveConfig, LiveRunner,
    LiveSessionSummary, PaperBroker,
};
use kurumi_exec::{
    BacktestConfig, BacktestRunner, ExecError, ExecutionReport, JsonlFileJournal, MemoryJournal,
    OrderRequest, OrderSide, OrderType, PaperMatchingEngine, RiskGate, StrategyContext,
    TimeInForce,
};
use kurumi_feed::{
    BarEvent, ChannelKind, InstrumentId, MarketDataHub, MarketEvent, MockReplayProvider,
    NasdaqWsConfig, NasdaqWsProvider, Subscription, Venue,
};
use kurumi_ta::{Ema, OhlcvBar, StreamingIndicator};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{WebSocketStream, accept_async};

const START_MS: i64 = 1_700_000_000_000;
const BAR_MS: i64 = 60_000;
const BARS: usize = 120;
const SYMBOL: &str = "AAPL";

// ---------------------------------------------------------------------------
// Shared strategy
// ---------------------------------------------------------------------------

/// A real EMA-cross strategy. Both execution paths instantiate this exact type.
struct EmaCrossStrategy {
    fast: Ema,
    slow: Ema,
    fast_previous: Option<f64>,
    slow_previous: Option<f64>,
    bars_seen: Arc<AtomicUsize>,
    resting_limit: Option<Decimal>,
    resting_submitted: bool,
}

impl EmaCrossStrategy {
    fn new(fast: usize, slow: usize) -> Result<Self, ExecError> {
        Ok(Self {
            fast: Ema::new(fast)?,
            slow: Ema::new(slow)?,
            fast_previous: None,
            slow_previous: None,
            bars_seen: Arc::new(AtomicUsize::new(0)),
            resting_limit: None,
            resting_submitted: false,
        })
    }

    fn with_resting_limit(mut self, price: Decimal) -> Self {
        self.resting_limit = Some(price);
        self
    }

    fn bars_seen(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.bars_seen)
    }
}

impl kurumi_exec::Strategy for EmaCrossStrategy {
    fn id(&self) -> &str {
        "ema-cross"
    }

    fn on_market_event(
        &mut self,
        event: &kurumi_exec::ExecMarketEvent,
        ctx: &StrategyContext<'_>,
    ) -> Result<Vec<kurumi_exec::OrderCommand>, ExecError> {
        let bar = match event {
            kurumi_exec::ExecMarketEvent::Bar { bar, .. } => *bar,
            kurumi_exec::ExecMarketEvent::Snapshot(snapshot) => {
                let Some(bar) = snapshot.bar(SYMBOL) else {
                    return Ok(Vec::new());
                };
                *bar
            }
            _ => return Ok(Vec::new()),
        };
        let fast = self.fast.next(bar.close())?;
        let slow = self.slow.next(bar.close())?;
        self.bars_seen.fetch_add(1, Ordering::SeqCst);
        let mut commands = Vec::new();
        if let (Some(fast), Some(slow), Some(previous_fast), Some(previous_slow)) =
            (fast, slow, self.fast_previous, self.slow_previous)
        {
            if previous_fast <= previous_slow && fast > slow {
                commands.push(kurumi_exec::OrderCommand::Submit(OrderRequest::new(
                    ctx.next_order_id(),
                    SYMBOL,
                    OrderSide::Buy,
                    OrderType::Market,
                    Decimal::from(10),
                    TimeInForce::Day,
                )));
            } else if previous_fast >= previous_slow && fast < slow {
                let position = ctx.position_qty(SYMBOL);
                if position > Decimal::ZERO {
                    commands.push(kurumi_exec::OrderCommand::Submit(
                        OrderRequest::new(
                            ctx.next_order_id(),
                            SYMBOL,
                            OrderSide::Sell,
                            OrderType::Market,
                            position.min(Decimal::from(10)),
                            TimeInForce::Day,
                        )
                        .reduce_only(),
                    ));
                }
            }
        }
        self.fast_previous = fast.or(self.fast_previous);
        self.slow_previous = slow.or(self.slow_previous);

        if !self.resting_submitted {
            if let Some(price) = self.resting_limit {
                self.resting_submitted = true;
                commands.push(kurumi_exec::OrderCommand::Submit(OrderRequest::new(
                    ctx.next_order_id(),
                    SYMBOL,
                    OrderSide::Buy,
                    OrderType::Limit { price },
                    Decimal::ONE,
                    TimeInForce::Gtc,
                )));
            }
        }
        Ok(commands)
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn bars() -> Result<Vec<OhlcvBar>, ExecError> {
    let mut bars = Vec::with_capacity(BARS);
    for index in 0..BARS {
        let phase = index as f64 * 0.25;
        let close = 100.0 + 8.0 * (phase * PI).sin() + index as f64 * 0.08;
        bars.push(OhlcvBar::new(
            START_MS + index as i64 * BAR_MS,
            close * 0.999,
            close * 1.002,
            close * 0.998,
            close,
            1_000.0,
            close * 1_000.0,
        )?);
    }
    Ok(bars)
}

fn fee() -> Result<FixedRateFee, ExecError> {
    Ok(FixedRateFee::new(Decimal::new(1, 3))?)
}

fn backtest_bars(bars: &[OhlcvBar]) -> Result<AlignedMarketData, ExecError> {
    let feed = MultiAssetFeed::new(BTreeMap::from([(SYMBOL.to_owned(), bars.to_vec())]))?;
    Ok(AlignedMarketData::from_feed(feed))
}

fn bar_events(bars: &[OhlcvBar]) -> Vec<MarketEvent> {
    bars.iter()
        .map(|bar| {
            MarketEvent::Bar(BarEvent {
                instrument: InstrumentId::new(SYMBOL, Venue::Nasdaq),
                interval_secs: 60,
                bar: *bar,
                is_final: true,
            })
        })
        .collect()
}

async fn wait_for_bars(counter: &Arc<AtomicUsize>, target: usize) {
    timeout(Duration::from_secs(15), async {
        while counter.load(Ordering::SeqCst) < target {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("live runner did not process the expected bars");
}

async fn replay_bars(
    bars: &[OhlcvBar],
) -> Result<(LiveSessionSummary, Arc<AtomicUsize>), ExecError> {
    replay_bars_with_limit(bars, None).await
}

async fn replay_bars_with_limit(
    bars: &[OhlcvBar],
    resting_limit: Option<Decimal>,
) -> Result<(LiveSessionSummary, Arc<AtomicUsize>), ExecError> {
    let mut hub = MarketDataHub::new(4096)?;
    let provider = Arc::new(MockReplayProvider::new(
        "mock",
        vec![Venue::Nasdaq],
        bar_events(bars),
    ));
    hub.register(provider)?;
    let market_events = hub.subscribe_events();
    hub.start()?;
    hub.subscribe(
        "mock",
        vec![Subscription::new(
            InstrumentId::new(SYMBOL, Venue::Nasdaq),
            vec![ChannelKind::Bars { interval_secs: 60 }],
        )],
    )
    .await?;

    let broker = Arc::new(PaperBroker::new(
        PaperMatchingEngine::new(Box::new(fee()?), Box::new(NoSlippage)),
        Decimal::from(1_000_000),
    )?);
    let mut runner = LiveRunner::new(
        LiveConfig {
            initial_cash: Decimal::from(1_000_000),
            fee_buffer_bps: Decimal::from(50),
            ..LiveConfig::default()
        },
        RiskGate::empty(),
        broker,
        Box::new(MemoryJournal::new()),
    )?;
    let mut strategy = EmaCrossStrategy::new(3, 8)?;
    if let Some(limit) = resting_limit {
        strategy = strategy.with_resting_limit(limit);
    }
    let counter = strategy.bars_seen();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let handle =
        tokio::spawn(async move { runner.run(&mut strategy, market_events, shutdown_rx).await });
    wait_for_bars(&counter, BARS).await;
    shutdown_tx
        .send(true)
        .map_err(|_| ExecError::strategy("shutdown receiver dropped"))?;
    let summary = timeout(Duration::from_secs(15), handle)
        .await
        .map_err(|_| ExecError::strategy("live runner did not shut down"))?
        .map_err(|error| ExecError::strategy(error.to_string()))??;
    hub.shutdown().await?;
    Ok((summary, counter))
}

// ---------------------------------------------------------------------------
// Test A: golden backtest/live parity
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backtest_and_live_produce_identical_fills_and_equity() {
    let bars = bars().expect("bars");
    let data = backtest_bars(&bars).expect("aligned data");

    let mut backtest = BacktestRunner::new(
        BacktestConfig {
            initial_cash: Decimal::from(1_000_000),
            fee_buffer_bps: Decimal::from(50),
            periods_per_year: 252.0,
            risk_free_rate: 0.0,
        },
        RiskGate::empty(),
        PaperMatchingEngine::new(Box::new(fee().expect("fee")), Box::new(NoSlippage)),
        Box::new(MemoryJournal::new()),
    )
    .expect("backtest runner");
    let mut backtest_strategy = EmaCrossStrategy::new(3, 8).expect("strategy");
    let report = backtest
        .run(data.slice(), &mut backtest_strategy)
        .expect("backtest");

    let (summary, _) = replay_bars(&bars).await.expect("live run");

    assert!(!report.fills.is_empty(), "the strategy must trade");
    assert_eq!(summary.fills.len(), report.fills.len());
    for (live, backtest) in summary.fills.iter().zip(report.fills.iter()) {
        assert_eq!(live.symbol, backtest.symbol);
        assert_eq!(live.side, backtest.side);
        assert_eq!(live.fill_price, backtest.fill_price);
        assert_eq!(live.fill_qty, backtest.fill_qty);
        assert_eq!(live.fee, backtest.fee);
        assert_eq!(live.slippage, backtest.slippage);
        assert_eq!(live.client_order_id, backtest.client_order_id);
        assert_eq!(live.exec_id, backtest.exec_id);
        assert_eq!(live.timestamp, backtest.timestamp);
    }
    assert_eq!(summary.equity_curve, report.equity_curve);
    assert_eq!(summary.final_equity, report.final_equity);
    assert_eq!(summary.initial_cash, report.initial_cash);
}

// ---------------------------------------------------------------------------
// Test A2: an in-range limit waits for a later bar in both paths
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backtest_and_live_defer_in_range_limit_to_a_later_bar() {
    let bars = bars().expect("bars");
    let data = backtest_bars(&bars).expect("aligned data");
    // Bar 0 has low 99.8, so this buy limit would fill on the signal bar under
    // same-bar matching. Bar 1 has low 105.53, so it must wait for a later bar.
    let limit = Decimal::from(102);

    let mut backtest = BacktestRunner::new(
        BacktestConfig {
            initial_cash: Decimal::from(1_000_000),
            fee_buffer_bps: Decimal::from(50),
            periods_per_year: 252.0,
            risk_free_rate: 0.0,
        },
        RiskGate::empty(),
        PaperMatchingEngine::new(Box::new(fee().expect("fee")), Box::new(NoSlippage)),
        Box::new(MemoryJournal::new()),
    )
    .expect("backtest runner");
    let mut backtest_strategy = EmaCrossStrategy::new(3, 8)
        .expect("strategy")
        .with_resting_limit(limit);
    let report = backtest
        .run(data.slice(), &mut backtest_strategy)
        .expect("backtest");

    let (summary, _) = replay_bars_with_limit(&bars, Some(limit))
        .await
        .expect("live run");

    // `EmaCrossStrategy` submits the resting limit before any EMA signal, so it
    // is the first deterministic client order id.
    let limit_id = "ord-000000000000";
    let backtest_limit_fill = report
        .fills
        .iter()
        .find(|fill| fill.client_order_id == limit_id)
        .expect("backtest resting limit fill");
    let live_limit_fill = summary
        .fills
        .iter()
        .find(|fill| fill.client_order_id == limit_id)
        .expect("live resting limit fill");

    assert!(
        backtest_limit_fill.timestamp.timestamp_millis() > START_MS,
        "an in-range limit must not fill on the signal bar"
    );
    assert_eq!(backtest_limit_fill.timestamp, live_limit_fill.timestamp);

    assert_eq!(summary.fills.len(), report.fills.len());
    for (live, backtest) in summary.fills.iter().zip(report.fills.iter()) {
        assert_eq!(live.symbol, backtest.symbol);
        assert_eq!(live.side, backtest.side);
        assert_eq!(live.fill_price, backtest.fill_price);
        assert_eq!(live.fill_qty, backtest.fill_qty);
        assert_eq!(live.fee, backtest.fee);
        assert_eq!(live.slippage, backtest.slippage);
        assert_eq!(live.client_order_id, backtest.client_order_id);
        assert_eq!(live.exec_id, backtest.exec_id);
        assert_eq!(live.timestamp, backtest.timestamp);
    }
    assert_eq!(summary.equity_curve, report.equity_curve);
    assert_eq!(summary.final_equity, report.final_equity);
    assert_eq!(summary.initial_cash, report.initial_cash);
}

// ---------------------------------------------------------------------------
// Test B: local WebSocket feed -> BarAggregator -> PaperBroker
// ---------------------------------------------------------------------------

async fn next_client_json(ws: &mut WebSocketStream<TcpStream>) -> Value {
    loop {
        match ws.next().await.expect("client frame").expect("valid frame") {
            Message::Text(text) => {
                return serde_json::from_str(text.as_str()).expect("client JSON");
            }
            Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
            Message::Close(_) => panic!("client closed during the handshake"),
        }
    }
}

async fn send_server_json(ws: &mut WebSocketStream<TcpStream>, value: &Value) {
    let text = serde_json::to_string(value).expect("server JSON");
    ws.send(Message::Text(text.into()))
        .await
        .expect("server frame");
}

fn trade_frame(index: usize) -> Value {
    let timestamp = START_MS + index as i64 * 20_000;
    let price = 100.0 + 12.0 * ((index as f64) * 0.7).sin();
    let at = DateTime::<Utc>::from_timestamp_millis(timestamp).expect("timestamp");
    json!({
        "T": "t",
        "S": SYMBOL,
        "i": index,
        "p": format!("{price:.4}"),
        "s": "10",
        "t": at.to_rfc3339(),
    })
}

async fn run_mock_ws(listener: TcpListener, frames: Vec<Value>, mut stop: watch::Receiver<bool>) {
    let Ok((stream, _)) = listener.accept().await else {
        return;
    };
    let mut ws = accept_async(stream).await.expect("ws handshake");
    send_server_json(&mut ws, &json!([{"T": "success", "msg": "connected"}])).await;
    let auth = next_client_json(&mut ws).await;
    assert_eq!(auth.get("action").and_then(Value::as_str), Some("auth"));
    assert_eq!(auth.get("key").and_then(Value::as_str), Some("key"));
    assert_eq!(auth.get("secret").and_then(Value::as_str), Some("secret"));
    send_server_json(&mut ws, &json!([{"T": "success", "msg": "authenticated"}])).await;
    let subscription = next_client_json(&mut ws).await;
    assert_eq!(
        subscription.get("action").and_then(Value::as_str),
        Some("subscribe")
    );
    send_server_json(&mut ws, &Value::Array(frames)).await;
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    let _ = ws.close(None).await;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn websocket_feed_aggregates_bars_and_cancels_on_shutdown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let mut frames = (0..30).map(trade_frame).collect::<Vec<_>>();
    frames.push(json!({
        "T": "q",
        "S": SYMBOL,
        "bp": "99.0000",
        "bs": "10",
        "ap": "99.1000",
        "as": "10",
        "t": DateTime::<Utc>::from_timestamp_millis(START_MS + 20_000)
            .expect("timestamp")
            .to_rfc3339(),
    }));
    let (stop_tx, stop_rx) = watch::channel(false);
    let server = tokio::spawn(run_mock_ws(listener, frames, stop_rx));

    let mut hub = MarketDataHub::new(256).expect("hub");
    let config = NasdaqWsConfig::custom(format!("ws://{address}"))
        .with_credentials("key", "secret")
        .with_heartbeat_interval(Duration::from_secs(30));
    let provider = Arc::new(
        NasdaqWsProvider::new(config)
            .expect("provider")
            .with_name("mock-nasdaq"),
    );
    hub.register(provider).expect("register");
    let market_events = hub.subscribe_events();
    hub.start().expect("start");
    hub.subscribe(
        "mock-nasdaq",
        vec![Subscription::new(
            InstrumentId::new(SYMBOL, Venue::Nasdaq),
            vec![ChannelKind::Trades, ChannelKind::Quotes],
        )],
    )
    .await
    .expect("subscribe");

    let path = std::env::temp_dir().join(format!(
        "kurumi-exec-live-e2e-{}-{}.jsonl",
        std::process::id(),
        START_MS
    ));
    let journal = JsonlFileJournal::create(&path).expect("journal");
    let broker: Arc<dyn ExecutionBroker> = Arc::new(
        PaperBroker::new(PaperMatchingEngine::frictionless(), Decimal::from(100_000))
            .expect("broker"),
    );
    let mut runner = LiveRunner::new(
        LiveConfig {
            initial_cash: Decimal::from(100_000),
            bar_interval_secs: Some(60),
            cancel_on_shutdown: true,
            ..LiveConfig::default()
        },
        RiskGate::empty(),
        broker,
        Box::new(journal),
    )
    .expect("runner");
    let mut strategy = EmaCrossStrategy::new(2, 4)
        .expect("strategy")
        .with_resting_limit(Decimal::ONE);
    let counter = strategy.bars_seen();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let handle =
        tokio::spawn(async move { runner.run(&mut strategy, market_events, shutdown_rx).await });
    wait_for_bars(&counter, 6).await;
    shutdown_tx.send(true).expect("shutdown");
    let summary = timeout(Duration::from_secs(15), handle)
        .await
        .expect("live shutdown")
        .expect("task")
        .expect("run");
    stop_tx.send(true).expect("stop server");
    let _ = timeout(Duration::from_secs(5), server).await;
    let _ = hub.shutdown().await;

    assert!(summary.events_processed > 0);
    assert!(summary.accepted_orders > 0);
    assert!(
        summary.canceled_orders > 0,
        "shutdown must cancel resting orders"
    );
    assert!(!summary.fills.is_empty(), "the EMA cross must trade");

    let contents = std::fs::read_to_string(&path).expect("journal read");
    let lines = contents.lines().collect::<Vec<_>>();
    assert!(!lines.is_empty());
    let mut saw_submit = false;
    let mut saw_fill = false;
    let mut saw_equity = false;
    let mut saw_canceled = false;
    for line in lines {
        let value: Value = serde_json::from_str(line).expect("valid JSONL");
        match value.get("kind").and_then(Value::as_str) {
            Some("submit") => saw_submit = true,
            Some("fill") => saw_fill = true,
            Some("equity_snapshot") => saw_equity = true,
            Some("canceled") => saw_canceled = true,
            _ => {}
        }
    }
    assert!(saw_submit && saw_fill && saw_equity && saw_canceled);
    let _ = std::fs::remove_file(path);
}

// ---------------------------------------------------------------------------
// Test C: Alpaca REST mock, Decimal order serialization and reconciliation
// ---------------------------------------------------------------------------

struct MockRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl MockRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

fn http_header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

async fn read_http_request(stream: &mut TcpStream) -> MockRequest {
    let mut buffer = Vec::new();
    let header_end = loop {
        if let Some(end) = http_header_end(&buffer) {
            break end;
        }
        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk).await.expect("request read");
        assert!(read > 0);
        buffer.extend_from_slice(&chunk[..read]);
    };
    let header_text = std::str::from_utf8(&buffer[..header_end]).expect("headers");
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
        assert!(read > 0);
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

async fn respond_json(stream: &mut TcpStream, status: u16, reason: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("response write");
    stream.shutdown().await.expect("response shutdown");
}

async fn run_mock_alpaca_http(listener: TcpListener, captured: Arc<StdMutex<Vec<Value>>>) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let captured = Arc::clone(&captured);
        tokio::spawn(async move {
            let request = read_http_request(&mut stream).await;
            assert_eq!(request.header("apca-api-key-id"), Some("key"));
            assert_eq!(request.header("apca-api-secret-key"), Some("secret"));
            let body = if request.body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&request.body).unwrap_or(Value::Null)
            };
            match (request.method.as_str(), request.path.as_str()) {
                ("GET", "/v2/account") => {
                    respond_json(
                        &mut stream,
                        200,
                        "OK",
                        r#"{"cash":"100000.00","equity":"100100.00","buying_power":"200000.00"}"#,
                    )
                    .await;
                }
                ("GET", "/v2/positions") => {
                    respond_json(
                        &mut stream,
                        200,
                        "OK",
                        r#"[{"symbol":"AAPL","qty":"10","avg_entry_price":"190.50"}]"#,
                    )
                    .await;
                }
                ("POST", "/v2/orders") => {
                    captured.lock().expect("capture").push(body.clone());
                    if body.get("symbol").and_then(Value::as_str) == Some("BAD") {
                        respond_json(
                            &mut stream,
                            422,
                            "Unprocessable Entity",
                            r#"{"message":"invalid symbol"}"#,
                        )
                        .await;
                    } else {
                        respond_json(
                            &mut stream,
                            200,
                            "OK",
                            r#"{"id":"order-1","client_order_id":"o-1","status":"accepted","filled_qty":"0","filled_avg_price":null}"#,
                        )
                        .await;
                    }
                }
                ("DELETE", "/v2/orders/cancel-1") => {
                    respond_json(&mut stream, 204, "No Content", "").await;
                }
                ("DELETE", path) if path.starts_with("/v2/orders") => {
                    respond_json(&mut stream, 200, "OK", "[]").await;
                }
                ("GET", path) if path.starts_with("/v2/orders?") => {
                    respond_json(
                        &mut stream,
                        200,
                        "OK",
                        r#"[{"id":"order-2","client_order_id":"poll-1","symbol":"AAPL","side":"buy","status":"filled","filled_qty":"2","filled_avg_price":"190.25"}]"#,
                    )
                    .await;
                }
                _ => {
                    respond_json(&mut stream, 404, "Not Found", r#"{"message":"missing"}"#).await;
                }
            }
        });
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn alpaca_rest_mock_reconciles_orders_and_rejects_422() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let captured = Arc::new(StdMutex::new(Vec::new()));
    let server = tokio::spawn(run_mock_alpaca_http(listener, Arc::clone(&captured)));

    let broker =
        AlpacaTradingBroker::new(format!("http://{address}"), "key", "secret").expect("broker");
    let snapshot: BrokerAccountSnapshot = broker.sync_account().await.expect("account");
    assert_eq!(snapshot.cash, Decimal::from(100_000));
    assert_eq!(snapshot.equity, Decimal::from(100_100));
    assert_eq!(
        snapshot
            .positions
            .get(SYMBOL)
            .map(|position| position.quantity),
        Some(Decimal::from(10))
    );
    assert_eq!(
        snapshot
            .positions
            .get(SYMBOL)
            .map(|position| position.avg_cost),
        Some(Decimal::new(19_050, 2))
    );

    let mut reports = broker.subscribe_reports();
    broker
        .submit_order(OrderRequest::new(
            kurumi_exec::ClientOrderId::new("o-1"),
            SYMBOL,
            OrderSide::Buy,
            OrderType::Limit {
                price: Decimal::new(19_025, 2),
            },
            Decimal::from(2),
            TimeInForce::Gtc,
        ))
        .await
        .expect("submit");
    let accepted = timeout(Duration::from_secs(5), reports.recv())
        .await
        .expect("report timeout")
        .expect("report");
    assert!(matches!(accepted, ExecutionReport::Accepted { .. }));

    let payload = {
        let captured = captured.lock().expect("capture");
        captured.last().cloned().expect("captured order")
    };
    assert_eq!(payload["client_order_id"], "o-1");
    assert_eq!(payload["symbol"], SYMBOL);
    assert_eq!(payload["qty"], "2");
    assert_eq!(payload["side"], "buy");
    assert_eq!(payload["type"], "limit");
    assert_eq!(payload["time_in_force"], "gtc");
    assert_eq!(payload["limit_price"], "190.25");

    broker
        .cancel_order(&kurumi_exec::ClientOrderId::new("cancel-1"))
        .await
        .expect("cancel");
    broker.cancel_all(None).await.expect("cancel all");
    let polled = broker.poll_orders().await.expect("poll");
    assert_eq!(polled.len(), 1);
    match polled.first() {
        Some(ExecutionReport::Fill {
            fill_price,
            fill_qty,
            ..
        }) => {
            assert_eq!(*fill_price, Decimal::new(19_025, 2));
            assert_eq!(*fill_qty, Decimal::from(2));
        }
        other => panic!("unexpected poll report: {other:?}"),
    }
    assert!(broker.poll_orders().await.expect("second poll").is_empty());

    let bad = broker
        .submit_order(OrderRequest::new(
            kurumi_exec::ClientOrderId::new("bad-1"),
            "BAD",
            OrderSide::Buy,
            OrderType::Market,
            Decimal::ONE,
            TimeInForce::Day,
        ))
        .await;
    assert!(
        matches!(&bad, Err(ExecError::BrokerError { message }) if message.contains("422")),
        "unexpected bad order result: {bad:?}"
    );
    server.abort();
}
