//! The pre-trade risk gate.
//!
//! [`RiskGate`] runs an ordered list of [`RiskRule`]s against every order before
//! it reaches the ledger. It owns the daily-loss kill switch: once
//! [`MaxDailyLossRule`] fires, the gate rejects every order that increases
//! exposure while continuing to allow orders that reduce or close an existing
//! position.
//!
//! All rules are synchronous and read the event timestamp from
//! [`RiskContext::now`]; none of them reads the wall clock.
//!
//! # Examples
//!
//! ```
//! use kurumi_exec::{
//!     ClientOrderId, ExecError, MaxOrderNotionalRule, OrderRequest, OrderSide, OrderType,
//!     RiskContext, RiskGate, TimeInForce,
//! };
//! use chrono::{DateTime, Utc};
//! use rust_decimal::Decimal;
//!
//! # fn main() -> Result<(), ExecError> {
//! let mut gate = RiskGate::new(vec![Box::new(MaxOrderNotionalRule::new(
//!     Decimal::from(1_000),
//! )?)]);
//! let ts = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
//! let request = OrderRequest::new(
//!     ClientOrderId::new("o-1"),
//!     "AAA",
//!     OrderSide::Buy,
//!     OrderType::Market,
//!     Decimal::from(20),
//!     TimeInForce::Day,
//! );
//! let context = RiskContext {
//!     now: ts,
//!     equity: Decimal::from(10_000),
//!     day_start_equity: Decimal::from(10_000),
//!     position_qty: Decimal::ZERO,
//!     reference_price: Some(Decimal::from(100)),
//!     available_cash: Decimal::from(10_000),
//! };
//! assert!(gate.evaluate(&request, &context).is_err());
//! # Ok(())
//! # }
//! ```

use std::collections::{BTreeSet, VecDeque};

use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::error::ExecError;
use crate::order::{ClientOrderId, OrderRequest, OrderSide};

/// The immutable market and account snapshot seen by a risk rule.
///
/// # Examples
///
/// ```
/// use kurumi_exec::RiskContext;
/// use chrono::{DateTime, Utc};
/// use rust_decimal::Decimal;
///
/// let ts = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
/// let context = RiskContext {
///     now: ts,
///     equity: Decimal::from(10_000),
///     day_start_equity: Decimal::from(10_000),
///     position_qty: Decimal::ZERO,
///     reference_price: Some(Decimal::from(100)),
///     available_cash: Decimal::from(10_000),
/// };
/// assert_eq!(context.reference_price, Some(Decimal::from(100)));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiskContext {
    /// Event timestamp; rules must use it instead of the wall clock.
    pub now: DateTime<Utc>,
    /// Current marked-to-market equity.
    pub equity: Decimal,
    /// Equity at the start of the trading day.
    pub day_start_equity: Decimal,
    /// Signed position quantity of the order's symbol.
    pub position_qty: Decimal,
    /// Reference price used for notional and weight estimates.
    pub reference_price: Option<Decimal>,
    /// Cash not reserved by open buy orders.
    pub available_cash: Decimal,
}

impl RiskContext {
    /// Returns the price used for notional checks, falling back to the order's
    /// own limit or stop price.
    fn price_for(&self, request: &OrderRequest) -> Option<Decimal> {
        self.reference_price.or_else(|| request.reference_price())
    }

    /// Returns `true` when the order increases absolute exposure.
    ///
    /// A sell that closes at most the current long and a buy that closes at most
    /// the current short are reducing orders; everything else increases
    /// exposure.
    #[must_use]
    pub fn increases_exposure(&self, request: &OrderRequest) -> bool {
        match request.side {
            OrderSide::Buy => {
                !(self.position_qty < Decimal::ZERO && request.quantity <= self.position_qty.abs())
            }
            OrderSide::Sell => {
                !(self.position_qty > Decimal::ZERO && request.quantity <= self.position_qty)
            }
        }
    }
}

