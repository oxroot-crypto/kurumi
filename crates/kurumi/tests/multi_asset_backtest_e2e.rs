//! End-to-end multi-asset portfolio backtests through the `kurumi` facade.
//!
//! The scenarios use four synthetic assets with different trend, volatility, and
//! correlation profiles. Three trading days of one instrument are deliberately
//! missing to exercise halt handling. The tests cover:
//!
//! - **A**: a 60/40 static target with `Any([Monthly, MaxAbsoluteDrift(5%)])`,
//!   verifying scheduled and drift-triggered rebalances and post-trade
//!   convergence to the target.
//! - **B**: a rolling 60-day Ledoit-Wolf + hierarchical risk-parity allocation
//!   with `Monthly` and `NextBarOpen`, verifying a complete run, non-negative
//!   cash, and benchmark-relative attribution against an equal-weight benchmark.
//! - **C**: a look-ahead bias probe that multiplies the last 100 days of prices
//!   by ten and asserts that the first 656 observations are bit-for-bit
//!   identical to the original run.

use std::collections::{BTreeMap, BTreeSet};

use kurumi::math::LedoitWolfShrinkage;
use kurumi::portfolio::chrono::{DateTime, Datelike, Duration, Utc, Weekday};
use kurumi::prelude::*;
use rust_decimal::Decimal;

const TRADING_DAYS: usize = 756;
const FUTURE_START: usize = 656;
const SYMBOLS: [&str; 4] = ["STOCK", "BOND", "GOLD", "COMMODITY"];
const INITIAL_STATE: u64 = 0x9E37_79B9_7F4A_7C15;

fn standard_normal(state: &mut u64) -> f64 {
    let mut uniform = || {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((*state >> 11) as f64) / ((1_u64 << 53) as f64)
    };
    let first = uniform().max(1e-12);
    let second = uniform();
    (-2.0 * first.ln()).sqrt() * (std::f64::consts::TAU * second).cos()
}

/// Generates 756 trading-day prices for four correlated assets.
///
/// Day 45 carries a deliberate stock rally and day 300 a bond sell-off, so the
/// drift trigger fires independently of the monthly schedule.
fn simulated_closes() -> BTreeMap<String, Vec<f64>> {
    let mut state = INITIAL_STATE;
    let mut prices = [100.0_f64, 90.0, 60.0, 70.0];
    let mut closes: BTreeMap<String, Vec<f64>> = SYMBOLS
        .iter()
        .map(|symbol| ((*symbol).to_owned(), Vec::with_capacity(TRADING_DAYS)))
        .collect();

    for day in 0..TRADING_DAYS {
        let market = 0.0002 + 0.0085 * standard_normal(&mut state);
        let mut returns = [
            0.0004 + 1.10 * market + 0.0080 * standard_normal(&mut state),
            0.00015 - 0.25 * market + 0.0025 * standard_normal(&mut state),
            0.0002 - 0.10 * market + 0.0060 * standard_normal(&mut state),
            0.00025 + 0.50 * market + 0.0090 * standard_normal(&mut state),
        ];
        if day == 45 {
            returns[0] += 0.25;
        }
        if day == 300 {
            returns[1] -= 0.08;
        }
        for (index, symbol) in SYMBOLS.iter().enumerate() {
            let Some(price) = prices.get_mut(index) else {
                continue;
            };
            *price *= 1.0 + returns.get(index).copied().unwrap_or(0.0);
            if let Some(series) = closes.get_mut(*symbol) {
                series.push(*price);
            }
        }
    }
    closes
}

fn trading_days(count: usize) -> Vec<DateTime<Utc>> {
    let mut current =
        DateTime::<Utc>::from_timestamp(1_672_531_200, 0).expect("2023-01-02 is valid");
    let mut days = Vec::with_capacity(count);
    while days.len() < count {
        if !matches!(current.weekday(), Weekday::Sat | Weekday::Sun) {
            days.push(current);
        }
        current += Duration::days(1);
    }
    days
}

fn missing_days() -> BTreeSet<(String, usize)> {
    BTreeSet::from([
        ("GOLD".to_owned(), 200),
        ("GOLD".to_owned(), 201),
        ("GOLD".to_owned(), 202),
    ])
}

fn build_feed(
    closes: &BTreeMap<String, Vec<f64>>,
    days: &[DateTime<Utc>],
    missing: &BTreeSet<(String, usize)>,
    multiplier: impl Fn(usize) -> f64,
) -> Result<MultiAssetFeed, KurumiError> {
    let mut series = BTreeMap::new();
    for (symbol, values) in closes {
        let mut bars = Vec::with_capacity(values.len());
        for (day, value) in values.iter().enumerate() {
            if missing.contains(&(symbol.clone(), day)) {
                continue;
            }
            let Some(timestamp) = days.get(day) else {
                continue;
            };
            let scale = multiplier(day);
            let close = value * scale;
            let open = if day == 0 {
                close
            } else {
                values.get(day - 1).copied().unwrap_or(*value) * multiplier(day - 1)
            };
            let high = open.max(close) * 1.002;
            let low = open.min(close) * 0.998;
            let bar = OhlcvBar::new(
                timestamp.timestamp_millis(),
                open,
                high,
                low,
                close,
                1_000_000.0,
                close * 1_000_000.0,
            )?;
            bars.push(bar);
        }
        series.insert(symbol.clone(), bars);
    }
    MultiAssetFeed::new(series)
}

