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

//! Price
//!
//! Prices in US dollars
//!

use super::Quantity;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{convert::TryFrom, fmt, ops, str};

/// A price, in US dollars
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Hash)]
pub struct Price(Decimal);

impl Price {
    /// Zero dollars
    pub const ZERO: Self = Price(Decimal::ZERO);
    /// $1
    pub const ONE: Self = Price(Decimal::ONE);
    /// $25
    pub const TWENTY_FIVE: Self = Price(Decimal::from_parts(25, 0, 0, false, 0));
    /// $100
    pub const ONE_HUNDRED: Self = Price(Decimal::ONE_HUNDRED);
    /// $1000
    pub const ONE_THOUSAND: Self = Price(Decimal::ONE_THOUSAND);

    /// Converts the price to a floating-point value
    ///
    /// Some prices cannot be represented exactly (e.g. $0.10) in a binary
    /// representation such as IEEE floats. So this method will introduce
    /// a tiny error factor, which may be amplified by further computations.
    pub fn to_approx_f64(&self) -> f64 {
        self.0.to_f64().unwrap()
    }

    /// Converts a floating-point value to a price
    ///
    /// If the conversion cannot be done, substitutes 0. This function is really
    /// meant for display/informational purposes only and should not be used for
    /// accounting.
    pub fn from_approx_f64_or_zero(p: f64) -> Price {
        Price(Decimal::try_from(p).unwrap_or_default())
    }

    /// Multiplies the price by a given scaling factor
    ///
    /// Because this uses floating-point numbers it will not give an exact
    /// result. Extreme caution should be used whenever using this in an
    /// accounting context.
    pub fn scale_approx(&self, scale: f64) -> Price {
        Price(self.0 * Decimal::try_from(scale).expect("scaling by a finite float"))
    }

    /// Absolute value of the price
    pub fn abs(&self) -> Price {
        Price(self.0.abs())
    }

    /// Given a price, return 1/100 the same price
    pub fn one_hundredth(&self) -> Price {
        Price(self.0 / Decimal::ONE_HUNDRED)
    }

    /// Given a price, double it
    pub fn double(&self) -> Price {
        Price(self.0 * Decimal::from(2))
    }

    /// Given a price, halve it
    pub fn half(&self) -> Price {
        Price(self.0 / Decimal::from(2))
    }

    /// Given a price, return 40% of the price (used for 1256 tax calculations)
    pub fn forty(&self) -> Price {
        Price(self.0 * Decimal::from(2) / Decimal::from(5))
    }

    /// Given a price, return 60% of the price (used for 1256 tax calculations)
    pub fn sixty(&self) -> Price {
        Price(self.0 * Decimal::from(3) / Decimal::from(5))
    }

    /// Convert the value to an integer, truncating any fractional part
    pub fn to_int(&self) -> i64 {
        self.0.to_i64().unwrap()
    }
}

impl From<Decimal> for Price {
    fn from(d: Decimal) -> Price {
        Price(d)
    }
}

impl str::FromStr for Price {
    type Err = rust_decimal::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Parse the LX-style "1,123.00" strings in their CSV
        let compressed: String = s
            .chars()
            .filter(|c| *c != '"' && *c != ',' && *c != '$')
            .collect();
        str::FromStr::from_str(&compressed).map(Price)
    }
}

impl fmt::Display for Price {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Alternate display adds a 000s separator
        if f.alternate() {
            if self.0 < Decimal::ZERO {
                f.write_str("-")?;
            }
            let val = self.0.abs();
            if val > Decimal::new(1_000_000_000, 0) {
                unimplemented!("have not written display logic for billion-dollar amounts")
            } else {
                let (trunc, fract) = (
                    val.trunc().to_i64().unwrap(),
                    (val.fract() * Decimal::ONE_HUNDRED).to_i64().unwrap(),
                );
                if trunc >= 1_000_000 {
                    write!(f, "{},", trunc / 1_000_000)?;
                    write!(f, "{:03},", (trunc / 1_000) % 1_000)?;
                    write!(f, "{:03}.{:02}", trunc % 1_000, fract)
                } else if trunc >= 1_000 {
                    write!(f, "{},", trunc / 1_000)?;
                    write!(f, "{:03}.{:02}", trunc % 1_000, fract)
                } else {
                    write!(f, "{}.{:02}", trunc % 1_000, fract)
                }
            }
        } else {
            let mut copy = self.0.round_dp(2);
            copy.rescale(2);
            fmt::Display::fmt(&copy, f)
        }
    }
}

