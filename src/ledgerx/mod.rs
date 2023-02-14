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
pub mod csv;
pub mod datafeed;
pub mod history;
pub mod json;

use crate::terminal::format_color;
use crate::units::{Price, Quantity, Underlying};
use log::{debug, info};
use serde::Deserialize;
use serde_json;
use std::collections::HashMap;
use time::OffsetDateTime;

pub use book::BookState;
pub use contract::{Contract, ContractId};
pub use datafeed::{BidAsk::Ask, BidAsk::Bid, ManifestId};
pub use history::tax::LotId;

/// Thresholds of interestingness
#[derive(Copy, Clone, Debug)]
pub struct Interestingness {
    pub min_arr: f64,
    pub min_vol: f64,
    pub max_loss80: f64,
    pub min_size: Quantity,
    pub min_yield_per_week: Price,
}

/// Threshold for a bid to be interesting enough for us to snipe
pub static BID_INTERESTING: Interestingness = Interestingness {
    min_arr: 0.05,
    min_vol: 0.5,
    max_loss80: 0.20,
    min_size: Quantity::Contracts(1),
    min_yield_per_week: Price::TWENTY_FIVE,
};
/// Threshold for a ask to be interesting enough for us to match
pub static ASK_INTERESTING: Interestingness = Interestingness {
    min_arr: 0.10,
    min_vol: 0.75,
    max_loss80: 0.10,
    min_size: Quantity::Contracts(1),
    min_yield_per_week: Price::TWENTY_FIVE,
};

impl Interestingness {
    pub fn is_interesting(
        &self,
        opt: &crate::option::Option,
        now: OffsetDateTime,
        btc_price: Price,
        order_price: Price,
        order_size: Quantity,
    ) -> bool {
        // Easy check: size
        if order_size < self.min_size {
            return false;
        }
        let min_yield = self
            .min_yield_per_week
            .scale_approx(opt.years_to_expiry(now) * 52.0);
        if order_price * order_size < min_yield {
            return false;
        }
        // Next, if the option is "free money" we consider it uninteresting for
        // now. We will do a manual free-money check where it makessense.
        let vol = match opt.bs_iv(now, btc_price, order_price) {
            Ok(vol) => vol,
            Err(_) => return false,
        };

        let arr = opt.arr(now, btc_price, order_price);
        let loss80 = opt.bs_loss80(now, btc_price, order_price).abs();

        if opt.in_the_money(btc_price) {
            return false;
        } // Ignore ITM bids, we don't really have a strategy for shorting ITMs
        if arr < self.min_arr {
            return false;
        } // ignore low-yield bids
        if loss80 > self.max_loss80 {
            return false;
        } // ignore bids with high likelihood of loss
        if vol < self.min_vol {
            return false;
        }
        true
    }
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
    let json: Response<T> = serde_json::from_slice(data)?;
    Ok(json.data)
}

/// Tracker for the state of the entire LX book
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LedgerX {
    contracts: HashMap<ContractId, (Contract, BookState)>,
    available_usd: Price,
    available_btc: bitcoin::Amount,
    last_btc_bid: Price,
    last_btc_ask: Price,
    last_btc_time: OffsetDateTime,
}

#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub enum UpdateResponse {
    /// Update was accepted; no new interesting info
    Accepted,
    /// Order was ignored because we didn't know the contract
    UnknownContract(datafeed::Order),
    /// Order was ignored because it wasn't a bitcoin order
    NonBtcOrder,
    /// Update was accepted and was for a day-ahead sway
    AcceptedBtc,
}

impl LedgerX {
    /// Create a new empty LX tracker
    pub fn new(btc_price: crate::price::BitcoinPrice) -> Self {
        LedgerX {
            contracts: HashMap::new(),
            available_usd: Price::ZERO,
            available_btc: bitcoin::Amount::ZERO,
            last_btc_bid: btc_price.btc_price,
            last_btc_ask: btc_price.btc_price,
            last_btc_time: btc_price.timestamp,
        }
    }

    /// Sets the "available balances" counter
    pub fn set_balances(&mut self, usd: Price, btc: bitcoin::Amount) {
        if self.available_usd != usd || self.available_btc != btc {
            info!("Update balances: ${}, {} BTC", usd, btc);
        }
        self.available_usd = usd;
        self.available_btc = btc;
    }

    /// Returns the current BTC price, as seen by the tracker
    ///
    /// Initially uses a price reference supplied at construction (probably coming
    /// from the BTCCharts data ultimately); later will use the midpoint of the LX
    /// current bid/ask for day-ahead swaps.
    pub fn current_price(&self) -> (Price, OffsetDateTime) {
        (
            (self.last_btc_bid + self.last_btc_ask).half(),
            self.last_btc_time,
        )
    }

    /// Go through the list of all contracts we're tracking and log the interesting ones
    pub fn log_interesting_contracts(&mut self) {
        // The borrowck forces us to collect all the contract IDs into a vector,
        // because we can't have a live self.contracts.keys() iterator while calling
        // self.log_interesting_contract. This is wasteful but what are you gonna do.
        let cids: Vec<ContractId> = self.contracts.keys().copied().collect();
        for cid in cids {
            self.log_interesting_contract(cid);
        }
    }

