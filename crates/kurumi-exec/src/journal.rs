//! The audit journal.
//!
//! [`AuditJournal`] is the append-only execution record. [`MemoryJournal`] keeps
//! records in memory for tests and analysis; [`JsonlFileJournal`] writes one JSON
//! object per line for durable, stream-friendly audit trails.
//!
//! Journal records cover submissions, risk rejections, execution reports, and
//! equity snapshots. Timestamps are event timestamps in Unix milliseconds and no
//! record contains a wall-clock read.
//!
//! # Examples
//!
//! ```
//! use kurumi_exec::{AuditJournal, ClientOrderId, JournalRecord, MemoryJournal};
//! use chrono::{DateTime, Utc};
//!
//! # fn main() -> Result<(), kurumi_exec::ExecError> {
//! let mut journal = MemoryJournal::default();
//! let ts = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
//! journal.record(&JournalRecord::canceled(ts, &ClientOrderId::new("o-1")))?;
//! assert_eq!(journal.len(), 1);
//! # Ok(())
//! # }
//! ```

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Serialize;

use crate::error::ExecError;
use crate::order::{ClientOrderId, ExecutionReport, OrderRequest};

/// One audit record.
///
/// Decimal values are stored as exact strings and timestamps as Unix
/// milliseconds, keeping the JSON representation stable and precision-safe.
///
/// # Examples
///
/// ```
/// use kurumi_exec::JournalRecord;
/// use chrono::{DateTime, Utc};
/// use rust_decimal::Decimal;
///
/// let ts = DateTime::<Utc>::from_timestamp_millis(1).expect("epoch");
/// let record = JournalRecord::equity(ts, Decimal::ONE, Decimal::ZERO, Decimal::ONE);
/// let json = record.to_json_line();
/// assert!(json.contains("equity_snapshot"));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JournalRecord {
    /// A strategy submitted an order.
    Submit {
        /// Event timestamp in Unix milliseconds.
        ts_millis: i64,
        /// Client order identifier.
        client_order_id: String,
        /// Instrument symbol.
        symbol: String,
        /// Order direction, `buy` or `sell`.
        side: String,
        /// Execution specification name.
        order_type: String,
        /// Limit price when the order has one.
        limit_price: Option<String>,
        /// Stop price when the order has one.
        stop_price: Option<String>,
        /// Ordered quantity.
        quantity: String,
        /// Time in force name.
        tif: String,
        /// Whether the order is reduce-only.
        reduce_only: bool,
        /// Caller tag.
        tag: Option<String>,
    },
    /// The runner accepted an order and reserved its assets.
    Accepted {
        /// Event timestamp in Unix milliseconds.
        ts_millis: i64,
        /// Client order identifier.
        client_order_id: String,
    },
    /// An order was cancelled.
    Canceled {
        /// Event timestamp in Unix milliseconds.
        ts_millis: i64,
        /// Client order identifier.
        client_order_id: String,
    },
    /// A fill was applied to the ledger.
    Fill {
        /// Fill timestamp in Unix milliseconds.
        ts_millis: i64,
        /// Client order identifier.
        client_order_id: String,
        /// Broker execution identifier.
        exec_id: String,
        /// Instrument symbol.
        symbol: String,
        /// Fill direction, `buy` or `sell`.
        side: String,
        /// Execution price.
        fill_price: String,
        /// Filled quantity.
        fill_qty: String,
        /// Fee charged.
        fee: String,
        /// Slippage embedded in the price.
        slippage: String,
    },
    /// A risk rule rejected an order.
    RiskRejected {
        /// Event timestamp in Unix milliseconds.
        ts_millis: i64,
        /// Client order identifier.
        client_order_id: String,
        /// Rule that rejected the order.
        rule: String,
        /// Rejection reason.
        reason: String,
    },
    /// A mark-to-market equity observation.
    EquitySnapshot {
        /// Observation timestamp in Unix milliseconds.
        ts_millis: i64,
        /// Cash balance.
        cash: String,
        /// Cash reserved by open buy orders.
        reserved_cash: String,
        /// Marked-to-market equity.
        equity: String,
    },
}

