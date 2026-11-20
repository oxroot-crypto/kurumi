//! Order identifiers, requests, commands, reports, and the order state machine.
//!
//! Every price, quantity, and fee in this module is a
//! [`rust_decimal::Decimal`]. The module is completely synchronous: it has no
//! runtime, clock, or network dependency.
//!
//! [`OrderRecord`] implements the strict finite-state machine
//!
//! ```text
//! PendingSubmit -> Open -> PartiallyFilled -> Filled
//!                     \-> PendingCancel -> Canceled
//!                     \-> Canceled / Rejected / Expired
//! ```
//!
//! and deduplicates fills by [`ExecId`], so replaying the same execution report
//! never applies a fill twice. Timestamps are always supplied by the caller,
//! never read from the wall clock.

use std::collections::BTreeSet;
use std::fmt;

use chrono::{DateTime, Utc};
use kurumi_core::KurumiError;
use rust_decimal::Decimal;

use crate::error::ExecError;

/// A caller-assigned order identifier.
///
/// Strategies generate these deterministically through
/// [`StrategyContext::next_order_id`](crate::StrategyContext::next_order_id), so
/// replaying the same event stream produces the same identifiers.
///
/// # Examples
///
/// ```
/// use kurumi_exec::ClientOrderId;
///
/// let id = ClientOrderId::new("order-0001");
/// assert_eq!(id.as_str(), "order-0001");
/// assert_eq!(id.to_string(), "order-0001");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientOrderId(String);

impl ClientOrderId {
    /// Creates an identifier from any string-like value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the identifier and returns the inner string.
    #[must_use]
    #[inline]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for ClientOrderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<String> for ClientOrderId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ClientOrderId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// A broker-assigned order identifier.
///
/// # Examples
///
/// ```
/// use kurumi_exec::BrokerOrderId;
///
/// assert_eq!(BrokerOrderId::new("b-1").as_str(), "b-1");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BrokerOrderId(String);

impl BrokerOrderId {
    /// Creates an identifier from any string-like value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BrokerOrderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<String> for BrokerOrderId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for BrokerOrderId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// A broker-assigned execution identifier.
///
/// [`OrderRecord::apply_report`] uses this key to make fill application
/// idempotent: a replayed report with a known identifier is ignored.
///
/// # Examples
///
/// ```
/// use kurumi_exec::ExecId;
///
/// assert_eq!(ExecId::new("exec-1").as_str(), "exec-1");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExecId(String);

impl ExecId {
    /// Creates an identifier from any string-like value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExecId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<String> for ExecId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ExecId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// The direction of an order or fill.
///
/// # Examples
///
/// ```
/// use kurumi_exec::OrderSide;
///
/// assert_ne!(OrderSide::Buy, OrderSide::Sell);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OrderSide {
    /// Buy the instrument.
    Buy,
    /// Sell the instrument.
    Sell,
}

impl OrderSide {
    /// Returns the side's sign: `1` for a buy and `-1` for a sell.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::OrderSide;
    ///
    /// assert_eq!(OrderSide::Buy.sign(), 1);
    /// assert_eq!(OrderSide::Sell.sign(), -1);
    /// ```
    #[must_use]
    #[inline]
    pub const fn sign(self) -> i32 {
        match self {
            Self::Buy => 1,
            Self::Sell => -1,
        }
    }

    /// Returns `true` for a buy order.
    #[must_use]
    #[inline]
    pub const fn is_buy(self) -> bool {
        matches!(self, Self::Buy)
    }

    /// Converts the execution side into the backtest side.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::OrderSide;
    ///
    /// assert_eq!(OrderSide::Buy.to_backtest_side(), kurumi_backtest::Side::Buy);
    /// ```
    #[must_use]
    #[inline]
    pub const fn to_backtest_side(self) -> kurumi_backtest::Side {
        match self {
            Self::Buy => kurumi_backtest::Side::Buy,
            Self::Sell => kurumi_backtest::Side::Sell,
        }
    }
}

/// The execution specification of an order.
///
/// Prices are validated by [`OrderRequest::validate`]. This type deliberately
/// keeps plain [`Decimal`] prices so the order can be constructed before a
/// [`Price`](kurumi_core::Price) is available.
///
/// # Examples
///
/// ```
/// use kurumi_exec::OrderType;
/// use rust_decimal::Decimal;
///
/// let limit = OrderType::Limit {
///     price: Decimal::from(100),
/// };
/// assert_eq!(limit.limit_price(), Some(Decimal::from(100)));
/// assert!(limit.is_limit());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderType {
    /// Execute immediately at the best available price.
    Market,
    /// Execute at `price` or better.
    Limit {
        /// The limit price.
        price: Decimal,
    },
    /// Activate a market order once `stop_price` is touched.
    Stop {
        /// The activation price.
        stop_price: Decimal,
    },
    /// Activate a limit order at `limit_price` once `stop_price` is touched.
    StopLimit {
        /// The activation price.
        stop_price: Decimal,
        /// The limit price used after activation.
        limit_price: Decimal,
    },
}

impl OrderType {
    /// Returns the limit price, when the order type has one.
    ///
    /// For [`OrderType::StopLimit`] this is `limit_price`.
    #[must_use]
    #[inline]
    pub const fn limit_price(self) -> Option<Decimal> {
        match self {
            Self::Limit { price } => Some(price),
            Self::StopLimit { limit_price, .. } => Some(limit_price),
            Self::Market | Self::Stop { .. } => None,
        }
    }

