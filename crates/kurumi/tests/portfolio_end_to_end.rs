//! End-to-end portfolio construction scenario across the facade crate.
//!
//! The scenario builds four synthetic assets with distinct return and
//! correlation profiles (equity, long government bonds, gold, and commodities)
//! plus a market benchmark, then runs the complete pipeline:
//!
//! 1. align the return series into a `(T, N)` panel;
//! 2. estimate a Ledoit-Wolf covariance matrix and the correlation matrix;
//! 3. compute per-asset beta and Jensen's alpha against the benchmark;
//! 4. solve GMV, maximum-Sharpe, ERC, and HRP allocations;
//! 5. verify risk contributions and diversification;
//! 6. generate Decimal rebalancing trades and apply them.

use approx::assert_relative_eq;
use kurumi::math::{CovarianceEstimator, LedoitWolfShrinkage};
use kurumi::portfolio::chrono::{DateTime, Utc};
use kurumi::prelude::*;
use kurumi::risk::{beta, jensen_alpha};
use rust_decimal::Decimal;

const OBSERVATIONS: usize = 400;
const SYMBOLS: [&str; 4] = ["STOCK", "BOND", "GOLD", "COMMODITY"];
const INITIAL_STATE: u64 = 0x2545_F491_4F6C_DD1D;

fn at(day: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(day * 86_400, 0).unwrap_or_default()
}

/// Draws an approximately standard-normal variate from a deterministic LCG
/// using the Box-Muller transform.
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

/// Simulates the four assets and the market benchmark from one common factor.
fn simulated_returns() -> (Vec<Vec<f64>>, Vec<f64>) {
    let mut state = INITIAL_STATE;
    let mut assets = vec![
        Vec::with_capacity(OBSERVATIONS),
        Vec::with_capacity(OBSERVATIONS),
        Vec::with_capacity(OBSERVATIONS),
        Vec::with_capacity(OBSERVATIONS),
    ];
    let mut benchmark = Vec::with_capacity(OBSERVATIONS);
    for _ in 0..OBSERVATIONS {
        let market = 0.0003 + 0.0085 * standard_normal(&mut state);
        let stock = 0.0004 + 1.10 * market + 0.010 * standard_normal(&mut state);
        let bond = 0.0002 - 0.30 * market + 0.0035 * standard_normal(&mut state);
        let gold = 0.0002 - 0.15 * market + 0.0075 * standard_normal(&mut state);
        let commodity = 0.0003 + 0.45 * market + 0.011 * standard_normal(&mut state);
        benchmark.push(market);
        assets[0].push(stock);
        assets[1].push(bond);
        assets[2].push(gold);
        assets[3].push(commodity);
    }
    (assets, benchmark)
}

fn build_panel(assets: &[Vec<f64>]) -> Result<ReturnMatrix, kurumi::core::KurumiError> {
    let mut series = Vec::with_capacity(assets.len());
    for (index, values) in assets.iter().enumerate() {
        let Some(symbol) = SYMBOLS.get(index) else {
            return Err(kurumi::core::KurumiError::InvalidParameter {
                parameter: "assets",
                reason: "more series than symbols".to_owned(),
            });
        };
        let observations = values
            .iter()
            .enumerate()
            .map(|(day, value)| (at(day as i64), *value))
            .collect::<Vec<_>>();
        series.push(AssetSeries::new(*symbol, observations)?);
    }
    ReturnMatrix::from_return_series(&series, AlignmentPolicy::InnerJoin)
}

fn portfolio_weights(
    symbols: &[String],
    weights: &kurumi::portfolio::nalgebra::DVector<f64>,
) -> Result<PortfolioWeights, kurumi::core::KurumiError> {
    PortfolioWeights::new(symbols.to_vec(), weights.clone())
}

