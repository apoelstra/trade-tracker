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

use super::{datafeed, MessageId};
use crate::option::{Call, Put};
use crate::units::{Asset, Price, Quantity, UtcTime};
use std::collections::BTreeMap;

/// Book state for a specific contract
#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub struct BookState {
    asset: Asset,
    bids: BTreeMap<(Price, MessageId), Order>,
    asks: BTreeMap<(Price, MessageId), Order>,
}

impl BookState {
    /// Create a new empty book state
    pub fn new(asset: Asset) -> BookState {
        BookState {
            asset,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
        }
    }

    /// Add an order to the book
    pub fn insert_order(&mut self, order: datafeed::Order) {
        let size = order.size.with_asset(self.asset);
        let book = match size.is_positive() {
            true => &mut self.bids,
            false => &mut self.asks,
        };

        // Annoyingly the price on a cancelled order is set to 0 (which I suppose makes
        // some sort of sense since it's a "null order") so we can't just look it up
        // by its (price, mid) pair. Similarly edited orders will have a different price
        // than their original price (we have an "original_price" field but I don't
        // believe it will do the right thing for repeated edits.)
        //
        // So we have to scan the whole book to find the mid.
        book.retain(|(_, mid), _| *mid != order.message_id);
        if size.is_nonzero() {
            let book_order = Order {
                price: order.price,
                size,
                message_id: order.message_id,
                timestamp: order.timestamp,
            };
            book.insert((order.price, order.message_id), book_order);
        }
    }

    /// Return the price and size of the best bid, or (0, 0) if there is none
    pub fn best_bid(&self) -> (Price, Quantity) {
        if let Some((_, last)) = self.bids.iter().next_back() {
            (last.price, last.size)
        } else {
            (Price::ZERO, Quantity::Zero)
        }
    }

    /// Return the price and size of the best ask, or (0, 0) if there is none
    pub fn best_ask(&self) -> (Price, Quantity) {
        if let Some((_, last)) = self.asks.iter().next() {
            (last.price, -last.size)
        } else {
            (Price::ZERO, Quantity::Zero)
        }
    }

    /// Returns the (gain in contracts, cost in USD) of buying into every offer
    pub fn clear_asks(&self) -> (Quantity, Price) {
        let mut ret_usd = Price::ZERO;
        let mut ret_contr = Quantity::Zero;
        for (_, order) in self.asks.iter() {
            ret_usd += order.price * order.size;
            ret_contr += order.size;
        }
        (ret_contr, ret_usd)
    }

    /// Returns the (cost in contracts, gain in USD) of selling into every bid
    pub fn clear_bids(
        &self,
        option: &crate::option::Option,
        mut max_usd: Price,
        mut max_btc: bitcoin::Amount,
    ) -> (Quantity, Price) {
        let mut ret_usd = Price::ZERO;
        let mut ret_contr = Quantity::Zero;
        for (_, order) in self.bids.iter() {
            let (max_sale, usd_per_100) = option.max_sale(order.price, max_usd, max_btc);
            let sale = max_sale.min(order.size);
            if sale.is_zero() {
                break;
            }
            assert!(
                sale.is_nonnegative(),
                "somehow our maximum sale amount is negative"
            );

            ret_usd += order.price * sale;
            ret_contr += sale;
            match option.pc {
                Call => max_btc -= sale.btc_equivalent().to_unsigned().unwrap(),
                Put => max_usd -= usd_per_100 * sale,
            }
        }
        (ret_contr, ret_usd)
    }

    /// Yield an iterator over all bids, from best to worst
    pub fn bids(&self) -> impl Iterator<Item = &Order> {
        self.bids.values().rev()
    }

    /// Yield an iterator over all asks, from best to worst
    pub fn asks(&self) -> impl Iterator<Item = &Order> {
        self.asks.values()
    }
}

/// An order, as recorded in the orderbook
#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub struct Order {
    /// The price at which the order is placed
    pub price: Price,
    /// The (signed) quantity
    pub size: Quantity,
    /// ID of the manifest
    pub message_id: MessageId,
    /// Timestamp that the order occured on
    pub timestamp: UtcTime,
}
