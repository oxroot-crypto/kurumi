//! Monetary amounts, currencies, and validated scalar newtypes.
//!
//! Discrete monetary values use [`rust_decimal::Decimal`] so ledger arithmetic
//! never round-trips through binary floating point. Continuous model
//! parameters use `f64` wrappers whose constructors reject `NaN` and infinite
//! values.

use crate::error::KurumiError;
use rust_decimal::Decimal;
use std::fmt;
use std::ops::{Add, Sub};

/// A currency supported by Kurumi.
///
/// The enum covers the common ISO 4217 fiat currencies plus the major crypto
/// asset symbols. `Currency` is deliberately a closed enum: implicit currency
/// conversion is never performed, and the variants this library understands are
/// part of its public contract.
///
/// # Examples
///
/// ```
/// use kurumi_core::Currency;
///
/// assert_eq!(Currency::Usd.code(), "USD");
/// assert_eq!(Currency::Eur.to_string(), "EUR");
/// assert!(Currency::Btc.is_crypto());
/// assert!(!Currency::Jpy.is_crypto());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Currency {
    /// United States dollar (ISO 4217 `USD`).
    Usd,
    /// Euro (ISO 4217 `EUR`).
    Eur,
    /// Pound sterling (ISO 4217 `GBP`).
    Gbp,
    /// Japanese yen (ISO 4217 `JPY`).
    Jpy,
    /// Swiss franc (ISO 4217 `CHF`).
    Chf,
    /// Chinese yuan renminbi (ISO 4217 `CNY`).
    Cny,
    /// Hong Kong dollar (ISO 4217 `HKD`).
    Hkd,
    /// Australian dollar (ISO 4217 `AUD`).
    Aud,
    /// Canadian dollar (ISO 4217 `CAD`).
    Cad,
    /// Singapore dollar (ISO 4217 `SGD`).
    Sgd,
    /// South Korean won (ISO 4217 `KRW`).
    Krw,
    /// Indian rupee (ISO 4217 `INR`).
    Inr,
    /// Brazilian real (ISO 4217 `BRL`).
    Brl,
    /// Mexican peso (ISO 4217 `MXN`).
    Mxn,
    /// South African rand (ISO 4217 `ZAR`).
    Zar,
    /// Swedish krona (ISO 4217 `SEK`).
    Sek,
    /// Norwegian krone (ISO 4217 `NOK`).
    Nok,
    /// Danish krone (ISO 4217 `DKK`).
    Dkk,
    /// New Zealand dollar (ISO 4217 `NZD`).
    Nzd,
    /// Bitcoin (`BTC`).
    Btc,
    /// Ether (`ETH`).
    Eth,
    /// Tether (`USDT`).
    Usdt,
    /// USD Coin (`USDC`).
    Usdc,
    /// BNB (`BNB`).
    Bnb,
    /// Solana (`SOL`).
    Sol,
    /// XRP (`XRP`).
    Xrp,
    /// Cardano (`ADA`).
    Ada,
    /// Dogecoin (`DOGE`).
    Doge,
}

