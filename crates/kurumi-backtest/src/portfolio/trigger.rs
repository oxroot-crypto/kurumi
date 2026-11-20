//! Rebalance triggers: calendar, drift, and their combinations.
//!
//! A [`RebalanceTrigger`] is a pure predicate over a [`TriggerContext`]. It
//! never mutates the portfolio or the calendar; the caller advances the context
//! and records the last rebalance after acting on a `true` result. This keeps
//! the trigger logic deterministic and easy to test at period boundaries.
//!
//! Calendar triggers fire on the **first observation** of a new period, and
//! also on the initial construction observation when
//! [`TriggerContext::last_rebalance_timestamp`] is `None`. Drift triggers compare
//! current and target weights in `f64`, the representation the portfolio
//! optimizers produce; all monetary state remains `Decimal` in the account.
//!
//! # Examples
//!
//! ```
//! use kurumi_backtest::portfolio::{CalendarTrigger, RebalanceTrigger, TriggerContext};
//! use kurumi_backtest::portfolio::chrono::{DateTime, Utc};
//! use std::collections::BTreeMap;
//!
//! let context = TriggerContext {
//!     current_timestamp: DateTime::<Utc>::from_timestamp_millis(0).expect("epoch"),
//!     last_rebalance_timestamp: None,
//!     bars_advanced: 0,
//!     last_rebalance_bar: None,
//!     current_weights: BTreeMap::new(),
//!     target_weights: BTreeMap::new(),
//! };
//! assert!(CalendarTrigger::Monthly.should_rebalance(&context));
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use chrono::{DateTime, Datelike, Utc};
use kurumi_core::KurumiError;

/// The immutable state a [`RebalanceTrigger`] inspects.
///
/// Weights are fractions of portfolio equity in `f64`. A symbol missing from a
/// weight map is treated as a zero weight, so the trigger compares the union of
/// the two universes. Non-finite weights make drift triggers return `false`
/// rather than produce an arbitrary decision.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::{RebalanceTrigger, TriggerContext};
/// use kurumi_backtest::portfolio::chrono::{DateTime, Utc};
/// use std::collections::BTreeMap;
///
/// let mut current = BTreeMap::new();
/// current.insert("AAA".to_owned(), 0.75);
/// current.insert("BBB".to_owned(), 0.25);
/// let trigger = kurumi_backtest::portfolio::DriftTrigger::max_absolute(0.2).expect("valid threshold");
/// let context = TriggerContext {
///     current_timestamp: DateTime::<Utc>::from_timestamp_millis(0).expect("epoch"),
///     last_rebalance_timestamp: None,
///     bars_advanced: 0,
///     last_rebalance_bar: None,
///     current_weights: current,
///     target_weights: BTreeMap::new(),
/// };
/// assert!(trigger.should_rebalance(&context));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct TriggerContext {
    /// Timestamp of the observation currently being processed.
    pub current_timestamp: DateTime<Utc>,
    /// Timestamp of the last rebalance, or `None` before the initial build.
    pub last_rebalance_timestamp: Option<DateTime<Utc>>,
    /// Number of price bars advanced since the backtest started.
    pub bars_advanced: usize,
    /// Value of [`TriggerContext::bars_advanced`] at the last rebalance.
    pub last_rebalance_bar: Option<usize>,
    /// Current portfolio weights, keyed by symbol.
    pub current_weights: BTreeMap<String, f64>,
    /// Target portfolio weights, keyed by symbol.
    pub target_weights: BTreeMap<String, f64>,
}

/// A predicate deciding whether the portfolio should rebalance now.
///
/// Implementations must be pure and deterministic for a given context. The
/// trait requires [`fmt::Debug`] so composite triggers remain inspectable.
pub trait RebalanceTrigger: fmt::Debug {
    /// Returns `true` when the portfolio should rebalance at this observation.
    fn should_rebalance(&self, ctx: &TriggerContext) -> bool;
}

