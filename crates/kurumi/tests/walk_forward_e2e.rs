//! End-to-end walk-forward analysis through the `kurumi` facade.
//!
//! The scenarios use four synthetic assets over 1,000 trading days with a
//! deliberate regime switch: the first half is a trending market and the second
//! half is a choppy, mean-reverting market. Every parameter set drives a rolling
//! `LedoitWolfShrinkage` + `HierarchicalRiskParity` allocator and a
//! `CompositeTrigger::Any([Monthly, MaxAbsoluteDrift])`. The tests cover:
//!
//! - **A**: rolling WFA with IS = 250, OOS = 100, warmup = 60, embargo = 5. The
//!   fold plan, the adjacent stitched OOS curve, and the complete overfitting
//!   diagnostics are verified.
//! - **B**: anchored WFA. Every fold starts its in-sample window at zero and the
//!   in-sample end grows strictly.
//! - **C**: a look-ahead probe that multiplies every price from the last fold's
//!   OOS start onward by five. All in-sample best-parameter selections, including
//!   the last fold's, must be bit-for-bit unchanged.

use std::collections::BTreeMap;

use kurumi::math::LedoitWolfShrinkage;
use kurumi::portfolio::chrono::{DateTime, Datelike, Duration, Utc, Weekday};
use kurumi::prelude::*;
use rust_decimal::Decimal;

const TRADING_DAYS: usize = 1_000;
const REGIME_SWITCH: usize = TRADING_DAYS / 2;
const SYMBOLS: [&str; 4] = ["STOCK", "BOND", "GOLD", "COMMODITY"];
const INITIAL_STATE: u64 = 0x9E37_79B9_7F4A_7C15;
const INITIAL_CASH: u64 = 1_000_000;
const RISK_FREE_RATE: f64 = 0.02;

/// Deterministic standard-normal draws from a linear congruential generator.
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