impl Currency {
    /// Returns the ticker symbol of this currency.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Currency;
    /// assert_eq!(Currency::Gbp.code(), "GBP");
    /// ```
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Usd => "USD",
            Self::Eur => "EUR",
            Self::Gbp => "GBP",
            Self::Jpy => "JPY",
            Self::Chf => "CHF",
            Self::Cny => "CNY",
            Self::Hkd => "HKD",
            Self::Aud => "AUD",
            Self::Cad => "CAD",
            Self::Sgd => "SGD",
            Self::Krw => "KRW",
            Self::Inr => "INR",
            Self::Brl => "BRL",
            Self::Mxn => "MXN",
            Self::Zar => "ZAR",
            Self::Sek => "SEK",
            Self::Nok => "NOK",
            Self::Dkk => "DKK",
            Self::Nzd => "NZD",
            Self::Btc => "BTC",
            Self::Eth => "ETH",
            Self::Usdt => "USDT",
            Self::Usdc => "USDC",
            Self::Bnb => "BNB",
            Self::Sol => "SOL",
            Self::Xrp => "XRP",
            Self::Ada => "ADA",
            Self::Doge => "DOGE",
        }
    }

    /// Returns `true` when this currency is a crypto asset symbol.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Currency;
    /// assert!(Currency::Eth.is_crypto());
    /// assert!(!Currency::Chf.is_crypto());
    /// ```
    #[must_use]
    pub const fn is_crypto(self) -> bool {
        matches!(
            self,
            Self::Btc
                | Self::Eth
                | Self::Usdt
                | Self::Usdc
                | Self::Bnb
                | Self::Sol
                | Self::Xrp
                | Self::Ada
                | Self::Doge
        )
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// A monetary amount in a specific currency.
///
/// `Money` pairs an exact [`Decimal`] amount with a [`Currency`]. Addition and
/// subtraction are checked against both currency mismatch and decimal overflow:
/// mixing currencies returns [`KurumiError::CurrencyMismatch`] and never performs
/// an implicit conversion.
///
/// # Examples
///
/// ```
/// use kurumi_core::{Currency, KurumiError, Money};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), KurumiError> {
/// let a = Money::new(Decimal::new(1050, 2), Currency::Usd);
/// let b = Money::new(Decimal::new(250, 2), Currency::Usd);
/// assert_eq!(a.checked_add(b)?.amount, Decimal::new(1300, 2));
///
/// let c = Money::new(Decimal::ONE, Currency::Eur);
/// assert!(matches!(
///     a.checked_add(c),
///     Err(KurumiError::CurrencyMismatch { .. })
/// ));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Money {
    /// The exact decimal amount, expressed in `currency`'s major unit.
    pub amount: Decimal,
    /// The currency in which `amount` is denominated.
    pub currency: Currency,
}

impl Money {
    /// Creates an amount in the given currency.
    ///
    /// This constructor cannot fail: every [`Decimal`] is a valid amount. Use
    /// the checked arithmetic methods for operations that can overflow or mix
    /// currencies.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Currency, Money};
    /// use rust_decimal::Decimal;
    ///
    /// let cash = Money::new(Decimal::new(9950, 2), Currency::Usd);
    /// assert_eq!(cash.currency, Currency::Usd);
    /// ```
    #[must_use]
    pub const fn new(amount: Decimal, currency: Currency) -> Self {
        Self { amount, currency }
    }

    /// Creates a zero amount in the given currency.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Currency, Money};
    ///
    /// assert!(Money::zero(Currency::Jpy).is_zero());
    /// ```
    #[must_use]
    pub const fn zero(currency: Currency) -> Self {
        Self {
            amount: Decimal::ZERO,
            currency,
        }
    }

    /// Returns `true` when the amount is exactly zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Currency, Money};
    ///
    /// assert!(Money::zero(Currency::Eur).is_zero());
    /// ```
    #[must_use]
    pub fn is_zero(self) -> bool {
        self.amount == Decimal::ZERO
    }

    /// Adds two amounts of the same currency.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::CurrencyMismatch`] if the currencies differ and
    /// [`KurumiError::NumericalOverflow`] if the decimal sum is not
    /// representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Currency, KurumiError, Money};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), KurumiError> {
    /// let total = Money::new(Decimal::new(150, 2), Currency::Eur)
    ///     .checked_add(Money::new(Decimal::new(50, 2), Currency::Eur))?;
    /// assert_eq!(total.amount, Decimal::new(200, 2));
    /// # Ok(())
    /// # }
    /// ```
    pub fn checked_add(self, other: Self) -> Result<Self, KurumiError> {
        if self.currency != other.currency {
            return Err(KurumiError::CurrencyMismatch {
                expected: self.currency,
                actual: other.currency,
            });
        }
        let amount =
            self.amount
                .checked_add(other.amount)
                .ok_or(KurumiError::NumericalOverflow {
                    operation: "Money::checked_add",
                })?;
        Ok(Self {
            amount,
            currency: self.currency,
        })
    }

