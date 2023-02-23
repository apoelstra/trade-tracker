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
//! Data structures for parsing and using the end-of-year CSV files that LX provides
//! as supporting documentation for its 1099s. In particular, using these documents,
//! we can extract LX's price references so that we can precisely reproduce their
//! shit.
//!
//! The relevant price references are specific to assignment/expiry dates, which do
//! not change even when we re-order lots or do other shenanigans. The relevant
//! dates will always be 4PM and 5PM on the days that options expired and were
//! assigned.
//!

use crate::units::Price;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::str::FromStr;
use time::OffsetDateTime;

pub fn price_ref(s: &str) -> Result<Option<(OffsetDateTime, Price)>, String> {
    let fields: Vec<_> = CsvIter::new(s, ',').collect();
    if fields.len() != 10 {
        return Err(format!(
            "Not sure how to parse CSV line with {} fields (expected 10)",
            fields.len()
        ));
    }

    // Distinguish by first field: in 2022 this is a numeric user ID, in 2021 it's text
    if fields[0].chars().all(char::is_numeric) {
        price_ref_2022(&fields)
    } else {
        price_ref_2021(&fields)
    }
}

/// Helper function that drops the first and last character of a string (expected to be "s)
fn trim1(s: &str) -> &str {
    if s.len() < 2 {
        s
    } else {
        &s[1..s.len() - 1]
    }
}

/// Helper function for parsing the 2021 CSV file
fn price_ref_2021(fields: &[&str]) -> Result<Option<(OffsetDateTime, Price)>, String> {
    if fields[0] != "Exercised" {
        return Ok(None);
    }

    // We have to parse the quantity, put/call and strike out of the description string
    let desc_fields: Vec<_> = CsvIter::new(trim1(fields[1]), ' ').collect();
    if desc_fields.len() != 6 {
        return Err(format!(
            "Description {} has {} fields, cannot parse.",
            fields[1],
            desc_fields.len()
        ));
    }
    // Parse the quantity, skipping the and final ,
    let qty = Decimal::from_str(&desc_fields[0][..desc_fields[0].len() - 1])
        .map_err(|e| format!("parsing quantity {}: {e}", desc_fields[0]))?;
    let qty_64 = qty.to_f64().unwrap();
    // Parse the strike; price parsing logic can already handle the $s and ,s and shit
    let strike = Price::from_str(desc_fields[5])
        .map_err(|e| format!("parsing price {}: {e}", desc_fields[5]))?;

    // Parse the date/basis of the exercise
    let ex_date = OffsetDateTime::parse(fields[2], time::Format::Rfc3339)
        .map_err(|e| format!("parsing exercise date {}: {e}", fields[2]))?;
    let ex_basis =
        Price::from_str(fields[5]).map_err(|e| format!("parsing exercise {}: {e}", fields[5]))?;

    // Figure out the BTC price reference
    let btc_price = if fields[1].contains("Call") {
        strike + ex_basis.scale_approx(100.0 / qty_64)
    } else {
        strike - ex_basis.scale_approx(100.0 / qty_64)
    };

    Ok(Some((ex_date, btc_price)))
}

/// Helper function for parsing the 2022 CSV file
fn price_ref_2022(fields: &[&str]) -> Result<Option<(OffsetDateTime, Price)>, String> {
    // Only option exercises provide (a) price information (b) at a date that
    // we need a price reference at. (Expiries have no price info and trades
    // either happened at a market price or at the same time as some exercise.)
    if !fields[1].contains("Exercise - 1256 Option -") {
        return Ok(None);
    }

    // If this is a BTC trade we don't care to extract a reference from it.
    if fields[3] == "BTC" {
        return Ok(None);
    }

    // Strip "s and ,s from the quantity string
    let quantity_s: String = fields[2]
        .chars()
        .filter(|&c| c != ',' && c != '"')
        .collect();
    let qty = Decimal::from_str(&quantity_s)
        .map_err(|e| format!("parsing quantity {quantity_s}: {e}"))?;
    let qty_64 = qty.to_f64().unwrap();

    // Parse the date/basis of the exercise
    let ex_date = OffsetDateTime::parse(fields[4], time::Format::Rfc3339)
        .map_err(|e| format!("parsing exercise date {}: {e}", fields[4]))?;
    let ex_basis =
        Price::from_str(fields[7]).map_err(|e| format!("parsing exercise {}: {e}", fields[7]))?;

    // Parse the strike price
    let strike_s = CsvIter::new(fields[3], '-')
        .nth(3)
        .ok_or_else(|| format!("[2021] no field 4 in contract {}", fields[3]))?;
    let strike =
        Price::from_str(strike_s).map_err(|e| format!("Parsing strike {strike_s}: {e}"))?;

    // Figure out the BTC price reference
    let btc_price = if fields[3].contains("Call") {
        strike + ex_basis.scale_approx(100.0 / qty_64)
    } else {
        strike - ex_basis.scale_approx(100.0 / qty_64)
    };

    Ok(Some((ex_date, btc_price)))
}

