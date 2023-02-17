// Trade Tracker
// Written in 2021 by
//   Andrew Poelstra <tradetracker@wpsoftware.net>
//
// To the extent possible under law, the author(s) have dedicated all
// copyright and related and neighboring rights to this software to
// the public domain worldwide. This software is distributed without
// any warranty.
//
// You should have received a copy of the CC0 Public Domain Dedication
// along with this software.
// If not, see <http://creativecommons.org/publicdomain/zero/1.0/>.
//

//! Quantities
//!
//! Data structures representing the various fundamental units used throughout
//! the codebase. In general, any use of bare `Decimal` or `u64` is a code
//! smell and should be replaced by one of these.
//!

use crate::units::{Asset, Price, Underlying};
use serde::Deserialize;
use std::{cmp, fmt, iter, ops};

/// A tradeable quantity of some object
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub enum Quantity {
    /// A unitless zero
    Zero,
    /// An (signed) amount of bitcoin
    Bitcoin(bitcoin::SignedAmount),
    /// A (signed) number of US dollars, represented in cents
    Cents(i64),
    /// A (signed) number of contracts
    Contracts(i64),
}

impl Quantity {
    /// Constructs a quantity of contracts by multiplying a number of BTC by 100
    pub fn contracts_from_ratio(available: Price, price_per_100: Price) -> Quantity {
        let n = (100.0 * (available / price_per_100)).floor() as i64;
        Quantity::Contracts(n)
    }

    /// Constructs a quantity of bitcoin by dividing a number of contracts by 100
    pub fn btc_from_contracts(n: i64) -> Quantity {
        Quantity::Bitcoin(bitcoin::SignedAmount::from_sat(n * 1_000_000))
    }

    /// Constructs a quantity of contracts by multiplying a number of BTC by 100
    pub fn contracts_from_btc(btc: bitcoin::Amount) -> Quantity {
        Quantity::Contracts(btc.to_sat() as i64 / 1_000_000)
    }

    /// Constructs a quantity of contracts by multiplying a number of BTC by 100
    pub fn contracts_from_signed_btc(btc: bitcoin::SignedAmount) -> Quantity {
        Quantity::Contracts(btc.to_sat() / 1_000_000)
    }

    /// Constructs a quantity from a number of contracts
    pub fn from_contracts(n: i64) -> Quantity {
        Quantity::Contracts(n)
    }

    /// The absolute value of a quantity
    pub fn abs(&self) -> Quantity {
        match *self {
            Quantity::Bitcoin(btc) => Quantity::Bitcoin(btc.abs()),
            Quantity::Contracts(n) => Quantity::Contracts(n.abs()),
            Quantity::Cents(n) => Quantity::Cents(n.abs()),
            Quantity::Zero => Quantity::Zero,
        }
    }

    /// Returns the number of BTC for a Bitcoin quantity, or the number of contracts / 100
    pub fn btc_equivalent(&self) -> bitcoin::SignedAmount {
        match *self {
            Quantity::Bitcoin(btc) => btc,
            Quantity::Contracts(n) => bitcoin::SignedAmount::from_sat(n * 1_000_000),
            Quantity::Cents(_) => panic!("tried to convert USD to Bitcoin"),
            Quantity::Zero => bitcoin::SignedAmount::ZERO,
        }
    }

    /// Whether this is a nonnegative number
    pub fn is_nonnegative(&self) -> bool {
        match *self {
            Quantity::Bitcoin(btc) => !btc.is_negative(),
            Quantity::Contracts(n) => n >= 0,
            Quantity::Cents(n) => n >= 0,
            Quantity::Zero => true,
        }
    }

    /// Whether this is a positive number
    pub fn is_positive(&self) -> bool {
        match *self {
            Quantity::Bitcoin(btc) => btc.is_positive(),
            Quantity::Contracts(n) => n > 0,
            Quantity::Cents(n) => n > 0,
            Quantity::Zero => false,
        }
    }

    /// Whether this is a nonzero number
    pub fn is_nonzero(&self) -> bool {
        match *self {
            Quantity::Bitcoin(btc) => btc.to_sat() != 0,
            Quantity::Contracts(n) => n != 0,
            Quantity::Cents(n) => n != 0,
            Quantity::Zero => false,
        }
    }

    /// Whether this represents zero
    pub fn is_zero(&self) -> bool {
        !self.is_nonzero()
    }

