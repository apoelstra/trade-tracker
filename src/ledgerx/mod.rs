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

//! LedgerX
//!
//! Data Structures etc for the LedgerX API
//!

pub mod book;
pub mod contract;
pub mod datafeed;
pub mod json;

use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json;
use std::collections::HashMap;
use time::OffsetDateTime;

pub use book::BookState;
pub use contract::{Contract, ContractId};
pub use datafeed::{BidAsk::Ask, BidAsk::Bid, Order};

/// The underlying physical asset
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Deserialize)]
pub enum Asset {
    /// Bitcoin
    #[serde(rename = "CBTC")]
    Btc,
    /// Ethereum
    #[serde(rename = "ETH")]
    Eth,
    /// US Dollars
    #[serde(rename = "USD")]
    Usd,
}

/// LedgerX API error
pub enum Error {
    /// Error parsing json
    JsonParsing {
        /// Copy of the JSON under question
        json: serde_json::Value,
        /// serde_json error
        error: serde_json::Error,
    },
    ///
    JsonDecoding {},
}

pub fn from_json_dot_data<'a, T: Deserialize<'a>>(
    data: &'a [u8],
) -> Result<Vec<T>, serde_json::Error> {
    #[derive(Deserialize)]
    struct Response<U> {
        data: Vec<U>,
    }
    let json: Response<T> = serde_json::from_slice(&data)?;
    Ok(json.data)
}

/// Tracker for the state of the entire LX book
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LedgerX {
    contracts: HashMap<ContractId, (Contract, BookState)>,
    last_btc_bid: Decimal,
    last_btc_ask: Decimal,
    last_btc_time: OffsetDateTime,
}

pub enum UpdateResponse<'c> {
    /// Update was accepted; no new interesting info
    Accepted,
    /// Update was ignored (probably: updated a contract that was not being tracked)
    Ignored,
    /// Update caused the best bid on a contract to change
    NewBestBid {
        contract: &'c Contract,
        price: Decimal,
        size: u64,
    },
    /// Update caused the best ask on a contract to change
    NewBestAsk {
        contract: &'c Contract,
        price: Decimal,
        size: u64,
    },
}

fn log_if_interesting(
    order: &Order,
    opt: &crate::option::Option,
    now: OffsetDateTime,
    btc_bid: Decimal,
    btc_ask: Decimal,
) {
    // ignore anything for less than $10 or single-contract orders
    if order.size < 2 || order.price < Decimal::from(10) {
        return;
    }

    if order.bid_ask == Bid {
        log_bid_if_interesting(order, opt, now, btc_bid, btc_ask);
    } else {
        log_ask_if_interesting(order, opt, now, btc_bid, btc_ask);
    }
}

fn log_bid_if_interesting(
    order: &Order,
    opt: &crate::option::Option,
    now: OffsetDateTime,
    btc_bid: Decimal,
    btc_ask: Decimal,
) {
    // For bids we'll just use the midpoint since we're not thinking
    // about any short-option-delta-neutral strategies. If we were
    // trying to go delta-neutral we should use the best BTC ask instead.
    let btc_price = (btc_bid + btc_ask) / Decimal::from(2);
    // For bids, we need to be able to compute volatility (otherwise
    // this is a "free money" bid, which we don't want to be short.
    if let Ok(vol) = opt.bs_iv(now, btc_price, order.price) {
        let arr = opt.arr(now, btc_price, order.price);
        let ddelta80 = opt.bs_dual_delta(now, btc_price, 0.80).abs();

        if arr < 0.1 {
            return;
        } // ignore low-yield bids
        if ddelta80 > 0.3 {
            return;
        } // ignore bids with high likelihood of getting assigned
        if vol < 0.5 {
            return;
        } // ignore bids with low volatility

        print!("Interesting bid: ");
        opt.print_option_data(now, btc_price);
        print!("    Price: ");
        opt.print_order_data(now, btc_price, order.price, order.size);
    }
}
fn log_ask_if_interesting(
    order: &Order,
    opt: &crate::option::Option,
    now: OffsetDateTime,
    btc_bid: Decimal,
    btc_ask: Decimal,
) {
    // Add a fudge factor because there's a race condition involved in
    // executing on something like this, so we want a nontrivial payout
    let btc_price = if opt.pc == crate::option::Call {
        btc_bid - Decimal::from(100)
    } else {
        btc_ask + Decimal::from(100)
    };
    // Asks are only interesting if they're "free money", that is, if we
    // can open a delta-neutral position which is guaranteed to pay out
    if order.price < opt.intrinsic_value(btc_price) {
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
    }
}

