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

//! Lots
//!
//! Utilities to track lots
//!

use crate::ledgerx::LotId;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, io, path};
use time::OffsetDateTime;

/// Database of known transactions
///
/// To add to this database, use the "record-lot" command with the CLI app.
#[derive(Clone, PartialEq, Eq, Debug, Default, Deserialize, Serialize)]
pub struct Database {
    map: HashMap<LotId, i64>,
}

impl Database {
    /// Construct a new empty database
    pub fn new() -> Self {
        Default::default()
    }

    /// Reads the database from a file
    pub fn load<P: AsRef<path::Path>>(filepath: P) -> Result<Self, anyhow::Error> {
        let filename = filepath.as_ref().to_string_lossy();
        let fh = fs::File::open(filepath.as_ref())
            .with_context(|| format!("opening lot database {filename}"))?;
        let bf = io::BufReader::new(fh);
        serde_json::from_reader(bf).with_context(|| format!("parsing lot database {filename}"))
    }

    /// Saves out the database to a file
    pub fn save<P: AsRef<path::Path>>(&self, filepath: P) -> Result<(), anyhow::Error> {
        let filename = filepath.as_ref().to_string_lossy();
        let fh = fs::File::create(filepath.as_ref())
            .with_context(|| format!("creating lot database {filename}"))?;
        let bf = io::BufWriter::new(fh);
        serde_json::to_writer(bf, self).with_context(|| format!("parsing lot database {filename}"))
    }

    /// Adds a transaction to the map
    ///
    /// If the transaction was already recorded, returns the existing timestamp
    pub fn insert_lot(&mut self, lot: LotId, timestamp: i64) -> Option<OffsetDateTime> {
        self.map
            .insert(lot, timestamp)
            .map(OffsetDateTime::from_unix_timestamp)
    }

    /// Look up a transaction matching a particular address/amount pair
    ///
    /// LX annoyingly does not provide any more information to identify transactions (well,
    /// there is also a timestamp but it's approximate). Furthermore, they dark-pattern
    /// their users into reusing bitcoin addresses.
    pub fn find_lot(&self, id: &LotId) -> Option<OffsetDateTime> {
        self.map
            .get(id)
            .map(|d| OffsetDateTime::from_unix_timestamp(*d))
    }
}