fn config(timing: ExecutionTiming) -> Result<PortfolioBacktestConfig, KurumiError> {
    let mut config = PortfolioBacktestConfig::new(Decimal::from(1_000_000))?;
    config.execution_timing = timing;
    config.lookback_window = 60;
    config.allow_fractional = true;
    config.periods_per_year = 252.0;
    Ok(config)
}

type CostModels = (Box<dyn FeeModel>, Box<dyn SlippageModel>);

fn cost_models() -> Result<CostModels, KurumiError> {
    Ok((
        Box::new(FixedRateFee::new(Decimal::new(5, 4))?),
        Box::new(FixedBpsSlippage::new(Decimal::from(5))?),
    ))
}

fn static_allocator() -> Result<StaticWeightAllocator, KurumiError> {
    StaticWeightAllocator::new(BTreeMap::from([
        ("STOCK".to_owned(), 0.6),
        ("BOND".to_owned(), 0.4),
    ]))
}

fn first_trading_days_of_month(days: &[DateTime<Utc>]) -> BTreeSet<i64> {
    let mut first = BTreeSet::new();
    for (index, day) in days.iter().enumerate() {
        let new_month = match index.checked_sub(1).and_then(|previous| days.get(previous)) {
            Some(previous) => previous.year() != day.year() || previous.month() != day.month(),
            None => true,
        };
        if new_month {
            first.insert(day.timestamp_millis());
        }
    }
    first
}

fn max_weight_deviation(event: &RebalanceEvent) -> f64 {
    let mut symbols = BTreeSet::new();
    symbols.extend(event.before_weights.keys().cloned());
    symbols.extend(event.target_weights.keys().cloned());
    symbols.extend(event.after_weights.keys().cloned());
    symbols.into_iter().fold(0.0, |maximum, symbol| {
        let target = event.target_weights.get(&symbol).copied().unwrap_or(0.0);
        let after = event.after_weights.get(&symbol).copied().unwrap_or(0.0);
        maximum.max((after - target).abs())
    })
}

fn equal_weight_benchmark(
    closes: &BTreeMap<String, Vec<f64>>,
    days: &[DateTime<Utc>],
) -> Vec<(DateTime<Utc>, f64)> {
    let mut prices = Vec::with_capacity(days.len());
    let mut level = 1.0;
    for (day_index, day) in days.iter().enumerate() {
        if day_index > 0 {
            let mut total = 0.0;
            let mut count = 0.0;
            for symbol in SYMBOLS {
                if let (Some(values), Some(previous)) = (
                    closes.get(symbol),
                    closes
                        .get(symbol)
                        .and_then(|values| values.get(day_index - 1)),
                ) {
                    if let Some(current) = values.get(day_index) {
                        total += current / previous - 1.0;
                        count += 1.0;
                    }
                }
            }
            if count > 0.0 {
                level *= 1.0 + total / count;
            }
        }
        prices.push((*day, level));
    }
    prices
}

#[test]
fn scenario_a_static_60_40_with_calendar_and_drift_triggers() -> Result<(), KurumiError> {
    let closes = simulated_closes();
    let days = trading_days(TRADING_DAYS);
    let feed = build_feed(&closes, &days, &missing_days(), |_| 1.0)?;
    let (fee, slippage) = cost_models()?;
    let mut engine =
        PortfolioBacktestEngine::new(config(ExecutionTiming::SignalBarClose)?, fee, slippage)?;
    let mut allocator = static_allocator()?;
    let trigger = CompositeTrigger::Any(vec![
        Box::new(CalendarTrigger::Monthly),
        Box::new(DriftTrigger::max_absolute(0.05)?),
    ]);
    let report = engine.run(feed, &trigger, &mut allocator)?;

    assert!(report.rebalance_count >= 30);
    assert!(report.metrics.annualized_volatility > 0.0);
    assert!(report.total_fees > Decimal::ZERO);
    assert!(report.total_slippage > Decimal::ZERO);
    assert!(report.cost_drag_bps > 0.0);

    let first_days = first_trading_days_of_month(&days);
    let calendar_events = report
        .rebalance_events
        .iter()
        .filter(|event| event.cause == RebalanceCause::Calendar)
        .collect::<Vec<_>>();
    assert!(!calendar_events.is_empty());
    for event in &calendar_events {
        assert!(
            first_days.contains(&event.timestamp.timestamp_millis()),
            "calendar event at {} is not the first trading day of a month",
            event.timestamp
        );
    }

    let drift_events = report
        .rebalance_events
        .iter()
        .filter(|event| event.cause == RebalanceCause::Drift)
        .collect::<Vec<_>>();
    assert!(
        !drift_events.is_empty(),
        "the day-45 rally must trigger a drift rebalance"
    );
    for event in &drift_events {
        assert!(
            !first_days.contains(&event.timestamp.timestamp_millis()),
            "drift event at {} coincides with a calendar rebalance",
            event.timestamp
        );
    }

    for event in &report.rebalance_events {
        let deviation = max_weight_deviation(event);
        assert!(
            deviation < 0.02,
            "post-trade deviation {deviation} at {} did not converge",
            event.timestamp
        );
    }
    Ok(())
}

