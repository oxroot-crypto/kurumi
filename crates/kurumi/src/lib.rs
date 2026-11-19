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
//! | `backtest` | `kurumi::backtest` | `kurumi-backtest` |
//! | `full` | all of the above | every member crate |
//!
//! The default feature set is `full`, which enables every module. Features are
//! additive and lower-layer features are implied by the features above them, so
//! enabling `risk` also enables `core`, `math`, `ta`, and `pricing`.
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

#[cfg(feature = "backtest")]
pub use kurumi_backtest as backtest;

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

    #[cfg(feature = "backtest")]
    pub use kurumi_backtest::{
        BacktestEngine, BacktestReport, EquityPoint, FeeModel, Fill, FixedBpsSlippage,
        FixedRateFee, Liquidity, MakerTakerFee, NoFee, NoSlippage, Order, OrderKind,
        PortfolioAccount, Side, SlippageModel, equity_returns,
    };
}