    /// Subtracts one amount from another of the same currency.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::CurrencyMismatch`] if the currencies differ and
    /// [`KurumiError::NumericalOverflow`] if the decimal difference is not
    /// representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Currency, KurumiError, Money};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), KurumiError> {
    /// let change = Money::new(Decimal::new(200, 2), Currency::Usd)
    ///     .checked_sub(Money::new(Decimal::new(50, 2), Currency::Usd))?;
    /// assert_eq!(change.amount, Decimal::new(150, 2));
    /// # Ok(())
    /// # }
    /// ```
    pub fn checked_sub(self, other: Self) -> Result<Self, KurumiError> {
        if self.currency != other.currency {
            return Err(KurumiError::CurrencyMismatch {
                expected: self.currency,
                actual: other.currency,
            });
        }
        let amount =
            self.amount
                .checked_sub(other.amount)
                .ok_or(KurumiError::NumericalOverflow {
                    operation: "Money::checked_sub",
                })?;
        Ok(Self {
            amount,
            currency: self.currency,
        })
    }

    /// Returns the additive inverse of this amount.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::NumericalOverflow`] when the negated amount is not
    /// representable.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Currency, KurumiError, Money};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), KurumiError> {
    /// let loss = Money::new(Decimal::new(25, 2), Currency::Usd).checked_neg()?;
    /// assert_eq!(loss.amount, Decimal::new(-25, 2));
    /// # Ok(())
    /// # }
    /// ```
    pub fn checked_neg(self) -> Result<Self, KurumiError> {
        let amount =
            Decimal::ZERO
                .checked_sub(self.amount)
                .ok_or(KurumiError::NumericalOverflow {
                    operation: "Money::checked_neg",
                })?;
        Ok(Self {
            amount,
            currency: self.currency,
        })
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.amount, self.currency)
    }
}

impl Add for Money {
    type Output = Result<Self, KurumiError>;

    /// Adds two amounts, returning a [`Result`] so currency mismatches and
    /// decimal overflow are explicit.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::{Currency, KurumiError, Money};
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), KurumiError> {
    /// let total = (Money::new(Decimal::ONE, Currency::Usd)
    ///     + Money::new(Decimal::ONE, Currency::Usd))?;
    /// assert_eq!(total.amount, Decimal::from(2));
    /// # Ok(())
    /// # }
    /// ```
    fn add(self, rhs: Self) -> Self::Output {
        self.checked_add(rhs)
    }
}

impl Sub for Money {
    type Output = Result<Self, KurumiError>;

    /// Subtracts two amounts, returning a [`Result`] so currency mismatches and
    /// decimal overflow are explicit.
    fn sub(self, rhs: Self) -> Self::Output {
        self.checked_sub(rhs)
    }
}

/// A validated, strictly positive price.
///
/// Prices are exact decimal values. The inner value is private so every `Price`
/// is guaranteed to be greater than zero, which is the precondition most
/// pricing formulas need.
///
/// # Examples
///
/// ```
/// use kurumi_core::{KurumiError, Price};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), KurumiError> {
/// let price = Price::new(Decimal::new(10125, 2))?;
/// assert_eq!(price.value(), Decimal::new(10125, 2));
/// assert!(Price::new(Decimal::ZERO).is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Price(Decimal);

