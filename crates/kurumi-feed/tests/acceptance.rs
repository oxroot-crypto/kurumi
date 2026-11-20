//! Offline acceptance tests for the real-time market-data stack.
//!
//! Every test runs against the in-memory [`MockReplayProvider`]; nothing here
//! opens a socket or reads credentials.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use kurumi_feed::{
    BarAggregator, BarEvent, ChannelKind, InstrumentId, ManagedConnector, MarketDataConnector,
    MarketDataHub, MarketEvent, MockReplayProvider, ReconnectPolicy, Subscription, TradeTick,
    Venue,
};
use kurumi_ta::{Ema, Rsi, StreamingIndicator};
use rust_decimal::Decimal;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::timeout;

/// Builds a UTC timestamp from whole Unix seconds.
fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(seconds, 0).expect("valid timestamp")
}

/// Builds an instrument identifier.
fn instrument(symbol: &str, venue: Venue) -> InstrumentId {
    InstrumentId::new(symbol, venue)
}

/// Builds a trade tick with a price expressed in cents.
fn trade(
    instrument: &InstrumentId,
    seconds: i64,
    price_cents: i64,
    size: i64,
    trade_id: &str,
) -> TradeTick {
    TradeTick {
        instrument: instrument.clone(),
        price: Decimal::new(price_cents, 2),
        size: Decimal::new(size, 0),
        trade_id: Some(trade_id.to_owned()),
        conditions: Vec::new(),
        exchange_ts: at(seconds),
        received_ts: at(seconds),
    }
}

