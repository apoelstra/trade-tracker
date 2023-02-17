// Trade Tracker
// Written in 2023 by
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

//! LedgerX History Configuration
//!
//! Parses and handles the configuration file which defines all the data needed to
//! produce end-of-year tax documentation.
//!
//! To produce these files the process is:
//!     1. Copy last year's file (the original, 2021, was hand made)
//!     2. Change the year field at the top.
//!     3. Delete the entire "lx_csv" array and replace it with one made from
//!        the CSV file that LX gives you (changing all "s to \"s and enclosing
//!        each line in quotes and adding commas).
//!
//!        Be sure to delete the header line from the LX CSV.
//!

use crate::ledgerx::history::LotId;
use crate::units::Price;
use serde::Deserialize;
use std::collections::HashMap;

/// The main configuration structure
///
/// BE VERY CAREFUL ABOUT CHANGING THIS and make sure that every previous
/// year's configuration can be reproduced. If there are changes this may
/// undermine our ability to produce audit-safe documentation.
#[derive(Clone, PartialEq, Eq, Deserialize, Debug)]
pub struct Configuration {
    /// The tax year under question
    year: i32,
    /// The LX-provided CSV file, crammed into a JSON string array
    lx_csv: Vec<String>,
    /// Date and bitcoin price data about every UTXO-based lot
    ///
    /// This can be copied forward from year to year, but needs to be extended with
    /// new coins as they come in (any coins that ever touch LedgerX).
    lots: HashMap<LotId, LotInfo>,
    /// Map of TXIDs to the raw transaction data
    ///
    /// The software will complain if any necessary entries are missing, or if existing
    /// entries don't match the claimed TXID. So it's pretty hard to mess this one up.
    transactions: HashMap<bitcoin::Txid, String>,
}

impl Configuration {
    /// Accessor for the tax year
    pub fn year(&self) -> i32 {
        self.year
    }

    /// Accessor for the lines of the LX csv file
    pub fn lx_csv(&self) -> &[String] {
        &self.lx_csv
    }

    /// Accessor for the lot database (infallible as this requires no further processing)
    pub fn lot_db(&self) -> &HashMap<LotId, LotInfo> {
        &self.lots
    }

    /// (Attempts to) construct a transaction database from the tx map
    ///
    /// Will fail if any of the raw transactions fail to parse, or if their
    /// TXIDs don't match the expected one.
    pub fn transaction_db(&self) -> anyhow::Result<crate::transaction::Database> {
        crate::transaction::Database::from_string_map(&self.transactions)
    }
}

/// Information about specific lots
#[derive(Clone, PartialEq, Eq, Deserialize, Debug)]
pub struct LotInfo {
    /// The bitcoin price reference to use for this lot
    ///
    /// NOT the basis of the lot. You need to multiply this price by the quantity
    /// (not included in the lot information; comes from the transaction data)
    /// to get the basis.
    #[serde(deserialize_with = "crate::units::deserialize_cents")]
    pub price: Price,
    /// The ID of the lot in question
    #[serde(deserialize_with = "deserialize_timestamp")]
    pub date: time::OffsetDateTime,
}

fn deserialize_timestamp<'de, D>(deser: D) -> Result<time::OffsetDateTime, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: i64 = Deserialize::deserialize(deser)?;
    Ok(time::OffsetDateTime::from_unix_timestamp(s))
}
