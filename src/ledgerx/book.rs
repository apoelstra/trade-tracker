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

use super::{Ask, Bid, Order};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

/// Book state for a specific contract
#[derive(Clone, PartialEq, Eq, Debug, Default, Hash)]
pub struct BookState {
    bids: BTreeMap<Decimal, Order>,
    asks: BTreeMap<Decimal, Order>,
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
        for entry in book.values_mut() {
            if entry.manifest_id == order.manifest_id {
                *entry = order;
                return;
            }
        }

        book.insert(order.price, order);
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
}
