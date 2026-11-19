//! Rebalancing trade generation from Decimal account holdings.
//!
//! The engine compares the current market value of every position with the
//! target value implied by a [`PortfolioWeights`] allocation and the account's
//! total net equity, then emits buy and sell orders in whole or fractional
//! units. All monetary arithmetic stays in [`Decimal`]; the only `f64`
//! conversion is the reported post-trade weight deviation.
//!
//! ```text
//! current_value_i = quantity_i * price_i
//! target_value_i  = total_equity * target_weight_i
//! trade_value_i   = target_value_i - current_value_i
//! ```
//!
//! A positive trade value is a buy and a negative value is a sell. Orders whose
//! absolute value is below `min_trade_value` are suppressed as noise, so the
//! emitted plan may leave small residual deviations. When
//! `allow_fractional` is `false`, the suggested quantity is floored to a whole
//! unit, which also produces a small residual.
//!
//! The target universe defines which symbols are traded. Holdings outside the
//! target universe are treated as a zero target weight and are sold down; every
//! target symbol must have a holding entry (a zero quantity is enough) so that
//! the engine knows its latest price.

use std::collections::{BTreeMap, BTreeSet};

use kurumi_core::KurumiError;
use rust_decimal::Decimal;

use crate::analytics::PortfolioWeights;

/// A current position and its latest price.
///
/// Quantities and prices are [`Decimal`] to keep monetary arithmetic exact. A
/// negative quantity represents a short position; the price must be strictly
/// positive. Symbols must be unique within a holdings list, which is checked
/// when [`generate_rebalance_actions`] runs.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::Holding;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let position = Holding::new("AAA", Decimal::from(100), Decimal::from(50))?;
/// assert_eq!(position.symbol(), "AAA");
/// assert_eq!(position.quantity(), Decimal::from(100));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holding {
    symbol: String,
    quantity: Decimal,
    price: Decimal,
}

impl Holding {
    /// Creates a validated holding.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the symbol is empty or the
    /// price is not strictly positive.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::Holding;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// assert!(Holding::new("AAA", Decimal::from(10), Decimal::ZERO).is_err());
    /// assert!(Holding::new("", Decimal::from(10), Decimal::ONE).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        symbol: impl Into<String>,
        quantity: Decimal,
        price: Decimal,
    ) -> Result<Self, KurumiError> {
        let symbol = symbol.into();
        if symbol.trim().is_empty() {
            return Err(KurumiError::InvalidParameter {
                parameter: "symbol",
                reason: "must not be empty".to_owned(),
            });
        }
        if price <= Decimal::ZERO {
            return Err(KurumiError::InvalidParameter {
                parameter: "price",
                reason: format!("price of `{symbol}` must be strictly positive"),
            });
        }
        Ok(Self {
            symbol,
            quantity,
            price,
        })
    }

    /// Returns the instrument symbol.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::Holding;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// # let holding = Holding::new("AAA", Decimal::ONE, Decimal::ONE)?;
    /// assert_eq!(holding.symbol(), "AAA");
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// Returns the held quantity.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::Holding;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// # let holding = Holding::new("AAA", Decimal::ONE, Decimal::ONE)?;
    /// assert_eq!(holding.quantity(), Decimal::ONE);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn quantity(&self) -> Decimal {
        self.quantity
    }

    /// Returns the latest price.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_portfolio::Holding;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
    /// # let holding = Holding::new("AAA", Decimal::ONE, Decimal::ONE)?;
    /// assert_eq!(holding.price(), Decimal::ONE);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn price(&self) -> Decimal {
        self.price
    }
}

/// The direction of a rebalancing trade.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::RebalanceSide;
///
/// assert_ne!(RebalanceSide::Buy, RebalanceSide::Sell);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebalanceSide {
    /// Increase the position.
    Buy,
    /// Decrease the position.
    Sell,
}

