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

//! Book State
//!
//! Tracks the book state for a specific contract
//!

use super::{Ask, Bid, ManifestId, Order};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

/// Book state for a specific contract
#[derive(Clone, PartialEq, Eq, Debug, Default, Hash)]
pub struct BookState {
    bids: BTreeMap<(Decimal, ManifestId), Order>,
    asks: BTreeMap<(Decimal, ManifestId), Order>,
}

impl BookState {
    /// Create a new empty book state
    pub fn new() -> BookState {
        Default::default()
    }

    /// Add an order to the book
    pub fn insert_order(&mut self, order: Order) {
        let book = match order.bid_ask {
            Bid => &mut self.bids,
            Ask => &mut self.asks,
        };

        if order.size == 0 {
            book.remove(&(order.price, order.manifest_id));
        } else {
            book.insert((order.price, order.manifest_id), order);
        }
    }

    /// Return the price and size of the best bid, or (0, 0) if there is none
    pub fn best_bid(&self) -> (Decimal, u64) {
        if let Some((_, last)) = self.bids.iter().rev().next() {
            (last.price, last.size)
        } else {
            (Decimal::from(0), 0)
        }
    }

    /// Return the price and size of the best ask, or (0, 0) if there is none
    pub fn best_ask(&self) -> (Decimal, u64) {
        if let Some((_, last)) = self.asks.iter().next() {
            (last.price, last.size)
        } else {
            (Decimal::from(0), 0)
        }
    }

    /// Returns the (gain in contracts, cost in USD) of buying into every offer
    pub fn clear_asks(&self) -> (u64, Decimal) {
        let mut ret_usd = Decimal::from(0);
        let mut ret_contr = 0;
        for (_, order) in self.asks.iter() {
            ret_usd += order.price * Decimal::from(order.size) / Decimal::from(100);
            ret_contr += order.size;
        }
        (ret_contr, ret_usd)
    }

    /// Returns the (cost in contracts, gain in USD) of selling into every bid
    pub fn clear_bids(&self) -> (u64, Decimal) {
        let mut ret_usd = Decimal::from(0);
        let mut ret_contr = 0;
        for (_, order) in self.bids.iter() {
            ret_usd += order.price * Decimal::from(order.size) / Decimal::from(100);
            ret_contr += order.size;
        }
        (ret_contr, ret_usd)
    }
}