/// A calendar- or interval-based rebalance schedule.
///
/// All period comparisons are on UTC timestamps. `Monthly`, `Quarterly`, and
/// `Weekly` fire on the first observation whose calendar period differs from
/// the last rebalance period, and therefore at most once per period when the
/// caller updates [`TriggerContext::last_rebalance_timestamp`] after firing.
/// Weekly periods follow ISO 8601 (Monday to Sunday).
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::{CalendarTrigger, RebalanceTrigger, TriggerContext};
/// use kurumi_backtest::portfolio::chrono::{DateTime, Utc};
/// use std::collections::BTreeMap;
///
/// let context = |timestamp: DateTime<Utc>| TriggerContext {
///     current_timestamp: timestamp,
///     last_rebalance_timestamp: None,
///     bars_advanced: 0,
///     last_rebalance_bar: None,
///     current_weights: BTreeMap::new(),
///     target_weights: BTreeMap::new(),
/// };
/// let january = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
/// assert!(CalendarTrigger::Monthly.should_rebalance(&context(january)));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarTrigger {
    /// Rebalance on the first observation of each calendar month.
    Monthly,
    /// Rebalance on the first observation of each calendar quarter.
    Quarterly,
    /// Rebalance on the first observation of each ISO 8601 week.
    Weekly,
    /// Rebalance every `bars` observations.
    FixedInterval {
        /// Number of observations between rebalances; must be at least one.
        ///
        /// Use [`CalendarTrigger::fixed_interval`] to validate the value. A
        /// directly constructed variant with `bars == 0` never fires.
        bars: usize,
    },
}

impl CalendarTrigger {
    /// Creates a [`CalendarTrigger::FixedInterval`] with a validated interval.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `bars` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::portfolio::CalendarTrigger;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// assert!(CalendarTrigger::fixed_interval(5).is_ok());
    /// assert!(CalendarTrigger::fixed_interval(0).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn fixed_interval(bars: usize) -> Result<Self, KurumiError> {
        if bars == 0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "bars",
                reason: "must be at least one".to_owned(),
            });
        }
        Ok(Self::FixedInterval { bars })
    }
}

impl RebalanceTrigger for CalendarTrigger {
    fn should_rebalance(&self, ctx: &TriggerContext) -> bool {
        match self {
            Self::Monthly => crossed_period(ctx, |timestamp| (timestamp.year(), timestamp.month())),
            Self::Quarterly => crossed_period(ctx, |timestamp| {
                (timestamp.year(), (timestamp.month() - 1) / 3)
            }),
            Self::Weekly => crossed_period(ctx, |timestamp| {
                let week = timestamp.iso_week();
                (week.year(), week.week())
            }),
            Self::FixedInterval { bars } => {
                if *bars == 0 {
                    return false;
                }
                match ctx.last_rebalance_bar {
                    None => true,
                    Some(last) => ctx.bars_advanced.saturating_sub(last) >= *bars,
                }
            }
        }
    }
}

fn crossed_period<F>(ctx: &TriggerContext, period: F) -> bool
where
    F: Fn(DateTime<Utc>) -> (i32, u32),
{
    match ctx.last_rebalance_timestamp {
        None => true,
        Some(last) => period(last) != period(ctx.current_timestamp),
    }
}