    /// Returns the stop activation price, when the order type has one.
    #[must_use]
    #[inline]
    pub const fn stop_price(self) -> Option<Decimal> {
        match self {
            Self::Stop { stop_price } | Self::StopLimit { stop_price, .. } => Some(stop_price),
            Self::Market | Self::Limit { .. } => None,
        }
    }

    /// Returns `true` for a plain limit order.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::OrderType;
    /// use rust_decimal::Decimal;
    ///
    /// assert!(OrderType::Limit { price: Decimal::ONE }.is_limit());
    /// assert!(!OrderType::Market.is_limit());
    /// ```
    #[must_use]
    #[inline]
    pub const fn is_limit(self) -> bool {
        matches!(self, Self::Limit { .. })
    }

    /// Returns `true` for a stop or stop-limit order.
    #[must_use]
    #[inline]
    pub const fn is_stop(self) -> bool {
        matches!(self, Self::Stop { .. } | Self::StopLimit { .. })
    }

    /// Returns a stable, human-readable name used by journals and diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Market => "market",
            Self::Limit { .. } => "limit",
            Self::Stop { .. } => "stop",
            Self::StopLimit { .. } => "stop_limit",
        }
    }
}

/// The order's time in force.
///
/// # Examples
///
/// ```
/// use kurumi_exec::TimeInForce;
///
/// assert!(!TimeInForce::Gtc.is_immediate());
/// assert!(TimeInForce::Ioc.is_immediate());
/// assert!(TimeInForce::Fok.is_immediate());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TimeInForce {
    /// Rest until the end of the trading day.
    #[default]
    Day,
    /// Rest until filled or cancelled.
    Gtc,
    /// Fill immediately, then expire the unfilled remainder.
    Ioc,
    /// Fill completely and immediately, otherwise expire entirely.
    Fok,
}

impl TimeInForce {
    /// Returns `true` when the order must be filled or expired immediately.
    #[must_use]
    #[inline]
    pub const fn is_immediate(self) -> bool {
        matches!(self, Self::Ioc | Self::Fok)
    }

    /// Returns a stable, human-readable name used by journals.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Gtc => "gtc",
            Self::Ioc => "ioc",
            Self::Fok => "fok",
        }
    }
}

/// A request to submit one order.
///
/// All fields are public because strategies build requests directly; call
/// [`OrderRequest::validate`] before submitting one. Quantities are always
/// strictly positive.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{ClientOrderId, OrderRequest, OrderSide, OrderType, TimeInForce};
/// use rust_decimal::Decimal;
///
/// let request = OrderRequest::new(
///     ClientOrderId::new("o-1"),
///     "AAA",
///     OrderSide::Buy,
///     OrderType::Limit { price: Decimal::from(100) },
///     Decimal::from(10),
///     TimeInForce::Day,
/// );
/// assert!(request.validate().is_ok());
/// assert_eq!(request.reference_price(), Some(Decimal::from(100)));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRequest {
    /// Caller-assigned identifier, unique per account.
    pub client_order_id: ClientOrderId,
    /// Instrument symbol.
    pub symbol: String,
    /// Order direction.
    pub side: OrderSide,
    /// Execution specification.
    pub order_type: OrderType,
    /// Positive order quantity.
    pub quantity: Decimal,
    /// Time in force.
    pub tif: TimeInForce,
    /// Whether the order may only reduce existing exposure.
    pub reduce_only: bool,
    /// Optional caller tag carried for reconciliation.
    pub tag: Option<String>,
}