#[test]
fn end_to_end_portfolio_optimization_and_rebalancing() -> Result<(), kurumi::core::KurumiError> {
    // Step 1: deterministic correlated market data and alignment.
    let (assets, benchmark) = simulated_returns();
    let returns = build_panel(&assets)?;
    assert_eq!(returns.dimensions(), (OBSERVATIONS, 4));
    let symbols = returns.symbols().to_vec();
    assert_eq!(symbols, SYMBOLS);

    // Step 2: Ledoit-Wolf covariance and the correlation matrix.
    let covariance = LedoitWolfShrinkage.estimate(returns.data())?;
    let correlation = returns.correlation_matrix()?;
    assert_eq!(covariance.shape(), (4, 4));
    assert_eq!(correlation.shape(), (4, 4));
    for index in 0..4 {
        assert_relative_eq!(
            correlation.get((index, index)).copied().unwrap_or_default(),
            1.0,
            epsilon = 1e-12
        );
    }

    // Step 3: per-asset beta and Jensen's alpha against the market.
    let mut betas = Vec::with_capacity(4);
    let mut alphas = Vec::with_capacity(4);
    for symbol in &symbols {
        let column = returns.column(symbol)?;
        let asset_beta = beta(column.as_slice(), &benchmark)?;
        let asset_alpha = jensen_alpha(column.as_slice(), &benchmark, 0.0)?;
        assert!(asset_beta.is_finite());
        assert!(asset_alpha.is_finite());
        betas.push(asset_beta);
        alphas.push(asset_alpha);
    }
    assert!(betas.iter().any(|value| *value > 0.5));
    assert!(betas.iter().any(|value| *value < 0.0));

    // Step 4: the four allocation strategies.
    let expected_returns = returns.mean_returns()?;
    let equal_weight = EqualWeight::new().optimize(&expected_returns, &covariance)?;
    let gmv = GlobalMinVariance::new().optimize(&expected_returns, &covariance)?;
    let max_sharpe = MaxSharpe::new().optimize(&expected_returns, &covariance)?;
    let erc = EqualRiskContribution::new().optimize(&expected_returns, &covariance)?;
    let hrp = HierarchicalRiskParity::with_linkage(HrpLinkage::Ward)
        .optimize(&expected_returns, &covariance)?;
    for result in [&equal_weight, &gmv, &max_sharpe, &erc, &hrp] {
        assert_relative_eq!(result.weights.sum(), 1.0, epsilon = 1e-8);
        assert!(
            result
                .weights
                .iter()
                .all(|weight| weight.is_finite() && *weight >= -1e-12)
        );
        assert!(result.volatility.is_finite() && result.volatility > 0.0);
    }

    // Acceptance: GMV volatility is no larger than any single asset or equal
    // weight.
    let equal_volatility = {
        let weights = portfolio_weights(&symbols, &equal_weight.weights)?;
        portfolio_volatility(&weights, &covariance)?
    };
    assert!(gmv.volatility <= equal_volatility + 1e-12);
    for index in 0..4 {
        let single_volatility = covariance
            .get((index, index))
            .copied()
            .unwrap_or_default()
            .sqrt();
        assert!(gmv.volatility <= single_volatility + 1e-12);
    }

    // Max-Sharpe dominates the other allocations' ratios.
    for result in [&equal_weight, &gmv, &erc, &hrp] {
        assert!(max_sharpe.sharpe_ratio >= result.sharpe_ratio - 1e-8);
    }

    // Acceptance: ERC percentage risk contributions match 1/N to below 1e-5.
    let erc_weights = portfolio_weights(&symbols, &erc.weights)?;
    let decomposition = risk_decomposition(&erc_weights, &covariance)?;
    for contribution in decomposition.pctr.iter() {
        assert_relative_eq!(*contribution, 0.25, epsilon = 1e-5);
    }

    // Diversification and frontier sanity.
    let equal_weights = portfolio_weights(&symbols, &equal_weight.weights)?;
    assert!(diversification_ratio(&equal_weights, &covariance)? > 1.0);
    let frontier = efficient_frontier(
        &OptimizationConstraints::default(),
        &expected_returns,
        &covariance,
        8,
    )?;
    assert_eq!(frontier.len(), 8);
    assert_relative_eq!(
        frontier[0].expected_return,
        gmv.expected_return,
        epsilon = 1e-10
    );
    for pair in frontier.windows(2) {
        assert!(pair[1].expected_return >= pair[0].expected_return - 1e-10);
        assert!(pair[1].volatility >= pair[0].volatility - 1e-7);
    }

    // Acceptance: HRP is stable for a nearly collinear universe.
    let collinear = {
        let correlations = 0.999_f64;
        let volatilities = [0.20_f64, 0.30, 0.25];
        let mut matrix = vec![0.0; 9];
        for row in 0..3 {
            for column in 0..3 {
                let value = if row == column {
                    volatilities[row] * volatilities[row]
                } else {
                    correlations * volatilities[row] * volatilities[column]
                };
                matrix[row * 3 + column] = value;
            }
        }
        kurumi::portfolio::nalgebra::DMatrix::from_row_slice(3, 3, &matrix)
    };
    let collinear_means = kurumi::portfolio::nalgebra::DVector::from_vec(vec![0.01, 0.02, 0.015]);
    let collinear_hrp = HierarchicalRiskParity::with_linkage(HrpLinkage::Single)
        .optimize(&collinear_means, &collinear)?;
    assert_relative_eq!(collinear_hrp.weights.sum(), 1.0, epsilon = 1e-9);
    assert!(
        collinear_hrp
            .weights
            .iter()
            .all(|weight| weight.is_finite() && *weight >= 0.0)
    );

    // Step 5: generate and apply rebalancing trades with Decimal amounts.
    let total_equity = Decimal::from(1_000_000);
    let target = portfolio_weights(&symbols, &gmv.weights)?;
    // Start exactly at 25% per asset; any non-uniform target then requires
    // both buys and sells.
    let quantities = [
        Decimal::from(2000),
        Decimal::from(2500),
        Decimal::from(1000),
        Decimal::from(5000),
    ];
    let prices = [
        Decimal::from(125),
        Decimal::from(100),
        Decimal::from(250),
        Decimal::from(50),
    ];
    let mut holdings = Vec::with_capacity(4);
    for (index, symbol) in symbols.iter().enumerate() {
        holdings.push(Holding::new(
            symbol.clone(),
            quantities[index],
            prices[index],
        )?);
    }
    let actions =
        generate_rebalance_actions(total_equity, &holdings, &target, Decimal::ZERO, true)?;
    assert!(!actions.is_empty());
    for action in &actions {
        assert!(action.quantity > Decimal::ZERO);
        assert!(action.estimated_value > Decimal::ZERO);
        assert!(action.post_trade_deviation.is_finite());
        assert!(action.post_trade_deviation < 1e-9);
    }
    assert!(
        actions
            .iter()
            .any(|action| action.side == RebalanceSide::Buy)
    );
    assert!(
        actions
            .iter()
            .any(|action| action.side == RebalanceSide::Sell)
    );

    let mut post_quantities = quantities;
    for action in &actions {
        let index = symbols
            .iter()
            .position(|symbol| symbol == &action.symbol)
            .ok_or(kurumi::core::KurumiError::InvalidParameter {
                parameter: "symbol",
                reason: "rebalance action left the target universe".to_owned(),
            })?;
        post_quantities[index] = match action.side {
            RebalanceSide::Buy => post_quantities[index] + action.quantity,
            RebalanceSide::Sell => post_quantities[index] - action.quantity,
        };
    }
    for (index, target_weight) in target.weights().iter().enumerate() {
        let value = post_quantities[index] * prices[index];
        let actual = f64::try_from(value / total_equity).unwrap_or_default();
        assert_relative_eq!(actual, *target_weight, epsilon = 1e-9);
    }

    Ok(())
}
