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
use crate::option::{Call, Put};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::cmp;
use std::collections::BTreeMap;
use time::OffsetDateTime;

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
    pub fn clear_bids(
        &self,
        option: &crate::option::Option,
        mut max_usd: Decimal,
        mut max_btc: Decimal,
    ) -> (u64, Decimal) {
        let mut ret_usd = Decimal::from(0);
        let mut ret_contr = 0;
        for (_, order) in self.bids.iter() {
            let (max_sale, usd_per_contract) = match option.pc {
                // For a call, we can sell as many as we have BTC to support
                Call => (
                    (max_btc * Decimal::from(100)).to_u64().unwrap(),
                    Decimal::from(0),
                ),
                // For a put it's a little more involved
                Put => {
                    let locked_per_100 = option.strike - order.price + Decimal::from(25);
                    let locked_per_1 = locked_per_100 / Decimal::from(100);
                    ((max_usd / locked_per_1).to_u64().unwrap(), locked_per_1)
                }
            };
            let sale = cmp::min(max_sale, order.size);
            if sale == 0 {
                break;
            }

            ret_usd += order.price * Decimal::from(sale) / Decimal::from(100);
            ret_contr += sale;
            match option.pc {
                Call => max_btc -= Decimal::from(sale) / Decimal::from(100),
                Put => max_usd -= Decimal::from(sale) * usd_per_contract,
            }
        }
        (ret_contr, ret_usd)
    }

    pub fn log_interesting_orders(
        &mut self,
        opt: &crate::option::Option,
        now: OffsetDateTime,
        btc_bid: Decimal,
        btc_ask: Decimal,
    ) {
        for bid in self.bids.values_mut() {
            log_bid_if_interesting(bid, opt, now, btc_bid, btc_ask);
        }
        for ask in self.asks.values_mut() {
            log_ask_if_interesting(ask, opt, now, btc_bid, btc_ask);
        }
    }
}

fn log_bid_if_interesting(
    order: &mut Order,
    opt: &crate::option::Option,
    now: OffsetDateTime,
    btc_bid: Decimal,
    btc_ask: Decimal,
) {
    if let Some(last_log) = order.last_log {
        // Refuse to log the same order more than once every 4 hours
        if now - last_log < time::Duration::hours(4) {
            return;
        }
    }
    // For bids we'll just use the midpoint since we're not thinking
    // about any short-option-delta-neutral strategies. If we were
    // trying to go delta-neutral we should use the best BTC ask instead.
    let btc_price = (btc_bid + btc_ask) / Decimal::from(2);
    // For bids, we need to be able to compute volatility (otherwise
    // this is a "free money" bid, which we don't want to be short.
    if let Ok(vol) = opt.bs_iv(now, btc_price, order.price) {
        let arr = opt.arr(now, btc_price, order.price);
        let ddelta80 = opt.bs_dual_delta(now, btc_price, 0.80).abs();

        if opt.in_the_money(btc_price) {
            return;
        } // Ignore ITM bids, we don't really have a strategy for shorting ITMs
        if arr < 0.05 {
            return;
        } // ignore low-yield bids
        if ddelta80 > 0.25 {
            return;
        } // ignore bids with high likelihood of getting assigned
        if vol < 0.5 {
            return;
        } // ignore bids with low volatility
        println!("");
        print!("Interesting bid: ");
        opt.print_option_data(now, btc_price);
        print!("    Price: ");
        opt.print_order_data(now, btc_price, order.price, order.size);

        order.last_log = Some(now);
    }
}

fn log_ask_if_interesting(
    order: &mut Order,
    opt: &crate::option::Option,
    now: OffsetDateTime,
    btc_bid: Decimal,
    btc_ask: Decimal,
) {
    if let Some(last_log) = order.last_log {
        // Refuse to log the same order more than once every 4 hours
        if now - last_log < time::Duration::hours(4) {
            return;
        }
    }
    // Add a fudge factor because there's a race condition involved in
    // executing on something like this, so we want a nontrivial payout
    let btc_price = if opt.pc == crate::option::Call {
        btc_bid - Decimal::from(150)
    } else {
        btc_ask + Decimal::from(150)
    };
    // Asks are only interesting if they're "free money", that is, if we
    // can open a delta-neutral position which is guaranteed to pay out
    if order.price < opt.intrinsic_value(btc_price) {
        println!("");
        print!("Apparent free money offer: ");
        opt.print_option_data(now, btc_price);
        print!("    Price: ");
        opt.print_order_data(now, btc_price, order.price, order.size);
        if opt.pc == crate::option::Call {
            println!(
                "       (strike {} + price {} is {}, vs BTC price {}",
                opt.strike,
                order.price,
                opt.strike + order.price,
                btc_price,
            );
        } else {
            println!(
                "       (strike {} - price {} is {}, vs BTC price {}",
                opt.strike,
                order.price,
                opt.strike - order.price,
                btc_price,
            );
        }

        order.last_log = Some(now);
    }
}
