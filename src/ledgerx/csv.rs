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

use rust_decimal::Decimal;
use std::str::FromStr;
use time::{time, OffsetDateTime};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum CsvType {
    BtcTrade,
    CallExercise { strike: Decimal },
    PutExercise { strike: Decimal },
    Other,
}

/// Structure representing a parsed line from a LX CSV file
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct CsvLine {
    ty: CsvType,
    quantity: Decimal,
    date_1: OffsetDateTime,
    date_2: OffsetDateTime,
    basis_1: Decimal,
    basis_2: Decimal,
}

impl CsvLine {
    /// Retrieve however many price references (0-2) that we can extract from this line
    pub fn price_references(&self) -> Vec<(OffsetDateTime, Decimal)> {
        let mut ret = vec![];
        match self.ty {
            CsvType::BtcTrade => {
                // For BTC trades, just look at any dates which are exactly 2100 or 2200 and
                // use those as a price reference.
                //
                // Note that date_1 corresponds to basis_2 and vice-versa!
                if self.date_1.time() == time!(21:00) || self.date_1.time() == time!(22:00) {
                    ret.push((self.date_1, self.basis_2 / self.quantity));
                }
                if self.date_2.time() == time!(21:00) || self.date_2.time() == time!(22:00) {
                    ret.push((self.date_2, self.basis_1 / self.quantity));
                }
                ret
            }
            // FIXME do this later
            CsvType::CallExercise { .. } => vec![],
            CsvType::PutExercise { .. } => vec![],
            CsvType::Other => vec![],
        }
    }
}

impl FromStr for CsvLine {
    type Err = String;

    fn from_str(mut s: &str) -> Result<CsvLine, String> {
        // If we only deal with ASCII strings we are free to byte-index them
        // without worrying about panics related to UTF-8 boundaries
        if !s.is_ascii() {
            return Err("non-ASCII string; not attempting to parse".into());
        }

        let ty_str = parse_until_next(&mut s, ',', 2)
            .ok_or_else(|| "Failed to parse first field".to_string())?;
        let quantity_str = parse_until_next(&mut s, ',', 2)
            .ok_or_else(|| "Failed to parse quantity".to_string())?;
        let mut contract_str = parse_until_next(&mut s, '"', 2)
            .ok_or_else(|| "Failed to parse quantity".to_string())?;
        let date_1_str = parse_until_next(&mut s, ',', 1)
            .ok_or_else(|| "Failed to parse quantity".to_string())?;
        let date_2_str = parse_until_next(&mut s, ',', 1)
            .ok_or_else(|| "Failed to parse quantity".to_string())?;
        let basis_1_str = parse_until_next(&mut s, ',', 1)
            .ok_or_else(|| "Failed to parse quantity".to_string())?;
        let basis_2_str = parse_until_next(&mut s, ',', 1)
            .ok_or_else(|| "Failed to parse quantity".to_string())?;

        let ty = if contract_str == "BTC" {
            CsvType::BtcTrade
        } else {
            let btc = parse_until_next(&mut contract_str, ' ', 1)
                .ok_or(format!("contract string {contract_str} first word"))?;
            let mini = parse_until_next(&mut contract_str, ' ', 1)
                .ok_or(format!("contract string {contract_str} second word"))?;
            let _expiry = parse_until_next(&mut contract_str, ' ', 1)
                .ok_or(format!("contract string {contract_str} third word"))?;
            let put_call = parse_until_next(&mut contract_str, '$', 1)
                .ok_or(format!("contract string {contract_str} fourth word"))?;
            let strike_str = parse_until_next(&mut contract_str, ',', 1)
                .ok_or(format!("contract string {contract_str} strike thousands"))?;

            if btc != "BTC" {
                return Err(format!("expected BTC, got {btc}"));
            }
            if mini != "Mini" {
                return Err(format!("expected Mini, got {mini}"));
            }
            if contract_str != "000.00" {
                return Err(format!("expected 000.00, got {contract_str}"));
            }

            let strike = Decimal::ONE_THOUSAND
                * Decimal::from_str(strike_str)
                    .map_err(|e| format!("Parsing strike {contract_str}: {e}"))?;
            match put_call {
                "Call " => CsvType::CallExercise { strike },
                "Put " => CsvType::PutExercise { strike },
                x => return Err(format!("unknown put/call string {x}")),
            }
        };
        Ok(CsvLine {
            ty: match (ty_str, ty) {
                ("Buy back", CsvType::BtcTrade) => ty,
                ("Sell", CsvType::BtcTrade) => ty,
                ("Exercised", _) => ty,
                _ => CsvType::Other,
            },
            quantity: Decimal::from_str(quantity_str)
                .map_err(|e| format!("parsing quantity {quantity_str}: {e}"))?,
            date_1: OffsetDateTime::parse(date_1_str, time::Format::Rfc3339)
                .map_err(|e| format!("parsing date 1 {date_1_str}: {e}"))?,
            date_2: OffsetDateTime::parse(date_2_str, time::Format::Rfc3339)
                .map_err(|e| format!("parsing date 2 {date_2_str}: {e}"))?,
            basis_1: Decimal::from_str(basis_1_str)
                .map_err(|e| format!("parsing basis 1 {basis_1_str}: {e}"))?,
            basis_2: Decimal::from_str(basis_2_str)
                .map_err(|e| format!("parsing basis 2 {basis_2_str}: {e}"))?,
        })
    }
}

fn parse_until_next<'s>(s: &mut &'s str, pat: char, skip_past: usize) -> Option<&'s str> {
    let idx = s.find(pat)?;
    let ret = Some(&s[..idx]);

    if s.len() < idx + skip_past {
        return None;
    }
    *s = &s[idx + skip_past..];
    ret
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::{date, time};

    #[test]
    fn exercise() {
        assert_eq!(
            CsvLine::from_str(
                "Exercised,\"6, BTC Mini 2021-07-16 Put $32,000.00\",2021-07-16T22:00:00Z,2021-07-15T17:10:37Z,47.10,13.34,33.76,-1256-,,,"
            ),
            Ok(CsvLine {
                ty: CsvType::PutExercise {
                    strike: Decimal::new(32_000, 0),
                },
                quantity: Decimal::new(6, 0),
                date_1: date!(2021-07-16).with_time(time!(22:00:00)).assume_utc(),
                date_2: date!(2021-07-15).with_time(time!(17:10:37)).assume_utc(),
                basis_1: Decimal::new(4710, 2),
                basis_2: Decimal::new(1334, 2),
            }),
        );

        assert_eq!(
            CsvLine::from_str(
                "Sell,\"0.01, BTC\",2021-04-14T21:00:00Z,2021-07-18T21:00:00Z,321.87,629.05,-307.18,Short-term,,,"
            ),
            Ok(CsvLine {
                ty: CsvType::BtcTrade,
                quantity: Decimal::new(1, 2),
                date_1: date!(2021-04-14).with_time(time!(21:00:00)).assume_utc(),
                date_2: date!(2021-07-18).with_time(time!(21:00:00)).assume_utc(),
                basis_1: Decimal::new(32187, 2),
                basis_2: Decimal::new(62905, 2),
            }),
        );
    }
}