#[test]
fn scenario_b_rolling_hrp_with_next_bar_open_and_benchmark_attribution() -> Result<(), KurumiError>
{
    let closes = simulated_closes();
    let days = trading_days(TRADING_DAYS);
    let feed = build_feed(&closes, &days, &missing_days(), |_| 1.0)?;
    let (fee, slippage) = cost_models()?;
    let mut engine =
        PortfolioBacktestEngine::new(config(ExecutionTiming::NextBarOpen)?, fee, slippage)?;
    let mut allocator =
        RollingOptimizerAllocator::new(LedoitWolfShrinkage, HierarchicalRiskParity::new())
            .with_min_lookback_periods(60)?;
    let report = engine.run(feed, &CalendarTrigger::Monthly, &mut allocator)?;

    assert!(report.rebalance_count >= 30);
    assert!(report.final_equity > Decimal::ZERO);
    assert!(report.min_cash >= Decimal::ZERO);
    for event in &report.rebalance_events {
        assert!(
            event.frozen_symbols.iter().all(|symbol| symbol == "GOLD"),
            "only the halted GOLD series may be frozen"
        );
        for weight in event
            .target_weights
            .values()
            .chain(event.after_weights.values())
        {
            assert!(weight.is_finite());
        }
    }
    for (_, weights) in &report.weight_history {
        for weight in weights.values() {
            assert!(weight.is_finite() && *weight >= -1e-12);
        }
    }

    let benchmark = equal_weight_benchmark(&closes, &days);
    let comparison = report.with_benchmark(&benchmark, 252.0, 0.0)?;
    assert_eq!(comparison.observations, TRADING_DAYS - 1);
    assert!(comparison.beta.is_finite());
    assert!(comparison.jensen_alpha.is_finite());
    assert!(comparison.tracking_error > 0.0);
    let information_ratio = comparison
        .information_ratio
        .expect("tracking error is positive");
    assert!(information_ratio.is_finite());
    assert!(comparison.excess_return.is_finite());
    Ok(())
}

#[test]
fn scenario_c_no_lookahead_bias_keeps_the_prefix_bit_identical() -> Result<(), KurumiError> {
    let closes = simulated_closes();
    let days = trading_days(TRADING_DAYS);
    let original = build_feed(&closes, &days, &missing_days(), |_| 1.0)?;
    let tampered = build_feed(&closes, &days, &missing_days(), |day| {
        if day >= FUTURE_START { 10.0 } else { 1.0 }
    })?;

    let run = |feed: MultiAssetFeed| -> Result<PortfolioBacktestReport, KurumiError> {
        let (fee, slippage) = cost_models()?;
        let mut engine =
            PortfolioBacktestEngine::new(config(ExecutionTiming::SignalBarClose)?, fee, slippage)?;
        let mut allocator = static_allocator()?;
        engine.run(feed, &CalendarTrigger::Monthly, &mut allocator)
    };
    let original_report = run(original)?;
    let tampered_report = run(tampered)?;

    let cutoff = days
        .get(FUTURE_START)
        .expect("cutoff trading day")
        .timestamp_millis();
    let prefix_curve = |report: &PortfolioBacktestReport| {
        report
            .equity_curve
            .iter()
            .filter(|(timestamp, _)| timestamp.timestamp_millis() < cutoff)
            .cloned()
            .collect::<Vec<_>>()
    };
    let prefix_weights = |report: &PortfolioBacktestReport| {
        report
            .weight_history
            .iter()
            .filter(|(timestamp, _)| timestamp.timestamp_millis() < cutoff)
            .cloned()
            .collect::<Vec<_>>()
    };
    let prefix_events = |report: &PortfolioBacktestReport| {
        report
            .rebalance_events
            .iter()
            .filter(|event| event.timestamp.timestamp_millis() < cutoff)
            .cloned()
            .collect::<Vec<_>>()
    };

    let original_curve = prefix_curve(&original_report);
    let tampered_curve = prefix_curve(&tampered_report);
    assert_eq!(original_curve.len(), FUTURE_START);
    assert_eq!(original_curve, tampered_curve);
    assert_eq!(
        prefix_weights(&original_report),
        prefix_weights(&tampered_report)
    );
    assert_eq!(
        prefix_events(&original_report),
        prefix_events(&tampered_report)
    );
    assert_ne!(
        original_report.equity_curve.last(),
        tampered_report.equity_curve.last(),
        "the future data must actually change the end of the run"
    );
    Ok(())
}