impl Price {
    /// Creates a price, requiring it to be strictly positive.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `amount` is zero or
    /// negative.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Price;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// let price = Price::new(Decimal::new(1999, 2))?;
    /// assert!(price.value() > Decimal::ZERO);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(amount: Decimal) -> Result<Self, KurumiError> {
        if amount <= Decimal::ZERO {
            return Err(KurumiError::InvalidParameter {
                parameter: "price",
                reason: "must be strictly positive".to_owned(),
            });
        }
        Ok(Self(amount))
    }

    /// Returns the exact decimal value.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Price;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert_eq!(Price::new(Decimal::ONE)?.value(), Decimal::ONE);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn value(self) -> Decimal {
        self.0
    }

    /// Consumes the wrapper and returns the exact decimal value.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Price;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert_eq!(Price::new(Decimal::ONE)?.into_inner(), Decimal::ONE);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn into_inner(self) -> Decimal {
        self.0
    }
}

impl TryFrom<Decimal> for Price {
    type Error = KurumiError;

    fn try_from(value: Decimal) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Price> for Decimal {
    fn from(value: Price) -> Self {
        value.0
    }
}

impl fmt::Display for Price {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A validated, non-negative quantity.
///
/// Quantities are exact decimal values. The inner value is private so every
/// `Quantity` is guaranteed to be non-negative; short positions are represented
/// by a side or a signed position elsewhere, not by a negative `Quantity`.
///
/// # Examples
///
/// ```
/// use kurumi_core::{KurumiError, Quantity};
/// use rust_decimal::Decimal;
///
/// # fn main() -> Result<(), KurumiError> {
/// let units = Quantity::new(Decimal::new(15, 1))?;
/// assert_eq!(units.value(), Decimal::new(15, 1));
/// assert!(Quantity::new(Decimal::new(-1, 0)).is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Quantity(Decimal);

impl Quantity {
    /// Creates a quantity, requiring it to be non-negative.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `amount` is negative.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Quantity;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert!(Quantity::new(Decimal::ZERO)?.value().is_zero());
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(amount: Decimal) -> Result<Self, KurumiError> {
        if amount < Decimal::ZERO {
            return Err(KurumiError::InvalidParameter {
                parameter: "quantity",
                reason: "must be non-negative".to_owned(),
            });
        }
        Ok(Self(amount))
    }

    /// Returns the exact decimal value.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Quantity;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert_eq!(Quantity::new(Decimal::new(5, 0))?.value(), Decimal::new(5, 0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn value(self) -> Decimal {
        self.0
    }

    /// Consumes the wrapper and returns the exact decimal value.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Quantity;
    /// use rust_decimal::Decimal;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert_eq!(Quantity::new(Decimal::ONE)?.into_inner(), Decimal::ONE);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn into_inner(self) -> Decimal {
        self.0
    }
}

impl TryFrom<Decimal> for Quantity {
    type Error = KurumiError;