impl JournalRecord {
    /// Records an order submission.
    #[must_use]
    pub fn submit(ts: DateTime<Utc>, request: &OrderRequest) -> Self {
        Self::Submit {
            ts_millis: ts.timestamp_millis(),
            client_order_id: request.client_order_id.to_string(),
            symbol: request.symbol.clone(),
            side: match request.side {
                crate::OrderSide::Buy => "buy".to_owned(),
                crate::OrderSide::Sell => "sell".to_owned(),
            },
            order_type: request.order_type.name().to_owned(),
            limit_price: request
                .order_type
                .limit_price()
                .map(|price| price.to_string()),
            stop_price: request
                .order_type
                .stop_price()
                .map(|price| price.to_string()),
            quantity: request.quantity.to_string(),
            tif: request.tif.name().to_owned(),
            reduce_only: request.reduce_only,
            tag: request.tag.clone(),
        }
    }

    /// Records an accepted order.
    #[must_use]
    pub fn accepted(ts: DateTime<Utc>, client_order_id: &ClientOrderId) -> Self {
        Self::Accepted {
            ts_millis: ts.timestamp_millis(),
            client_order_id: client_order_id.to_string(),
        }
    }

    /// Records a cancelled order.
    #[must_use]
    pub fn canceled(ts: DateTime<Utc>, client_order_id: &ClientOrderId) -> Self {
        Self::Canceled {
            ts_millis: ts.timestamp_millis(),
            client_order_id: client_order_id.to_string(),
        }
    }

    /// Records a fill execution report.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::JournalError`] when `report` is not a fill.
    pub fn fill(report: &ExecutionReport) -> Result<Self, ExecError> {
        match report {
            ExecutionReport::Fill {
                client_order_id,
                exec_id,
                symbol,
                side,
                fill_price,
                fill_qty,
                fee,
                slippage,
                ts,
            } => Ok(Self::Fill {
                ts_millis: ts.timestamp_millis(),
                client_order_id: client_order_id.to_string(),
                exec_id: exec_id.to_string(),
                symbol: symbol.clone(),
                side: match side {
                    crate::OrderSide::Buy => "buy".to_owned(),
                    crate::OrderSide::Sell => "sell".to_owned(),
                },
                fill_price: fill_price.to_string(),
                fill_qty: fill_qty.to_string(),
                fee: fee.to_string(),
                slippage: slippage.to_string(),
            }),
            _ => Err(ExecError::JournalError {
                message: "only fill reports can be journaled as fills".to_owned(),
            }),
        }
    }

    /// Records a risk rejection.
    #[must_use]
    pub fn risk_rejected(
        ts: DateTime<Utc>,
        client_order_id: &ClientOrderId,
        rule: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::RiskRejected {
            ts_millis: ts.timestamp_millis(),
            client_order_id: client_order_id.to_string(),
            rule: rule.into(),
            reason: reason.into(),
        }
    }

    /// Records a marked-to-market equity observation.
    #[must_use]
    pub fn equity(
        ts: DateTime<Utc>,
        cash: Decimal,
        reserved_cash: Decimal,
        equity: Decimal,
    ) -> Self {
        Self::EquitySnapshot {
            ts_millis: ts.timestamp_millis(),
            cash: cash.to_string(),
            reserved_cash: reserved_cash.to_string(),
            equity: equity.to_string(),
        }
    }