    /// Whether this has the same sign as some other quantity
    ///
    /// Considers 0 to be the same as either sign.
    pub fn has_same_sign(&self, other: Quantity) -> bool {
        if *self == Quantity::Zero || other == Quantity::Zero {
            true
        } else {
            self.is_nonnegative() == other.is_nonnegative()
        }
    }

    /// Whether this quantity has the same units as some other quantity
    ///
    /// Considers zero to have the same units as any quantity
    #[allow(clippy::match_like_matches_macro)]
    pub fn has_same_unit(&self, other: Quantity) -> bool {
        match (*self, other) {
            (Quantity::Zero, _) | (_, Quantity::Zero) => true,
            (Quantity::Bitcoin(_), Quantity::Bitcoin(_)) => true,
            (Quantity::Contracts(_), Quantity::Contracts(_)) => true,
            (Quantity::Cents(_), Quantity::Cents(_)) => true,
            _ => false,
        }
    }

    /// Whether this quantity has the same units as some other quantity
    ///
    /// # Panics
    ///
    /// Panics if the two quantites differ in units. To check this, call
    /// [Quantity::has_same_unit] before calling this method.
    pub fn min(&self, other: Quantity) -> Quantity {
        match (*self, other) {
            (Quantity::Bitcoin(btc), Quantity::Zero) => {
                if btc.is_positive() {
                    *self
                } else {
                    other
                }
            }
            (Quantity::Zero, Quantity::Bitcoin(btc)) => {
                if btc.is_positive() {
                    other
                } else {
                    *self
                }
            }
            (Quantity::Contracts(n), Quantity::Zero) => {
                if n >= 0 {
                    *self
                } else {
                    other
                }
            }
            (Quantity::Zero, Quantity::Contracts(n)) => {
                if n < 0 {
                    *self
                } else {
                    other
                }
            }
            (Quantity::Bitcoin(btc), Quantity::Bitcoin(other)) => {
                Quantity::Bitcoin(cmp::min(btc, other))
            }
            (Quantity::Contracts(n), Quantity::Contracts(other)) => {
                Quantity::Contracts(cmp::min(n, other))
            }
            _ => panic!("Cannot take minimum of {} and {}", self, other),
        }
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Quantity::Bitcoin(btc) => fmt::Display::fmt(&btc, f),
            Quantity::Contracts(n) => {
                fmt::Display::fmt(&n, f)?;
                f.write_str(" cts")
            }
            Quantity::Cents(n) => {
                f.write_str("$")?;
                fmt::Display::fmt(&(n / 100), f)?;
                write!(f, ".{:02}", n % 100)
            }
            Quantity::Zero => fmt::Display::fmt("ZERO", f),
        }
    }
}

impl From<bitcoin::SignedAmount> for Quantity {
    fn from(amt: bitcoin::SignedAmount) -> Quantity {
        Quantity::Bitcoin(amt)
    }
}

impl From<bitcoin::Amount> for Quantity {
    fn from(amt: bitcoin::Amount) -> Quantity {
        Quantity::Bitcoin(amt.to_signed().expect("can this overflow even happen"))
    }
}

impl cmp::PartialOrd for Quantity {
    fn partial_cmp(&self, other: &Quantity) -> Option<cmp::Ordering> {
        match (self, other) {
            (Quantity::Bitcoin(amt), Quantity::Bitcoin(other)) => amt.partial_cmp(other),
            (Quantity::Contracts(n), Quantity::Contracts(other)) => n.partial_cmp(other),
            (Quantity::Cents(n), Quantity::Cents(other)) => n.partial_cmp(other),
            (Quantity::Bitcoin(amt), Quantity::Zero) => amt.to_sat().partial_cmp(&0),
            (Quantity::Zero, Quantity::Bitcoin(amt)) => 0.partial_cmp(&amt.to_sat()),
            (Quantity::Contracts(n), Quantity::Zero) => n.partial_cmp(&0),
            (Quantity::Zero, Quantity::Contracts(n)) => 0.partial_cmp(n),
            (Quantity::Cents(n), Quantity::Zero) => n.partial_cmp(&0),
            (Quantity::Zero, Quantity::Cents(n)) => 0.partial_cmp(n),
            _ => None,
        }
    }
}

impl ops::Neg for Quantity {
    type Output = Quantity;
    fn neg(self) -> Quantity {
        match self {
            Quantity::Zero => Quantity::Zero,
            Quantity::Bitcoin(btc) => Quantity::Bitcoin(
                // should PR upstream to add Neg for SignedAmount..
                bitcoin::SignedAmount::from_sat(-btc.to_sat()),
            ),
            Quantity::Contracts(n) => Quantity::Contracts(-n),
            Quantity::Cents(n) => Quantity::Cents(-n),
        }
    }
}

