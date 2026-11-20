//! Acceptance test: four-asset monthly HRP rebalance through the execution
//! backtest runner.

use std::collections::BTreeMap;
use std::f64::consts::PI;

use kurumi_backtest::portfolio::{
    AlignedMarketData, CalendarTrigger, InsufficientDataPolicy, MultiAssetFeed,
    RollingOptimizerAllocator,
};
use kurumi_backtest::{FixedRateFee, NoSlippage};
use kurumi_exec::chrono::DateTime;
use kurumi_exec::{
    BacktestConfig, BacktestRunner, ExecBacktestReport, ExecError, MemoryJournal,
    PaperMatchingEngine, PortfolioRebalanceStrategy, RebalanceStrategyConfig, RiskGate,
};
use kurumi_math::SampleCovariance;
use kurumi_portfolio::HierarchicalRiskParity;
use kurumi_ta::OhlcvBar;
use rust_decimal::Decimal;

const DAY_MS: i64 = 86_400_000;

fn build_feed() -> Result<MultiAssetFeed, ExecError> {
    let symbols = ["AAA", "BBB", "CCC", "DDD"];
    let bases = [100.0_f64, 50.0, 200.0, 25.0];
    let mut series = BTreeMap::new();
    for (index, symbol) in symbols.iter().enumerate() {
        let base = bases.get(index).copied().unwrap_or(100.0);
        let mut bars = Vec::new();
        for day in 0..120_i64 {
            let phase = day as f64 * 0.21 + index as f64 * 0.7;
            let close = base * (1.0 + 0.12 * (phase).sin() + 0.0005 * day as f64);
            let open = close * 0.999;
            let high = close * 1.01;
            let low = close * 0.99;
            let bar = OhlcvBar::new(
                day * DAY_MS,
                open,
                high,
                low,
                close,
                1_000.0,
                close * 1_000.0,
            )?;
            bars.push(bar);
        }
        series.insert((*symbol).to_owned(), bars);
    }
    Ok(MultiAssetFeed::new(series)?)
}

#[test]
fn four_asset_monthly_hrp_rebalance_runs_through_backtest_runner() {
    let feed = build_feed().expect("feed");
    let data = AlignedMarketData::from_feed(feed);
    let symbols = vec![
        "AAA".to_owned(),
        "BBB".to_owned(),
        "CCC".to_owned(),
        "DDD".to_owned(),
    ];
    let allocator = RollingOptimizerAllocator::new(SampleCovariance, HierarchicalRiskParity::new())
        .with_min_lookback_periods(10)
        .expect("lookback")
        .with_insufficient_data_policy(InsufficientDataPolicy::EqualWeight);
    let config = RebalanceStrategyConfig {
        lookback_window: 60,
        cash_buffer_ratio: Decimal::new(1, 2),
        ..RebalanceStrategyConfig::default()
    };
    let mut strategy = PortfolioRebalanceStrategy::with_config(
        symbols,
        Box::new(CalendarTrigger::Monthly),
        Box::new(allocator),
        config,
    )
    .expect("strategy");
    let mut runner = BacktestRunner::new(
        BacktestConfig {
            initial_cash: Decimal::from(1_000_000),
            fee_buffer_bps: Decimal::from(50),
            periods_per_year: 252.0,
            risk_free_rate: 0.02,
        },
        RiskGate::empty(),
        PaperMatchingEngine::new(
            Box::new(FixedRateFee::new(Decimal::new(1, 3)).expect("fee")),
            Box::new(NoSlippage),
        ),
        Box::new(MemoryJournal::new()),
    )
    .expect("runner");

    let report = runner.run(data.slice(), &mut strategy).expect("run");

    assert_eq!(report.equity_curve.len(), 120);
    assert_eq!(report.initial_cash, Decimal::from(1_000_000));
    assert!(report.final_equity > Decimal::ZERO);
    assert!(report.metrics.is_some());
    assert!(
        report.risk_rejections.is_empty(),
        "unexpected rejections: {:?}",
        report.risk_rejections
    );
    assert!(report.accepted_orders >= 8, "fills: {:?}", report.fills);
    assert!(report.fills.len() >= 8, "fills: {:?}", report.fills);
    assert!(report.total_fees > Decimal::ZERO);
    assert!(matches!(
        strategy.last_cause(),
        Some(kurumi_backtest::portfolio::RebalanceCause::Calendar)
            | Some(kurumi_backtest::portfolio::RebalanceCause::Initial)
    ));
    assert!(
        strategy
            .last_target_weights()
            .values()
            .all(|weight| weight.is_finite() && *weight >= 0.0)
    );
}