    /// Serializes the record as one compact JSON line without a trailing
    /// newline.
    ///
    /// The writer is hand-rolled so the default Sans-I/O build does not depend
    /// on `serde_json`; the output is the same object shape produced by the
    /// `Serialize` implementation.
    #[must_use]
    pub fn to_json_line(&self) -> String {
        match self {
            Self::Submit {
                ts_millis,
                client_order_id,
                symbol,
                side,
                order_type,
                limit_price,
                stop_price,
                quantity,
                tif,
                reduce_only,
                tag,
            } => format!(
                concat!(
                    r#"{{"kind":"submit","ts_millis":{},"client_order_id":{},"symbol":{},"side":{},"order_type":{},"#,
                    r#""limit_price":{},"stop_price":{},"quantity":{},"tif":{},"reduce_only":{},"tag":{}}}"#
                ),
                ts_millis,
                json_string(client_order_id),
                json_string(symbol),
                json_string(side),
                json_string(order_type),
                json_optional(limit_price.as_deref()),
                json_optional(stop_price.as_deref()),
                json_string(quantity),
                json_string(tif),
                reduce_only,
                json_optional(tag.as_deref()),
            ),
            Self::Accepted {
                ts_millis,
                client_order_id,
            } => format!(
                r#"{{"kind":"accepted","ts_millis":{},"client_order_id":{}}}"#,
                ts_millis,
                json_string(client_order_id),
            ),
            Self::Canceled {
                ts_millis,
                client_order_id,
            } => format!(
                r#"{{"kind":"canceled","ts_millis":{},"client_order_id":{}}}"#,
                ts_millis,
                json_string(client_order_id),
            ),
            Self::Fill {
                ts_millis,
                client_order_id,
                exec_id,
                symbol,
                side,
                fill_price,
                fill_qty,
                fee,
                slippage,
            } => format!(
                concat!(
                    r#"{{"kind":"fill","ts_millis":{},"client_order_id":{},"exec_id":{},"symbol":{},"#,
                    r#""side":{},"fill_price":{},"fill_qty":{},"fee":{},"slippage":{}}}"#
                ),
                ts_millis,
                json_string(client_order_id),
                json_string(exec_id),
                json_string(symbol),
                json_string(side),
                json_string(fill_price),
                json_string(fill_qty),
                json_string(fee),
                json_string(slippage),
            ),
            Self::RiskRejected {
                ts_millis,
                client_order_id,
                rule,
                reason,
            } => format!(
                r#"{{"kind":"risk_rejected","ts_millis":{},"client_order_id":{},"rule":{},"reason":{}}}"#,
                ts_millis,
                json_string(client_order_id),
                json_string(rule),
                json_string(reason),
            ),
            Self::EquitySnapshot {
                ts_millis,
                cash,
                reserved_cash,
                equity,
            } => format!(
                r#"{{"kind":"equity_snapshot","ts_millis":{},"cash":{},"reserved_cash":{},"equity":{}}}"#,
                ts_millis,
                json_string(cash),
                json_string(reserved_cash),
                json_string(equity),
            ),
        }
    }
}

/// An append-only audit sink.
///
/// Implementations must be `Send` so the live runner can move its journal between
/// Tokio workers.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{AuditJournal, ExecError, JournalRecord, MemoryJournal};
///
/// struct Counter;
///
/// impl AuditJournal for Counter {
///     fn record(&mut self, _record: &JournalRecord) -> Result<(), ExecError> {
///         Ok(())
///     }
/// }
///
/// assert!(Counter.record(&JournalRecord::Canceled {
///     ts_millis: 0,
///     client_order_id: "o-1".to_owned(),
/// }).is_ok());
/// ```
pub trait AuditJournal: Send {
    /// Appends one record.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::JournalError`] when the sink cannot persist the
    /// record.
    fn record(&mut self, record: &JournalRecord) -> Result<(), ExecError>;

    /// Flushes buffered records.
    ///
    /// The default implementation does nothing.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::JournalError`] when flushing fails.
    fn flush(&mut self) -> Result<(), ExecError> {
        Ok(())
    }
}

/// An in-memory journal useful for tests and post-run analysis.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{AuditJournal, JournalRecord, MemoryJournal};
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let mut journal = MemoryJournal::default();
/// journal.record(&JournalRecord::Canceled {
///     ts_millis: 0,
///     client_order_id: "o-1".to_owned(),
/// })?;
/// assert_eq!(journal.records().len(), 1);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemoryJournal {
    records: Vec<JournalRecord>,
}

impl MemoryJournal {
    /// Creates an empty journal.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns every record in append order.
    #[must_use]
    pub fn records(&self) -> &[JournalRecord] {
        &self.records
    }

    /// Returns the number of records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Returns `true` when no record has been written.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Removes every record.
    pub fn clear(&mut self) {
        self.records.clear();
    }
}

impl AuditJournal for MemoryJournal {
    fn record(&mut self, record: &JournalRecord) -> Result<(), ExecError> {
        self.records.push(record.clone());
        Ok(())
    }
}