/// A pre-trade risk rule.
///
/// Implementations must be `Send` so a [`RiskGate`] can be owned by the live
/// runner. `check` is called for every order; `on_accepted` is called only after
/// every rule passed, which is the place to record rate-limit timestamps and seen
/// identifiers. `kill_switch_triggered` is polled by [`RiskGate`] before the
/// rules run.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{ExecError, RiskContext, RiskRule, OrderRequest};
///
/// struct AlwaysAllow;
///
/// impl RiskRule for AlwaysAllow {
///     fn name(&self) -> &str {
///         "AlwaysAllow"
///     }
///
///     fn check(
///         &mut self,
///         _request: &OrderRequest,
///         _ctx: &RiskContext,
///     ) -> Result<(), ExecError> {
///         Ok(())
///     }
/// }
///
/// assert_eq!(AlwaysAllow.name(), "AlwaysAllow");
/// ```
pub trait RiskRule: Send {
    /// Returns the stable rule name recorded in journals and rejections.
    fn name(&self) -> &str;

    /// Checks one order.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::RiskRejected`] (or
    /// [`ExecError::DuplicateClientOrderId`]) when the order violates the rule.
    fn check(&mut self, request: &OrderRequest, ctx: &RiskContext) -> Result<(), ExecError>;

    /// Records state for an order that passed every rule.
    fn on_accepted(&mut self, _request: &OrderRequest, _ctx: &RiskContext) {}

    /// Returns `true` when the rule's kill-switch condition is met.
    ///
    /// The default implementation never fires.
    fn kill_switch_triggered(&self, _ctx: &RiskContext) -> bool {
        false
    }
}

/// The ordered pre-trade risk pipeline and daily-loss kill switch.
///
/// # Examples
///
/// ```
/// use kurumi_exec::RiskGate;
///
/// let gate = RiskGate::empty();
/// assert!(!gate.kill_switch_active());
/// ```
pub struct RiskGate {
    rules: Vec<Box<dyn RiskRule>>,
    kill_switch_active: bool,
}

impl RiskGate {
    /// Creates a gate from an ordered rule list.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::RiskGate;
    ///
    /// let gate = RiskGate::new(Vec::new());
    /// assert!(!gate.kill_switch_active());
    /// ```
    #[must_use]
    pub fn new(rules: Vec<Box<dyn RiskRule>>) -> Self {
        Self {
            rules,
            kill_switch_active: false,
        }
    }

    /// Creates a gate with no rules.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    /// Appends a rule and returns the gate.
    #[must_use]
    pub fn with_rule(mut self, rule: Box<dyn RiskRule>) -> Self {
        self.rules.push(rule);
        self
    }

    /// Returns `true` when the daily-loss kill switch is latched.
    #[must_use]
    #[inline]
    pub fn kill_switch_active(&self) -> bool {
        self.kill_switch_active
    }

    /// Clears the kill switch after an operator intervention.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::RiskGate;
    ///
    /// let mut gate = RiskGate::empty();
    /// gate.reset_kill_switch();
    /// assert!(!gate.kill_switch_active());
    /// ```
    pub fn reset_kill_switch(&mut self) {
        self.kill_switch_active = false;
    }

    /// Latches the kill switch when any rule reports its condition.
    ///
    /// The runner calls this before each order, but exposing it lets a live
    /// adapter latch the switch between orders.
    pub fn update_kill_switch(&mut self, ctx: &RiskContext) {
        if self
            .rules
            .iter()
            .any(|rule| rule.kill_switch_triggered(ctx))
        {
            self.kill_switch_active = true;
        }
    }

