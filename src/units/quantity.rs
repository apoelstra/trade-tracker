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

use crate::units::Asset;
use std::{fmt, iter, ops};

/// A tradeable quantity of some object
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Quantity {
    /// A unitless zero
    Zero,
    /// An (signed) amount of bitcoin
    Bitcoin(bitcoin::SignedAmount),
    /// A (signed) number of contracts
    Contracts(i64),
}

impl Quantity {
    /// Constructs a quantity from a number of contracts
    pub fn from_contracts(n: i64) -> Quantity {
        Quantity::Contracts(n)
    }

    /// The absolute value of a quantity
    pub fn abs(&self) -> Quantity {
        match *self {
            Quantity::Bitcoin(btc) => Quantity::Bitcoin(btc.abs()),
            Quantity::Contracts(n) => Quantity::Contracts(n.abs()),
            Quantity::Zero => Quantity::Zero,
        }
    }

    /// Whether this is a nonnegative number
    pub fn is_nonnegative(&self) -> bool {
        match *self {
            Quantity::Bitcoin(btc) => !btc.is_negative(),
            Quantity::Contracts(n) => n >= 0,
            Quantity::Zero => true,
        }
    }

    /// Whether this is a positive number
    pub fn is_positive(&self) -> bool {
        match *self {
            Quantity::Bitcoin(btc) => btc.is_positive(),
            Quantity::Contracts(n) => n > 0,
            Quantity::Zero => false,
        }
    }

    /// Whether this is a positive number
    pub fn is_nonzero(&self) -> bool {
        match *self {
            Quantity::Bitcoin(btc) => btc.to_sat() != 0,
            Quantity::Contracts(n) => n != 0,
            Quantity::Zero => false,
        }
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
    pub fn has_same_unit(&self, other: Quantity) -> bool {
        match (*self, other) {
            (Quantity::Zero, _) | (_, Quantity::Zero) => true,
            (Quantity::Bitcoin(_), Quantity::Bitcoin(_)) => true,
            (Quantity::Contracts(_), Quantity::Contracts(_)) => true,
            _ => false,
        }
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Quantity::Bitcoin(btc) => fmt::Display::fmt(&btc, f),
            Quantity::Contracts(n) => write!(f, "{} contracts", n),
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
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Hash)]
pub struct UnknownQuantity {
    inner: i64,
}

impl UnknownQuantity {
    pub fn set_asset(&self, asset: Asset) -> Quantity {
        match asset {
            Asset::Btc => {
                assert!(self.inner >= 0, "negative quantity of Bitcoins");
                Quantity::Bitcoin(bitcoin::SignedAmount::from_sat(self.inner))
            }
            Asset::Eth => unimplemented!("ethereum quantity"),
            Asset::Usd => panic!("tried to interpret 'quantity' of dollars"),
            Asset::Option { .. } => Quantity::Contracts(self.inner),
        }
    }
}