impl OrderRequest {
    /// Builds a request with `reduce_only = false` and no tag.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::{ClientOrderId, OrderRequest, OrderSide, OrderType, TimeInForce};
    /// use rust_decimal::Decimal;
    ///
    /// let request = OrderRequest::new(
    ///     ClientOrderId::new("o-1"),
    ///     "AAA",
    ///     OrderSide::Sell,
    ///     OrderType::Market,
    ///     Decimal::ONE,
    ///     TimeInForce::Day,
    /// );
    /// assert!(!request.reduce_only);
    /// assert!(request.tag.is_none());
    /// ```
    #[must_use]
    pub fn new(
        client_order_id: ClientOrderId,
        symbol: impl Into<String>,
        side: OrderSide,
        order_type: OrderType,
        quantity: Decimal,
        tif: TimeInForce,
    ) -> Self {
        Self {
            client_order_id,
            symbol: symbol.into(),
            side,
            order_type,
            quantity,
            tif,
            reduce_only: false,
            tag: None,
        }
    }

    /// Marks the order as reduce-only.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::{ClientOrderId, OrderRequest, OrderSide, OrderType, TimeInForce};
    /// use rust_decimal::Decimal;
    ///
    /// let request = OrderRequest::new(
    ///     ClientOrderId::new("o-1"),
    ///     "AAA",
    ///     OrderSide::Sell,
    ///     OrderType::Market,
    ///     Decimal::ONE,
    ///     TimeInForce::Day,
    /// ).reduce_only();
    /// assert!(request.reduce_only);
    /// ```
    #[must_use]
    pub fn reduce_only(mut self) -> Self {
        self.reduce_only = true;
        self
    }

    /// Attaches a caller tag.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Validates the structural invariants of the request.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::InvalidOrder`] when the symbol is empty, the
    /// quantity is not strictly positive, or a limit or stop price is not
    /// strictly positive. A stop-limit may place its limit on either side of the
    /// stop; matching simply waits for the limit after activation.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::{ClientOrderId, OrderRequest, OrderSide, OrderType, TimeInForce};
    /// use rust_decimal::Decimal;
    ///
    /// let invalid = OrderRequest::new(
    ///     ClientOrderId::new("o-1"),
    ///     "AAA",
    ///     OrderSide::Buy,
    ///     OrderType::Market,
    ///     Decimal::ZERO,
    ///     TimeInForce::Day,
    /// );
    /// assert!(invalid.validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<(), ExecError> {
        if self.symbol.trim().is_empty() {
            return Err(ExecError::invalid_order("symbol must not be empty"));
        }
        if self.client_order_id.as_str().trim().is_empty() {
            return Err(ExecError::invalid_order(
                "client_order_id must not be empty",
            ));
        }
        if self.quantity <= Decimal::ZERO {
            return Err(ExecError::invalid_order(
                "quantity must be strictly positive",
            ));
        }
        if self
            .order_type
            .limit_price()
            .is_some_and(|price| price <= Decimal::ZERO)
        {
            return Err(ExecError::invalid_order(
                "limit price must be strictly positive",
            ));
        }
        if self
            .order_type
            .stop_price()
            .is_some_and(|price| price <= Decimal::ZERO)
        {
            return Err(ExecError::invalid_order(
                "stop price must be strictly positive",
            ));
        }
        Ok(())
    }

    /// Returns the price implied by the order type, if any.
    ///
    /// A limit or stop-limit order returns its limit price and a stop order
    /// returns its stop price. A market order returns `None`.
    #[must_use]
    #[inline]
    pub fn reference_price(&self) -> Option<Decimal> {
        self.order_type
            .limit_price()
            .or_else(|| self.order_type.stop_price())
    }

    /// Returns `true` when the order has no explicit limit price and is
    /// matched at the prevailing price, either directly or after activation.
    #[must_use]
    #[inline]
    pub fn is_market_like(&self) -> bool {
        matches!(self.order_type, OrderType::Market | OrderType::Stop { .. })
    }
}

/// A command sent from a strategy to the execution runner.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{ClientOrderId, OrderCommand};
///
/// let command = OrderCommand::CancelAll {
///     symbol: Some("AAA".to_owned()),
/// };
/// assert!(matches!(command, OrderCommand::CancelAll { .. }));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderCommand {
    /// Submit a new order.
    Submit(OrderRequest),
    /// Cancel one order by client identifier.
    Cancel {
        /// Identifier of the order to cancel.
        client_order_id: ClientOrderId,
    },
    /// Cancel every open order, optionally restricted to one symbol.
    CancelAll {
        /// Symbol filter; `None` cancels every symbol.
        symbol: Option<String>,
    },
}

