//! End-to-end integration test across every workspace crate.
//!
//! The scenario:
//!
//! 1. generate a deterministic synthetic price series;
//! 2. compute a dual moving-average signal in `kurumi-ta`;
//! 3. execute the resulting market orders in `kurumi-backtest`;
//! 4. value a European put and its delta hedge with `kurumi-pricing`;
//! 5. report the Sharpe ratio, maximum drawdown, and Value at Risk from
//!    `kurumi-risk`.
//!
//! The option leg also checks the textbook put-call parity benchmark from Hull,
//! *Options, Futures, and Other Derivatives*:
//!
//! ```text
//! C - P = S e^(-qT) - K e^(-rT)
//! ```

use approx::assert_relative_eq;
use kurumi::prelude::*;
use rust_decimal::Decimal;

const BARS: usize = 260;
const FAST_PERIOD: usize = 10;
const SLOW_PERIOD: usize = 30;
const TRADE_QUANTITY: i64 = 500;
const INITIAL_CASH: i64 = 1_000_000;

fn synthetic_prices(length: usize) -> Vec<f64> {
    // A deterministic multi-cycle series: a slow sine trend plus a
    // linear-congruential noise term. No external RNG, so the test is exactly
    // reproducible.
    let mut prices = Vec::with_capacity(length);
    let mut price = 100.0_f64;
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    for index in 0..length {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let unit = ((state >> 11) as f64) / ((1_u64 << 53) as f64);
        let noise = (unit - 0.5) * 0.01;
        let cycle = (index as f64 * 0.30).sin() * 0.012;
        let drift = 0.0004;
        price *= 1.0 + drift + cycle + noise;
        prices.push(price);
    }
    prices
}

fn bars_from_prices(prices: &[f64]) -> Result<Vec<OhlcvBar>, kurumi::core::KurumiError> {
    let mut bars = Vec::with_capacity(prices.len());
    for (index, close) in prices.iter().enumerate() {
        let open = if index == 0 {
            prices[0]
        } else {
            prices[index - 1]
        };
        let high = open.max(*close) * 1.002;
        let low = open.min(*close) * 0.998;
        bars.push(OhlcvBar::new(
            index as i64 * 86_400_000,
            open,
            high,
            low,
            *close,
            1_000.0,
            1_000.0 * close,
        )?);
    }
    Ok(bars)
}

fn price_returns(prices: &[f64]) -> Vec<f64> {
    prices
        .windows(2)
        .map(|pair| pair[1] / pair[0] - 1.0)
        .collect()
}

