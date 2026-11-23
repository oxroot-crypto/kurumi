//! Build a two-asset return matrix and decompose 60/40 portfolio risk.
//!
//! Run with `cargo run -p kurumi --example portfolio_risk`.

use kurumi::portfolio::chrono::{DateTime, Utc};
use kurumi::portfolio::nalgebra::DVector;
use kurumi::prelude::*;

fn at(day: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a = AssetSeries::new(
        "A",
        vec![
            (at(0), 100.0),
            (at(1), 102.0),
            (at(2), 101.0),
            (at(3), 105.0),
            (at(4), 107.0),
            (at(5), 110.0),
        ],
    )?;
    let b = AssetSeries::new(
        "B",
        vec![
            (at(0), 50.0),
            (at(1), 49.5),
            (at(2), 50.5),
            (at(3), 51.0),
            (at(4), 52.0),
            (at(5), 53.0),
        ],
    )?;

    // Align both series on their common timestamps and build a (T, N) matrix.
    let returns =
        ReturnMatrix::from_price_series(&[a, b], AlignmentPolicy::InnerJoin, ReturnMethod::Simple)?;
    let covariance = returns.covariance_matrix()?;

    let weights = PortfolioWeights::new(
        vec!["A".to_owned(), "B".to_owned()],
        DVector::from_vec(vec![0.6, 0.4]),
    )?;
    let risk = risk_decomposition(&weights, &covariance)?;

    println!("portfolio_volatility={:.6}", risk.portfolio_volatility);
    println!("pctr_A={:.4} pctr_B={:.4}", risk.pctr[0], risk.pctr[1]);

    Ok(())
}