/// The lifecycle state of an order.
///
/// # Examples
///
/// ```
/// use kurumi_exec::OrderStatus;
///
/// assert!(!OrderStatus::Open.is_terminal());
/// assert!(OrderStatus::Filled.is_terminal());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OrderStatus {
    /// The order has been reserved locally but not yet accepted.
    PendingSubmit,
    /// The order is live at the broker.
    Open,
    /// The order is live and has received at least one fill.
    PartiallyFilled,
    /// The order is fully filled.
    Filled,
    /// A cancellation has been requested but not yet acknowledged.
    PendingCancel,
    /// The order was cancelled.
    Canceled,
    /// The order was rejected.
    Rejected,
    /// The order expired, for example an unfilled IOC order.
    Expired,
}

impl OrderStatus {
    /// Returns `true` when no further report may transition the order.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::OrderStatus;
    ///
    /// assert!(OrderStatus::Expired.is_terminal());
    /// assert!(!OrderStatus::PendingSubmit.is_terminal());
    /// ```
    #[must_use]
    #[inline]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Filled | Self::Canceled | Self::Rejected | Self::Expired
        )
    }

    /// Returns `true` when the order is live and can still receive fills.
    #[must_use]
    #[inline]
    pub const fn is_live(self) -> bool {
        matches!(
            self,
            Self::Open | Self::PartiallyFilled | Self::PendingCancel
        )
    }

    /// Returns a stable, human-readable name used by journals.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::PendingSubmit => "pending_submit",
            Self::Open => "open",
            Self::PartiallyFilled => "partially_filled",
            Self::Filled => "filled",
            Self::PendingCancel => "pending_cancel",
            Self::Canceled => "canceled",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
        }
    }
}

/// An execution report emitted by a broker or by the paper matching engine.
///
/// Every variant carries the client order identifier and a caller-supplied
/// timestamp. Timestamps are never read from the wall clock.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{ClientOrderId, ExecutionReport};
/// use chrono::{DateTime, Utc};
///
/// let ts = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
/// let report = ExecutionReport::Accepted {
///     client_order_id: ClientOrderId::new("o-1"),
///     ts,
/// };
/// assert_eq!(report.client_order_id().as_str(), "o-1");
/// assert_eq!(report.ts(), ts);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionReport {
    /// The broker accepted the order and it is now open.
    Accepted {
        /// Identifier of the accepted order.
        client_order_id: ClientOrderId,
        /// Report timestamp.
        ts: DateTime<Utc>,
    },
    /// The order received a fill.
    Fill {
        /// Identifier of the filled order.
        client_order_id: ClientOrderId,
        /// Broker execution identifier used for deduplication.
        exec_id: ExecId,
        /// Instrument symbol.
        symbol: String,
        /// Fill direction.
        side: OrderSide,
        /// Execution price.
        fill_price: Decimal,
        /// Filled quantity.
        fill_qty: Decimal,
        /// Fee charged for the fill.
        fee: Decimal,
        /// Slippage cost embedded in the fill.
        slippage: Decimal,
        /// Fill timestamp.
        ts: DateTime<Utc>,
    },
    /// The order was cancelled.
    Canceled {
        /// Identifier of the cancelled order.
        client_order_id: ClientOrderId,
        /// Report timestamp.
        ts: DateTime<Utc>,
    },
    /// The order was rejected.
    Rejected {
        /// Identifier of the rejected order.
        client_order_id: ClientOrderId,
        /// Human-readable rejection reason.
        reason: String,
        /// Report timestamp.
        ts: DateTime<Utc>,
    },
    /// The order expired without filling, for example an IOC order.
    Expired {
        /// Identifier of the expired order.
        client_order_id: ClientOrderId,
        /// Report timestamp.
        ts: DateTime<Utc>,
    },
}

impl ExecutionReport {
    /// Returns the report's client order identifier.
    #[must_use]
    #[inline]
    pub fn client_order_id(&self) -> &ClientOrderId {
        match self {
            Self::Accepted {
                client_order_id, ..
            }
            | Self::Fill {
                client_order_id, ..
            }
            | Self::Canceled {
                client_order_id, ..
            }
            | Self::Rejected {
                client_order_id, ..
            }
            | Self::Expired {
                client_order_id, ..
            } => client_order_id,
        }
    }

    /// Returns the report's timestamp.
    #[must_use]
    #[inline]
    pub fn ts(&self) -> DateTime<Utc> {
        match self {
            Self::Accepted { ts, .. }
            | Self::Fill { ts, .. }
            | Self::Canceled { ts, .. }
            | Self::Rejected { ts, .. }
            | Self::Expired { ts, .. } => *ts,
        }
    }

    /// Returns the execution identifier of a fill.
    #[must_use]
    #[inline]
    pub fn exec_id(&self) -> Option<&ExecId> {
        match self {
            Self::Fill { exec_id, .. } => Some(exec_id),
            _ => None,
        }
    }

