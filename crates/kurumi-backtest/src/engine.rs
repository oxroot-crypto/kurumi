//! The event-driven backtest kernel.
//!
//! Orders are submitted as events and matched when the next [`OhlcvBar`] is fed
//! to [`BacktestEngine::on_bar`]. Market orders execute at the bar's opening
//! price; limit orders execute only when the bar's low (for a buy) or high (for
//! a sell) reaches the limit, and never at a price worse than the limit after
//! slippage. Maker/taker liquidity is inferred from the order style, and the
//! account is marked to the bar close after every bar.
//!
//! Bar prices are `f64`; they are converted to a 12-decimal [`Decimal`] for the
//! ledger, which keeps the accounting deterministic without rounding the
//! exchange tick size away. The account does not enforce a margin or
//! buying-power check, so callers must size orders with `kurumi-risk` or their
//! own risk limits.

use kurumi_core::{Currency, KurumiError, Price};
use kurumi_risk::MetricsConfig;
use kurumi_ta::OhlcvBar;
use rust_decimal::Decimal;

use crate::account::PortfolioAccount;
use crate::fee::{FeeModel, Liquidity};
use crate::order::{Fill, Order, OrderKind, Side};
use crate::report::BacktestReport;
use crate::slippage::SlippageModel;

/// The four prices of a bar converted into the decimal ledger.
#[derive(Debug, Clone, Copy)]
struct BarPrices {
    open: Price,
    high: Price,
    low: Price,
    close: Price,
}

/// An order waiting for the next bar.
#[derive(Debug, Clone, Copy)]
struct PendingOrder {
    id: u64,
    order: Order,
}

/// A single-instrument event-driven backtest engine.
///
/// # Examples
///
/// ```
/// use kurumi_backtest::{
///     BacktestEngine, NoFee, NoSlippage, Order, PortfolioAccount, Side,
/// };
/// use kurumi_core::{Currency, Quantity};
/// use kurumi_ta::OhlcvBar;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
/// let account = PortfolioAccount::new(Decimal::from(10_000), Currency::Usd)?;
/// let mut engine = BacktestEngine::new(account, Box::new(NoFee), Box::new(NoSlippage));
///
/// let id = engine.submit(Order::market(
///     Side::Buy,
///     Quantity::new(Decimal::from(10))?,
/// ))?;
/// assert_eq!(id, 1);
///
/// let bar = OhlcvBar::new(0, 100.0, 101.0, 99.0, 100.5, 1_000.0, 100_500.0)?;
/// let fills = engine.on_bar(&bar)?;
/// assert_eq!(fills.len(), 1);
/// assert_eq!(engine.account().position(), Decimal::from(10));
/// # Ok(())
/// # }
/// ```
pub struct BacktestEngine {
    account: PortfolioAccount,
    fee_model: Box<dyn FeeModel>,
    slippage_model: Box<dyn SlippageModel>,
    pending: Vec<PendingOrder>,
    fills: Vec<Fill>,
    next_order_id: u64,
}

