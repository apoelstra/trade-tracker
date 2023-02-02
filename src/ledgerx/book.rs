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
use crate::terminal::format_color;
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

        // Annoyingly the price on a cancelled order is set to 0 (which I suppose makes
        // some sort of sense since it's a "null order") so we can't just look it up
        // by its (price, mid) pair. Similarly edited orders will have a different price
        // than their original price (we have an "original_price" field but I don't
        // believe it will do the right thing for repeated edits.)
        //
        // So we have to scan the whole book to find the mid.
        book.retain(|(_, mid), _| *mid != order.manifest_id);
        if order.size > 0 {
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
            let (max_sale, usd_per_contract) = option.max_sale(order.price, max_usd, max_btc);
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
        available_usd: Decimal,
        available_btc: Decimal,
    ) {
        for bid in self.bids.values_mut() {
            log_bid_if_interesting(
                bid,
                opt,
                now,
                btc_bid,
                btc_ask,
                available_usd,
                available_btc,
            );
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
    available_usd: Decimal,
    available_btc: Decimal,
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
    // Also, cap the order size at the amount that we could actually
    // take. Puts especially may seem worthwhile but unless we have
    // a big pile of cash on hand, we can only get a couple bucks out.
    let (max_size, _) = opt.max_sale(order.price, available_usd, available_btc);
    let order_size = cmp::min(max_size, order.size);
    // For bids, we need to be able to compute volatility (otherwise
    // this is a "free money" bid, which we don't want to be short.
    if super::BID_INTERESTING.is_interesting(opt, now, btc_price, order.price, order_size) {
        println!("");
        println!("Date: {}", now);
        print!("{}", format_color("Interesting bid: ", 110, 250, 250));
        opt.print_option_data(now, btc_price);
        print!("    Price: ");
        opt.print_order_data(now, btc_price, order.price, order_size);

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
        println!("Date: {}", now);
        print!("{}", format_color("Apparent free money ask: ", 80, 255, 80));
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