impl ops::Add for Quantity {
    type Output = Quantity;
    fn add(self, other: Quantity) -> Quantity {
        if self == Quantity::Zero {
            other
        } else {
            match (self, other) {
                (Quantity::Bitcoin(amt), Quantity::Bitcoin(other)) => {
                    Quantity::Bitcoin(amt + other)
                }
                (Quantity::Contracts(n), Quantity::Contracts(other)) => {
                    Quantity::Contracts(n + other)
                }
                (Quantity::Cents(n), Quantity::Cents(other)) => Quantity::Cents(n + other),
                _ => panic!("Cannot add {} to {}", other, self),
            }
        }
    }
}

impl ops::Sub for Quantity {
    type Output = Quantity;
    fn sub(self, other: Quantity) -> Quantity {
        self + -other
    }
}

impl ops::AddAssign for Quantity {
    fn add_assign(&mut self, other: Quantity) {
        *self = *self + other;
    }
}

impl ops::SubAssign for Quantity {
    fn sub_assign(&mut self, other: Quantity) {
        *self = *self - other;
    }
}

impl iter::Sum for Quantity {
    fn sum<I>(iter: I) -> Self
    where
        I: Iterator<Item = Self>,
    {
        iter.fold(Quantity::Zero, |acc, n| acc + n)
    }
}

/// A tradeable quantity whose units are not yet determined
///
/// There is not much you can do with this object without first determining
/// its units; but you can deserialize and serialize it
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Hash, Deserialize)]
#[serde(from = "i64")]
pub struct UnknownQuantity {
    inner: i64,
}

impl From<i64> for UnknownQuantity {
    fn from(n: i64) -> UnknownQuantity {
        UnknownQuantity { inner: n }
    }
}

impl UnknownQuantity {
    /// Consruct a unknown quantity from an integer number of base units
    pub fn from_i64(n: i64) -> Self {
        From::from(n)
    }

    /// Define the quantity based on a given asset
    pub fn with_asset(&self, asset: Asset) -> Quantity {
        match asset {
            Asset::Btc => Quantity::Bitcoin(bitcoin::SignedAmount::from_sat(self.inner)),
            Asset::NextDay { underlying, .. } => match underlying {
                Underlying::Btc => Quantity::Bitcoin(bitcoin::SignedAmount::from_sat(self.inner)),
                Underlying::Eth => unimplemented!("ethereum nextday quantity"),
            },
            Asset::Eth => unimplemented!("ethereum quantity"),
            Asset::Usd => Quantity::Cents(self.inner),
            Asset::Option { .. } => Quantity::Contracts(self.inner),
            Asset::Future { .. } => Quantity::Contracts(self.inner),
        }
    }

    /// Define the quantity based on a given asset, using 1/100th-of-a-coin size base units
    /// rather than satoshis
    pub fn with_asset_trade(&self, asset: Asset) -> Quantity {
        match asset {
            Asset::Btc => {
                Quantity::Bitcoin(bitcoin::SignedAmount::from_sat(self.inner * 1_000_000))
            }
            Asset::NextDay { underlying, .. } => match underlying {
                Underlying::Btc => {
                    Quantity::Bitcoin(bitcoin::SignedAmount::from_sat(self.inner * 1_000_000))
                }
                Underlying::Eth => unimplemented!("ethereum nextday quantity"),
            },
            Asset::Eth => unimplemented!("ethereum quantity"),
            Asset::Usd => Quantity::Cents(self.inner),
            Asset::Option { .. } => Quantity::Contracts(self.inner),
            Asset::Future { .. } => Quantity::Contracts(self.inner),
        }
    }

    /// Interpret the number as Bitcoins, in satoshis.
    pub fn as_sats(&self) -> bitcoin::SignedAmount {
        bitcoin::SignedAmount::from_sat(self.inner)
    }
}

impl ops::Add for UnknownQuantity {
    type Output = UnknownQuantity;
    fn add(self, other: Self) -> Self {
        UnknownQuantity {
            inner: self.inner + other.inner,
        }
    }
}

impl ops::Sub for UnknownQuantity {
    type Output = UnknownQuantity;
    fn sub(self, other: Self) -> Self {
        UnknownQuantity {
            inner: self.inner - other.inner,
        }
    }
}

impl ops::Neg for UnknownQuantity {
    type Output = Self;
    fn neg(self) -> Self {
        UnknownQuantity { inner: -self.inner }
    }
}