impl BacktestEngine {
    /// Creates an engine around an existing account and cost models.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{BacktestEngine, NoFee, NoSlippage, PortfolioAccount};
    /// use kurumi_core::Currency;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let account = PortfolioAccount::new(Decimal::from(1_000), Currency::Usd)?;
    /// let engine = BacktestEngine::new(account, Box::new(NoFee), Box::new(NoSlippage));
    /// assert_eq!(engine.account().cash(), Decimal::from(1_000));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn new(
        account: PortfolioAccount,
        fee_model: Box<dyn FeeModel>,
        slippage_model: Box<dyn SlippageModel>,
    ) -> Self {
        Self {
            account,
            fee_model,
            slippage_model,
            pending: Vec::new(),
            fills: Vec::new(),
            next_order_id: 1,
        }
    }

    /// Creates an engine with a fresh cash account.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`PortfolioAccount::new`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{BacktestEngine, NoFee, NoSlippage};
    /// use kurumi_core::Currency;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let engine = BacktestEngine::with_cash(
    ///     Decimal::from(25_000),
    ///     Currency::Usd,
    ///     Box::new(NoFee),
    ///     Box::new(NoSlippage),
    /// )?;
    /// assert_eq!(engine.account().initial_cash(), Decimal::from(25_000));
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_cash(
        initial_cash: Decimal,
        currency: Currency,
        fee_model: Box<dyn FeeModel>,
        slippage_model: Box<dyn SlippageModel>,
    ) -> Result<Self, KurumiError> {
        let account = PortfolioAccount::new(initial_cash, currency)?;
        Ok(Self::new(account, fee_model, slippage_model))
    }

    /// Submits an order for the next bar and returns its identifier.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the quantity is zero, and
    /// [`KurumiError::NumericalOverflow`] when the order identifiers are
    /// exhausted.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{BacktestEngine, NoFee, NoSlippage, Order, Side};
    /// use kurumi_core::{Currency, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let mut engine = BacktestEngine::with_cash(
    ///     Decimal::from(1_000),
    ///     Currency::Usd,
    ///     Box::new(NoFee),
    ///     Box::new(NoSlippage),
    /// )?;
    /// assert_eq!(
    ///     engine.submit(Order::market(Side::Buy, Quantity::new(Decimal::ONE)?))?,
    ///     1
    ///   );
    /// assert!(engine.submit(Order::market(Side::Buy, Quantity::new(Decimal::ZERO)?)).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn submit(&mut self, order: Order) -> Result<u64, KurumiError> {
        if order.quantity().value() <= Decimal::ZERO {
            return Err(KurumiError::InvalidParameter {
                parameter: "quantity",
                reason: "an order quantity must be strictly positive".to_owned(),
            });
        }
        let id = self.next_order_id;
        self.next_order_id =
            self.next_order_id
                .checked_add(1)
                .ok_or(KurumiError::NumericalOverflow {
                    operation: "BacktestEngine::submit",
                })?;
        self.pending.push(PendingOrder { id, order });
        Ok(id)
    }

    /// Cancels a pending order.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when no pending order has the
    /// supplied identifier.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{BacktestEngine, NoFee, NoSlippage, Order, Side};
    /// use kurumi_core::{Currency, Quantity};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let mut engine = BacktestEngine::with_cash(
    ///     Decimal::from(1_000),
    ///     Currency::Usd,
    ///     Box::new(NoFee),
    ///     Box::new(NoSlippage),
    /// )?;
    /// let id = engine.submit(Order::market(Side::Buy, Quantity::new(Decimal::ONE)?))?;
    /// engine.cancel(id)?;
    /// assert_eq!(engine.pending_order_count(), 0);
    /// assert!(engine.cancel(id).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn cancel(&mut self, order_id: u64) -> Result<(), KurumiError> {
        let index = self
            .pending
            .iter()
            .position(|queued| queued.id == order_id)
            .ok_or(KurumiError::InvalidParameter {
                parameter: "order_id",
                reason: "no pending order has this identifier".to_owned(),
            })?;
        self.pending.remove(index);
        Ok(())
    }

    /// Matches every pending order against `bar`, records the close mark, and
    /// returns the fills produced by this bar.
    ///
    /// Market orders execute at the bar's open. A buy limit needs the bar low to
    /// reach the limit; a sell limit needs the bar high to reach it. Slippage is
    /// applied and then clamped so a limit fill never violates its limit price.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] for a bar price that is not
    /// representable as a positive decimal, and propagates errors from the fee,
    /// slippage, and account modules.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{BacktestEngine, NoFee, NoSlippage, Order, Side};
    /// use kurumi_core::{Currency, Quantity};
    /// use kurumi_ta::OhlcvBar;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let mut engine = BacktestEngine::with_cash(
    ///     Decimal::from(10_000),
    ///     Currency::Usd,
    ///     Box::new(NoFee),
    ///     Box::new(NoSlippage),
    /// )?;
    /// engine.submit(Order::market(Side::Buy, Quantity::new(Decimal::from(10))?))?;
    /// let bar = OhlcvBar::new(0, 100.0, 101.0, 99.0, 100.5, 1_000.0, 100_500.0)?;
    /// let fills = engine.on_bar(&bar)?;
    /// assert_eq!(fills.first().expect("one fill").price().value(), Decimal::from(100));
    /// # Ok(())
    /// # }
    /// ```
    pub fn on_bar(&mut self, bar: &OhlcvBar) -> Result<Vec<Fill>, KurumiError> {
        let prices = BarPrices {
            open: price_from_f64(bar.open(), "open")?,
            high: price_from_f64(bar.high(), "high")?,
            low: price_from_f64(bar.low(), "low")?,
            close: price_from_f64(bar.close(), "close")?,
        };

        let pending = std::mem::take(&mut self.pending);
        let mut unfilled = Vec::new();
        let mut new_fills = Vec::new();
        let mut queue = pending.into_iter();
        while let Some(queued) = queue.next() {
            match self.execute(queued.id, queued.order, &prices, bar.timestamp()) {
                Ok(Some(fill)) => {
                    if let Err(error) = self.account.apply_fill(&fill) {
                        // Keep the order pending: the ledger did not move.
                        unfilled.push(queued);
                        unfilled.extend(queue);
                        self.pending = unfilled;
                        return Err(error);
                    }
                    self.fills.push(fill);
                    new_fills.push(fill);
                }
                Ok(None) => unfilled.push(queued),
                Err(error) => {
                    unfilled.push(queued);
                    unfilled.extend(queue);
                    self.pending = unfilled;
                    return Err(error);
                }
            }
        }
        self.pending = unfilled;
        self.account.record_equity(bar.timestamp(), prices.close)?;
        Ok(new_fills)
    }

    /// Returns the account ledger.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{BacktestEngine, NoFee, NoSlippage};
    /// use kurumi_core::Currency;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let engine = BacktestEngine::with_cash(
    ///     Decimal::from(100),
    ///     Currency::Usd,
    ///     Box::new(NoFee),
    ///     Box::new(NoSlippage),
    /// )?;
    /// assert_eq!(engine.account().currency(), Currency::Usd);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub const fn account(&self) -> &PortfolioAccount {
        &self.account
    }

    /// Returns every fill produced so far, in execution order.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{BacktestEngine, NoFee, NoSlippage};
    /// use kurumi_core::Currency;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let engine = BacktestEngine::with_cash(
    ///     Decimal::from(100),
    ///     Currency::Usd,
    ///     Box::new(NoFee),
    ///     Box::new(NoSlippage),
    /// )?;
    /// assert!(engine.fills().is_empty());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn fills(&self) -> &[Fill] {
        &self.fills
    }

    /// Returns the number of orders still waiting for a bar.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{BacktestEngine, NoFee, NoSlippage};
    /// use kurumi_core::Currency;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let engine = BacktestEngine::with_cash(
    ///     Decimal::from(100),
    ///     Currency::Usd,
    ///     Box::new(NoFee),
    ///     Box::new(NoSlippage),
    /// )?;
    /// assert_eq!(engine.pending_order_count(), 0);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn pending_order_count(&self) -> usize {
        self.pending.len()
    }

    /// Builds a [`BacktestReport`] from the recorded equity curve.
    ///
    /// At least three bars must have been processed so the return series has
    /// two observations.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`BacktestReport::from_equity_curve`].
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_backtest::{BacktestEngine, NoFee, NoSlippage, Order, Side};
    /// use kurumi_core::{Currency, Quantity};
    /// use kurumi_risk::MetricsConfig;
    /// use kurumi_ta::OhlcvBar;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_backtest::KurumiError> {
    /// let mut engine = BacktestEngine::with_cash(
    ///     Decimal::from(10_000),
    ///     Currency::Usd,
    ///     Box::new(NoFee),
    ///     Box::new(NoSlippage),
    /// )?;
    /// engine.submit(Order::market(Side::Buy, Quantity::new(Decimal::from(10))?))?;
    /// for (index, close) in [100.0, 105.0, 110.0].iter().enumerate() {
    ///     let bar = OhlcvBar::new(
    ///         index as i64,
    ///         *close,
    ///         close + 1.0,
    ///         close - 1.0,
    ///         *close,
    ///         1_000.0,
    ///         close * 1_000.0,
    ///     )?;
    ///     engine.on_bar(&bar)?;
    /// }
    /// let report = engine.report(MetricsConfig::default())?;
    /// assert_eq!(report.metrics.observations, 2);
    /// assert_eq!(report.fill_count, 1);
    /// # Ok(())
    /// # }
    /// ```
    pub fn report(&self, config: MetricsConfig) -> Result<BacktestReport, KurumiError> {
        BacktestReport::from_equity_curve(
            self.account.equity_curve(),
            self.account.initial_cash(),
            self.account.total_fees(),
            self.fills.len(),
            self.account.currency(),
            config,
        )
    }

    fn execute(
        &self,
        order_id: u64,
        order: Order,
        prices: &BarPrices,
        timestamp: i64,
    ) -> Result<Option<Fill>, KurumiError> {
        let Some(reference) = reference_price(order, prices) else {
            return Ok(None);
        };
        let slipped = self.slippage_model.fill_price(order.side(), reference)?;
        let execution = match order.kind() {
            OrderKind::Market => slipped,
            OrderKind::Limit { price } => match order.side() {
                Side::Buy => slipped.min(price),
                Side::Sell => slipped.max(price),
            },
        };
        let liquidity = if order.is_market() {
            Liquidity::Taker
        } else {
            Liquidity::Maker
        };
        let fee = self
            .fee_model
            .fee(execution, order.quantity(), order.side(), liquidity)?;
        let fill = Fill::new(
            order_id,
            timestamp,
            order.side(),
            execution,
            order.quantity(),
            fee,
            liquidity,
        )?;
        Ok(Some(fill))
    }
}

