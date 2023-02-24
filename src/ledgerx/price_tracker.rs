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

//! Bitcoin Price Tracker
//!
//! Data structure which attempts to produce a BTC price reference based
//! on activity in the LX orderbook.
//!

use crate::ledgerx::{datafeed::Order, BookState};
use crate::units::{Asset, Price};
use log::debug;

/// A price reference
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reference {
    /// The BTC book state, from which we obtain our price reference
    book_state: BookState,
    /// Time of the most recent update to the book
    last_update: time::OffsetDateTime,
}

impl Reference {
    /// Create a new empty price reference
    pub fn new() -> Self {
        Reference {
            book_state: BookState::new(Asset::Btc),
            last_update: time::OffsetDateTime::now_utc(),
        }
    }

    /// Returns a price reference, if one can be obtained.
    pub fn reference(&self) -> Option<(Price, time::OffsetDateTime)> {
        let (bid, _) = self.book_state.best_bid();
        let (ask, _) = self.book_state.best_ask();
        // Don't return invalid data. Let the caller deal with it if
        // we're missing data.
        if bid == Price::ZERO || ask == Price::ZERO {
            debug!("[price reference] reference no data");
            return None;
        }
        let pref = (bid + ask).half();
        debug!(
            "[price reference] reference {} ({})",
            pref, self.last_update
        );
        Some((pref, self.last_update))
    }

    /// Returns the current best bid, or 0 if none exists
    pub fn best_bid(&self) -> Price {
        self.book_state.best_bid().0
    }

    /// Returns the current best ask, or 0 if none exists
    pub fn best_ask(&self) -> Price {
        self.book_state.best_ask().0
    }

    /// Adds an order to the price reference
    pub fn insert_order(&mut self, order: Order) {
        let (size, price) = (order.size, order.price);
        self.last_update = order.timestamp;
        self.book_state.insert_order(order);

        // Log current state (don't filter invalid data here)
        let (bid, _) = self.book_state.best_bid();
        let (ask, _) = self.book_state.best_ask();
        debug!(
            "[price reference] record order at price {} size {} time {}; reference {} (bb {} ba {})",
            price,
            size,
            self.last_update,
            (bid + ask).half(),
            bid,
            ask,
        );
    }
}