/// Generates 1,000 trading-day closes for four correlated assets.
///
/// Days before [`REGIME_SWITCH`] carry a positive market drift plus a slow
/// cycle; afterwards the drift collapses and a fast deterministic oscillation
/// makes the market choppy, so the parameter grid's preferred lookback and drift
/// threshold can change between regimes.
fn simulated_closes() -> BTreeMap<String, Vec<f64>> {
    let mut state = INITIAL_STATE;
    let mut prices = [100.0_f64, 90.0, 60.0, 70.0];
    let mut closes: BTreeMap<String, Vec<f64>> = SYMBOLS
        .iter()
        .map(|symbol| ((*symbol).to_owned(), Vec::with_capacity(TRADING_DAYS)))
        .collect();

    for day in 0..TRADING_DAYS {
        let market = if day < REGIME_SWITCH {
            0.0012 + 0.0040 * standard_normal(&mut state) + 0.08 * (day as f64 * 0.05).sin()
        } else {
            0.0003 + 0.0060 * standard_normal(&mut state) - 0.010 * (day as f64 * 0.80).sin()
        };
        let returns = [
            0.0006 + 1.10 * market + 0.0060 * standard_normal(&mut state),
            0.0002 - 0.25 * market + 0.0020 * standard_normal(&mut state),
            0.0003 - 0.15 * market + 0.0045 * standard_normal(&mut state),
            0.0004 + 0.55 * market + 0.0070 * standard_normal(&mut state),
        ];
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

/// Weekday-only timestamps starting at 2023-01-02.
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

/// Builds a feed, scaling a bar's prices by `multiplier(day_index)`.
fn build_feed(
    closes: &BTreeMap<String, Vec<f64>>,
    days: &[DateTime<Utc>],
    multiplier: impl Fn(usize) -> f64,
) -> Result<MultiAssetFeed, KurumiError> {
    let mut series = BTreeMap::new();
    for (symbol, values) in closes {
        let mut bars = Vec::with_capacity(values.len());
        for (day, value) in values.iter().enumerate() {
            let Some(timestamp) = days.get(day) else {
                continue;
            };
            let close = value * multiplier(day);
            let open = if day == 0 {
                close
            } else {
                values.get(day - 1).copied().unwrap_or(*value) * multiplier(day - 1)
            };
            let high = open.max(close) * 1.002;
            let low = open.min(close) * 0.998;
            bars.push(OhlcvBar::new(
                timestamp.timestamp_millis(),
                open,
                high,
                low,
                close,
                1_000_000.0,
                close * 1_000_000.0,
            )?);
        }
        series.insert(symbol.clone(), bars);
    }
    MultiAssetFeed::new(series)
}

fn aligned_data(multiplier: impl Fn(usize) -> f64) -> Result<AlignedMarketData, KurumiError> {
    let closes = simulated_closes();
    let days = trading_days(TRADING_DAYS);
    Ok(AlignedMarketData::from_feed(build_feed(
        &closes, &days, multiplier,
    )?))
}

/// The 3 x 3 parameter grid: lookback x drift threshold.
#[derive(Debug, Clone, Copy, PartialEq)]
struct StrategyParams {
    lookback: usize,
    drift_threshold: f64,
}

fn parameter_grid() -> Vec<StrategyParams> {
    let mut params = Vec::with_capacity(9);
    for lookback in [20_usize, 40, 60] {
        for drift_threshold in [0.03_f64, 0.05, 0.08] {
            params.push(StrategyParams {
                lookback,
                drift_threshold,
            });
        }
    }
    params
}

fn grid_config() -> GridSearchConfig {
    GridSearchConfig::new(ObjectiveMetric::SharpeRatio)
        .with_filter(
            ResultFilter::new()
                .with_min_rebalance_count(1)
                .with_max_drawdown_limit(0.90),
        )
        .with_retain_all_reports(false)
}

fn walk_forward_config(mode: WalkForwardMode) -> WalkForwardConfig {
    WalkForwardConfig::new(250, 100, grid_config())
        .with_mode(mode)
        .with_embargo_bars(5)
        .with_warmup_bars(60)
        .with_periods_per_year(252.0)
        .with_risk_free_rate(RISK_FREE_RATE)
}

/// Builds one strategy: rolling Ledoit-Wolf + HRP allocation and a composite
/// monthly/drift rebalance trigger.
fn strategy_factory(params: &StrategyParams) -> Result<StrategySpec, KurumiError> {
    let mut config = PortfolioBacktestConfig::new(Decimal::from(INITIAL_CASH))?;
    config.lookback_window = params.lookback;
    config.allow_fractional = true;
    config.periods_per_year = 252.0;
    config.risk_free_rate = RISK_FREE_RATE;
    config.execution_timing = ExecutionTiming::SignalBarClose;

    let allocator =
        RollingOptimizerAllocator::new(LedoitWolfShrinkage, HierarchicalRiskParity::new())
            .with_min_lookback_periods(20)?;
    let trigger = CompositeTrigger::Any(vec![
        Box::new(CalendarTrigger::Monthly),
        Box::new(DriftTrigger::max_absolute(params.drift_threshold)?),
    ]);
    Ok(
        StrategySpec::new(config, Box::new(trigger), Box::new(allocator))
            .with_fee_model(Box::new(FixedRateFee::new(Decimal::new(5, 4))?))
            .with_slippage_model(Box::new(FixedBpsSlippage::new(Decimal::from(5))?)),
    )
}

fn run_walk_forward(
    data: &AlignedMarketData,
    mode: WalkForwardMode,
) -> Result<WalkForwardReport<StrategyParams>, KurumiError> {
    let params = parameter_grid();
    WalkForwardAnalyzer::new(walk_forward_config(mode)).run(data, &params, strategy_factory)
}

#[test]
fn scenario_a_rolling_wfa_stitches_oos_curve_and_reports_diagnostics() -> Result<(), KurumiError> {
    let data = aligned_data(|_| 1.0)?;
    let report = run_walk_forward(&data, WalkForwardMode::Rolling)?;

    // (1000 - 250 - 5 - 100) / 100 + 1 = 7 complete folds.
    assert_eq!(report.mode, WalkForwardMode::Rolling);
    assert_eq!(report.folds.len(), 7);

    for (index, fold) in report.folds.iter().enumerate() {
        assert_eq!(fold.window.fold_id, index);
        assert_eq!(fold.window.is_start, index * 100);
        assert_eq!(fold.window.is_bars(), 250);
        assert_eq!(fold.window.oos_bars(), 100);
        assert_eq!(fold.window.is_warmup(), if index == 0 { 0 } else { 60 });
        assert_eq!(fold.window.oos_warmup(), 60);
        assert_eq!(fold.window.oos_start, fold.window.is_end + 5);
        assert!(fold.window.is_end <= fold.window.oos_start);
        assert_eq!(fold.oos_report.equity_curve.len(), 100);
    }

    // The stitched curve contains every OOS bar of every fold, in order.
    assert_eq!(report.stitched_oos_equity_curve.len(), 7 * 100);
    assert_eq!(report.oos_metrics.observations, 699);

    let mut expected = Vec::with_capacity(700);
    for fold in &report.folds {
        let start = fold.window.oos_start;
        for (offset, (timestamp, _)) in fold.oos_report.equity_curve.iter().enumerate() {
            let expected_snapshot = data
                .snapshots()
                .get(start + offset)
                .map(|snapshot| snapshot.timestamp());
            assert_eq!(Some(*timestamp), expected_snapshot);
            if let Some(previous) = expected.last() {
                assert!(timestamp > previous, "timestamps must strictly increase");
            }
            expected.push(*timestamp);
        }
    }
    assert_eq!(
        report
            .stitched_oos_equity_curve
            .iter()
            .map(|(timestamp, _)| *timestamp)
            .collect::<Vec<_>>(),
        expected
    );
    assert!(report.oos_initial_equity > Decimal::ZERO);
    assert!(report.oos_final_equity > Decimal::ZERO);

    let metrics = &report.oos_metrics;
    assert!(metrics.annualized_return.is_finite());
    assert!(metrics.annualized_volatility.is_finite());
    assert!(metrics.annualized_volatility > 0.0);
    assert!(metrics.max_drawdown.is_finite() && metrics.max_drawdown >= 0.0);
    assert!(metrics.sharpe_ratio.is_some_and(|value| value.is_finite()));
    assert!(metrics.sortino_ratio.is_some_and(|value| value.is_finite()));
    assert!(report.oos_total_turnover.is_finite());
    assert!(report.oos_total_turnover >= 0.0);
    assert!(report.oos_annualized_turnover.is_finite());
    assert!(report.oos_annualized_turnover >= 0.0);

    let diagnostics = &report.diagnostics;
    let mean_is_annualized = diagnostics
        .mean_is_best_annualized_return
        .expect("every admissible fold reports an annualized return");
    assert!(mean_is_annualized.is_finite());
    if mean_is_annualized > 0.0 {
        assert!(
            diagnostics
                .walk_forward_efficiency
                .is_some_and(|value| value.is_finite())
        );
    } else {
        assert!(diagnostics.walk_forward_efficiency.is_none());
    }
    match (metrics.sharpe_ratio, diagnostics.mean_is_best_sharpe) {
        (Some(oos), Some(is)) => {
            let degradation = diagnostics
                .sharpe_degradation
                .expect("both Sharpe ratios are defined");
            assert!((degradation - (oos - is)).abs() <= 1e-12);
        }
        _ => assert!(diagnostics.sharpe_degradation.is_none()),
    }
    assert!((0.0..=1.0).contains(&diagnostics.oos_profitable_fold_ratio));
    assert!((0.0..=1.0).contains(&diagnostics.parameter_stability_ratio));
    Ok(())
}

#[test]
fn scenario_b_anchored_wfa_starts_every_window_at_zero() -> Result<(), KurumiError> {
    let data = aligned_data(|_| 1.0)?;
    let report = run_walk_forward(&data, WalkForwardMode::Anchored)?;

    assert_eq!(report.mode, WalkForwardMode::Anchored);
    assert_eq!(report.folds.len(), 7);
    assert_eq!(report.stitched_oos_equity_curve.len(), 700);
    assert_eq!(report.oos_metrics.observations, 699);

    for (index, fold) in report.folds.iter().enumerate() {
        assert_eq!(fold.window.is_start, 0);
        assert_eq!(fold.window.is_warmup(), 0);
        assert_eq!(fold.window.is_end, 250 + index * 100);
        assert_eq!(fold.window.oos_end - fold.window.oos_start, 100);
        if let Some(previous) = index
            .checked_sub(1)
            .and_then(|previous| report.folds.get(previous))
        {
            assert!(
                previous.window.is_end < fold.window.is_end,
                "anchored in-sample ends must strictly increase"
            );
            assert_eq!(previous.window.oos_end, fold.window.oos_start);
        }
    }
    for pair in report.stitched_oos_equity_curve.windows(2) {
        let (Some(left), Some(right)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        assert!(right.0 > left.0);
    }
    Ok(())
}

#[test]
fn scenario_c_no_lookahead_keeps_every_in_sample_winner() -> Result<(), KurumiError> {
    let baseline_data = aligned_data(|_| 1.0)?;
    let config = walk_forward_config(WalkForwardMode::Rolling);
    let folds = generate_folds(TRADING_DAYS, &config)?;
    let last = folds.last().copied().expect("at least one fold");

    // Multiply every price in the last fold's out-of-sample period by five.
    let cutoff = last.oos_start;
    let tampered_data = aligned_data(|day| if day >= cutoff { 5.0 } else { 1.0 })?;

    let baseline = run_walk_forward(&baseline_data, WalkForwardMode::Rolling)?;
    let tampered = run_walk_forward(&tampered_data, WalkForwardMode::Rolling)?;

    assert_eq!(baseline.folds.len(), tampered.folds.len());
    for (baseline_fold, tampered_fold) in baseline.folds.iter().zip(tampered.folds.iter()) {
        assert_eq!(baseline_fold.window, tampered_fold.window);
        assert_eq!(
            baseline_fold.best_params, tampered_fold.best_params,
            "fold {} in-sample winner changed after tampering future prices",
            baseline_fold.window.fold_id
        );
        assert_eq!(
            baseline_fold.is_result.best_index, tampered_fold.is_result.best_index,
            "fold {} in-sample ranking changed after tampering future prices",
            baseline_fold.window.fold_id
        );
        assert_eq!(baseline_fold.used_fallback, tampered_fold.used_fallback);
        assert_eq!(
            baseline_fold
                .is_best_summary
                .map(|summary| summary.annualized_return),
            tampered_fold
                .is_best_summary
                .map(|summary| summary.annualized_return)
        );
    }

    // The tampered future must actually reach the last fold's OOS evaluation.
    let baseline_last = baseline.folds.last().expect("last fold");
    let tampered_last = tampered.folds.last().expect("last fold");
    assert_ne!(
        baseline_last.oos_report.final_equity, tampered_last.oos_report.final_equity,
        "the tampered OOS prices must change the last fold's result"
    );
    assert_ne!(
        baseline.oos_final_equity, tampered.oos_final_equity,
        "the tampered OOS prices must change the stitched curve"
    );
    Ok(())
}

#[test]
fn scenario_d_embargo_boundary_tamper_keeps_the_last_in_sample_result() -> Result<(), KurumiError> {
    let baseline_data = aligned_data(|_| 1.0)?;
    let config = walk_forward_config(WalkForwardMode::Rolling);
    let folds = generate_folds(TRADING_DAYS, &config)?;
    let last = folds.last().copied().expect("at least one fold");

    // Tamper from the last fold's IS end, so the embargo bars
    // [is_end, oos_start) are also tampered. IS must never observe them.
    let tampered_data = aligned_data(|day| if day >= last.is_end { 5.0 } else { 1.0 })?;

    let baseline = run_walk_forward(&baseline_data, WalkForwardMode::Rolling)?;
    let tampered = run_walk_forward(&tampered_data, WalkForwardMode::Rolling)?;

    let baseline_last = baseline.folds.last().expect("last fold");
    let tampered_last = tampered.folds.last().expect("last fold");
    assert_eq!(baseline_last.window, tampered_last.window);
    assert_eq!(
        baseline_last.best_params, tampered_last.best_params,
        "tampering the embargo bars must not change the last fold's IS winner"
    );
    assert_eq!(
        baseline_last.is_result.best_index, tampered_last.is_result.best_index,
        "tampering the embargo bars must not change the last fold's IS ranking"
    );
    assert_eq!(
        baseline_last
            .is_best_summary
            .map(|summary| summary.annualized_return),
        tampered_last
            .is_best_summary
            .map(|summary| summary.annualized_return)
    );
    assert_ne!(
        baseline_last.oos_report.final_equity, tampered_last.oos_report.final_equity,
        "the tampered embargo/OOS data must still reach the OOS evaluation"
    );
    Ok(())
}