/// An iterator over the fields of a CSV string
struct CsvIter<'s> {
    remaining: &'s str,
    sep: char,
}

impl<'s> CsvIter<'s> {
    /// Construct a new iterator from the given string
    fn new(s: &str, sep: char) -> CsvIter {
        CsvIter { remaining: s, sep }
    }
}

impl<'s> Iterator for CsvIter<'s> {
    type Item = &'s str;
    fn next(&mut self) -> Option<&'s str> {
        if self.remaining.is_empty() {
            return None;
        }

        let mut escape = false;
        let mut scanning = true;
        for (n, ch) in self.remaining.chars().enumerate() {
            if ch == '\\' {
                escape = true;
            } else if !escape && ch == '"' {
                scanning = !scanning;
            } else if !escape && scanning && ch == self.sep {
                let ret = &self.remaining[..n];
                self.remaining = &self.remaining[n + 1..];
                return Some(ret);
            } else if escape {
                escape = false;
            }
        }

        let ret = self.remaining;
        self.remaining = "";
        Some(ret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::{date, time};

    #[test]
    fn exercise_2021() {
        let res = price_ref(
            "Exercised,\"6, BTC Mini 2021-07-16 Put $32,000.00\",2021-07-16T22:00:00Z,2021-07-15T17:10:37Z,47.10,13.34,33.76,-1256-,,,"
        );
        let (date, price) = if let Ok(Some((date, price))) = res {
            (date, price)
        } else {
            panic!("Unexpected result: {:?}", res);
        };
        assert_eq!(
            date,
            date!(2021 - 07 - 16)
                .with_time(time!(22:00:00))
                .assume_utc()
        );
        assert_eq!(price.to_int(), 31777);

        assert_eq!(
            price_ref(
                "Sell,\"0.01, BTC\",2021-04-14T21:00:00Z,2021-07-18T21:00:00Z,321.87,629.05,-307.18,Short-term,,,"
            ),
            Ok(None),
        );
    }

    #[test]
    fn lines_2022() {
        assert_eq!(
            price_ref(
                "3197933266,Expire - 1256 Option - Call,15.00,BTC-Mini-14JAN2022-46000-Call,2022-01-14T22:00:00.000Z,2022-01-11T02:51:03.755Z,27.75,0.00,27.75,- 1256 - ",
            ),
            Ok(None),
        );

        assert_eq!(
            price_ref(
                "3197933266,Exercise - 1256 Option - Call,500.00,BTC-Mini-04FEB2022-40000-Call,2022-02-04T22:00:00.000Z,2022-01-27T23:08:44.124Z,\"1,565.00\",\"3,223.90\",\"-1,658.90\",- 1256 - ",
            ),
            Ok(Some((
                date!(2022-02-04).with_time(time!(22:00:00)).assume_utc(),
                crate::price!(40644.78),
            ))),
        );
    }

    #[test]
    fn missing_basis_2022() {
        assert_eq!(
            price_ref(
                "3197933266,Exercise - 1256 Option - Call,4.5752,BTC,*,2022-02-04T22:00:00.000Z,\"185,957.997456\",*,-,-",
            ),
            Ok(None),
        );
    }

    #[test]
    fn big_quantity_2022() {
        assert_eq!(
            price_ref(
                "3197933266,Expire - 1256 Option - Call,\"1,000.00\",BTC-Mini-10JUN2022-32000-Call,2022-06-10T21:00:00.000Z,2022-06-09T16:41:55.801Z,320.00,0.00,320.00,- 1256 -",
            ),
            Ok(None),
        );
    }

    #[test]
    fn extract_price_2022() {
        assert_eq!(
            price_ref(
                "3197933266,Expire - 1256 Option - Put,1.00,BTC-Mini-27MAY2022-30000-Put,2022-05-27T21:00:00.000Z,2022-05-06T13:14:46.539Z,3.85,0.00,3.85,- 1256 - ",
            ),
            Ok(None),
        );
    }
}