    /// Returns the filled quantity of a fill.
    #[must_use]
    #[inline]
    pub fn fill_qty(&self) -> Option<Decimal> {
        match self {
            Self::Fill { fill_qty, .. } => Some(*fill_qty),
            _ => None,
        }
    }
}

/// The canonical record of one order and its state-machine progress.
///
/// The record is the single source of truth for an order's fills. It tracks the
/// cumulative filled quantity, the volume-weighted average fill price, every
/// applied [`ExecId`], and the accrued fees and slippage.
///
/// # Examples
///
/// ```
/// use kurumi_exec::{
///     ClientOrderId, ExecId, ExecutionReport, OrderRecord, OrderRequest, OrderSide, OrderType,
///     OrderStatus, TimeInForce,
/// };
/// use chrono::{DateTime, Utc};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), kurumi_exec::ExecError> {
/// let ts = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
/// let request = OrderRequest::new(
///     ClientOrderId::new("o-1"),
///     "AAA",
///     OrderSide::Buy,
///     OrderType::Market,
///     Decimal::from(10),
///     TimeInForce::Day,
/// );
/// let mut record = OrderRecord::new(request);
/// record.apply_report(&ExecutionReport::Accepted {
///     client_order_id: ClientOrderId::new("o-1"),
///     ts,
/// })?;
/// let fill = ExecutionReport::Fill {
///     client_order_id: ClientOrderId::new("o-1"),
///     exec_id: ExecId::new("e-1"),
///     symbol: "AAA".to_owned(),
///     side: OrderSide::Buy,
///     fill_price: Decimal::from(100),
///     fill_qty: Decimal::from(10),
///     fee: Decimal::ZERO,
///     slippage: Decimal::ZERO,
///     ts,
/// };
/// assert!(record.apply_report(&fill)?);
/// assert_eq!(record.status(), OrderStatus::Filled);
/// assert!(!record.apply_report(&fill)?);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct OrderRecord {
    request: OrderRequest,
    status: OrderStatus,
    filled_qty: Decimal,
    avg_fill_price: Decimal,
    applied_exec_ids: BTreeSet<ExecId>,
    total_fee: Decimal,
    total_slippage: Decimal,
    rejection_reason: Option<String>,
    last_update_ts: Option<DateTime<Utc>>,
}

impl OrderRecord {
    /// Creates a record in [`OrderStatus::PendingSubmit`].
    ///
    /// The request is not validated here; [`ExecutionLedger::reserve_for_order`](crate::ExecutionLedger::reserve_for_order)
    /// validates it before a record is created.
    #[must_use]
    pub fn new(request: OrderRequest) -> Self {
        Self {
            request,
            status: OrderStatus::PendingSubmit,
            filled_qty: Decimal::ZERO,
            avg_fill_price: Decimal::ZERO,
            applied_exec_ids: BTreeSet::new(),
            total_fee: Decimal::ZERO,
            total_slippage: Decimal::ZERO,
            rejection_reason: None,
            last_update_ts: None,
        }
    }

    /// Returns the original order request.
    #[must_use]
    #[inline]
    pub fn request(&self) -> &OrderRequest {
        &self.request
    }

    /// Returns the client order identifier.
    #[must_use]
    #[inline]
    pub fn client_order_id(&self) -> &ClientOrderId {
        &self.request.client_order_id
    }

    /// Returns the instrument symbol.
    #[must_use]
    #[inline]
    pub fn symbol(&self) -> &str {
        &self.request.symbol
    }

    /// Returns the order direction.
    #[must_use]
    #[inline]
    pub fn side(&self) -> OrderSide {
        self.request.side
    }

    /// Returns the execution specification.
    #[must_use]
    #[inline]
    pub fn order_type(&self) -> OrderType {
        self.request.order_type
    }

    /// Returns the requested quantity.
    #[must_use]
    #[inline]
    pub fn quantity(&self) -> Decimal {
        self.request.quantity
    }

    /// Returns the time in force.
    #[must_use]
    #[inline]
    pub fn tif(&self) -> TimeInForce {
        self.request.tif
    }

    /// Returns `true` when the order is reduce-only.
    #[must_use]
    #[inline]
    pub fn reduce_only(&self) -> bool {
        self.request.reduce_only
    }

    /// Returns the current lifecycle state.
    #[must_use]
    #[inline]
    pub fn status(&self) -> OrderStatus {
        self.status
    }

    /// Returns the cumulative filled quantity.
    #[must_use]
    #[inline]
    pub fn filled_qty(&self) -> Decimal {
        self.filled_qty
    }

