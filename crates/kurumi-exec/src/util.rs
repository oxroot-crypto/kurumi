//! Internal conversion helpers shared by the execution modules.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::error::ExecError;

/// Converts an `f64` market value into an exact `Decimal` at 12-decimal
/// precision, rejecting non-finite and non-positive values.
pub(crate) fn decimal_from_f64(value: f64, symbol: &str) -> Result<Decimal, ExecError> {
    let decimal = Decimal::from_f64_retain(value).ok_or_else(|| {
        ExecError::strategy(format!(
            "price of `{symbol}` is not representable as a Decimal"
        ))
    })?;
    let rounded = decimal.round_dp(12);
    if rounded <= Decimal::ZERO {
        return Err(ExecError::strategy(format!(
            "price of `{symbol}` is not strictly positive at 12-decimal precision"
        )));
    }
    Ok(rounded)
}

/// Converts a `Decimal` into an `f64`, falling back to zero for values outside
/// the `f64` range. Financial decimals are far below that range in practice.
pub(crate) fn f64_from_decimal(value: Decimal) -> f64 {
    value.to_f64().unwrap_or(0.0)
}