super::impl_ops_0!(Price, Add, add);
super::impl_ops_0!(Price, Sub, sub);
super::impl_assign_ops_0!(Price, AddAssign, add_assign);
super::impl_assign_ops_0!(Price, SubAssign, sub_assign);

impl ops::Neg for Price {
    type Output = Self;
    fn neg(self) -> Self {
        Price(-self.0)
    }
}

// Dividing two prices gets you a unitless floating-point ratio
impl ops::Div<Price> for Price {
    type Output = f64;
    fn div(self, other: Price) -> f64 {
        if other.0 == Decimal::ZERO {
            panic!("Tried to divide price {} by zero", self);
        }
        (self.0 / other.0).to_f64().unwrap()
    }
}

// Multiplying or dividing a price by a quantity gets you another price
impl ops::Mul<Quantity> for Price {
    type Output = Price;
    fn mul(self, other: Quantity) -> Price {
        match other {
            Quantity::Bitcoin(btc) => Price(self.0 * Decimal::new(btc.to_sat(), 8)),
            Quantity::Contracts(n) => Price(self.0 * Decimal::new(n, 2)),
            Quantity::Cents(_) => panic!(
                "Tried to multiply price {} by dollar-quantity {}",
                self, other
            ),
            Quantity::Zero => Price::ZERO,
        }
    }
}

impl ops::Div<Quantity> for Price {
    type Output = Price;
    fn div(self, other: Quantity) -> Price {
        assert!(
            other.is_nonzero(),
            "Trying to divide a price {} by a zero quantity",
            self,
        );
        match other {
            Quantity::Bitcoin(btc) => Price(self.0 / Decimal::new(btc.to_sat(), 8)),
            Quantity::Contracts(n) => Price(self.0 / Decimal::new(n, 2)),
            Quantity::Cents(_) => panic!(
                "Tried to divide price {} by dollar-quantity {}",
                self, other
            ),
            Quantity::Zero => unreachable!(),
        }
    }
}

/// Construct a price from a decimal expression, e.g. price!(100.00) or price!(123)
#[macro_export]
macro_rules! price {
    ($num:expr) => {
        $num.to_string().parse::<$crate::units::Price>().unwrap()
    };
}

/// Serialize a price via serde in dollars
pub fn serialize_dollars<S>(obj: &Price, ser: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    Serialize::serialize(&obj.0, ser)
}

/// Deserialize a price via serde in dollars
pub fn deserialize_dollars<'de, D>(deser: D) -> Result<Price, D::Error>
where
    D: Deserializer<'de>,
{
    let dollars: Decimal = Deserialize::deserialize(deser)?;
    Ok(Price(dollars))
}

/// Deserialize a price via serde which is given as in integer number of pennies
pub fn deserialize_cents<'de, D>(deser: D) -> Result<Price, D::Error>
where
    D: Deserializer<'de>,
{
    let cents: i64 = Deserialize::deserialize(deser)?;
    Ok(Price(Decimal::new(cents, 2)))
}

/// Deserialize a price via serde which is given as in integer number of pennies
pub fn deserialize_cents_opt<'de, D>(deser: D) -> Result<Option<Price>, D::Error>
where
    D: Deserializer<'de>,
{
    let cents: Option<i64> = Deserialize::deserialize(deser)?;
    Ok(cents.map(|cents| Price(Decimal::new(cents, 2))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_from_str() {
        assert_eq!("123".parse(), Ok(Price(Decimal::new(123, 0))));
        assert_eq!("123.45".parse(), Ok(Price(Decimal::new(12345, 2))));
        assert_eq!("$1000".parse::<Price>(), Ok(Price(Decimal::new(1000, 0))));
        assert_eq!("$1,000".parse::<Price>(), Ok(Price(Decimal::new(1000, 0))));
        assert_eq!("1,000".parse::<Price>(), Ok(Price(Decimal::new(1000, 0))));
        assert!("123xy".parse::<Price>().is_err());
    }

    #[test]
    fn price_display() {
        assert_eq!(format!("{}", price!(123)), "123.00");
        assert_eq!(format!("{}", price!(123.4)), "123.40");
        assert_eq!(format!("{}", price!(123.04)), "123.04");
        assert_eq!(format!("{}", price!(123.45)), "123.45");

        assert_eq!(format!("{}", price!(123456789)), "123456789.00");
        assert_eq!(format!("{:#}", price!(123456789)), "123,456,789.00");
        assert_eq!(format!("{:#}", price!(1234567.89)), "1,234,567.89");
        assert_eq!(format!("{:#}", price!(34567.09)), "34,567.09");
    }
}