/// A weight-drift rebalance policy.
///
/// A symbol missing from either weight map is treated as weight zero. All
/// comparisons use `>=`, so a deviation exactly equal to the threshold fires.
/// Thresholds must be finite and non-negative; use the constructors to validate
/// them. A directly constructed variant with an invalid threshold never fires.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::{DriftTrigger, RebalanceTrigger, TriggerContext};
/// use kurumi_backtest::portfolio::chrono::{DateTime, Utc};
/// use std::collections::BTreeMap;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let mut current = BTreeMap::new();
/// current.insert("AAA".to_owned(), 0.75);
/// current.insert("BBB".to_owned(), 0.25);
/// let mut target = BTreeMap::new();
/// target.insert("AAA".to_owned(), 0.50);
/// target.insert("BBB".to_owned(), 0.50);
/// let context = TriggerContext {
///     current_timestamp: DateTime::<Utc>::from_timestamp_millis(0).expect("epoch"),
///     last_rebalance_timestamp: None,
///     bars_advanced: 0,
///     last_rebalance_bar: None,
///     current_weights: current,
///     target_weights: target,
/// };
/// assert!(DriftTrigger::max_absolute(0.25)?.should_rebalance(&context));
/// assert!(!DriftTrigger::max_absolute(0.30)?.should_rebalance(&context));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DriftTrigger {
    /// Fires when any symbol's absolute weight deviation reaches the threshold.
    MaxAbsoluteDrift {
        /// Required absolute deviation `|w_current - w_target|`.
        threshold: f64,
    },
    /// Fires when one-way turnover `0.5 * sum(|w_current - w_target|)` reaches
    /// the threshold.
    L1NormDrift {
        /// Required one-way turnover.
        threshold: f64,
    },
    /// Fires when a non-zero target weight's relative deviation reaches the
    /// threshold. Symbols with a zero target weight are ignored, because their
    /// relative deviation is undefined.
    RelativeDrift {
        /// Required ratio `|w_current - w_target| / |w_target|`.
        threshold: f64,
    },
}

impl DriftTrigger {
    /// Creates a [`DriftTrigger::MaxAbsoluteDrift`] with a validated threshold.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `threshold` is `NaN`,
    /// infinite, or negative.
    pub fn max_absolute(threshold: f64) -> Result<Self, KurumiError> {
        validate_threshold(threshold)?;
        Ok(Self::MaxAbsoluteDrift { threshold })
    }

    /// Creates a [`DriftTrigger::L1NormDrift`] with a validated threshold.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `threshold` is `NaN`,
    /// infinite, or negative.
    pub fn l1_norm(threshold: f64) -> Result<Self, KurumiError> {
        validate_threshold(threshold)?;
        Ok(Self::L1NormDrift { threshold })
    }

    /// Creates a [`DriftTrigger::RelativeDrift`] with a validated threshold.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `threshold` is `NaN`,
    /// infinite, or negative.
    pub fn relative(threshold: f64) -> Result<Self, KurumiError> {
        validate_threshold(threshold)?;
        Ok(Self::RelativeDrift { threshold })
    }

    fn threshold(self) -> f64 {
        match self {
            Self::MaxAbsoluteDrift { threshold }
            | Self::L1NormDrift { threshold }
            | Self::RelativeDrift { threshold } => threshold,
        }
    }
}

impl RebalanceTrigger for DriftTrigger {
    fn should_rebalance(&self, ctx: &TriggerContext) -> bool {
        let threshold = self.threshold();
        if !threshold.is_finite() || threshold < 0.0 {
            return false;
        }
        match self {
            Self::MaxAbsoluteDrift { .. } => {
                for symbol in union_symbols(ctx) {
                    let current = weight(&ctx.current_weights, symbol);
                    let target = weight(&ctx.target_weights, symbol);
                    if !current.is_finite() || !target.is_finite() {
                        return false;
                    }
                    if (current - target).abs() >= threshold {
                        return true;
                    }
                }
                false
            }
            Self::L1NormDrift { .. } => {
                let mut deviation = 0.0;
                for symbol in union_symbols(ctx) {
                    let current = weight(&ctx.current_weights, symbol);
                    let target = weight(&ctx.target_weights, symbol);
                    if !current.is_finite() || !target.is_finite() {
                        return false;
                    }
                    deviation += (current - target).abs();
                }
                if !deviation.is_finite() {
                    return false;
                }
                0.5 * deviation >= threshold
            }
            Self::RelativeDrift { .. } => {
                for (symbol, target) in &ctx.target_weights {
                    if *target == 0.0 {
                        continue;
                    }
                    let current = weight(&ctx.current_weights, symbol);
                    if !current.is_finite() || !target.is_finite() {
                        return false;
                    }
                    if (current - target).abs() / target.abs() >= threshold {
                        return true;
                    }
                }
                false
            }
        }
    }
}