#[test]
fn timestamps_are_the_only_clock() {
    // The first observation seeds the run; the same price path must produce an
    // equity curve whose timestamps are exactly the feed timestamps.
    let feed = build_feed().expect("feed");
    let data = AlignedMarketData::from_feed(feed);
    let mut strategy = PortfolioRebalanceStrategy::with_config(
        vec!["AAA".to_owned(), "BBB".to_owned()],
        Box::new(CalendarTrigger::Monthly),
        Box::new(
            kurumi_backtest::StaticWeightAllocator::new(BTreeMap::from([
                ("AAA".to_owned(), 0.5),
                ("BBB".to_owned(), 0.5),
            ]))
            .expect("weights"),
        ),
        RebalanceStrategyConfig::default(),
    )
    .expect("strategy");
    let mut runner = BacktestRunner::frictionless(Decimal::from(100_000)).expect("runner");
    let report = runner.run(data.slice(), &mut strategy).expect("run");
    for (index, point) in report.equity_curve.iter().enumerate() {
        let expected = DateTime::from_timestamp_millis(index as i64 * DAY_MS).expect("timestamp");
        assert_eq!(point.timestamp, expected);
    }
}

#[test]
fn periodic_returns_are_finite() {
    let feed = build_feed().expect("feed");
    let data = AlignedMarketData::from_feed(feed);
    let mut strategy = PortfolioRebalanceStrategy::with_config(
        vec!["AAA".to_owned(), "BBB".to_owned(), "CCC".to_owned()],
        Box::new(CalendarTrigger::Monthly),
        Box::new(
            kurumi_backtest::StaticWeightAllocator::new(BTreeMap::from([
                ("AAA".to_owned(), 0.4),
                ("BBB".to_owned(), 0.3),
                ("CCC".to_owned(), 0.2),
            ]))
            .expect("weights"),
        ),
        RebalanceStrategyConfig::default(),
    )
    .expect("strategy");
    let mut runner = BacktestRunner::frictionless(Decimal::from(100_000)).expect("runner");
    let report = runner.run(data.slice(), &mut strategy).expect("run");
    assert!(report.returns.iter().all(|value| value.is_finite()));
    let metrics = report.metrics.expect("metrics");
    assert!(metrics.max_drawdown >= 0.0);
    assert!(metrics.annualized_volatility >= 0.0);
    assert!(!report.equity_curve.is_empty());
    assert!(PI.is_finite());
}

fn run_static_portfolio() -> ExecBacktestReport {
    let feed = build_feed().expect("feed");
    let data = AlignedMarketData::from_feed(feed);
    let mut strategy = PortfolioRebalanceStrategy::with_config(
        vec!["AAA".to_owned(), "BBB".to_owned(), "CCC".to_owned()],
        Box::new(CalendarTrigger::Monthly),
        Box::new(
            kurumi_backtest::StaticWeightAllocator::new(BTreeMap::from([
                ("AAA".to_owned(), 0.4),
                ("BBB".to_owned(), 0.3),
                ("CCC".to_owned(), 0.2),
            ]))
            .expect("weights"),
        ),
        RebalanceStrategyConfig::default(),
    )
    .expect("strategy");
    let mut runner = BacktestRunner::new(
        BacktestConfig::new(Decimal::from(100_000)).expect("config"),
        RiskGate::empty(),
        PaperMatchingEngine::new(
            Box::new(FixedRateFee::new(Decimal::new(1, 3)).expect("fee")),
            Box::new(NoSlippage),
        ),
        Box::new(MemoryJournal::new()),
    )
    .expect("runner");
    runner.run(data.slice(), &mut strategy).expect("run")
}

#[test]
fn identical_inputs_produce_bit_identical_reports() {
    let first = run_static_portfolio();
    let second = run_static_portfolio();
    assert_eq!(first.final_equity, second.final_equity);
    assert_eq!(first.fills, second.fills);
    assert_eq!(first.equity_curve, second.equity_curve);
    assert_eq!(first.risk_rejections, second.risk_rejections);
    assert_eq!(first.total_fees, second.total_fees);
}