impl LedgerX {
    /// Create a new empty LX tracker
    pub fn new(btc_price: crate::price::BitcoinPrice) -> Self {
        LedgerX {
            contracts: HashMap::new(),
            last_btc_bid: btc_price.btc_price,
            last_btc_ask: btc_price.btc_price,
            last_btc_time: btc_price.timestamp,
        }
    }

    /// Returns the current BTC price, as seen by the tracker
    ///
    /// Initially uses a price reference supplied at construction (probably coming
    /// from the BTCCharts data ultimately); later will use the midpoint of the LX
    /// current bid/ask for day-ahead swaps.
    pub fn current_price(&self) -> (Decimal, OffsetDateTime) {
        (
            (self.last_btc_bid + self.last_btc_ask) / Decimal::from(2),
            self.last_btc_time,
        )
    }

    /// Add a new contract to the tracker
    ///
    /// Some checks will be done as to whether this is an "interesting" option
    /// at the current price, and if so, we print a log message.
    pub fn add_contract(&mut self, c: Contract) {
        // If this is an "interesting" contract then log it
        if let Some(opt) = c.as_option() {
            let (btc_price, now) = self.current_price();
            if !opt.in_the_money(btc_price) && opt.expiry >= now {
                let option = c.as_option().unwrap();
                let ddelta80 = option.bs_dual_delta(now, btc_price, 0.80);
                if ddelta80.abs() < 0.01 {
                    print!("Interesting contract: ");
                    option.print_option_data(now, btc_price);
                }
            }
        }
        // Add it to the tracker
        self.contracts.insert(c.id(), (c, BookState::new()));
    }

    /// Remove a contract from the tracker
    pub fn remove_contract(&mut self, c_id: ContractId) {
        self.contracts.remove(&c_id);
    }

    /// Inserts a new order into the book
    pub fn insert_order(&mut self, order: Order) -> UpdateResponse {
        let (_, now) = self.current_price(); // get price reference prior to mutable borrow
        let (contract, book_state) = match self.contracts.get_mut(&order.contract_id) {
            Some(c) => (&mut c.0, &mut c.1),
            None => return UpdateResponse::Ignored,
        };
        // Insert the order and signal if the best bid/ask has changed.
        book_state.insert_order(order);
        let new_bb = book_state.best_bid();
        let new_ba = book_state.best_ask();

        let is_bb = order.bid_ask == Bid && new_bb.0 == order.price;
        let is_ba = order.bid_ask == Ask && new_ba.0 == order.price;
        if is_bb || is_ba {
            match contract.ty() {
                // For option we just log
                contract::Type::Option { opt, .. } => {
                    log_if_interesting(&order, &opt, now, self.last_btc_bid, self.last_btc_ask);
                }
                // For day-ahead swaps update the current BTC price reference
                contract::Type::NextDay { .. } => {
                    if is_bb {
                        self.last_btc_bid = new_bb.0;
                    }
                    if is_ba {
                        self.last_btc_ask = new_ba.0;
                    }
                    self.last_btc_time = order.timestamp;
                }
                contract::Type::Future { .. } => { /* ignore */ }
            }
        }
        if is_bb {
            UpdateResponse::NewBestBid {
                contract: contract,
                price: new_bb.0,
                size: new_bb.1,
            }
        } else if is_ba {
            UpdateResponse::NewBestAsk {
                contract: contract,
                price: new_ba.0,
                size: new_ba.1,
            }
        } else {
            UpdateResponse::Accepted
        }
    }

    /// Initializes the orderbook with the date from the book state API endpoint
    pub fn initialize_orderbooks(
        &mut self,
        data: json::BookStateMessage,
        timestamp: OffsetDateTime,
    ) {
        for order in data.data.book_states {
            self.insert_order(Order::from((order, timestamp)));
        }
    }
}
