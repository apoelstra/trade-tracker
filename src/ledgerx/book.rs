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
use crate::terminal::format_color;
use crate::units::{Asset, Price, Quantity};
use log::info;
use std::collections::BTreeMap;
use time::OffsetDateTime;

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
                last_log: None,
            };
            book.insert((order.price, order.message_id), book_order);
        }
    }

    /// Return the price and size of the best bid, or (0, 0) if there is none
    pub fn best_bid(&self) -> (Price, Quantity) {
        if let Some((_, last)) = self.bids.iter().rev().next() {
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

    pub fn log_interesting_orders(
        &mut self,
        opt: &crate::option::Option,
        now: OffsetDateTime,
        btc_bid: Price,
        btc_ask: Price,
        available_usd: Price,
        available_btc: bitcoin::Amount,
    ) {
        let (best_bid, _) = self.best_bid();
        let mut bid_depth = Quantity::Zero;
        for bid in self.bids.values_mut().rev() {
            // Don't bother logging bids that are less than 50% of the best
            if bid.price < best_bid.half() {
                break;
            }
            // Similarly, if you need to sell more than 100 contracts to get
            // this bid, don't bother logging. If it were interesting, the
            // better ones would be interesting too.
            if bid_depth > Quantity::Contracts(100) {
                break;
            }
            bid_depth += bid.size;
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

        let (best_ask, _) = self.best_ask();
        let mut ask_depth = Quantity::Zero;
        for ask in self.asks.values_mut() {
            // Don't bother logging asks that are more than 200% of the best
            if ask.price < best_ask.double() {
                break;
            }
            // Similarly, if you need to sell more than 100 contracts to get
            // this ask, don't bother logging. If it were interesting, the
            // better ones would be interesting too.
            if ask_depth > Quantity::Contracts(100) {
                break;
            }
            ask_depth += ask.size;
            log_ask_if_interesting(
                ask,
                opt,
                now,
                btc_bid,
                btc_ask,
                available_usd,
                available_btc,
            );
        }
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
    pub timestamp: OffsetDateTime,
    /// The most recent time this order was logged as "interesting"
    pub last_log: Option<OffsetDateTime>,
}

fn log_bid_if_interesting(
    order: &mut Order,
    opt: &crate::option::Option,
    now: OffsetDateTime,
    btc_bid: Price,
    btc_ask: Price,
    available_usd: Price,
    available_btc: bitcoin::Amount,
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
    let btc_price = (btc_bid + btc_ask).half();
    // Also, cap the order size at the amount that we could actually
    // take. Puts especially may seem worthwhile but unless we have
    // a big pile of cash on hand, we can only get a couple bucks out.
    let (max_size, _) = opt.max_sale(order.price, available_usd, available_btc);
    let order_size = max_size.min(order.size);
    // For bids, we need to be able to compute volatility (otherwise
    // this is a "free money" bid, which we don't want to be short.
    if super::BID_INTERESTING.is_interesting(opt, now, btc_price, order.price, order_size) {
        opt.log_option_data(
            format_color("Interesting bid: ", 110, 250, 250),
            now,
            btc_price,
        );
        opt.log_order_data("    Price: ", now, btc_price, order.price, Some(order_size));

        order.last_log = Some(now);
    }
}

fn log_ask_if_interesting(
    order: &mut Order,
    opt: &crate::option::Option,
    now: OffsetDateTime,
    btc_bid: Price,
    btc_ask: Price,
    available_usd: Price,
    available_btc: bitcoin::Amount,
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
        btc_bid - crate::price!(150)
    } else {
        btc_ask + crate::price!(150)
    };
    // Asks may be interesting if they're "free money", that is, if we
    // can open a delta-neutral position which is guaranteed to pay out
    if order.price < opt.intrinsic_value(btc_price) {
        let profit100 = opt.intrinsic_value(btc_price) - order.price;
        let max_buy100 =
            Quantity::contracts_from_ratio(available_usd, order.price).min(-order.size);
        let max_profit = profit100 * max_buy100;
        // Don't bother if there is less than $100 to be made
        if max_profit < Price::ONE_HUNDRED {
            return;
        }

        opt.log_option_data(
            format_color("Apparent free money ask: ", 80, 255, 80),
            now,
            btc_price,
        );
        opt.log_order_data(
            "    Price: ",
            now,
            btc_price,
            order.price,
            Some(-order.size),
        );
        if opt.pc == crate::option::Call {
            info!(
                "       Strike {} + {} = {}, vs BTC price {}. By arbing {} units you can make {}.",
                opt.strike,
                order.price,
                opt.strike + order.price,
                btc_price,
                max_buy100,
                max_profit,
            );
        } else {
            info!(
                "       Strike {} - {} is {}, vs BTC price {}. By arbing {} units you can make {}.",
                opt.strike,
                order.price,
                opt.strike - order.price,
                btc_price,
                max_buy100,
                max_profit,
            );
        }

        order.last_log = Some(now);
    } else {
        // Otherwise they may be interesting to match
        let (max_ask_size, _) = opt.max_sale(order.price, available_usd, available_btc);
        if super::ASK_INTERESTING.is_interesting(opt, now, btc_price, order.price, max_ask_size) {
            opt.log_option_data(
                format_color("Interesting ask (to match): ", 180, 180, 250),
                now,
                btc_price,
            );
            opt.log_order_data(
                "    Price: ",
                now,
                btc_price,
                order.price,
                Some(max_ask_size),
            );
            order.last_log = Some(now);
        }
    }
}