#[test]
fn end_to_end_dual_moving_average_backtest_with_delta_hedge()
-> Result<(), kurumi::core::KurumiError> {
    // Step 1: deterministic market data.
    let prices = synthetic_prices(BARS);
    assert_eq!(prices.len(), BARS);
    assert!(prices.iter().all(|price| price.is_finite() && *price > 0.0));
    let bars = bars_from_prices(&prices)?;

    // Step 2: dual moving-average signal in kurumi-ta.
    let mut fast = Sma::new(FAST_PERIOD)?;
    let mut slow = Sma::new(SLOW_PERIOD)?;
    let fast_average = fast.compute_batch(&prices)?;
    let slow_average = slow.compute_batch(&prices)?;

    // Step 3: execute in kurumi-backtest. Orders are submitted on the completed
    // signal and fill at the next bar's open.
    let mut engine = BacktestEngine::with_cash(
        Decimal::from(INITIAL_CASH),
        Currency::Usd,
        Box::new(FixedRateFee::new(Decimal::new(5, 4))?),
        Box::new(FixedBpsSlippage::new(Decimal::from(2))?),
    )?;

    let quantity = Quantity::new(Decimal::from(TRADE_QUANTITY))?;
    let mut long = false;
    for (index, bar) in bars.iter().enumerate() {
        if index > 0 {
            let signal_index = index - 1;
            let fast_value = fast_average.get(signal_index).copied().flatten();
            let slow_value = slow_average.get(signal_index).copied().flatten();
            if let (Some(fast_value), Some(slow_value)) = (fast_value, slow_value) {
                let wants_long = fast_value > slow_value;
                if wants_long && !long {
                    engine.submit(Order::market(Side::Buy, quantity))?;
                    long = true;
                } else if !wants_long && long {
                    engine.submit(Order::market(Side::Sell, quantity))?;
                    long = false;
                }
            }
        }
        engine.on_bar(bar)?;
    }

    assert!(
        engine.fills().len() >= 2,
        "the dual moving-average signal should trade at least twice"
    );
    assert_eq!(engine.account().equity_curve().len(), BARS);
    assert!(
        engine
            .fills()
            .iter()
            .all(|fill| fill.fee() >= Decimal::ZERO)
    );

    // Step 4: price a European option on the final observation and build a
    // delta hedge.
    let final_price = prices.last().copied().unwrap_or(100.0);
    let returns = equity_returns(engine.account().equity_curve())?;
    let realized_volatility = annualized_volatility(&price_returns(&prices), 252.0)?;
    assert!(realized_volatility > 0.0);

    let risk_free_rate = 0.03;
    let market = MarketData::new(final_price, risk_free_rate, 0.0, realized_volatility)?;
    let pricing_engine = BlackScholesMertonEngine::new(market);

    let start_date = Date::from_ymd(2024, 1, 1)?;
    let last_date = start_date.checked_add_days((BARS - 1) as i64)?;
    let expiry = last_date.checked_add_days(90)?;
    let strike = final_price;

    let put = OptionContract::new(OptionType::Put, strike, expiry, ExerciseStyle::European)?;
    let call = OptionContract::new(OptionType::Call, strike, expiry, ExerciseStyle::European)?;
    let put_value = pricing_engine.price(&put, last_date)?;
    let call_value = pricing_engine.price(&call, last_date)?;

    let put_greeks = put_value
        .greeks
        .ok_or(kurumi::core::KurumiError::InvalidParameter {
            parameter: "greeks",
            reason: "the Black-Scholes engine must return sensitivities".to_owned(),
        })?;
    assert!(put_greeks.delta < 0.0 && put_greeks.delta > -1.0);

    // Delta hedge a long 100-contract put position with the underlying: the
    // negative put delta turns into a positive underlying hedge.
    let option_units = 100.0;
    let hedge_units = -(put_greeks.delta) * option_units;
    let hedge_notional = hedge_units * final_price;
    assert!(hedge_units.is_finite() && hedge_units > 0.0);
    assert!(hedge_notional.is_finite() && hedge_notional > 0.0);

    // Put-call parity benchmark (Hull, chapter 11).
    let time_to_expiry = put.time_to_expiry(last_date)?;
    let parity_expected = final_price * (0.0 * time_to_expiry).exp()
        - strike * (-risk_free_rate * time_to_expiry).exp();
    assert_relative_eq!(
        call_value.npv - put_value.npv,
        parity_expected,
        epsilon = 1e-8
    );

    // Step 5: performance and risk report from kurumi-risk.
    let config = MetricsConfig::new(252.0, 0.0)?;
    let report = engine.report(config)?;
    assert_eq!(report.fill_count, engine.fills().len());
    assert_eq!(report.metrics.observations, BARS - 1);
    assert!(report.final_equity > Decimal::ZERO);
    assert!((0.0..=1.0).contains(&report.metrics.max_drawdown));

    let sharpe =
        report
            .metrics
            .sharpe_ratio
            .ok_or(kurumi::core::KurumiError::InvalidParameter {
                parameter: "sharpe_ratio",
                reason: "the synthetic series must have non-zero volatility".to_owned(),
            })?;
    assert!(sharpe.is_finite());
    assert_relative_eq!(sharpe, sharpe_ratio(&returns, config)?, epsilon = 1e-12);
    assert_relative_eq!(
        report.metrics.max_drawdown,
        max_drawdown(&returns)?,
        epsilon = 1e-12
    );

    let var = historical_var(&returns, 0.95)?;
    assert!(var >= 0.0 && var.is_finite());

    Ok(())
}
