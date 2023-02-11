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

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::{fmt, str};

/// A price, in US dollars
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Hash)]
pub struct Price(Decimal);

impl From<Decimal> for Price {
    fn from(d: Decimal) -> Price {
        Price(d)
    }
}

impl str::FromStr for Price {
    type Err = rust_decimal::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        str::FromStr::from_str(s).map(Price)
    }
}

impl fmt::Display for Price {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Alternate display prepends the $, adds a 000s separator, etc
        if f.alternate() {
            f.write_str("$")?;
            if self.0 > Decimal::new(1_000_000_000, 0) {
                unimplemented!("have not written display logic for billion-dollar amounts")
            } else {
                let (trunc, fract) = (
                    self.0.trunc().to_u64().unwrap(),
                    (self.0.fract() * Decimal::ONE_HUNDRED).to_u64().unwrap(),
                );
                if trunc >= 1_000_000 {
                    write!(f, "{},", trunc / 1_000_000)?;
                }
                if trunc >= 1_000 {
                    write!(f, "{},", (trunc / 1_000) % 1_000)?;
                }
                write!(f, "{}.{:02}", trunc % 1_000, fract)
            }
        } else {
            let mut copy = self.0.round_dp(2);
            copy.rescale(2);
            fmt::Display::fmt(&copy, f)
        }
    }
}

/// Construct a price from a decimal expression, e.g. price!(100.00) or price!(123)
#[macro_export]
macro_rules! price {
    ($num:expr) => {
        $num.to_string().parse::<Price>().unwrap()
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_from_str() {
        assert_eq!("123".parse(), Ok(Price(Decimal::new(123, 0))));
        assert_eq!("123.45".parse(), Ok(Price(Decimal::new(12345, 2))));
        assert!("123xy".parse::<Price>().is_err());
        assert!("$1000".parse::<Price>().is_err());
        assert!("$1,000".parse::<Price>().is_err());
        assert!("1,000".parse::<Price>().is_err());
    }

    #[test]
    fn price_display() {
        assert_eq!(format!("{}", price!(123)), "123.00");
        assert_eq!(format!("{}", price!(123.4)), "123.40");
        assert_eq!(format!("{}", price!(123.04)), "123.04");
        assert_eq!(format!("{}", price!(123.45)), "123.45");

        assert_eq!(format!("{}", price!(123456789)), "123456789.00");
        assert_eq!(format!("{:#}", price!(123456789)), "$123,456,789.00");
        assert_eq!(format!("{:#}", price!(1234567.89)), "$1,234,567.89");
        assert_eq!(format!("{:#}", price!(34567.09)), "$34,567.09");
    }
}
