//! Run a single-asset backtest with a proportional fee and fixed slippage.
//!
//! Run with `cargo run -p kurumi --example backtest`.

use kurumi::prelude::*;
use rust_decimal::Decimal;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = BacktestEngine::with_cash(
        Decimal::from(100_000),
        Currency::Usd,
        Box::new(FixedRateFee::new(Decimal::new(5, 4))?),
        Box::new(FixedBpsSlippage::new(Decimal::from(2))?),
    )?;

    engine.submit(Order::market(Side::Buy, Quantity::new(Decimal::from(100))?))?;

    for (index, price) in [100.0, 102.0, 104.0].iter().enumerate() {
        let bar = OhlcvBar::new(
            index as i64,
            *price,
            price + 1.0,
            price - 1.0,
            *price,
            1_000.0,
            price * 1_000.0,
        )?;
        engine.on_bar(&bar)?;
    }

    let report = engine.report(MetricsConfig::default())?;
    println!(
        "fills={} final_equity={}",
        report.fill_count, report.final_equity
    );

    Ok(())
}