/// One suggested rebalancing trade.
///
/// `quantity` and `estimated_value` are non-negative magnitudes; the direction
/// is given by `side`. `post_trade_deviation` is the absolute difference
/// between the post-trade weight and the target weight for the same symbol,
/// computed after the suggestion is applied.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::RebalanceSide;
///
/// let side = RebalanceSide::Buy;
/// assert_eq!(side, RebalanceSide::Buy);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RebalanceAction {
    /// Instrument to trade.
    pub symbol: String,
    /// Buy or sell direction.
    pub side: RebalanceSide,
    /// Suggested trade size in units of the instrument, as a positive
    /// magnitude.
    pub quantity: Decimal,
    /// Estimated traded notional `quantity * price`, as a positive magnitude.
    pub estimated_value: Decimal,
    /// Absolute post-trade weight deviation from the target, in weight units.
    pub post_trade_deviation: f64,
}

/// Generates the trades that move the current holdings toward `target`.
///
/// # Arguments
///
/// - `total_equity`: total account net liquidation value in the account
///   currency. It is the base for translating weights into target values, so it
///   may differ from the sum of position values when the account also holds
///   cash.
/// - `holdings`: current positions with their latest prices. Every target
///   symbol must appear; symbols may not repeat.
/// - `target`: target portfolio weights. Holdings outside its universe are
///   treated as zero-weight and sold down.
/// - `min_trade_value`: per-order noise threshold in the account currency;
///   strictly smaller trades are suppressed. Must be non-negative.
/// - `allow_fractional`: when `false`, quantities are floored to whole units.
///
/// # Errors
///
/// Returns [`KurumiError::InvalidParameter`] when `total_equity` is not
/// positive, `min_trade_value` is negative, a holding repeats a symbol, a
/// target symbol is missing from `holdings`, a target weight cannot be
/// represented as a [`Decimal`], or `allow_fractional` is `false` and a price
/// is not positive (already guaranteed by [`Holding::new`]). Returns
/// [`KurumiError::NumericalOverflow`] when a Decimal multiplication or division
/// overflows.
///
/// # Examples
///
/// ```
/// use kurumi_portfolio::{Holding, PortfolioWeights, RebalanceSide, generate_rebalance_actions};
/// use kurumi_portfolio::nalgebra::DVector;
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_portfolio::KurumiError> {
/// let holdings = vec![
///     Holding::new("AAA", Decimal::from(600), Decimal::from(100))?,
///     Holding::new("BBB", Decimal::from(400), Decimal::from(100))?,
/// ];
/// let target = PortfolioWeights::new(
///     vec!["AAA".to_owned(), "BBB".to_owned()],
///     DVector::from_vec(vec![0.5, 0.5]),
/// )?;
/// let actions = generate_rebalance_actions(
///     Decimal::from(100_000),
///     &holdings,
///     &target,
///     Decimal::ZERO,
///     true,
/// )?;
/// assert_eq!(actions.len(), 2);
/// assert_eq!(actions[0].side, RebalanceSide::Sell);
/// assert_eq!(actions[1].side, RebalanceSide::Buy);
/// # Ok(())
/// # }
/// ```
pub fn generate_rebalance_actions(
    total_equity: Decimal,
    holdings: &[Holding],
    target: &PortfolioWeights,
    min_trade_value: Decimal,
    allow_fractional: bool,
) -> Result<Vec<RebalanceAction>, KurumiError> {
    if total_equity <= Decimal::ZERO {
        return Err(KurumiError::InvalidParameter {
            parameter: "total_equity",
            reason: "must be strictly positive".to_owned(),
        });
    }
    if min_trade_value < Decimal::ZERO {
        return Err(KurumiError::InvalidParameter {
            parameter: "min_trade_value",
            reason: "must be non-negative".to_owned(),
        });
    }

    let mut positions: BTreeMap<String, &Holding> = BTreeMap::new();
    for holding in holdings {
        if positions
            .insert(holding.symbol().to_owned(), holding)
            .is_some()
        {
            return Err(KurumiError::InvalidParameter {
                parameter: "holdings",
                reason: format!("duplicate symbol `{}`", holding.symbol()),
            });
        }
    }

    let target_symbols = target.symbols();
    let known: BTreeSet<&str> = positions.keys().map(String::as_str).collect();
    for symbol in target_symbols {
        if !known.contains(symbol.as_str()) {
            return Err(KurumiError::InvalidParameter {
                parameter: "holdings",
                reason: format!("target symbol `{symbol}` has no price in the holdings"),
            });
        }
    }

    let mut symbols: Vec<String> = target_symbols.to_vec();
    let mut extra = positions
        .keys()
        .filter(|symbol| !target_symbols.iter().any(|target| target == *symbol))
        .cloned()
        .collect::<Vec<_>>();
    extra.sort();
    symbols.extend(extra);

    let mut actions = Vec::new();
    for symbol in symbols {
        let Some(holding) = positions.get(&symbol) else {
            return Err(index_error(&symbol));
        };
        let weight = if target_symbols.iter().any(|target| target == &symbol) {
            target.weight(&symbol)?
        } else {
            0.0
        };
        let weight_decimal =
            Decimal::try_from(weight).map_err(|_| KurumiError::InvalidParameter {
                parameter: "target",
                reason: format!("weight of `{symbol}` cannot be represented as a Decimal"),
            })?;
        let target_value = total_equity
            .checked_mul(weight_decimal)
            .ok_or_else(overflow)?;
        let current_value = holding
            .quantity()
            .checked_mul(holding.price())
            .ok_or_else(overflow)?;
        let difference = target_value
            .checked_sub(current_value)
            .ok_or_else(overflow)?;
        if difference == Decimal::ZERO {
            continue;
        }
        let side = if difference > Decimal::ZERO {
            RebalanceSide::Buy
        } else {
            RebalanceSide::Sell
        };
        let magnitude = difference.abs();
        let quantity = magnitude
            .checked_div(holding.price())
            .ok_or_else(overflow)?;
        let quantity = if allow_fractional {
            quantity
        } else {
            quantity.floor()
        };
        if quantity <= Decimal::ZERO {
            continue;
        }
        let trade_value = quantity.checked_mul(holding.price()).ok_or_else(overflow)?;
        if trade_value < min_trade_value {
            continue;
        }
        let post_value = match side {
            RebalanceSide::Buy => current_value.checked_add(trade_value),
            RebalanceSide::Sell => current_value.checked_sub(trade_value),
        }
        .ok_or_else(overflow)?;
        let post_weight = post_value.checked_div(total_equity).ok_or_else(overflow)?;
        let post_weight =
            f64::try_from(post_weight).map_err(|_| KurumiError::InvalidParameter {
                parameter: "total_equity",
                reason: "post-trade weight cannot be represented as an f64".to_owned(),
            })?;
        let deviation = (post_weight - weight).abs();
        if !deviation.is_finite() {
            return Err(KurumiError::NumericalOverflow {
                operation: "rebalance_deviation",
            });
        }
        actions.push(RebalanceAction {
            symbol,
            side,
            quantity,
            estimated_value: trade_value,
            post_trade_deviation: deviation,
        });
    }
    Ok(actions)
}

fn overflow() -> KurumiError {
    KurumiError::NumericalOverflow {
        operation: "rebalance_decimal_arithmetic",
    }
}

fn index_error(symbol: &str) -> KurumiError {
    KurumiError::InvalidParameter {
        parameter: "holdings",
        reason: format!("missing holding for `{symbol}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn weights(values: &[f64]) -> PortfolioWeights {
        let symbols = values
            .iter()
            .enumerate()
            .map(|(index, _)| format!("A{index}"))
            .collect::<Vec<_>>();
        PortfolioWeights::new(symbols, nalgebra::DVector::from_vec(values.to_vec()))
            .expect("valid weights")
    }

    #[test]
    fn rebalance_generates_balanced_buy_and_sell_orders() {
        let holdings = vec![
            Holding::new("A0", Decimal::from(600), Decimal::from(100)).expect("holding"),
            Holding::new("A1", Decimal::from(400), Decimal::from(100)).expect("holding"),
        ];
        let target = weights(&[0.5, 0.5]);
        let actions = generate_rebalance_actions(
            Decimal::from(100_000),
            &holdings,
            &target,
            Decimal::ZERO,
            true,
        )
        .expect("rebalance succeeds");
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].symbol, "A0");
        assert_eq!(actions[0].side, RebalanceSide::Sell);
        assert_eq!(actions[0].quantity, Decimal::from(100));
        assert_eq!(actions[0].estimated_value, Decimal::from(10_000));
        assert_relative_eq!(actions[0].post_trade_deviation, 0.0, epsilon = 1e-15);
        assert_eq!(actions[1].side, RebalanceSide::Buy);
        assert_eq!(actions[1].quantity, Decimal::from(100));
    }

    #[test]
    fn min_trade_value_suppresses_noise_orders() {
        let holdings = vec![
            Holding::new("A0", Decimal::from(500), Decimal::from(100)).expect("holding"),
            Holding::new("A1", Decimal::from(499), Decimal::from(100)).expect("holding"),
        ];
        let target = weights(&[0.5, 0.5]);
        let actions = generate_rebalance_actions(
            Decimal::from(100_000),
            &holdings,
            &target,
            Decimal::from(200),
            true,
        )
        .expect("rebalance succeeds");
        // The one-share buy is worth 100, below the 200 noise threshold.
        assert!(actions.is_empty());
    }

    #[test]
    fn whole_units_are_floored() {
        let holdings = vec![
            Holding::new("A0", Decimal::from(250), Decimal::from(100)).expect("holding"),
            Holding::new("A1", Decimal::from(250), Decimal::from(100)).expect("holding"),
        ];
        let target = weights(&[0.5, 0.5]);
        let actions = generate_rebalance_actions(
            Decimal::from(100_000),
            &holdings,
            &target,
            Decimal::ZERO,
            false,
        )
        .expect("rebalance succeeds");
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].quantity, Decimal::from(250));
        assert_eq!(actions[1].quantity, Decimal::from(250));
    }

    #[test]
    fn holdings_outside_the_target_are_sold() {
        let holdings = vec![
            Holding::new("A0", Decimal::from(500), Decimal::from(100)).expect("holding"),
            Holding::new("LEGACY", Decimal::from(10), Decimal::from(100)).expect("holding"),
        ];
        let target = PortfolioWeights::new(
            vec!["A0".to_owned()],
            nalgebra::DVector::from_vec(vec![1.0]),
        )
        .expect("valid weights");
        let actions = generate_rebalance_actions(
            Decimal::from(100_000),
            &holdings,
            &target,
            Decimal::ZERO,
            true,
        )
        .expect("rebalance succeeds");
        let legacy = actions
            .iter()
            .find(|action| action.symbol == "LEGACY")
            .expect("legacy position is sold");
        assert_eq!(legacy.side, RebalanceSide::Sell);
        assert_eq!(legacy.quantity, Decimal::from(10));
    }

    #[test]
    fn missing_target_price_is_rejected() {
        let holdings = vec![Holding::new("A0", Decimal::ONE, Decimal::ONE).expect("holding")];
        let target = weights(&[0.5, 0.5]);
        assert!(
            generate_rebalance_actions(
                Decimal::from(100),
                &holdings,
                &target,
                Decimal::ZERO,
                true,
            )
            .is_err()
        );
    }

    #[test]
    fn invalid_equity_and_threshold_are_rejected() {
        let holdings = vec![];
        let target = weights(&[1.0]);
        assert!(
            generate_rebalance_actions(Decimal::ZERO, &holdings, &target, Decimal::ZERO, true,)
                .is_err()
        );
        assert!(
            generate_rebalance_actions(
                Decimal::from(100),
                &holdings,
                &target,
                -Decimal::ONE,
                true,
            )
            .is_err()
        );
    }
}