    /// Returns the volume-weighted average fill price, or zero before the first
    /// fill.
    #[must_use]
    #[inline]
    pub fn avg_fill_price(&self) -> Decimal {
        self.avg_fill_price
    }

    /// Returns the quantity still open.
    #[must_use]
    #[inline]
    pub fn remaining_qty(&self) -> Decimal {
        if self.filled_qty >= self.request.quantity {
            Decimal::ZERO
        } else {
            self.request.quantity - self.filled_qty
        }
    }

    /// Returns every execution identifier applied to the order.
    #[must_use]
    #[inline]
    pub fn applied_exec_ids(&self) -> &BTreeSet<ExecId> {
        &self.applied_exec_ids
    }

    /// Returns the total fee accrued by the order's fills.
    #[must_use]
    #[inline]
    pub fn total_fee(&self) -> Decimal {
        self.total_fee
    }

    /// Returns the total slippage accrued by the order's fills.
    #[must_use]
    #[inline]
    pub fn total_slippage(&self) -> Decimal {
        self.total_slippage
    }

    /// Returns the rejection reason, when the order was rejected.
    #[must_use]
    #[inline]
    pub fn rejection_reason(&self) -> Option<&str> {
        self.rejection_reason.as_deref()
    }

    /// Returns the timestamp of the last applied report.
    #[must_use]
    #[inline]
    pub fn last_update_ts(&self) -> Option<DateTime<Utc>> {
        self.last_update_ts
    }

    /// Requests cancellation and moves the order to
    /// [`OrderStatus::PendingCancel`].
    ///
    /// Returns `true` when the state changed and `false` when the order was
    /// already pending cancellation. A terminal order cannot be cancelled.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::InvalidStateTransition`] for a terminal order.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::{ClientOrderId, OrderRecord, OrderRequest, OrderSide, OrderType, OrderStatus, TimeInForce};
    /// use chrono::{DateTime, Utc};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_exec::ExecError> {
    /// let ts = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
    /// let request = OrderRequest::new(
    ///     ClientOrderId::new("o-1"), "AAA", OrderSide::Buy, OrderType::Market,
    ///     Decimal::ONE, TimeInForce::Day,
    /// );
    /// let mut record = OrderRecord::new(request);
    /// assert!(record.request_cancel(ts)?);
    /// assert_eq!(record.status(), OrderStatus::PendingCancel);
    /// # Ok(())
    /// # }
    /// ```
    pub fn request_cancel(&mut self, ts: DateTime<Utc>) -> Result<bool, ExecError> {
        match self.status {
            OrderStatus::PendingCancel => Ok(false),
            OrderStatus::PendingSubmit | OrderStatus::Open | OrderStatus::PartiallyFilled => {
                self.status = OrderStatus::PendingCancel;
                self.last_update_ts = Some(ts);
                Ok(true)
            }
            _ => Err(ExecError::invalid_transition(format!(
                "cannot cancel order `{}` in state `{}`",
                self.request.client_order_id,
                self.status.name()
            ))),
        }
    }

