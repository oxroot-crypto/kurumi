//! Kurumi: the all-in-one facade for the workspace.
//!
//! Each workspace member is re-exported behind an additive Cargo feature:
//!
//! | Feature | Re-export | Member crate |
//! | --- | --- | --- |
//! | `core` | `kurumi::core` | `kurumi-core` |
//! | `math` | `kurumi::math` | `kurumi-math` |
//! | `ta` | `kurumi::ta` | `kurumi-ta` |
//! | `pricing` | `kurumi::pricing` | `kurumi-pricing` |
//! | `risk` | `kurumi::risk` | `kurumi-risk` |
//! | `portfolio` | `kurumi::portfolio` | `kurumi-portfolio` |
//! | `backtest` | `kurumi::backtest` | `kurumi-backtest` |
//! | `exec` | `kurumi::exec` | `kurumi-exec` |
//! | `feed` | `kurumi::feed` | `kurumi-feed` |
//! | `parallel` | `kurumi-backtest`'s Rayon switch | `kurumi-backtest` |
//! | `full` | all of the above | every member crate |
//!
//! The default feature set is `full` plus `parallel`, which enables every module
//! and the parallel parameter sweep. Features are additive and lower-layer
//! features are implied by the features above them, so enabling `risk` also
//! enables `core`, `math`, `ta`, and `pricing`, and `exec` implies `backtest`. The `feed` feature already
//! implies `ta`; a `--no-default-features` build that omits `feed` never links
//! an asynchronous runtime.
//!
//! The [`prelude`] module re-exports the types and traits used most often in
//! application code. Import it with `use kurumi::prelude::*;` after enabling the
//! corresponding features.
//!
//! Python and WebAssembly bindings are planned as additional opt-in features of
//! this crate. Keep that binding surface thin: it must delegate to the Rust API
//! rather than reimplement financial logic.
//!
//! # Examples
//!
//! ```
//! use kurumi::prelude::*;
//!
//! # fn main() -> Result<(), kurumi::core::KurumiError> {
//! let returns = [0.01, -0.02, 0.015, 0.005];
//! let metrics = PerformanceMetrics::from_returns(&returns, MetricsConfig::default())?;
//! assert_eq!(metrics.observations, 4);
//! # Ok(())
//! # }
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

#[cfg(feature = "core")]
pub use kurumi_core as core;

#[cfg(feature = "math")]
pub use kurumi_math as math;

#[cfg(feature = "ta")]
pub use kurumi_ta as ta;

#[cfg(feature = "pricing")]
pub use kurumi_pricing as pricing;

#[cfg(feature = "risk")]
pub use kurumi_risk as risk;

#[cfg(feature = "portfolio")]
pub use kurumi_portfolio as portfolio;

#[cfg(feature = "backtest")]
pub use kurumi_backtest as backtest;

#[cfg(feature = "exec")]
pub use kurumi_exec as exec;

#[cfg(feature = "feed")]
pub use kurumi_feed as feed;

/// The most commonly used types and traits from every enabled module.
///
/// The contents are additive with the feature graph: `prelude` always exists,
/// but each item is only exported when its feature is enabled. With
/// `--no-default-features` the module is empty.
///
/// # Examples
///
/// ```
/// use kurumi::prelude::*;
///
/// # fn main() -> Result<(), kurumi::core::KurumiError> {
/// let bar = OhlcvBar::new(0, 100.0, 101.0, 99.0, 100.5, 1_000.0, 100_500.0)?;
/// let mut sma = Sma::new(2)?;
/// assert_eq!(sma.next(bar.close())?, None);
/// assert_eq!(sma.next(bar.close())?, Some(100.5));
/// # Ok(())
/// # }
/// ```
pub mod prelude {
    #[cfg(feature = "core")]
    pub use kurumi_core::{
        BusinessDayConvention, Calendar, Currency, Date, DayCountConvention, KurumiError, Money,
        Price, Quantity, Rate, Volatility, Weekday, WeekendCalendar,
    };

    #[cfg(feature = "math")]
    pub use kurumi_math::{
        CubicSplineInterpolator, Interpolator, LinearInterpolator, LogLinearInterpolator,
        RootFinderConfig, brent, covariance, mean, newton_raphson, sample_std_dev, sample_variance,
        standard_normal_cdf, standard_normal_pdf,
    };

    #[cfg(feature = "ta")]
    pub use kurumi_ta::{
        Atr, BollingerBands, BollingerBandsOutput, Ema, Macd, MacdOutput, OhlcvBar, RingBuffer,
        Roc, RollingVariance, RollingWindow, Rsi, Sma, Stochastic, StochasticOutput,
        StreamingIndicator, Wma,
    };