    fn try_from(value: Decimal) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Quantity> for Decimal {
    fn from(value: Quantity) -> Self {
        value.0
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A validated continuous rate.
///
/// Rates are dimensionless `f64` values: a rate of `0.05` represents 5%. The
/// constructor rejects `NaN` and infinite values. Negative rates are permitted
/// because several markets quote them.
///
/// # Examples
///
/// ```
/// use kurumi_core::{KurumiError, Rate};
///
/// # fn main() -> Result<(), KurumiError> {
/// let rate = Rate::new(0.035)?;
/// assert!((rate.value() - 0.035).abs() < f64::EPSILON);
/// assert!(Rate::new(f64::NAN).is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Rate(f64);

impl Rate {
    /// Creates a rate, requiring a finite value.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `value` is `NaN` or
    /// infinite.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Rate;
    ///
    /// assert!(Rate::new(-0.0025).is_ok());
    /// assert!(Rate::new(f64::INFINITY).is_err());
    /// ```
    pub fn new(value: f64) -> Result<Self, KurumiError> {
        if !value.is_finite() {
            return Err(KurumiError::InvalidParameter {
                parameter: "rate",
                reason: "must be finite".to_owned(),
            });
        }
        Ok(Self(value))
    }

    /// Returns the rate as an `f64`.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Rate;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert!((Rate::new(0.05)?.value() - 0.05).abs() < f64::EPSILON);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn value(self) -> f64 {
        self.0
    }

    /// Consumes the wrapper and returns the `f64` value.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Rate;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert_eq!(Rate::new(1.0)?.into_inner(), 1.0);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn into_inner(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for Rate {
    type Error = KurumiError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Rate> for f64 {
    fn from(value: Rate) -> Self {
        value.0
    }
}

impl fmt::Display for Rate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A validated, non-negative volatility.
///
/// Volatilities are dimensionless `f64` values: a volatility of `0.2`
/// represents 20% annualized standard deviation. The constructor rejects `NaN`,
/// infinite, and negative values; zero volatility is allowed for degenerate
/// deterministic models.
///
/// # Examples
///
/// ```
/// use kurumi_core::{KurumiError, Volatility};
///
/// # fn main() -> Result<(), KurumiError> {
/// let vol = Volatility::new(0.2)?;
/// assert!((vol.value() - 0.2).abs() < f64::EPSILON);
/// assert!(Volatility::new(-0.1).is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Volatility(f64);

impl Volatility {
    /// Creates a volatility, requiring a finite, non-negative value.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `value` is `NaN`,
    /// infinite, or negative.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Volatility;
    ///
    /// assert!(Volatility::new(0.0).is_ok());
    /// assert!(Volatility::new(-0.01).is_err());
    /// assert!(Volatility::new(f64::NAN).is_err());
    /// ```
    pub fn new(value: f64) -> Result<Self, KurumiError> {
        if !value.is_finite() || value < 0.0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "volatility",
                reason: "must be finite and non-negative".to_owned(),
            });
        }
        Ok(Self(value))
    }

    /// Returns the volatility as an `f64`.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Volatility;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert!((Volatility::new(0.2)?.value() - 0.2).abs() < f64::EPSILON);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn value(self) -> f64 {
        self.0
    }

    /// Consumes the wrapper and returns the `f64` value.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_core::Volatility;
    ///
    /// # fn main() -> Result<(), kurumi_core::KurumiError> {
    /// assert_eq!(Volatility::new(1.0)?.into_inner(), 1.0);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn into_inner(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for Volatility {
    type Error = KurumiError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Volatility> for f64 {
    fn from(value: Volatility) -> Self {
        value.0
    }
}