    /// Checks every rule for one order without recording it.
    ///
    /// When the kill switch is active, reducing orders bypass the rule list and
    /// increasing orders are rejected with the `MaxDailyLoss` rule.
    ///
    /// Call [`RiskGate::accept`] after the ledger has also accepted the order so
    /// stateful rules record only orders that were actually submitted.
    ///
    /// # Errors
    ///
    /// Returns the first rule rejection, or
    /// [`ExecError::RiskRejected`] for an exposure-increasing order while the
    /// kill switch is active.
    pub fn check(&mut self, request: &OrderRequest, ctx: &RiskContext) -> Result<(), ExecError> {
        request.validate()?;
        self.update_kill_switch(ctx);
        if self.kill_switch_active {
            if ctx.increases_exposure(request) {
                return Err(ExecError::risk_rejected(
                    "MaxDailyLoss",
                    "kill switch is active: only reducing orders are accepted",
                ));
            }
            return Ok(());
        }
        for rule in &mut self.rules {
            rule.check(request, ctx)?;
        }
        Ok(())
    }

    /// Records an order that passed both [`RiskGate::check`] and the ledger's
    /// reservation check.
    ///
    /// This is the hook that advances sliding-window rate limits and remembers
    /// accepted client order identifiers.
    pub fn accept(&mut self, request: &OrderRequest, ctx: &RiskContext) {
        for rule in &mut self.rules {
            rule.on_accepted(request, ctx);
        }
    }

    /// Checks and records one order in a single call.
    ///
    /// This convenience method is appropriate when the caller has no further
    /// acceptance step. The backtest runner uses [`RiskGate::check`] followed by
    /// [`RiskGate::accept`] after the ledger reservation succeeds.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`RiskGate::check`].
    pub fn evaluate(&mut self, request: &OrderRequest, ctx: &RiskContext) -> Result<(), ExecError> {
        self.check(request, ctx)?;
        self.accept(request, ctx);
        Ok(())
    }

    /// Returns the rule names in evaluation order.
    pub fn rule_names(&self) -> impl Iterator<Item = &str> {
        self.rules.iter().map(|rule| rule.name())
    }
}

/// Rejects an order whose notional exceeds a single-order limit.
///
/// This is the fat-finger guard. The notional is `reference_price * quantity`
/// using the risk context's mark or the order's own limit or stop price.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{MaxOrderNotionalRule, RiskRule};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let rule = MaxOrderNotionalRule::new(Decimal::from(10_000))?;
/// assert_eq!(rule.name(), "MaxOrderNotional");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaxOrderNotionalRule {
    max_notional: Decimal,
}

impl MaxOrderNotionalRule {
    /// Creates the rule.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::RiskRejected`] when `max_notional` is not strictly
    /// positive.
    pub fn new(max_notional: Decimal) -> Result<Self, ExecError> {
        if max_notional <= Decimal::ZERO {
            return Err(ExecError::strategy(
                "MaxOrderNotional requires a positive limit",
            ));
        }
        Ok(Self { max_notional })
    }

    /// Returns the configured maximum notional.
    #[must_use]
    #[inline]
    pub fn max_notional(&self) -> Decimal {
        self.max_notional
    }
}

impl RiskRule for MaxOrderNotionalRule {
    fn name(&self) -> &str {
        "MaxOrderNotional"
    }

    fn check(&mut self, request: &OrderRequest, ctx: &RiskContext) -> Result<(), ExecError> {
        let price = ctx.price_for(request).ok_or_else(|| {
            ExecError::risk_rejected(self.name(), "no reference price is available")
        })?;
        let notional = price.checked_mul(request.quantity).ok_or(
            kurumi_core::KurumiError::NumericalOverflow {
                operation: "MaxOrderNotional",
            },
        )?;
        if notional > self.max_notional {
            return Err(ExecError::risk_rejected(
                self.name(),
                format!(
                    "order notional {notional} exceeds the limit {}",
                    self.max_notional
                ),
            ));
        }
        Ok(())
    }
}

/// Rejects a buy that pushes one symbol's weight above a cap.
///
/// Only orders that increase the long position are checked, so sells that
/// reduce or close a position always pass.
///
/// # Examples
///
/// ```
/// use kurumi_exec::MaxPositionWeightRule;
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let rule = MaxPositionWeightRule::new(0.25)?;
/// assert_eq!(rule.max_weight(), 0.25);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaxPositionWeightRule {
    max_weight: f64,
}

