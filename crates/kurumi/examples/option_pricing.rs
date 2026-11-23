//! Price a one-year at-the-money European call.
//!
//! Run with `cargo run -p kurumi --example option_pricing`.

use kurumi::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let market = MarketData::new(100.0, 0.05, 0.0, 0.20)?;
    let engine = BlackScholesMertonEngine::new(market);
    let call = OptionContract::new(
        OptionType::Call,
        100.0,
        Date::from_ymd(2025, 1, 1)?,
        ExerciseStyle::European,
    )?;

    let value = engine.price(&call, Date::from_ymd(2024, 1, 1)?)?;
    println!("npv={:.4}", value.npv);

    if let Some(greeks) = value.greeks {
        println!("delta={:.4} vega={:.4}", greeks.delta, greeks.vega);
    }

    Ok(())
}
