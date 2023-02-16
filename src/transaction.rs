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

//! Bitcoin Transactions
//!
//! Utilities to manage Bitcoin Transactions
//!

use anyhow::Context;
use std::collections::HashMap;

/// Database of known transactions
///
/// To add to this database, use the "record-tx" command with the CLI app.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Database {
    map: HashMap<bitcoin::Txid, bitcoin::Transaction>,
}

impl Database {
    /// Construct a new empty database
    pub fn from_string_map(map: &HashMap<bitcoin::Txid, String>) -> anyhow::Result<Self> {
        let mut ret = HashMap::with_capacity(map.len());
        for (txid, s) in map {
            let bytes: Vec<u8> = bitcoin::hashes::hex::FromHex::from_hex(s)
                .with_context(|| format!("decoding string for {txid} as hex"))?;
            let tx: bitcoin::Transaction = bitcoin::consensus::deserialize(&bytes)
                .with_context(|| format!("decoding hex for {txid} as transaction"))?;

            if tx.txid() != *txid {
                return Err(anyhow::Error::msg(format!(
                    "txid {txid} maps to transaction with txid {}",
                    tx.txid()
                )));
            }
            ret.insert(*txid, tx);
        }

        Ok(Database { map: ret })
    }

    /// Look up a transaction matching a particular address/amount pair
    ///
    /// LX annoyingly does not provide any more information to identify transactions (well,
    /// there is also a timestamp but it's approximate). Furthermore, they dark-pattern
    /// their users into reusing bitcoin addresses.
    pub fn find_tx_for_deposit(
        &self,
        address: &bitcoin::Address,
        amount: bitcoin::Amount,
    ) -> Option<(&bitcoin::Transaction, u32)> {
        for tx in self.map.values() {
            for (n, out) in tx.output.iter().enumerate() {
                if out.value == amount.to_sat() && out.script_pubkey == address.script_pubkey() {
                    return Some((tx, n as u32));
                }
            }
        }
        None
    }

    /// Look up a specific txout
    pub fn find_txout(&self, outpoint: bitcoin::OutPoint) -> Option<&bitcoin::TxOut> {
        self.map.get(&outpoint.txid).and_then(|tx| {
            if tx.output.len() > outpoint.vout as usize {
                Some(&tx.output[outpoint.vout as usize])
            } else {
                None
            }
        })
    }
}