impl MaxPositionWeightRule {
    /// Creates the rule.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::StrategyError`] unless `max_weight` is finite and in
    /// `(0, 1]`.
    pub fn new(max_weight: f64) -> Result<Self, ExecError> {
        if !max_weight.is_finite() || max_weight <= 0.0 || max_weight > 1.0 {
            return Err(ExecError::strategy(
                "MaxPositionWeight requires a weight in (0, 1]",
            ));
        }
        Ok(Self { max_weight })
    }

    /// Returns the configured maximum weight.
    #[must_use]
    #[inline]
    pub fn max_weight(&self) -> f64 {
        self.max_weight
    }
}

impl RiskRule for MaxPositionWeightRule {
    fn name(&self) -> &str {
        "MaxPositionWeight"
    }

    fn check(&mut self, request: &OrderRequest, ctx: &RiskContext) -> Result<(), ExecError> {
        if request.side != OrderSide::Buy || !ctx.increases_exposure(request) {
            return Ok(());
        }
        if ctx.equity <= Decimal::ZERO {
            return Err(ExecError::risk_rejected(
                self.name(),
                "equity is not positive",
            ));
        }
        let price = ctx.price_for(request).ok_or_else(|| {
            ExecError::risk_rejected(self.name(), "no reference price is available")
        })?;
        let new_quantity = ctx.position_qty.checked_add(request.quantity).ok_or(
            kurumi_core::KurumiError::NumericalOverflow {
                operation: "MaxPositionWeight",
            },
        )?;
        if new_quantity <= Decimal::ZERO {
            return Ok(());
        }
        let new_value =
            new_quantity
                .checked_mul(price)
                .ok_or(kurumi_core::KurumiError::NumericalOverflow {
                    operation: "MaxPositionWeight",
                })?;
        let equity = ctx.equity.to_f64().unwrap_or(0.0);
        if equity <= 0.0 {
            return Err(ExecError::risk_rejected(
                self.name(),
                "equity is not positive",
            ));
        }
        let weight = new_value.to_f64().unwrap_or(f64::INFINITY) / equity;
        if weight > self.max_weight {
            return Err(ExecError::risk_rejected(
                self.name(),
                format!(
                    "post-trade weight {weight:.6} exceeds the {} cap",
                    self.max_weight
                ),
            ));
        }
        Ok(())
    }
}

/// Latches the kill switch when the daily loss ratio is exceeded.
///
/// The condition is
///
/// ```text
/// (day_start_equity - equity) / day_start_equity >= max_loss_ratio
/// ```
///
/// The rule itself also rejects exposure-increasing orders when it is used
/// standalone; inside [`RiskGate`] the gate's latched switch enforces the same
/// policy.
///
/// # Examples
///
/// ```
/// use kurumi_exec::MaxDailyLossRule;
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let rule = MaxDailyLossRule::new(0.05)?;
/// assert_eq!(rule.max_loss_ratio(), 0.05);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaxDailyLossRule {
    max_loss_ratio: f64,
}

impl MaxDailyLossRule {
    /// Creates the rule.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::StrategyError`] unless `max_loss_ratio` is finite
    /// and in `[0, 1)`.
    pub fn new(max_loss_ratio: f64) -> Result<Self, ExecError> {
        if !max_loss_ratio.is_finite() || !(0.0..1.0).contains(&max_loss_ratio) {
            return Err(ExecError::strategy(
                "MaxDailyLoss requires a loss ratio in [0, 1)",
            ));
        }
        Ok(Self { max_loss_ratio })
    }

    /// Returns the configured loss ratio.
    #[must_use]
    #[inline]
    pub fn max_loss_ratio(&self) -> f64 {
        self.max_loss_ratio
    }

