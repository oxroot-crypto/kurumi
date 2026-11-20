//! Internal JSON field helpers shared by the Nasdaq adapters.
//!
//! The helpers keep exchange prices on the decimal path: a JSON number is
//! converted through its retained textual representation (the workspace enables
//! `serde_json`'s `arbitrary_precision` feature), never through `f64`.

use std::str::FromStr;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

use crate::error::FeedError;

/// Builds a protocol error attributed to `provider`.
pub(crate) fn protocol_error(provider: &str, reason: impl Into<String>) -> FeedError {
    FeedError::ProtocolError {
        provider: provider.to_owned(),
        reason: reason.into(),
    }
}

/// Returns the value stored under `name`, or `None` when it is absent or null.
pub(crate) fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    value.get(name).filter(|item| !item.is_null())
}

/// Reads a required string field.
pub(crate) fn required_string(
    value: &Value,
    name: &str,
    provider: &str,
) -> Result<String, FeedError> {
    match field(value, name) {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(protocol_error(
            provider,
            format!("field `{name}` must be a JSON string"),
        )),
        None => Err(protocol_error(
            provider,
            format!("missing required field `{name}`"),
        )),
    }
}

/// Reads an optional string field.
pub(crate) fn optional_string(value: &Value, name: &str) -> Option<String> {
    match field(value, name) {
        Some(Value::String(text)) => Some(text.clone()),
        _ => None,
    }
}

/// Reads a required decimal field without routing through `f64`.
///
/// JSON strings and numbers are both accepted; numbers are converted through
/// their retained literal representation.
pub(crate) fn required_decimal(
    value: &Value,
    name: &str,
    provider: &str,
) -> Result<Decimal, FeedError> {
    match field(value, name) {
        Some(Value::String(text)) => parse_decimal(text, name, provider),
        Some(Value::Number(number)) => parse_decimal(&number.to_string(), name, provider),
        Some(_) => Err(protocol_error(
            provider,
            format!("field `{name}` must be a decimal string or number"),
        )),
        None => Err(protocol_error(
            provider,
            format!("missing required field `{name}`"),
        )),
    }
}

/// Reads an optional decimal field.
pub(crate) fn optional_decimal(
    value: &Value,
    name: &str,
    provider: &str,
) -> Result<Option<Decimal>, FeedError> {
    if field(value, name).is_none() {
        return Ok(None);
    }
    required_decimal(value, name, provider).map(Some)
}

/// Reads a required unsigned integer field.
pub(crate) fn required_u64(value: &Value, name: &str, provider: &str) -> Result<u64, FeedError> {
    match field(value, name) {
        Some(Value::Number(number)) => number.as_u64().ok_or_else(|| {
            protocol_error(
                provider,
                format!("field `{name}` must be an unsigned integer"),
            )
        }),
        Some(Value::String(text)) => text.parse::<u64>().map_err(|_| {
            protocol_error(
                provider,
                format!("field `{name}` must be an unsigned integer"),
            )
        }),
        Some(_) => Err(protocol_error(
            provider,
            format!("field `{name}` must be an unsigned integer"),
        )),
        None => Err(protocol_error(
            provider,
            format!("missing required field `{name}`"),
        )),
    }
}

/// Reads a required RFC 3339 timestamp.
pub(crate) fn required_rfc3339(
    value: &Value,
    name: &str,
    provider: &str,
) -> Result<DateTime<Utc>, FeedError> {
    let text = required_string(value, name, provider)?;
    DateTime::parse_from_rfc3339(&text)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|error| {
            protocol_error(
                provider,
                format!("field `{name}` is not an RFC 3339 timestamp: {error}"),
            )
        })
}

/// Converts a string or JSON number to a `Decimal`.
fn parse_decimal(text: &str, name: &str, provider: &str) -> Result<Decimal, FeedError> {
    Decimal::from_str(text).map_err(|error| {
        protocol_error(
            provider,
            format!("field `{name}` is not a valid decimal: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_keep_their_decimal_representation() {
        let value = json!({"price": 187.5123});
        let price = required_decimal(&value, "price", "test").expect("valid decimal");
        assert_eq!(price, Decimal::new(1_875_123, 4));
    }

    #[test]
    fn strings_and_integers_are_accepted() {
        let value = json!({"price": "187.5123", "size": 10});
        assert_eq!(
            required_decimal(&value, "price", "test").expect("valid decimal"),
            Decimal::new(1_875_123, 4)
        );
        assert_eq!(
            required_decimal(&value, "size", "test").expect("valid decimal"),
            Decimal::new(10, 0)
        );
    }

    #[test]
    fn missing_fields_are_protocol_errors() {
        let value = json!({});
        assert!(matches!(
            required_decimal(&value, "price", "test"),
            Err(FeedError::ProtocolError { .. })
        ));
    }
}
