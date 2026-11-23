//! Streaming SMA and risk metrics with the `kurumi` facade.
//!
//! Run with `cargo run -p kurumi --example quickstart`.

use kurumi::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Streaming SMA: O(1) per update, no allocation after construction.
    let mut sma = Sma::new(3)?;
    let closes = [100.0, 101.0, 102.0, 103.0];
    let averages = closes
        .iter()
        .map(|close| sma.next(*close))
        .collect::<Result<Vec<_>, _>>()?;
    println!("SMA(3): {averages:?}");

    // Risk metrics from four periodic simple returns.
    let returns = [0.01, -0.02, 0.015, 0.005];
    let metrics = PerformanceMetrics::from_returns(&returns, MetricsConfig::default())?;
    println!(
        "observations={} annualized_volatility={:.6}",
        metrics.observations, metrics.annualized_volatility
    );

    Ok(())
}
