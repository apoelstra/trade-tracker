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
                let ddelta80 = option.bs_dual_delta(&now, btc_price, 0.80);
                if ddelta80.abs() < 0.01 {
                    print!("Interesting contract: ");
                    option.print_option_data(&now, btc_price);
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
        let (btc_price, now) = self.current_price(); // get price reference prior to mutable borrow
        let (contract, book_state) = match self.contracts.get_mut(&order.contract_id) {
            Some(c) => (&mut c.0, &mut c.1),
            None => return UpdateResponse::Ignored,
        };
        // For day-ahead swaps, update the price reference. For options, (maybe) log.
        if let contract::Type::Option { opt, .. } = contract.ty() {
            if order.size > 10 && order.price > Decimal::from(5) {
                if let Ok(vol) = opt.bs_iv(&now, btc_price, order.price) {
                    let ddelta80 = opt.bs_dual_delta(&now, btc_price, 0.80);
                    if order.bid_ask == Bid && (vol > 0.8 || ddelta80.abs() < 0.05) {
                        print!("Interesting bid: ");
                        opt.print_option_data(&now, btc_price);
                        print!("    Price: ");
                        opt.print_order_data(&now, btc_price, order.price, order.size);
                    }
                } else if opt.in_the_money(btc_price) && order.bid_ask == Ask {
                    if opt.strike + order.price < btc_price - Decimal::from(250) {
                        print!("Apparent free money offer: ");
                        opt.print_option_data(&now, btc_price);
                        print!("    Price: ");
                        opt.print_order_data(&now, btc_price, order.price, order.size);
                        println!(
                            "       (strike {} + price {} is {}, vs BTC price {}",
                            opt.strike,
                            order.price,
                            opt.strike + order.price,
                            btc_price,
                        );
                    }
                }
            }
        }
        // Insert the order and signal if the best bid/ask has changed.
        let old_bb = book_state.best_bid();
        let old_ba = book_state.best_ask();
        book_state.insert_order(order);
        let new_bb = book_state.best_bid();
        let new_ba = book_state.best_ask();
        // For day-ahead swaps update the current BTC price reference
        if let contract::Type::NextDay { .. } = contract.ty() {
            self.last_btc_bid = new_bb.0;
            self.last_btc_ask = new_ba.0;
            self.last_btc_time = order.timestamp;
        }
        if old_bb != new_bb {
            UpdateResponse::NewBestBid {
                contract: contract,
                price: new_bb.0,
                size: new_bb.1,
            }
        } else if old_ba != new_ba {
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