    fn breached(&self, ctx: &RiskContext) -> bool {
        if ctx.day_start_equity <= Decimal::ZERO {
            return true;
        }
        let start = ctx.day_start_equity.to_f64().unwrap_or(0.0);
        let equity = ctx.equity.to_f64().unwrap_or(0.0);
        if start <= 0.0 {
            return true;
        }
        (start - equity) / start >= self.max_loss_ratio
    }
}

impl RiskRule for MaxDailyLossRule {
    fn name(&self) -> &str {
        "MaxDailyLoss"
    }

    fn check(&mut self, request: &OrderRequest, ctx: &RiskContext) -> Result<(), ExecError> {
        if self.breached(ctx) && ctx.increases_exposure(request) {
            return Err(ExecError::risk_rejected(
                self.name(),
                "daily loss limit is breached",
            ));
        }
        Ok(())
    }

    fn kill_switch_triggered(&self, ctx: &RiskContext) -> bool {
        self.breached(ctx)
    }
}

/// Rejects orders that would exceed a sliding-window submission rate.
///
/// The window is `[now - window_secs, now]` and uses the event timestamp, never
/// the wall clock.
///
/// # Examples
///
/// ```
/// use kurumi_exec::OrderRateLimitRule;
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let rule = OrderRateLimitRule::new(10, 60)?;
/// assert_eq!(rule.max_orders(), 10);
/// assert_eq!(rule.window_secs(), 60);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct OrderRateLimitRule {
    max_orders: usize,
    window_secs: i64,
    timestamps: VecDeque<DateTime<Utc>>,
}

impl OrderRateLimitRule {
    /// Creates the rule.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::StrategyError`] when `max_orders` or `window_secs`
    /// is zero.
    pub fn new(max_orders: usize, window_secs: i64) -> Result<Self, ExecError> {
        if max_orders == 0 || window_secs <= 0 || Duration::try_seconds(window_secs).is_none() {
            return Err(ExecError::strategy(
                "OrderRateLimit requires a positive count and a representable window",
            ));
        }
        Ok(Self {
            max_orders,
            window_secs,
            timestamps: VecDeque::new(),
        })
    }

    /// Returns the maximum number of orders per window.
    #[must_use]
    #[inline]
    pub fn max_orders(&self) -> usize {
        self.max_orders
    }

    /// Returns the window length in seconds.
    #[must_use]
    #[inline]
    pub fn window_secs(&self) -> i64 {
        self.window_secs
    }

    fn prune(&mut self, now: DateTime<Utc>) {
        let duration = Duration::try_seconds(self.window_secs).unwrap_or_default();
        let cutoff = now
            .checked_sub_signed(duration)
            .unwrap_or(DateTime::<Utc>::MIN_UTC);
        while self
            .timestamps
            .front()
            .is_some_and(|timestamp| *timestamp <= cutoff)
        {
            self.timestamps.pop_front();
        }
    }
}

impl RiskRule for OrderRateLimitRule {
    fn name(&self) -> &str {
        "OrderRateLimit"
    }

    fn check(&mut self, _request: &OrderRequest, ctx: &RiskContext) -> Result<(), ExecError> {
        self.prune(ctx.now);
        if self.timestamps.len() >= self.max_orders {
            return Err(ExecError::risk_rejected(
                self.name(),
                format!(
                    "more than {} orders in the last {} seconds",
                    self.max_orders, self.window_secs
                ),
            ));
        }
        Ok(())
    }

    fn on_accepted(&mut self, _request: &OrderRequest, ctx: &RiskContext) {
        self.timestamps.push_back(ctx.now);
    }
}

/// Rejects a client order identifier that has already been accepted.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{DuplicateOrderIdRule, RiskRule};
///
/// let rule = DuplicateOrderIdRule::new();
/// assert_eq!(rule.name(), "DuplicateOrderId");
/// ```
#[derive(Debug, Clone, Default)]
pub struct DuplicateOrderIdRule {
    seen: BTreeSet<ClientOrderId>,
}

