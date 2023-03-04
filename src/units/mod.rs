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

//! Units
//!
//! Data structures representing the various fundamental units used throughout
//! the codebase. In general, any use of bare `Decimal` or `u64` is a code
//! smell and should be replaced by one of these.
//!

mod asset;
mod price;
mod quantity;

pub use asset::{Asset, BudgetAsset, DepositAsset, TaxAsset, TaxAsset2022, Underlying};
pub use price::{
    deserialize_cents, deserialize_cents_opt, deserialize_dollars, serialize_dollars, Price,
};
pub use quantity::{Quantity, UnknownQuantity};

macro_rules! impl_ops_0 {
    ($outer:ty, $op:ident, $opfn:ident) => {
        impl std::ops::$op<$outer> for $outer {
            type Output = Self;
            fn $opfn(self, other: $outer) -> Self {
                From::from(std::ops::$op::$opfn(self.0, other.0))
            }
        }

        impl<'a> std::ops::$op<&'a $outer> for $outer {
            type Output = Self;
            fn $opfn(self, other: &'a $outer) -> Self {
                From::from(std::ops::$op::$opfn(self.0, other.0))
            }
        }

        impl<'a> std::ops::$op<$outer> for &'a $outer {
            type Output = $outer;
            fn $opfn(self, other: $outer) -> $outer {
                From::from(std::ops::$op::$opfn(self.0, other.0))
            }
        }

        impl<'a, 'b> std::ops::$op<&'a $outer> for &'b $outer {
            type Output = $outer;
            fn $opfn(self, other: &'a $outer) -> $outer {
                From::from(std::ops::$op::$opfn(self.0, other.0))
            }
        }
    };
}
use impl_ops_0; // exported to submodules

macro_rules! impl_assign_ops_0 {
    ($outer:ty, $op:ident, $opfn:ident) => {
        impl std::ops::$op<$outer> for $outer {
            fn $opfn(&mut self, other: $outer) {
                std::ops::$op::$opfn(&mut self.0, other.0)
            }
        }

        impl<'a> std::ops::$op<&'a $outer> for $outer {
            fn $opfn(&mut self, other: &'a $outer) {
                std::ops::$op::$opfn(&mut self.0, other.0)
            }
        }
    };
}
use impl_assign_ops_0; // exported to submodules