/// A journal that appends one JSON object per line.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{AuditJournal, JournalRecord, JsonlFileJournal};
/// use std::env;
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let path = env::temp_dir().join(format!("kurumi-exec-doc-{}.jsonl", std::process::id()));
/// let mut journal = JsonlFileJournal::create(&path)?;
/// journal.record(&JournalRecord::Canceled {
///     ts_millis: 0,
///     client_order_id: "o-1".to_owned(),
/// })?;
/// journal.flush()?;
/// let contents = std::fs::read_to_string(&path).unwrap_or_default();
/// assert!(contents.contains("o-1"));
/// let _ = std::fs::remove_file(path);
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct JsonlFileJournal {
    writer: BufWriter<File>,
}

impl JsonlFileJournal {
    /// Creates a new journal file, truncating an existing file.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::JournalError`] when the file cannot be created.
    pub fn create(path: impl AsRef<Path>) -> Result<Self, ExecError> {
        let file = File::create(path).map_err(|error| ExecError::JournalError {
            message: error.to_string(),
        })?;
        Ok(Self {
            writer: BufWriter::new(file),
        })
    }

    /// Opens an existing journal for appending.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::JournalError`] when the file cannot be opened.
    pub fn append(path: impl AsRef<Path>) -> Result<Self, ExecError> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| ExecError::JournalError {
                message: error.to_string(),
            })?;
        Ok(Self {
            writer: BufWriter::new(file),
        })
    }
}

impl AuditJournal for JsonlFileJournal {
    fn record(&mut self, record: &JournalRecord) -> Result<(), ExecError> {
        let line = record.to_json_line();
        self.writer
            .write_all(line.as_bytes())
            .and_then(|()| self.writer.write_all(b"\n"))
            .map_err(|error| ExecError::JournalError {
                message: error.to_string(),
            })
    }

    fn flush(&mut self) -> Result<(), ExecError> {
        self.writer
            .flush()
            .map_err(|error| ExecError::JournalError {
                message: error.to_string(),
            })
    }
}

/// Escapes a string as a JSON string literal.
fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                escaped.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}

fn json_optional(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), json_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_journal_keeps_records_in_order() {
        let mut journal = MemoryJournal::new();
        let ts = DateTime::<Utc>::from_timestamp_millis(10).expect("timestamp");
        journal
            .record(&JournalRecord::canceled(ts, &ClientOrderId::new("o-1")))
            .expect("record");
        journal
            .record(&JournalRecord::equity(
                ts,
                Decimal::from(100),
                Decimal::ZERO,
                Decimal::from(100),
            ))
            .expect("record");
        assert_eq!(journal.len(), 2);
        assert!(matches!(
            journal.records().first(),
            Some(JournalRecord::Canceled { .. })
        ));
    }

    #[test]
    fn json_line_escapes_strings() {
        let record = JournalRecord::RiskRejected {
            ts_millis: 1,
            client_order_id: "o-1".to_owned(),
            rule: "Rule".to_owned(),
            reason: "quote \" backslash \\ newline \n".to_owned(),
        };
        let line = record.to_json_line();
        let parsed: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
        assert_eq!(parsed["reason"], "quote \" backslash \\ newline \n");
    }

    #[test]
    fn jsonl_journal_writes_one_object_per_line() {
        let path = std::env::temp_dir().join(format!(
            "kurumi-exec-journal-{}-{}.jsonl",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        {
            let mut journal = JsonlFileJournal::create(&path).expect("create");
            let ts = DateTime::<Utc>::from_timestamp_millis(0).expect("timestamp");
            journal
                .record(&JournalRecord::canceled(ts, &ClientOrderId::new("o-1")))
                .expect("record");
            journal
                .record(&JournalRecord::equity(
                    ts,
                    Decimal::from(10),
                    Decimal::ZERO,
                    Decimal::from(10),
                ))
                .expect("record");
            journal.flush().expect("flush");
        }
        let contents = std::fs::read_to_string(&path).expect("read");
        let lines = contents.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        for line in lines {
            let value: serde_json::Value = serde_json::from_str(line).expect("valid json");
            assert!(value.get("kind").is_some());
        }
        let _ = std::fs::remove_file(path);
    }
}