/// Collects exactly `count` trade ids from one broadcast receiver.
async fn collect_trade_ids(
    receiver: &mut broadcast::Receiver<MarketEvent>,
    count: usize,
) -> Vec<String> {
    let mut ids = Vec::with_capacity(count);
    while ids.len() < count {
        match receiver.recv().await {
            Ok(MarketEvent::Trade(tick)) => ids.push(tick.trade_id.unwrap_or_default()),
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                panic!("subscriber lagged by {skipped} events")
            }
            Err(broadcast::error::RecvError::Closed) => {
                panic!("hub closed before all events arrived")
            }
        }
    }
    ids
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn managed_connector_reconnects_after_two_disconnects_and_resubscribes() {
    let aapl = instrument("AAPL", Venue::Nasdaq);
    let subscription = Subscription::new(aapl.clone(), vec![ChannelKind::Trades]);
    let events: Vec<MarketEvent> = (0..10)
        .map(|index| {
            MarketEvent::Trade(trade(&aapl, index, 10_000 + index, 1, &format!("t{index}")))
        })
        .collect();
    let provider = Arc::new(
        MockReplayProvider::new("mock-nasdaq", vec![Venue::Nasdaq], events)
            .with_disconnects(vec![2, 5]),
    );
    let connector: Arc<dyn MarketDataConnector> = provider.clone();
    let policy = ReconnectPolicy {
        initial_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(4),
        backoff_multiplier: 2.0,
        max_retries: Some(5),
        heartbeat_timeout: Duration::from_secs(5),
    };
    let managed = ManagedConnector::new(connector, policy).expect("valid policy");
    let (event_tx, mut event_rx) = mpsc::channel(256);
    let (_command_tx, command_rx) = mpsc::channel(8);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let initial = vec![subscription.clone()];
    let handle = tokio::spawn(async move {
        managed
            .run(initial, event_tx, command_rx, shutdown_rx)
            .await
    });

    let mut received = Vec::new();
    timeout(Duration::from_secs(10), async {
        while received.len() < 10 {
            match event_rx.recv().await {
                Some(MarketEvent::Trade(tick)) => received.push(tick),
                Some(_) => {}
                None => break,
            }
        }
    })
    .await
    .expect("managed connector stopped before all trades arrived");

    assert_eq!(received.len(), 10);
    for (index, tick) in received.iter().enumerate() {
        let expected = format!("t{index}");
        assert_eq!(tick.trade_id.as_deref(), Some(expected.as_str()));
        assert_eq!(tick.price, Decimal::new(10_000 + index as i64, 2));
    }
    assert_eq!(provider.connect_calls(), 3);

    let recorded = provider.initial_subscriptions();
    assert_eq!(recorded.len(), 3);
    for subscriptions in &recorded {
        assert_eq!(subscriptions.len(), 1);
        assert_eq!(subscriptions.first(), Some(&subscription));
    }

    let _ = shutdown_tx.send(true);
    let outcome = timeout(Duration::from_secs(5), handle)
        .await
        .expect("run task did not stop");
    assert!(outcome.expect("run task panicked").is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hub_fans_two_venues_out_to_two_subscribers_and_stops_cleanly() {
    let aapl = instrument("AAPL", Venue::Nasdaq);
    let msft = instrument("MSFT", Venue::Nyse);
    let nasdaq_events: Vec<MarketEvent> = (0..6)
        .map(|index| {
            MarketEvent::Trade(trade(&aapl, index, 20_000 + index, 1, &format!("a{index}")))
        })
        .collect();
    let nyse_events: Vec<MarketEvent> = (0..6)
        .map(|index| {
            MarketEvent::Trade(trade(&msft, index, 30_000 + index, 1, &format!("b{index}")))
        })
        .collect();

    let nasdaq_provider: Arc<dyn MarketDataConnector> = Arc::new(MockReplayProvider::new(
        "nasdaq-mock",
        vec![Venue::Nasdaq],
        nasdaq_events,
    ));
    let nyse_provider: Arc<dyn MarketDataConnector> = Arc::new(MockReplayProvider::new(
        "nyse-mock",
        vec![Venue::Nyse],
        nyse_events,
    ));

    let mut hub = MarketDataHub::new(256).expect("valid capacity");
    hub.register(nasdaq_provider).expect("nasdaq registers");
    hub.register(nyse_provider).expect("nyse registers");
    assert_eq!(
        hub.provider_names(),
        vec!["nasdaq-mock".to_owned(), "nyse-mock".to_owned()]
    );

    let mut first = hub.subscribe_events();
    let mut second = hub.subscribe_events();
    hub.start().expect("hub starts");

    hub.subscribe(
        "nasdaq-mock",
        vec![Subscription::new(aapl, vec![ChannelKind::Trades])],
    )
    .await
    .expect("nasdaq subscription");
    hub.subscribe(
        "nyse-mock",
        vec![Subscription::new(msft, vec![ChannelKind::Trades])],
    )
    .await
    .expect("nyse subscription");

    let (first_ids, second_ids) = tokio::join!(
        timeout(Duration::from_secs(10), collect_trade_ids(&mut first, 12)),
        timeout(Duration::from_secs(10), collect_trade_ids(&mut second, 12)),
    );
    let mut first_ids = first_ids.expect("first subscriber did not receive every event");
    let mut second_ids = second_ids.expect("second subscriber did not receive every event");

    let mut expected: Vec<String> = (0..6).map(|index| format!("a{index}")).collect();
    expected.extend((0..6).map(|index| format!("b{index}")));
    expected.sort();

    first_ids.sort();
    second_ids.sort();
    assert_eq!(first_ids, expected);
    assert_eq!(second_ids, expected);

    hub.shutdown().await.expect("hub shuts down cleanly");
}

#[test]
fn bar_aggregator_handles_boundaries_late_ticks_and_feeds_kurumi_ta() {
    let aapl = instrument("AAPL", Venue::Nasdaq);
    let mut aggregator = BarAggregator::new(60).expect("valid interval");
    let start = 1_700_000_040;
    assert_eq!(start % 60, 0);

    // First bucket: open 100, high 102, close 102, volume 5, turnover 506.
    assert!(
        aggregator
            .on_trade(&trade(&aapl, start + 10, 10_000, 2, "a"))
            .expect("first trade")
            .is_none()
    );
    assert!(
        aggregator
            .on_trade(&trade(&aapl, start + 20, 10_200, 3, "b"))
            .expect("second trade")
            .is_none()
    );

    // A trade in the next bucket finalizes the first one.
    let first = aggregator
        .on_trade(&trade(&aapl, start + 70, 10_100, 1, "c"))
        .expect("rollover")
        .expect("first bar");
    assert_eq!(first.instrument, aapl);
    assert_eq!(first.interval_secs, 60);
    assert!(first.is_final);
    assert_eq!(first.bar.timestamp(), start * 1_000);
    approx::assert_relative_eq!(first.bar.open(), 100.0, epsilon = 1e-12);
    approx::assert_relative_eq!(first.bar.high(), 102.0, epsilon = 1e-12);
    approx::assert_relative_eq!(first.bar.low(), 100.0, epsilon = 1e-12);
    approx::assert_relative_eq!(first.bar.close(), 102.0, epsilon = 1e-12);
    approx::assert_relative_eq!(first.bar.volume(), 5.0, epsilon = 1e-12);
    approx::assert_relative_eq!(first.bar.turnover(), 506.0, epsilon = 1e-12);

    // A tick from the finalized bucket is dropped as late.
    assert!(
        aggregator
            .on_trade(&trade(&aapl, start + 30, 9_000, 1, "late"))
            .expect("late trade")
            .is_none()
    );

    // Out-of-order arrivals: the exchange timestamp, not arrival order,
    // determines the close; the high and low still cover every accepted trade.
    assert!(
        aggregator
            .on_trade(&trade(&aapl, start + 90, 10_300, 1, "d"))
            .expect("out-of-order first")
            .is_none()
    );
    assert!(
        aggregator
            .on_trade(&trade(&aapl, start + 75, 10_000, 1, "e"))
            .expect("out-of-order second")
            .is_none()
    );

    let mut bars: Vec<BarEvent> = vec![first];
    bars.push(
        aggregator
            .on_trade(&trade(&aapl, start + 130, 10_400, 1, "f"))
            .expect("second rollover")
            .expect("second bar"),
    );
    bars.push(
        aggregator
            .on_trade(&trade(&aapl, start + 190, 10_500, 1, "g"))
            .expect("third rollover")
            .expect("third bar"),
    );
    bars.extend(aggregator.flush().expect("flush"));

    assert_eq!(bars.len(), 4);
    let second = bars.get(1).expect("second bar");
    assert_eq!(second.bar.timestamp(), (start + 60) * 1_000);
    approx::assert_relative_eq!(second.bar.open(), 101.0, epsilon = 1e-12);
    approx::assert_relative_eq!(second.bar.high(), 103.0, epsilon = 1e-12);
    approx::assert_relative_eq!(second.bar.low(), 100.0, epsilon = 1e-12);
    approx::assert_relative_eq!(second.bar.close(), 103.0, epsilon = 1e-12);
    approx::assert_relative_eq!(second.bar.volume(), 3.0, epsilon = 1e-12);
    approx::assert_relative_eq!(second.bar.turnover(), 304.0, epsilon = 1e-12);

    let closes: Vec<f64> = bars.iter().map(|bar| bar.bar.close()).collect();
    for (actual, expected) in closes.iter().zip([102.0, 103.0, 104.0, 105.0]) {
        approx::assert_relative_eq!(*actual, expected, epsilon = 1e-12);
    }

    let mut ema = Ema::new(3).expect("valid period");
    let ema_outputs = ema.compute_batch(&closes).expect("ema batch");
    assert_eq!(ema_outputs.first(), Some(&None));
    assert_eq!(ema_outputs.get(1), Some(&None));
    let seeded = ema_outputs.get(2).copied().flatten().expect("ema seeded");
    approx::assert_relative_eq!(seeded, 103.0, epsilon = 1e-12);
    let updated = ema_outputs.get(3).copied().flatten().expect("ema updated");
    approx::assert_relative_eq!(updated, 104.0, epsilon = 1e-12);

    let mut rsi = Rsi::new(2).expect("valid period");
    let rsi_outputs = rsi.compute_batch(&closes).expect("rsi batch");
    assert_eq!(rsi_outputs.first(), Some(&None));
    assert_eq!(rsi_outputs.get(1), Some(&None));
    let first_rsi = rsi_outputs.get(2).copied().flatten().expect("rsi seeded");
    approx::assert_relative_eq!(first_rsi, 100.0, epsilon = 1e-12);
    let second_rsi = rsi_outputs.get(3).copied().flatten().expect("rsi updated");
    approx::assert_relative_eq!(second_rsi, 100.0, epsilon = 1e-12);
}