impl DuplicateOrderIdRule {
    /// Creates an empty rule.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of accepted identifiers remembered.
    #[must_use]
    pub fn seen_count(&self) -> usize {
        self.seen.len()
    }
}

impl RiskRule for DuplicateOrderIdRule {
    fn name(&self) -> &str {
        "DuplicateOrderId"
    }

    fn check(&mut self, request: &OrderRequest, _ctx: &RiskContext) -> Result<(), ExecError> {
        if self.seen.contains(&request.client_order_id) {
            return Err(ExecError::DuplicateClientOrderId {
                client_order_id: request.client_order_id.to_string(),
            });
        }
        Ok(())
    }

    fn on_accepted(&mut self, request: &OrderRequest, _ctx: &RiskContext) {
        self.seen.insert(request.client_order_id.clone());
    }
}

/// Restricts orders to an allowlist and/or rejects a denylist.
///
/// An empty allowlist allows every symbol; a denylist entry always wins.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{RiskRule, SymbolFilterRule};
/// use std::collections::BTreeSet;
///
/// let rule = SymbolFilterRule::allowlist(BTreeSet::from(["AAA".to_owned()]));
/// assert_eq!(rule.name(), "SymbolFilter");
/// ```
#[derive(Debug, Clone, Default)]
pub struct SymbolFilterRule {
    allowlist: BTreeSet<String>,
    denylist: BTreeSet<String>,
}

impl SymbolFilterRule {
    /// Creates a filter from an allowlist and a denylist.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::StrategyError`] when the two lists overlap.
    pub fn new(allowlist: BTreeSet<String>, denylist: BTreeSet<String>) -> Result<Self, ExecError> {
        if allowlist.iter().any(|symbol| denylist.contains(symbol)) {
            return Err(ExecError::strategy(
                "a symbol cannot be both allowed and denied",
            ));
        }
        Ok(Self {
            allowlist,
            denylist,
        })
    }

    /// Creates a filter with only an allowlist.
    #[must_use]
    pub fn allowlist(allowlist: BTreeSet<String>) -> Self {
        Self {
            allowlist,
            denylist: BTreeSet::new(),
        }
    }

    /// Creates a filter with only a denylist.
    #[must_use]
    pub fn denylist(denylist: BTreeSet<String>) -> Self {
        Self {
            allowlist: BTreeSet::new(),
            denylist,
        }
    }

    /// Returns the allowed symbols.
    #[must_use]
    pub fn allowed(&self) -> &BTreeSet<String> {
        &self.allowlist
    }

    /// Returns the denied symbols.
    #[must_use]
    pub fn denied(&self) -> &BTreeSet<String> {
        &self.denylist
    }
}

impl RiskRule for SymbolFilterRule {
    fn name(&self) -> &str {
        "SymbolFilter"
    }