    /// Applies one execution report to the order.
    ///
    /// Returns `Ok(true)` when the report changed the record and `Ok(false)`
    /// when a fill's [`ExecId`] was already applied, which makes replay
    /// idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::InvalidStateTransition`] when the report does not
    /// belong to this order, when the transition is illegal for the current
    /// status, or when a fill is inconsistent with the order (wrong symbol or
    /// side, non-positive quantity, or more than the remaining quantity).
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_exec::{
    ///     ClientOrderId, ExecId, ExecutionReport, OrderRecord, OrderRequest, OrderSide, OrderType,
    ///     TimeInForce,
    /// };
    /// use chrono::{DateTime, Utc};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_exec::ExecError> {
    /// let ts = DateTime::<Utc>::from_timestamp_millis(0).expect("epoch");
    /// let request = OrderRequest::new(
    ///     ClientOrderId::new("o-1"), "AAA", OrderSide::Buy, OrderType::Market,
    ///     Decimal::from(10), TimeInForce::Day,
    /// );
    /// let mut record = OrderRecord::new(request);
    /// let accepted = ExecutionReport::Accepted {
    ///     client_order_id: ClientOrderId::new("o-1"),
    ///     ts,
    /// };
    /// assert!(record.apply_report(&accepted)?);
    /// assert!(record.apply_report(&accepted).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn apply_report(&mut self, report: &ExecutionReport) -> Result<bool, ExecError> {
        if report.client_order_id() != &self.request.client_order_id {
            return Err(ExecError::invalid_transition(format!(
                "report for `{}` cannot be applied to `{}`",
                report.client_order_id(),
                self.request.client_order_id
            )));
        }
        match report {
            ExecutionReport::Accepted { ts, .. } => {
                if self.status != OrderStatus::PendingSubmit {
                    return Err(ExecError::invalid_transition(format!(
                        "cannot accept order `{}` in state `{}`",
                        self.request.client_order_id,
                        self.status.name()
                    )));
                }
                self.status = OrderStatus::Open;
                self.last_update_ts = Some(*ts);
                Ok(true)
            }
            ExecutionReport::Fill {
                exec_id,
                symbol,
                side,
                fill_price,
                fill_qty,
                fee,
                slippage,
                ts,
                ..
            } => {
                if symbol != self.request.symbol.as_str() {
                    return Err(ExecError::invalid_transition(format!(
                        "fill symbol `{symbol}` does not match order symbol `{}`",
                        self.request.symbol
                    )));
                }
                if side != &self.request.side {
                    return Err(ExecError::invalid_transition(
                        "fill side does not match order side",
                    ));
                }
                if self.applied_exec_ids.contains(exec_id) {
                    return Ok(false);
                }
                if !matches!(
                    self.status,
                    OrderStatus::Open | OrderStatus::PartiallyFilled | OrderStatus::PendingCancel
                ) {
                    return Err(ExecError::invalid_transition(format!(
                        "cannot fill order `{}` in state `{}`",
                        self.request.client_order_id,
                        self.status.name()
                    )));
                }
                if *fill_qty <= Decimal::ZERO {
                    return Err(ExecError::invalid_transition(
                        "fill quantity must be strictly positive",
                    ));
                }
                let remaining = self.remaining_qty();
                if *fill_qty > remaining {
                    return Err(ExecError::invalid_transition(format!(
                        "fill quantity {fill_qty} exceeds remaining quantity {remaining}"
                    )));
                }
                if *fill_price <= Decimal::ZERO {
                    return Err(ExecError::invalid_transition(
                        "fill price must be strictly positive",
                    ));
                }
                if *fee < Decimal::ZERO || *slippage < Decimal::ZERO {
                    return Err(ExecError::invalid_transition(
                        "fee and slippage must be non-negative",
                    ));
                }
                let new_filled = self.filled_qty.checked_add(*fill_qty).ok_or(
                    KurumiError::NumericalOverflow {
                        operation: "OrderRecord::apply_report",
                    },
                )?;
                let weighted_old = self.filled_qty.checked_mul(self.avg_fill_price).ok_or(
                    KurumiError::NumericalOverflow {
                        operation: "OrderRecord::apply_report",
                    },
                )?;
                let weighted_fill =
                    fill_qty
                        .checked_mul(*fill_price)
                        .ok_or(KurumiError::NumericalOverflow {
                            operation: "OrderRecord::apply_report",
                        })?;
                let total_notional = weighted_old.checked_add(weighted_fill).ok_or(
                    KurumiError::NumericalOverflow {
                        operation: "OrderRecord::apply_report",
                    },
                )?;
                self.avg_fill_price = total_notional.checked_div(new_filled).ok_or(
                    KurumiError::NumericalOverflow {
                        operation: "OrderRecord::apply_report",
                    },
                )?;
                self.filled_qty = new_filled;
                self.total_fee =
                    self.total_fee
                        .checked_add(*fee)
                        .ok_or(KurumiError::NumericalOverflow {
                            operation: "OrderRecord::apply_report",
                        })?;
                self.total_slippage = self.total_slippage.checked_add(*slippage).ok_or(
                    KurumiError::NumericalOverflow {
                        operation: "OrderRecord::apply_report",
                    },
                )?;
                self.applied_exec_ids.insert(exec_id.clone());
                self.status = if self.remaining_qty().is_zero() {
                    OrderStatus::Filled
                } else {
                    OrderStatus::PartiallyFilled
                };
                self.last_update_ts = Some(*ts);
                Ok(true)
            }
            ExecutionReport::Canceled { ts, .. } => {
                self.ensure_cancelable("cancel")?;
                self.status = OrderStatus::Canceled;
                self.last_update_ts = Some(*ts);
                Ok(true)
            }
            ExecutionReport::Expired { ts, .. } => {
                self.ensure_cancelable("expire")?;
                self.status = OrderStatus::Expired;
                self.last_update_ts = Some(*ts);
                Ok(true)
            }
            ExecutionReport::Rejected { reason, ts, .. } => {
                self.ensure_cancelable("reject")?;
                self.status = OrderStatus::Rejected;
                self.rejection_reason = Some(reason.clone());
                self.last_update_ts = Some(*ts);
                Ok(true)
            }
        }
    }