fn validate_threshold(threshold: f64) -> Result<(), KurumiError> {
    if !threshold.is_finite() || threshold < 0.0 {
        return Err(KurumiError::InvalidParameter {
            parameter: "threshold",
            reason: "must be finite and non-negative".to_owned(),
        });
    }
    Ok(())
}

fn union_symbols(ctx: &TriggerContext) -> BTreeSet<&str> {
    ctx.current_weights
        .keys()
        .map(String::as_str)
        .chain(ctx.target_weights.keys().map(String::as_str))
        .collect()
}

fn weight(weights: &BTreeMap<String, f64>, symbol: &str) -> f64 {
    weights.get(symbol).copied().unwrap_or(0.0)
}

/// A boolean combination of rebalance triggers.
///
/// `Any` fires when at least one child fires; `All` fires when every child
/// fires. An empty `Any` is `false` and an empty `All` is `true`, following the
/// usual identity of disjunction and conjunction.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::portfolio::{
///     CalendarTrigger, CompositeTrigger, DriftTrigger, RebalanceTrigger, TriggerContext,
/// };
/// use kurumi_backtest::portfolio::chrono::{DateTime, Utc};
/// use std::collections::BTreeMap;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let context = TriggerContext {
///     current_timestamp: DateTime::<Utc>::from_timestamp_millis(0).expect("epoch"),
///     last_rebalance_timestamp: None,
///     bars_advanced: 0,
///     last_rebalance_bar: None,
///     current_weights: BTreeMap::new(),
///     target_weights: BTreeMap::new(),
/// };
/// let trigger = CompositeTrigger::Any(vec![
///     Box::new(CalendarTrigger::Monthly),
///     Box::new(DriftTrigger::max_absolute(0.05)?),
/// ]);
/// assert!(trigger.should_rebalance(&context));
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub enum CompositeTrigger {
    /// Fires when at least one child trigger fires.
    Any(Vec<Box<dyn RebalanceTrigger>>),
    /// Fires when every child trigger fires.
    All(Vec<Box<dyn RebalanceTrigger>>),
}

