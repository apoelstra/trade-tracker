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
use crate::units::{Asset, Price, UtcTime};
use log::debug;

/// A price reference
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reference {
    /// The BTC book state, from which we obtain our price reference
    book_state: BookState,
    /// Last available best bid
    last_best_bid: Price,
    /// Last available best ask
    last_best_ask: Price,
    /// Time of the most recent update to the book
    last_update: UtcTime,
}

impl Reference {
    /// Create a new empty price reference, using an input price as a starting point
    pub fn new(price_ref: crate::price::BitcoinPrice) -> Self {
        Reference {
            book_state: BookState::new(Asset::Btc),
            last_best_bid: price_ref.btc_price,
            last_best_ask: price_ref.btc_price,
            last_update: price_ref.timestamp,
        }
    }

    fn log<D: std::fmt::Display>(&self, reason: D) {
        debug!(
            "[price reference] {}; {} (bb {} ba {}, last update {})",
            reason,
            (self.last_best_bid + self.last_best_ask).half(),
            self.last_best_bid,
            self.last_best_ask,
            self.last_update
        );
    }

    /// Returns a price reference, if one can be obtained.
    pub fn reference(&self) -> (Price, UtcTime) {
        let pref = (self.last_best_bid + self.last_best_ask).half();
        (pref, self.last_update)
    }

    /// Returns the current best bid, or 0 if none exists
    pub fn best_bid(&self) -> Price {
        self.last_best_bid
    }

    /// Returns the current best ask, or 0 if none exists
    pub fn best_ask(&self) -> Price {
        self.last_best_ask
    }

    /// Adds an order to the price reference
    pub fn insert_order(&mut self, order: Order) {
        let (size, price, time) = (order.size, order.price, order.timestamp);
        self.book_state.insert_order(order);

        let (bid, _) = self.book_state.best_bid();
        let (ask, _) = self.book_state.best_ask();
        // Don't return invalid data. Let the caller deal with it if
        // we're missing data.
        if bid != Price::ZERO {
            self.last_best_bid = bid;
            self.last_update = UtcTime::now();
        }
        if ask != Price::ZERO {
            self.last_best_ask = ask;
            self.last_update = UtcTime::now();
        }

        self.log(format_args!(
            "record order at price {} size {} time {}",
            price, size, time,
        ));
    }

    /// Clear out the order book (but retain the current "last best bid/ask" data)
    pub fn clear_book(&mut self) {
        self.book_state = BookState::new(Asset::Btc);
        self.log("clear book");
    }
}