    #[cfg(feature = "pricing")]
    pub use kurumi_pricing::{
        BlackScholesMertonEngine, Compounding, CrrBinomialEngine, ExerciseStyle, Greeks,
        Instrument, MarketData, OptionContract, OptionType, PricingEngine, ValuationResult,
        ZeroCurve, implied_volatility,
    };

    #[cfg(feature = "risk")]
    pub use kurumi_risk::{
        DrawdownStats, MetricsConfig, PerformanceMetrics, VolatilityTargetSizer, annualized_return,
        annualized_volatility, calmar_ratio, drawdown_stats, historical_expected_shortfall,
        historical_var, kelly_fraction, max_drawdown, parametric_expected_shortfall,
        parametric_var, profit_loss_ratio, sharpe_ratio, sortino_ratio, total_return,
        volatility_target_fraction, win_rate,
    };

    #[cfg(feature = "portfolio")]
    pub use kurumi_portfolio::{
        AlignmentPolicy, AssetSeries, EqualRiskContribution, EqualWeight, GlobalMinVariance,
        HierarchicalRiskParity, Holding, HrpLinkage, InverseVolatility, MaxSharpe,
        OptimizationConstraints, OptimizationResult, PortfolioOptimizer, PortfolioVar,
        PortfolioWeights, RebalanceAction, RebalanceSide, ReturnMatrix, ReturnMethod,
        RiskDecomposition, RollingBetaPanel, RollingPanel, TargetReturn, annualized_sharpe_ratio,
        diversification_ratio, efficient_frontier, generate_rebalance_actions,
        historical_portfolio_var, parametric_portfolio_var, portfolio_expected_return,
        portfolio_volatility, risk_decomposition, rolling_beta_and_correlation, rolling_betas,
        rolling_correlations,
    };

    #[cfg(feature = "feed")]
    pub use kurumi_feed::{
        BarAggregator, BarEvent, BookDelta, BookSide, ChannelKind, ConnectionState, FeedError,
        FeedStatusEvent, InstrumentId, ItchDecoder, ItchStreamProvider, LastQuote, LastSale,
        ManagedConnector, MarketDataConnector, MarketDataHub, MarketEvent, MockReplayProvider,
        NasdaqWsConfig, NasdaqWsProvider, NcdsBar, NcdsBarPrecision, NcdsPollingConnector,
        NcdsRestClient, NcdsRestConfig, NcdsSnapshot, QuoteTick, ReconnectPolicy, Subscription,
        TradeTick, Venue,
    };

    #[cfg(feature = "backtest")]
    pub use kurumi_backtest::{
        AlignedMarketData, AllocationContext, AssetPosition, BacktestEngine, BacktestReport,
        BenchmarkComparison, CalendarTrigger, CompositeTrigger, CustomAllocator, DriftTrigger,
        EquityPoint, ExecutionTiming, FeeModel, Fill, FixedBpsSlippage, FixedRateFee, FoldWindow,
        GridSearchConfig, GridSearchResult, GridSearchRunner, GridTrial, InsufficientDataPolicy,
        Liquidity, MakerTakerFee, MarketSlice, MarketSnapshot, MultiAssetAccount, MultiAssetFeed,
        MultiAssetFill, NoFee, NoSlippage, ObjectiveMetric, Order, OrderKind, PortfolioAccount,
        PortfolioBacktestConfig, PortfolioBacktestEngine, PortfolioBacktestReport, RebalanceCause,
        RebalanceEvent, RebalanceTrigger, ResultFilter, RollingOptimizerAllocator, Side,
        SlippageModel, StaticWeightAllocator, StrategySpec, TargetWeightAllocator, TrialStatus,
        TrialSummary, TriggerContext, WalkForwardAnalyzer, WalkForwardConfig,
        WalkForwardDiagnostics, WalkForwardFold, WalkForwardMode, WalkForwardReport,
        equity_returns, generate_folds,
    };

    #[cfg(feature = "exec")]
    pub use kurumi_exec::{
        BacktestConfig, BacktestRunner, ClientOrderId, ExecError, ExecMarketEvent, ExecutionLedger,
        ExecutionReport, MemoryJournal, OrderCommand, OrderRecord, OrderRequest, OrderSide,
        OrderStatus, OrderType, PaperMatchingEngine, PortfolioRebalanceStrategy, RiskGate,
        Strategy, StrategyContext, TimeInForce,
    };
}