impl RebalanceTrigger for CompositeTrigger {
    fn should_rebalance(&self, ctx: &TriggerContext) -> bool {
        match self {
            Self::Any(triggers) => triggers.iter().any(|trigger| trigger.should_rebalance(ctx)),
            Self::All(triggers) => triggers.iter().all(|trigger| trigger.should_rebalance(ctx)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurumi_core::Date;

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        let date = Date::from_ymd(year, month, day).expect("valid test date");
        DateTime::<Utc>::from_timestamp_millis(i64::from(date.days_since_epoch()) * 86_400_000)
            .expect("valid test timestamp")
    }

    fn context(
        current_timestamp: DateTime<Utc>,
        last_rebalance_timestamp: Option<DateTime<Utc>>,
        bars_advanced: usize,
        last_rebalance_bar: Option<usize>,
        current: &[(&str, f64)],
        target: &[(&str, f64)],
    ) -> TriggerContext {
        TriggerContext {
            current_timestamp,
            last_rebalance_timestamp,
            bars_advanced,
            last_rebalance_bar,
            current_weights: current
                .iter()
                .map(|(symbol, weight)| ((*symbol).to_owned(), *weight))
                .collect(),
            target_weights: target
                .iter()
                .map(|(symbol, weight)| ((*symbol).to_owned(), *weight))
                .collect(),
        }
    }

    #[test]
    fn monthly_calendar_fires_initially_and_across_year_boundaries() {
        let trigger = CalendarTrigger::Monthly;
        assert!(trigger.should_rebalance(&context(at(2024, 1, 2), None, 0, None, &[], &[])));
        assert!(trigger.should_rebalance(&context(
            at(2024, 1, 2),
            Some(at(2023, 12, 29)),
            1,
            Some(0),
            &[],
            &[]
        )));
        assert!(trigger.should_rebalance(&context(
            at(2026, 1, 1),
            Some(at(2025, 12, 31)),
            250,
            Some(249),
            &[],
            &[]
        )));
    }

    #[test]
    fn monthly_calendar_does_not_fire_twice_in_the_same_month() {
        let trigger = CalendarTrigger::Monthly;
        assert!(!trigger.should_rebalance(&context(
            at(2024, 1, 31),
            Some(at(2024, 1, 2)),
            20,
            Some(1),
            &[],
            &[]
        )));
    }

    #[test]
    fn monthly_calendar_handles_leap_february() {
        let trigger = CalendarTrigger::Monthly;
        assert!(trigger.should_rebalance(&context(
            at(2024, 2, 29),
            Some(at(2024, 1, 31)),
            40,
            Some(20),
            &[],
            &[]
        )));
        assert!(!trigger.should_rebalance(&context(
            at(2024, 2, 29),
            Some(at(2024, 2, 1)),
            40,
            Some(20),
            &[],
            &[]
        )));
        assert!(trigger.should_rebalance(&context(
            at(2024, 3, 1),
            Some(at(2024, 2, 29)),
            41,
            Some(40),
            &[],
            &[]
        )));
    }

    #[test]
    fn quarterly_calendar_crosses_quarter_boundaries_only() {
        let trigger = CalendarTrigger::Quarterly;
        assert!(trigger.should_rebalance(&context(
            at(2024, 4, 1),
            Some(at(2024, 3, 29)),
            60,
            Some(59),
            &[],
            &[]
        )));
        assert!(!trigger.should_rebalance(&context(
            at(2024, 3, 29),
            Some(at(2024, 1, 2)),
            60,
            Some(1),
            &[],
            &[]
        )));
        assert!(trigger.should_rebalance(&context(
            at(2025, 1, 2),
            Some(at(2024, 12, 30)),
            250,
            Some(249),
            &[],
            &[]
        )));
    }

    #[test]
    fn weekly_calendar_uses_iso_weeks() {
        let trigger = CalendarTrigger::Weekly;
        assert!(trigger.should_rebalance(&context(
            at(2024, 3, 11),
            Some(at(2024, 3, 8)),
            10,
            Some(9),
            &[],
            &[]
        )));
        assert!(!trigger.should_rebalance(&context(
            at(2024, 3, 15),
            Some(at(2024, 3, 11)),
            14,
            Some(10),
            &[],
            &[]
        )));
        assert!(!trigger.should_rebalance(&context(
            at(2024, 3, 10),
            Some(at(2024, 3, 8)),
            9,
            Some(8),
            &[],
            &[]
        )));
        assert!(trigger.should_rebalance(&context(
            at(2024, 1, 1),
            Some(at(2023, 12, 29)),
            250,
            Some(249),
            &[],
            &[]
        )));
    }

    #[test]
    fn fixed_interval_counts_advanced_bars() {
        let trigger = CalendarTrigger::fixed_interval(3).expect("valid interval");
        assert!(trigger.should_rebalance(&context(at(2024, 1, 1), None, 0, None, &[], &[])));
        assert!(trigger.should_rebalance(&context(
            at(2024, 1, 5),
            Some(at(2024, 1, 2)),
            5,
            Some(2),
            &[],
            &[]
        )));
        assert!(!trigger.should_rebalance(&context(
            at(2024, 1, 4),
            Some(at(2024, 1, 2)),
            4,
            Some(2),
            &[],
            &[]
        )));
        assert!(
            !CalendarTrigger::FixedInterval { bars: 0 }.should_rebalance(&context(
                at(2024, 1, 1),
                None,
                0,
                None,
                &[],
                &[]
            ))
        );
        assert!(CalendarTrigger::fixed_interval(0).is_err());
    }

    #[test]
    fn drift_triggers_fire_at_the_threshold_boundary() {
        let current = [("AAA", 0.75), ("BBB", 0.25)];
        let target = [("AAA", 0.50), ("BBB", 0.50)];
        let ctx = context(at(2024, 1, 2), None, 0, None, &current, &target);

        assert!(
            DriftTrigger::max_absolute(0.25)
                .expect("valid threshold")
                .should_rebalance(&ctx)
        );
        assert!(
            !DriftTrigger::max_absolute(0.250_000_1)
                .expect("valid threshold")
                .should_rebalance(&ctx)
        );
        assert!(
            DriftTrigger::l1_norm(0.25)
                .expect("valid threshold")
                .should_rebalance(&ctx)
        );
        assert!(
            !DriftTrigger::l1_norm(0.250_000_1)
                .expect("valid threshold")
                .should_rebalance(&ctx)
        );
        assert!(
            DriftTrigger::relative(0.50)
                .expect("valid threshold")
                .should_rebalance(&ctx)
        );
        assert!(
            !DriftTrigger::relative(0.500_000_1)
                .expect("valid threshold")
                .should_rebalance(&ctx)
        );
    }

    #[test]
    fn drift_triggers_compare_the_union_of_both_universes() {
        let ctx = context(
            at(2024, 1, 2),
            None,
            0,
            None,
            &[("AAA", 0.4)],
            &[("BBB", 0.9)],
        );
        assert!(
            DriftTrigger::max_absolute(0.9)
                .expect("valid threshold")
                .should_rebalance(&ctx)
        );
        assert!(
            DriftTrigger::l1_norm(0.65)
                .expect("valid threshold")
                .should_rebalance(&ctx)
        );
        assert!(
            DriftTrigger::relative(1.0)
                .expect("valid threshold")
                .should_rebalance(&ctx)
        );
        assert!(
            !DriftTrigger::relative(1.000_001)
                .expect("valid threshold")
                .should_rebalance(&ctx)
        );
    }

    #[test]
    fn relative_drift_ignores_zero_target_weights() {
        let ctx = context(
            at(2024, 1, 2),
            None,
            0,
            None,
            &[("AAA", 1.0)],
            &[("AAA", 0.0)],
        );
        assert!(
            !DriftTrigger::relative(0.01)
                .expect("valid threshold")
                .should_rebalance(&ctx)
        );
        assert!(
            DriftTrigger::max_absolute(1.0)
                .expect("valid threshold")
                .should_rebalance(&ctx)
        );
    }

    #[test]
    fn invalid_thresholds_are_rejected_and_never_fire() {
        assert!(DriftTrigger::max_absolute(f64::NAN).is_err());
        assert!(DriftTrigger::l1_norm(f64::INFINITY).is_err());
        assert!(DriftTrigger::relative(-0.1).is_err());

        let ctx = context(at(2024, 1, 2), None, 0, None, &[("AAA", 1.0)], &[]);
        assert!(
            !DriftTrigger::MaxAbsoluteDrift {
                threshold: f64::NAN
            }
            .should_rebalance(&ctx)
        );
    }

    #[test]
    fn composite_any_and_all_combine_policy_results() {
        let same_month = context(
            at(2024, 1, 31),
            Some(at(2024, 1, 2)),
            20,
            Some(1),
            &[("AAA", 0.75), ("BBB", 0.25)],
            &[("AAA", 0.50), ("BBB", 0.50)],
        );
        let drifted_any = CompositeTrigger::Any(vec![
            Box::new(CalendarTrigger::Monthly),
            Box::new(DriftTrigger::max_absolute(0.20).expect("valid threshold")),
        ]);
        assert!(drifted_any.should_rebalance(&same_month));

        let all = CompositeTrigger::All(vec![
            Box::new(CalendarTrigger::Monthly),
            Box::new(DriftTrigger::max_absolute(0.40).expect("valid threshold")),
        ]);
        assert!(!all.should_rebalance(&same_month));

        let calendar_only = CompositeTrigger::Any(vec![Box::new(CalendarTrigger::Monthly)]);
        let new_month = context(
            at(2024, 2, 1),
            Some(at(2024, 1, 31)),
            21,
            Some(20),
            &[],
            &[],
        );
        assert!(calendar_only.should_rebalance(&new_month));

        let empty_any = CompositeTrigger::Any(Vec::new());
        let empty_all = CompositeTrigger::All(Vec::new());
        assert!(!empty_any.should_rebalance(&new_month));
        assert!(empty_all.should_rebalance(&new_month));
    }
}