fn reference_price(order: Order, prices: &BarPrices) -> Option<Price> {
    match order.kind() {
        OrderKind::Market => Some(prices.open),
        OrderKind::Limit { price } => match order.side() {
            Side::Buy if prices.low <= price => Some(prices.open.min(price)),
            Side::Sell if prices.high >= price => Some(prices.open.max(price)),
            _ => None,
        },
    }
}

fn price_from_f64(value: f64, parameter: &'static str) -> Result<Price, KurumiError> {
    let decimal = Decimal::from_f64_retain(value).ok_or(KurumiError::InvalidParameter {
        parameter,
        reason: "must be finite".to_owned(),
    })?;
    Price::new(decimal.round_dp(12)).map_err(|_| KurumiError::InvalidParameter {
        parameter,
        reason: "must be strictly positive at 12-decimal precision".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fee::{FixedRateFee, MakerTakerFee, NoFee};
    use crate::slippage::{FixedBpsSlippage, NoSlippage};
    use kurumi_core::Quantity;

    fn bar(timestamp: i64, open: f64, high: f64, low: f64, close: f64) -> OhlcvBar {
        OhlcvBar::new(timestamp, open, high, low, close, 1_000.0, 100_000.0)
            .expect("valid test bar")
    }

    fn quantity(value: i64) -> Quantity {
        Quantity::new(Decimal::from(value)).expect("valid test quantity")
    }

    #[test]
    fn market_order_fills_at_the_next_bar_open() {
        let mut engine = BacktestEngine::with_cash(
            Decimal::from(10_000),
            Currency::Usd,
            Box::new(NoFee),
            Box::new(NoSlippage),
        )
        .expect("valid engine");
        engine
            .submit(Order::market(Side::Buy, quantity(10)))
            .expect("submitted");
        let fills = engine
            .on_bar(&bar(0, 100.0, 101.0, 99.0, 100.5))
            .expect("processed");
        assert_eq!(fills.len(), 1);
        let fill = fills.first().expect("one fill");
        assert_eq!(fill.price().value(), Decimal::from(100));
        assert_eq!(fill.liquidity(), Liquidity::Taker);
        assert_eq!(engine.account().cash(), Decimal::from(9_000));
        assert_eq!(engine.account().position(), Decimal::from(10));
        assert_eq!(engine.account().equity_curve().len(), 1);
    }

    #[test]
    fn fees_and_slippage_are_applied_to_the_fill() {
        let fee = FixedRateFee::new(Decimal::new(1, 3)).expect("valid fee");
        let slippage = FixedBpsSlippage::new(Decimal::from(10)).expect("valid slippage");
        let mut engine = BacktestEngine::with_cash(
            Decimal::from(10_000),
            Currency::Usd,
            Box::new(fee),
            Box::new(slippage),
        )
        .expect("valid engine");
        engine
            .submit(Order::market(Side::Buy, quantity(10)))
            .expect("submitted");
        let fills = engine
            .on_bar(&bar(0, 100.0, 101.0, 99.0, 100.5))
            .expect("processed");
        let fill = fills.first().expect("one fill");
        // 100 * 1.001 = 100.1; fee = 100.1 * 10 * 0.001 = 1.001.
        assert_eq!(fill.price().value(), Decimal::new(1001, 1));
        assert_eq!(fill.fee(), Decimal::new(1001, 3));
        assert_eq!(engine.account().total_fees(), Decimal::new(1001, 3));
    }

    #[test]
    fn limit_buys_wait_for_the_bar_low_to_reach_the_limit() {
        let mut engine = BacktestEngine::with_cash(
            Decimal::from(10_000),
            Currency::Usd,
            Box::new(NoFee),
            Box::new(NoSlippage),
        )
        .expect("valid engine");
        let limit = Price::new(Decimal::from(95)).expect("valid limit");
        engine
            .submit(Order::limit(Side::Buy, quantity(10), limit))
            .expect("submitted");

        let first = engine
            .on_bar(&bar(0, 100.0, 101.0, 99.0, 100.0))
            .expect("processed");
        assert!(first.is_empty());
        assert_eq!(engine.pending_order_count(), 1);

        let second = engine
            .on_bar(&bar(1, 96.0, 97.0, 94.0, 95.5))
            .expect("processed");
        assert_eq!(second.len(), 1);
        let fill = second.first().expect("one fill");
        assert_eq!(fill.price().value(), Decimal::from(95));
        assert_eq!(fill.liquidity(), Liquidity::Maker);
        assert_eq!(engine.pending_order_count(), 0);
    }

    #[test]
    fn limit_fills_never_breach_the_limit_after_slippage() {
        let slippage = FixedBpsSlippage::new(Decimal::from(100)).expect("valid slippage");
        let mut engine = BacktestEngine::with_cash(
            Decimal::from(10_000),
            Currency::Usd,
            Box::new(NoFee),
            Box::new(slippage),
        )
        .expect("valid engine");
        let limit = Price::new(Decimal::from(95)).expect("valid limit");
        engine
            .submit(Order::limit(Side::Buy, quantity(10), limit))
            .expect("submitted");
        let fills = engine
            .on_bar(&bar(0, 96.0, 97.0, 90.0, 94.0))
            .expect("processed");
        let fill = fills.first().expect("one fill");
        assert_eq!(fill.price().value(), Decimal::from(95));
    }

    #[test]
    fn sell_limits_wait_for_the_bar_high_to_reach_the_limit() {
        let mut engine = BacktestEngine::with_cash(
            Decimal::from(10_000),
            Currency::Usd,
            Box::new(NoFee),
            Box::new(NoSlippage),
        )
        .expect("valid engine");
        let limit = Price::new(Decimal::from(105)).expect("valid limit");
        engine
            .submit(Order::limit(Side::Sell, quantity(10), limit))
            .expect("submitted");
        let fills = engine
            .on_bar(&bar(0, 100.0, 106.0, 99.0, 105.0))
            .expect("processed");
        let fill = fills.first().expect("one fill");
        assert_eq!(fill.price().value(), Decimal::from(105));
    }

    #[test]
    fn maker_taker_fees_follow_the_order_style() {
        let fee = MakerTakerFee::new(Decimal::new(1, 2), Decimal::new(5, 2)).expect("valid fee");
        let mut engine = BacktestEngine::with_cash(
            Decimal::from(10_000),
            Currency::Usd,
            Box::new(fee),
            Box::new(NoSlippage),
        )
        .expect("valid engine");
        engine
            .submit(Order::market(Side::Buy, quantity(10)))
            .expect("market submitted");
        let fill = engine
            .on_bar(&bar(0, 100.0, 101.0, 99.0, 100.0))
            .expect("processed")
            .first()
            .copied()
            .expect("one fill");
        // 1,000 notional at 5% taker rate.
        assert_eq!(fill.fee(), Decimal::from(50));

        let limit = Price::new(Decimal::from(100)).expect("valid limit");
        engine
            .submit(Order::limit(Side::Buy, quantity(10), limit))
            .expect("limit submitted");
        let fill = engine
            .on_bar(&bar(1, 100.0, 101.0, 99.0, 100.0))
            .expect("processed")
            .first()
            .copied()
            .expect("one fill");
        // 1,000 notional at 1% maker rate.
        assert_eq!(fill.fee(), Decimal::from(10));
    }

    #[test]
    fn cancel_removes_the_pending_order() {
        let mut engine = BacktestEngine::with_cash(
            Decimal::from(1_000),
            Currency::Usd,
            Box::new(NoFee),
            Box::new(NoSlippage),
        )
        .expect("valid engine");
        let id = engine
            .submit(Order::market(Side::Buy, quantity(1)))
            .expect("submitted");
        assert_eq!(engine.pending_order_count(), 1);
        engine.cancel(id).expect("cancelled");
        assert_eq!(engine.pending_order_count(), 0);
        assert!(engine.cancel(id).is_err());
    }

    #[test]
    fn report_is_built_from_the_equity_curve() {
        let mut engine = BacktestEngine::with_cash(
            Decimal::from(10_000),
            Currency::Usd,
            Box::new(NoFee),
            Box::new(NoSlippage),
        )
        .expect("valid engine");
        engine
            .submit(Order::market(Side::Buy, quantity(10)))
            .expect("submitted");
        for (index, close) in [100.0, 105.0, 110.0].iter().enumerate() {
            engine
                .on_bar(&bar(index as i64, *close, close + 1.0, close - 1.0, *close))
                .expect("processed");
        }
        let report = engine
            .report(MetricsConfig::default())
            .expect("valid report");
        assert_eq!(report.fill_count, 1);
        assert_eq!(report.metrics.observations, 2);
        assert_eq!(report.final_equity, Decimal::from(10_100));
        assert_eq!(report.initial_equity, Decimal::from(10_000));
    }

    #[test]
    fn submit_rejects_zero_quantity() {
        let mut engine = BacktestEngine::with_cash(
            Decimal::from(1_000),
            Currency::Usd,
            Box::new(NoFee),
            Box::new(NoSlippage),
        )
        .expect("valid engine");
        assert!(
            engine
                .submit(Order::market(Side::Buy, quantity(0)))
                .is_err()
        );
    }
}
