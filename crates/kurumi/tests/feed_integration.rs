//! Facade integration test for the opt-in `feed` feature.
//!
//! The file compiles to an empty test crate when `feed` is disabled, so the
//! synchronous-only facade build stays free of the asynchronous runtime.

#![cfg(feature = "feed")]

use kurumi::prelude::*;

#[test]
fn prelude_exposes_the_nasdaq_feed_surface() {
    let config = NasdaqWsConfig::alpaca_sip().with_credentials("key", "secret");
    let provider = NasdaqWsProvider::new(config).expect("valid provider configuration");
    assert_eq!(provider.name(), "nasdaq-ws");
    assert!(MarketDataHub::new(16).is_ok());
    assert!(BarAggregator::new(60).is_ok());
    assert!(
        NcdsRestConfig::new(
            "https://example.com",
            "https://example.com/v1/auth/token",
            "client",
            "secret",
            std::time::Duration::from_secs(1),
        )
        .validate()
        .is_ok()
    );
}