    /// Log a single interesting contract
    fn log_interesting_contract(&mut self, cid: ContractId) {
        let (btc_price, now) = self.current_price();
        if let Some(&mut (ref mut c, ref mut book)) = self.contracts.get_mut(&cid) {
            // Only log BTC contracts
            if c.underlying() != Underlying::Btc {
                return;
            }
            if let Some(last_log) = c.last_log {
                // Refuse to log the same contract more than once every 4 hours
                if now - last_log < time::Duration::hours(4) {
                    return;
                }
            }
            // Log the contract itself
            if let Some(opt) = c.as_option() {
                let mut interesting = true;

                interesting &= !opt.in_the_money(btc_price); // Only OTM options
                interesting &= opt.expiry >= now; // only active options

                let ddelta80 = opt.bs_dual_delta(now, btc_price, 0.80);
                interesting &= ddelta80.abs() < 0.05; // only options with <5% chance of ending ITM

                // Compute yield from matching bids
                let (clear_bid_size, clear_bid_yield) =
                    book.clear_bids(&opt, self.available_usd, self.available_btc);
                // Compute yield from matching best ask, with as much money as we can
                let (ask_price, _) = book.best_ask();
                let (ask_size, _) = opt.max_sale(ask_price, self.available_usd, self.available_btc);
                let ask_yield = ask_price * ask_size;

                // only options where we could maybe make $100/wk
                let yield_limit = Price::ONE_HUNDRED.scale_approx(opt.years_to_expiry(now) * 52.0);
                interesting &= clear_bid_yield > yield_limit || ask_yield > yield_limit;
                if interesting {
                    opt.log_option_data(
                        format_color("Interesting contract: ", 250, 110, 250),
                        now,
                        btc_price,
                    );
                    if clear_bid_size.is_positive() {
                        let clear_price = clear_bid_yield / clear_bid_size;
                        opt.log_order_data(
                            "      Order to clear: ",
                            now,
                            btc_price,
                            clear_price,
                            Some(clear_bid_size),
                        );
                    }

                    let (bid_price, bid_size) = book.best_ask();
                    let (max_bid_size, _) =
                        opt.max_sale(bid_price, self.available_usd, self.available_btc);
                    opt.log_order_data(
                        "            Best bid: ",
                        now,
                        btc_price,
                        bid_price,
                        Some(bid_size.min(max_bid_size)),
                    );
                    opt.log_order_data(
                        " Best ask  (matched): ",
                        now,
                        btc_price,
                        ask_price,
                        Some(ask_size),
                    );
                    c.last_log = Some(now);
                }
            }
            // Log open orders
            if let contract::Type::Option { opt, .. } = c.ty() {
                book.log_interesting_orders(
                    &opt,
                    now,
                    self.last_btc_bid,
                    self.last_btc_ask,
                    self.available_usd,
                    self.available_btc,
                );
            }
        }
    }

    /// Add a new contract to the tracker
    ///
    /// Some checks will be done as to whether this is an "interesting" option
    /// at the current price, and if so, we print a log message.
    pub fn add_contract(&mut self, c: Contract) {
        info!("Add contract {}: {}", c.id(), c.label());
        let asset = c.asset();
        self.contracts.insert(c.id(), (c, BookState::new(asset)));
    }

    /// Remove a contract from the tracker
    pub fn remove_contract(&mut self, c_id: ContractId) {
        if let Some((c, _)) = self.contracts.remove(&c_id) {
            info!("Remove contract {}: {}", c.id(), c.label());
        } else {
            debug!("Removed unknown contract {}", c_id);
        }
    }

    /// Inserts a new order into the book
    pub fn insert_order(&mut self, order: datafeed::Order) -> UpdateResponse {
        let (contract, book_state) = match self.contracts.get_mut(&order.contract_id) {
            Some(c) => (&mut c.0, &mut c.1),
            None => {
                debug!(
                    "Received order mid {} for unknown contract {}",
                    order.manifest_id, order.contract_id,
                );
                return UpdateResponse::UnknownContract(order);
            }
        };
        if contract.underlying() != Underlying::Btc {
            debug!(
                "Ignoring order mid {} for non-BTC contract {}",
                order.manifest_id, order.contract_id,
            );
            return UpdateResponse::NonBtcOrder;
        }
        let timestamp = order.timestamp;
        // Insert the order and signal if the best bid/ask has changed.
        // Note that we check whether it changed by comparing the before-and-after values.
        // Anything "more clever" than this may fail to catch edge cases where e.g. the
        // previous best order has its size reduced to 0 and is dropped from the book.
        let old_bb = book_state.best_bid();
        let old_ba = book_state.best_ask();
        debug!("Inserting into contract {}: {:?}", contract.id(), order);
        book_state.insert_order(order);
        let new_bb = book_state.best_bid();
        let new_ba = book_state.best_ask();

        let is_bb = old_bb != new_bb;
        let is_ba = old_ba != new_ba;
        // For day-ahead swaps update the current BTC price reference
        if let contract::Type::NextDay { .. } = contract.ty() {
            if contract.underlying() == Underlying::Btc && (is_bb || is_ba) {
                if is_bb && new_bb.0 > Price::ZERO {
                    self.last_btc_bid = new_bb.0;
                }
                if is_ba && new_ba.0 > Price::ZERO {
                    self.last_btc_ask = new_ba.0;
                }
                self.last_btc_time = timestamp;
            }
            UpdateResponse::AcceptedBtc
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
            self.insert_order(datafeed::Order::from((order, timestamp)));
        }
        self.log_interesting_contract(data.data.contract_id);
    }
}