    fn check(&mut self, request: &OrderRequest, _ctx: &RiskContext) -> Result<(), ExecError> {
        if self.denylist.contains(&request.symbol) {
            return Err(ExecError::risk_rejected(
                self.name(),
                format!("symbol `{}` is denied", request.symbol),
            ));
        }
        if !self.allowlist.is_empty() && !self.allowlist.contains(&request.symbol) {
            return Err(ExecError::risk_rejected(
                self.name(),
                format!("symbol `{}` is not allowed", request.symbol),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{OrderType, TimeInForce};

    fn ts(second: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(second, 0).expect("valid timestamp")
    }

    fn request(id: &str, side: OrderSide, qty: i64) -> OrderRequest {
        OrderRequest::new(
            ClientOrderId::new(id),
            "AAA",
            side,
            OrderType::Market,
            Decimal::from(qty),
            TimeInForce::Day,
        )
    }

    fn context(now: i64, equity: i64, position: i64) -> RiskContext {
        RiskContext {
            now: ts(now),
            equity: Decimal::from(equity),
            day_start_equity: Decimal::from(10_000),
            position_qty: Decimal::from(position),
            reference_price: Some(Decimal::from(100)),
            available_cash: Decimal::from(10_000),
        }
    }

    #[test]
    fn kill_switch_blocks_buys_but_allows_reducing_sells() {
        let mut gate = RiskGate::new(vec![Box::new(MaxDailyLossRule::new(0.05).expect("rule"))]);
        let context = context(0, 9_000, 100);
        let buy = request("b-1", OrderSide::Buy, 10);
        assert!(matches!(
            gate.evaluate(&buy, &context),
            Err(ExecError::RiskRejected { ref rule, .. }) if rule == "MaxDailyLoss"
        ));
        assert!(gate.kill_switch_active());
        let sell = request("s-1", OrderSide::Sell, 100);
        assert!(gate.evaluate(&sell, &context).is_ok());
        let oversized_sell = request("s-2", OrderSide::Sell, 101);
        assert!(gate.evaluate(&oversized_sell, &context).is_err());
    }

    #[test]
    fn notional_and_weight_rules_reject_large_buys() {
        let notional = MaxOrderNotionalRule::new(Decimal::from(1_000)).expect("rule");
        let mut gate = RiskGate::new(vec![
            Box::new(notional),
            Box::new(MaxPositionWeightRule::new(0.2).expect("rule")),
        ]);
        let context = context(0, 10_000, 0);
        assert!(
            gate.evaluate(&request("b-1", OrderSide::Buy, 5), &context)
                .is_ok()
        );
        assert!(
            gate.evaluate(&request("b-2", OrderSide::Buy, 11), &context)
                .is_err()
        );
        assert!(
            gate.evaluate(&request("b-3", OrderSide::Buy, 21), &context)
                .is_err()
        );
    }

    #[test]
    fn position_weight_rule_blocks_only_larger_longs() {
        let mut gate = RiskGate::new(vec![Box::new(
            MaxPositionWeightRule::new(0.2).expect("rule"),
        )]);
        let flat = context(0, 10_000, 0);
        assert!(
            gate.evaluate(&request("b-1", OrderSide::Buy, 10), &flat)
                .is_ok()
        );
        assert!(
            gate.evaluate(&request("b-2", OrderSide::Buy, 25), &flat)
                .is_err()
        );
        let held = context(0, 10_000, 50);
        assert!(
            gate.evaluate(&request("s-1", OrderSide::Sell, 50), &held)
                .is_ok()
        );
    }

    #[test]
    fn rate_limit_uses_the_event_clock() {
        let mut gate = RiskGate::new(vec![Box::new(
            OrderRateLimitRule::new(1, 60).expect("rule"),
        )]);
        let first = context(0, 10_000, 0);
        let second = context(30, 10_000, 0);
        let third = context(61, 10_000, 0);
        assert!(
            gate.evaluate(&request("a", OrderSide::Buy, 1), &first)
                .is_ok()
        );
        assert!(
            gate.evaluate(&request("b", OrderSide::Buy, 1), &second)
                .is_err()
        );
        assert!(
            gate.evaluate(&request("c", OrderSide::Buy, 1), &third)
                .is_ok()
        );
    }

    #[test]
    fn duplicate_ids_and_symbol_filters_are_enforced() {
        let mut gate = RiskGate::new(vec![
            Box::new(DuplicateOrderIdRule::new()),
            Box::new(SymbolFilterRule::allowlist(BTreeSet::from([
                "AAA".to_owned()
            ]))),
        ]);
        let context = context(0, 10_000, 0);
        assert!(
            gate.evaluate(&request("dup", OrderSide::Buy, 1), &context)
                .is_ok()
        );
        assert!(matches!(
            gate.evaluate(&request("dup", OrderSide::Buy, 1), &context),
            Err(ExecError::DuplicateClientOrderId { .. })
        ));
        let mut other = request("other", OrderSide::Buy, 1);
        other.symbol = "BBB".to_owned();
        assert!(gate.evaluate(&other, &context).is_err());
    }
}