    fn ensure_cancelable(&self, action: &str) -> Result<(), ExecError> {
        if self.status.is_terminal() {
            return Err(ExecError::invalid_transition(format!(
                "cannot {action} order `{}` in terminal state `{}`",
                self.request.client_order_id,
                self.status.name()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(millis: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp_millis(millis).expect("valid timestamp")
    }

    fn request(qty: i64) -> OrderRequest {
        OrderRequest::new(
            ClientOrderId::new("o-1"),
            "AAA",
            OrderSide::Buy,
            OrderType::Limit {
                price: Decimal::from(100),
            },
            Decimal::from(qty),
            TimeInForce::Gtc,
        )
    }

    fn accepted() -> ExecutionReport {
        ExecutionReport::Accepted {
            client_order_id: ClientOrderId::new("o-1"),
            ts: ts(0),
        }
    }

    fn fill(exec: &str, qty: i64, price: i64, at: i64) -> ExecutionReport {
        ExecutionReport::Fill {
            client_order_id: ClientOrderId::new("o-1"),
            exec_id: ExecId::new(exec),
            symbol: "AAA".to_owned(),
            side: OrderSide::Buy,
            fill_price: Decimal::from(price),
            fill_qty: Decimal::from(qty),
            fee: Decimal::ONE,
            slippage: Decimal::ZERO,
            ts: ts(at),
        }
    }

    #[test]
    fn pending_order_accepts_and_then_rejects_duplicate_ack() {
        let mut record = OrderRecord::new(request(10));
        assert_eq!(record.status(), OrderStatus::PendingSubmit);
        assert!(record.apply_report(&accepted()).expect("accept"));
        assert_eq!(record.status(), OrderStatus::Open);
        assert!(record.apply_report(&accepted()).is_err());
    }

    #[test]
    fn fills_accumulate_and_duplicate_exec_ids_are_idempotent() {
        let mut record = OrderRecord::new(request(10));
        record.apply_report(&accepted()).expect("accept");
        assert!(
            record
                .apply_report(&fill("e-1", 4, 100, 1))
                .expect("first fill")
        );
        assert_eq!(record.status(), OrderStatus::PartiallyFilled);
        assert_eq!(record.filled_qty(), Decimal::from(4));
        assert!(
            !record
                .apply_report(&fill("e-1", 4, 999, 2))
                .expect("replay")
        );
        assert_eq!(record.filled_qty(), Decimal::from(4));
        assert_eq!(record.avg_fill_price(), Decimal::from(100));
        assert!(
            record
                .apply_report(&fill("e-2", 6, 110, 3))
                .expect("second fill")
        );
        assert_eq!(record.status(), OrderStatus::Filled);
        assert_eq!(record.avg_fill_price(), Decimal::from(106));
        assert!(
            !record
                .apply_report(&fill("e-2", 6, 110, 4))
                .expect("replay")
        );
    }

    #[test]
    fn terminal_order_rejects_new_fills_and_cancels() {
        let mut record = OrderRecord::new(request(10));
        record.apply_report(&accepted()).expect("accept");
        record
            .apply_report(&fill("e-1", 10, 100, 1))
            .expect("full fill");
        assert_eq!(record.status(), OrderStatus::Filled);
        assert!(record.apply_report(&fill("e-2", 1, 100, 2)).is_err());
        assert!(
            record
                .apply_report(&ExecutionReport::Canceled {
                    client_order_id: ClientOrderId::new("o-1"),
                    ts: ts(3),
                })
                .is_err()
        );
        assert!(record.request_cancel(ts(4)).is_err());
    }

    #[test]
    fn stop_limit_may_place_its_limit_on_either_side_of_the_stop() {
        let mut request = OrderRequest::new(
            ClientOrderId::new("sl-1"),
            "AAA",
            OrderSide::Buy,
            OrderType::StopLimit {
                stop_price: Decimal::from(105),
                limit_price: Decimal::from(103),
            },
            Decimal::from(10),
            TimeInForce::Gtc,
        );
        assert!(request.validate().is_ok());
        request.side = OrderSide::Sell;
        assert!(request.validate().is_ok());
    }

    #[test]
    fn overfill_and_wrong_side_are_rejected() {
        let mut record = OrderRecord::new(request(10));
        record.apply_report(&accepted()).expect("accept");
        assert!(record.apply_report(&fill("e-1", 11, 100, 1)).is_err());
        let mut wrong_side = fill("e-2", 1, 100, 2);
        if let ExecutionReport::Fill { side, .. } = &mut wrong_side {
            *side = OrderSide::Sell;
        }
        assert!(record.apply_report(&wrong_side).is_err());
        assert_eq!(record.filled_qty(), Decimal::ZERO);
    }
}
