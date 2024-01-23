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

//! CSV
//!
//! Basic support for printing data in comma-separated-value format
//!

use crate::units::UtcTime;
use std::fmt;

/// Trait for objects that can be printed in CSV format
pub trait PrintCsv {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result;
}

/// Wrapper around a `PrintCsv` used for println! etc
pub struct CsvPrinter<P: PrintCsv>(pub P);

impl<P: PrintCsv> fmt::Display for CsvPrinter<P> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.print(f)
    }
}

/// Wrapper around a date that will output only the date
#[derive(Copy, Clone)]
pub struct DateOnly(pub UtcTime);
impl PrintCsv for DateOnly {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // It took a ton of experimenting to get a date format that gnumeric
        // will recognize and parse correctly..
        write!(f, "{}", self.0.format("%F"))
    }
}

/// Wrapper around a date that will output both date and time
#[derive(Copy, Clone)]
pub struct DateTime(pub UtcTime);
impl PrintCsv for DateTime {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0.format("%FT%T.%fZ"))
    }
}

/// Wrapper around an implied volatility result
#[derive(Copy, Clone)]
pub struct Iv(pub Result<f64, f64>);
impl PrintCsv for Iv {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Ok(iv) = self.0 {
            write!(f, "{iv}")
        } else {
            f.write_str("\"free money\"")
        }
    }
}

/// Wrapper around an ARR result
#[derive(Copy, Clone)]
pub struct Arr(pub f64);
impl PrintCsv for Arr {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // don't encode ARRs greater than 10000%, it's silly and fucks up the cell width
        if self.0 < 100.0 {
            write!(f, "{}", self.0)?;
        }
        Ok(())
    }
}

impl PrintCsv for crate::units::BudgetAsset {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            crate::units::BudgetAsset::Btc => f.write_str(",BTC,"),
            crate::units::BudgetAsset::Eth => f.write_str(",ETH,"),
            crate::units::BudgetAsset::Usd => f.write_str(",USD,"),
            crate::units::BudgetAsset::Option { underlying, option } => {
                assert_eq!(
                    underlying,
                    crate::units::Underlying::Btc,
                    "non-BTC budget asset ID (do you need to update your spreadsheet?)",
                );
                DateTime(option.expiry).print(f)?;
                write!(f, ",{},{}", option.pc.to_char(), option.strike)
            }
        }
    }
}

impl PrintCsv for crate::units::Quantity {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use bitcoin::util::amount::Denomination::Bitcoin;
        match *self {
            crate::units::Quantity::Bitcoin(btc) => fmt::Display::fmt(&btc.display_in(Bitcoin), f),
            crate::units::Quantity::Cents(n) => write!(f, "{}.{:02}", n / 100, n % 100),
            crate::units::Quantity::Contracts(n) => fmt::Display::fmt(&n, f),
            crate::units::Quantity::Zero => f.write_str("0"),
        }
    }
}

impl PrintCsv for crate::units::TaxAsset {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("\"")?;
        fmt::Display::fmt(self, f)?;
        f.write_str("\"")
    }
}

macro_rules! impl_display {
    ($ty:ty) => {
        impl PrintCsv for $ty {
            fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }
    };
}

impl_display!(usize);
impl_display!(i32);
impl_display!(i64);
impl_display!(u32);
impl_display!(u64);
impl_display!(crate::units::Price);
impl_display!(crate::units::TaxAsset2022);
impl_display!(rust_decimal::Decimal);

macro_rules! impl_string {
    ($ty:ty) => {
        impl PrintCsv for $ty {
            fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
                if self.contains(',') {
                    write!(f, "\"{}\"", self)
                } else {
                    write!(f, "{}", self)
                }
            }
        }
    };
}

impl_string!(String);
impl_string!(&str);
impl_string!(str);

macro_rules! impl_tuple {
    ($($ty:ident $idx:tt)*) => {
        impl<$($ty: PrintCsv,)*> PrintCsv for ($($ty,)*) {
            #[allow(unused_assignments)]
            fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
                let mut comma = false;
                $(
                    if comma {
                        f.write_str(",")?;
                    }
                    self.$idx.print(f)?;
                    comma = true;
                )*
                Ok(())
            }
        }
    }
}

impl_tuple!(A 0);
impl_tuple!(A 0 B 1);
impl_tuple!(A 0 B 1 C 2);
impl_tuple!(A 0 B 1 C 2 D 3);
impl_tuple!(A 0 B 1 C 2 D 3 E 4);
impl_tuple!(A 0 B 1 C 2 D 3 E 4 F 5);
impl_tuple!(A 0 B 1 C 2 D 3 E 4 F 5 G 6);
impl_tuple!(A 0 B 1 C 2 D 3 E 4 F 5 G 6 H 7);
impl_tuple!(A 0 B 1 C 2 D 3 E 4 F 5 G 6 H 7 I 8);
impl_tuple!(A 0 B 1 C 2 D 3 E 4 F 5 G 6 H 7 I 8 J 9);
impl_tuple!(A 0 B 1 C 2 D 3 E 4 F 5 G 6 H 7 I 8 J 9 K 10);
impl_tuple!(A 0 B 1 C 2 D 3 E 4 F 5 G 6 H 7 I 8 J 9 K 10 L 11);
impl_tuple!(A 0 B 1 C 2 D 3 E 4 F 5 G 6 H 7 I 8 J 9 K 10 L 11 M 12);
impl_tuple!(A 0 B 1 C 2 D 3 E 4 F 5 G 6 H 7 I 8 J 9 K 10 L 11 M 12 N 13);

impl<P: PrintCsv> PrintCsv for Option<P> {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Some(p) => p.print(f),
            None => Ok(()), // "write the empty string"
        }
    }
}

impl<'a, P: PrintCsv> PrintCsv for &'a P {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        (*self).print(f)
    }
}