impl fmt::Display for Volatility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(value: i64, scale: u32) -> Decimal {
        Decimal::new(value, scale)
    }

    #[test]
    fn currency_codes_round_trip_through_display() {
        for currency in [
            Currency::Usd,
            Currency::Eur,
            Currency::Gbp,
            Currency::Jpy,
            Currency::Chf,
            Currency::Cny,
            Currency::Btc,
            Currency::Eth,
            Currency::Usdt,
        ] {
            assert_eq!(currency.to_string(), currency.code());
        }
    }

    #[test]
    fn crypto_classification_matches_the_asset_class() {
        assert!(Currency::Btc.is_crypto());
        assert!(Currency::Doge.is_crypto());
        assert!(!Currency::Usd.is_crypto());
        assert!(!Currency::Zar.is_crypto());
    }

    #[test]
    fn same_currency_addition_is_exact() {
        let left = Money::new(dec(1050, 2), Currency::Usd);
        let right = Money::new(dec(250, 2), Currency::Usd);
        let sum = left.checked_add(right).expect("same-currency addition");
        assert_eq!(sum, Money::new(dec(1300, 2), Currency::Usd));
    }

    #[test]
    fn same_currency_subtraction_is_exact_and_may_go_negative() {
        let left = Money::new(dec(250, 2), Currency::Usd);
        let right = Money::new(dec(1050, 2), Currency::Usd);
        let difference = left.checked_sub(right).expect("same-currency subtraction");
        assert_eq!(difference.amount, dec(-800, 2));
    }

    #[test]
    fn cross_currency_addition_returns_currency_mismatch() {
        let usd = Money::new(Decimal::ONE, Currency::Usd);
        let eur = Money::new(Decimal::ONE, Currency::Eur);
        assert!(matches!(
            usd.checked_add(eur),
            Err(KurumiError::CurrencyMismatch {
                expected: Currency::Usd,
                actual: Currency::Eur,
            })
        ));
    }

    #[test]
    fn cross_currency_subtraction_returns_currency_mismatch() {
        let btc = Money::new(Decimal::ONE, Currency::Btc);
        let usdt = Money::new(Decimal::ONE, Currency::Usdt);
        assert!(matches!(
            btc.checked_sub(usdt),
            Err(KurumiError::CurrencyMismatch { .. })
        ));
    }

    #[test]
    fn money_operator_overloads_return_results() {
        let total = (Money::new(Decimal::ONE, Currency::Usd)
            + Money::new(Decimal::ONE, Currency::Usd))
        .expect("operator addition");
        assert_eq!(total.amount, Decimal::from(2));

        let difference = (Money::new(Decimal::ONE, Currency::Usd)
            - Money::new(Decimal::ONE, Currency::Usd))
        .expect("operator subtraction");
        assert!(difference.is_zero());
    }

    #[test]
    fn money_negation_flips_the_sign() {
        let negative = Money::new(dec(25, 2), Currency::Usd)
            .checked_neg()
            .expect("negation");
        assert_eq!(negative.amount, dec(-25, 2));
        assert_eq!(negative.currency, Currency::Usd);
    }

    #[test]
    fn money_overflow_is_reported_as_a_typed_error() {
        let max = Money::new(Decimal::MAX, Currency::Usd);
        let one = Money::new(Decimal::ONE, Currency::Usd);
        assert!(matches!(
            max.checked_add(one),
            Err(KurumiError::NumericalOverflow { .. })
        ));
    }

    #[test]
    fn price_accepts_positive_values_and_rejects_non_positive_values() {
        assert!(Price::new(dec(1, 2)).is_ok());
        assert!(Price::new(Decimal::ZERO).is_err());
        assert!(Price::new(dec(-1, 2)).is_err());
    }

    #[test]
    fn quantity_accepts_zero_and_rejects_negative_values() {
        assert!(Quantity::new(Decimal::ZERO).is_ok());
        assert!(Quantity::new(dec(1, 1)).is_ok());
        assert!(Quantity::new(dec(-1, 1)).is_err());
    }

    #[test]
    fn rate_rejects_non_finite_values_but_accepts_negative_rates() {
        assert!(Rate::new(-0.0025).is_ok());
        assert!(Rate::new(f64::NAN).is_err());
        assert!(Rate::new(f64::INFINITY).is_err());
        assert!(Rate::new(f64::NEG_INFINITY).is_err());
    }

    #[test]
    fn volatility_rejects_negative_and_non_finite_values() {
        assert!(Volatility::new(0.0).is_ok());
        assert!(Volatility::new(0.25).is_ok());
        assert!(Volatility::new(-0.25).is_err());
        assert!(Volatility::new(f64::NAN).is_err());
        assert!(Volatility::new(f64::INFINITY).is_err());
    }

    #[test]
    fn checked_constructors_accept_decimal_newtypes() {
        let price: Price = dec(125, 2).try_into().expect("valid price");
        let quantity: Quantity = dec(3, 0).try_into().expect("valid quantity");
        assert_eq!(price.value(), dec(125, 2));
        assert_eq!(quantity.value(), dec(3, 0));
        assert_eq!(Decimal::from(price), dec(125, 2));
        assert_eq!(Decimal::from(quantity), dec(3, 0));
    }

    #[test]
    fn display_formats_include_the_currency() {
        let money = Money::new(dec(1234, 2), Currency::Usd);
        assert_eq!(money.to_string(), "12.34 USD");
        assert_eq!(
            Price::new(dec(1234, 2)).expect("price").to_string(),
            "12.34"
        );
        assert_eq!(Quantity::new(dec(5, 0)).expect("quantity").to_string(), "5");
    }
}
