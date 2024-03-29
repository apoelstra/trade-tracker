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
pub mod interesting;
pub mod json;
pub mod own_orders;

use self::interesting::{AskStats, BidStats};
use self::json::CreateOrder;
use crate::price::BitcoinPrice;
use crate::terminal::ColorFormat;
use crate::units::{Asset, Price, Quantity, Underlying, UtcTime};
use log::{debug, info, warn};
use serde::Deserialize;
use serde_json;
use std::collections::HashMap;
use std::sync::mpsc::Sender;

pub use book::BookState;
pub use contract::{Contract, ContractId};
pub use datafeed::{CustomerId, MessageId};

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
    price_ref: BitcoinPrice,
    own_orders: own_orders::Tracker,
    available_usd: Price,
    available_btc: bitcoin::Amount,
}

#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub enum OrderResponse {
    /// This order was our own
    OursOk,
    /// This order was our own and it was filled!
    OursFilled,
    /// Update was accepted into order book; no new interesting info
    OtherTracked,
    /// Order was ignored because it was a non-BTC order or otherwise
    /// categorically ignored.
    OtherUntracked,
    /// Order was ignored because we didn't know the contract
    UnknownContract(datafeed::Order),
}

impl LedgerX {
    /// Create a new empty LX tracker
    pub fn new(btc_price: crate::price::BitcoinPrice) -> Self {
        LedgerX {
            contracts: HashMap::new(),
            own_orders: own_orders::Tracker::new(),
            price_ref: btc_price,
            available_usd: Price::ZERO,
            available_btc: bitcoin::Amount::ZERO,
        }
    }

    /// Sets the "available balances" counter
    pub fn set_balances(&mut self, usd: Price, btc: bitcoin::Amount) {
        if self.available_usd != usd || self.available_btc != btc {
            info!("Update balances: ${}, {}", usd, btc);
        }
        self.available_usd = usd;
        self.available_btc = btc;
    }

    /// Reduces the available balances on the assumption that a recently-opened
    /// order will be taken.
    ///
    /// Takes current balances as &mut pointers rather than taking &mut self
    /// because of borrowck dumbness. But you should always call this by passing
    /// `&mut self.available_usd, &mut self.available_btc`.
    ///
    /// If this is wrong, then the system will be in an incorrect state until
    /// the next call to [`set_balances`]. It may therefore fail to make trades
    /// that it should. But because it will have an incorrectly low view of
    /// the current balances, this is "harmless" in that it won't try to make
    /// trades it shouldn't.
    pub fn preemptively_dock_balances(
        available_usd: &mut Price,
        available_btc: &mut bitcoin::Amount,
        usd: Price,
        btc: bitcoin::Amount,
    ) {
        *available_usd -= usd;
        *available_btc -= btc;
        if usd != Price::ZERO || btc != bitcoin::Amount::ZERO {
            info!(
                "Preemptively docking balances by ${}, {} to ${}, {}",
                usd, btc, available_usd, available_btc
            );
        }
    }

    /// Updates the price reference.
    pub fn set_current_price(&mut self, price: BitcoinPrice) {
        self.price_ref = price;
    }

    /// Go through the list of all open orders and log them all
    pub fn log_open_orders(&self) {
        for order in self.own_orders.open_order_iter() {
            if let Some((contract, _)) = self.contracts.get(&order.contract_id) {
                let size = order.size.with_asset_trade(contract.asset());
                match contract.ty() {
                    contract::Type::Option { opt, .. } => {
                        info!("Open order {}:", order.message_id);
                        opt.log_option_data(
                            "    ",
                            self.price_ref.timestamp,
                            self.price_ref.btc_price,
                        );
                        opt.log_order_data(
                            "    ",
                            self.price_ref.timestamp,
                            self.price_ref.btc_price,
                            order.price,
                            Some(size),
                        );
                        info!("");
                    }
                    contract::Type::NextDay { .. } => {
                        info!(
                            "Open order {}: {} BTC @ {}",
                            order.message_id, size, order.price
                        );
                    }
                    contract::Type::Future { .. } => {
                        info!(
                            "Open order {}: {} future?? @ {}",
                            order.message_id, size, order.price
                        );
                    }
                }
            } else {
                warn!(
                    "Have open order for CID {} that we're not tracking.",
                    order.contract_id
                );
            }
        }
    }

    /// Go through the list of all contracts we're tracking and open standing orders on them.
    ///
    /// This function is the core (arguably, the entirety) of our trading algo. Currently
    /// it opens limit asks on each contract subject to various constraints:
    ///
    /// 1. It must have a sufficiently high IV and ARR, and sufficiently low loss80.
    /// 2. The IV must not be too high (otherwise the order is just dumb and LX will
    ///    probably flag me for it).
    ///
    /// If these conditions can't be simultaneously met, no order is opened.
    pub fn open_standing_orders(&mut self, tx: &Sender<crate::connect::Message>) {
        let mut order_count = 0;
        let now = UtcTime::now();
        for cid in self.contracts.keys() {
            if let Some((c, book)) = self.contracts.get(cid) {
                if let Some(stats) = AskStats::standing_order(
                    self.price_ref,
                    c,
                    self.available_usd,
                    self.available_btc,
                    book.best_ask().0,
                ) {
                    // for now just log
                    let opt = match interesting::extract_option(c, self.price_ref) {
                        Some(opt) => opt,
                        None => continue,
                    };

                    let msg;
                    if stats.order_size().is_positive() {
                        msg = ColorFormat::white("Sell to open: ");
                        order_count += 1;
                        let order =
                            CreateOrder::new_ask(c, stats.order_size(), stats.order_price());
                        tx.send(crate::connect::Message::OpenOrder(order)).unwrap();
                    } else {
                        msg = ColorFormat::pale_yellow("  Would sell: ");
                    }

                    opt.log_option_data(&msg, now, self.price_ref.btc_price);
                    opt.log_order_data(
                        &msg,
                        now,
                        self.price_ref.btc_price,
                        stats.order_price(),
                        Some(stats.order_size()),
                    );
                    info!("");
                }
            }
        }
        info!("Opened {} orders.", order_count);
    }

    /// Go through the list of all contracts we're tracking and log the interesting ones
    pub fn log_interesting_contracts(&mut self, tx: &Sender<crate::connect::Message>) {
        for cid in self.contracts.keys() {
            if let Some((c, book)) = self.contracts.get(cid) {
                let (usd, btc) = self.log_interesting_contract(c, book, tx);
                // Pre-emptively dock our balances based on
                Self::preemptively_dock_balances(
                    &mut self.available_usd,
                    &mut self.available_btc,
                    usd,
                    btc,
                );
            }
        }
    }

    /// Log a single interesting contract
    ///
    /// This function may do more than log -- it may attempt to match bids that are
    /// interesting. In this case, it returns the total USD and BTC committed.
    fn log_interesting_contract(
        &self,
        c: &Contract,
        book: &BookState,
        tx: &Sender<crate::connect::Message>,
    ) -> (Price, bitcoin::Amount) {
        let btc_price = self.price_ref;
        let now = UtcTime::now();
        // Extract option, assuming it matches the relevant parameters
        // (is an option, hasn't expired, BTC not ETH, etc)
        let opt = match interesting::extract_option(c, self.price_ref) {
            Some(opt) => opt,
            None => return (Price::ZERO, bitcoin::Amount::ZERO),
        };

        // Compute the yield threshold below which the absolute return
        // is too low to be worth logging (though it may be worth acting
        // on autonomously). We set this to $25/day which is roughly $750/mo
        // for now.
        let dte = opt.years_to_expiry(now) * 365.0;
        let yield_threshold = Price::TWENTY_FIVE.scale_approx(dte);

        // Iterate through all open bids.
        let mut available_usd = self.available_usd;
        let mut available_btc = self.available_btc;

        let mut best_bid = match BidStats::from_order(btc_price, c, Price::ZERO, Quantity::Zero) {
            Some(stat) => stat,
            None => return (Price::ZERO, bitcoin::Amount::ZERO),
        };
        let mut acc = best_bid;
        let mut acc_current_funds = best_bid;

        let mut asks_to_make = vec![];

        for bid in book.bids() {
            let mut stat = match BidStats::from_order(btc_price, c, bid.price, bid.size) {
                Some(stat) => stat,
                None => break,
            };
            // Once one order is uninteresting, the rest will be.
            if stat.interestingness() <= interesting::Interestingness::No {
                break;
            }

            // Skip 0-size bids which sometimes show up on LX
            if bid.size.is_zero() {
                continue;
            }

            // Record unadjusted values
            if best_bid.order_size().is_zero() {
                best_bid = stat;
            }
            acc += stat;

            // Adjust for available funds
            if available_usd < stat.lockup_usd() || available_btc < stat.lockup_btc() {
                stat.limit_to_funds(available_usd, available_btc);
            }
            available_usd -= stat.lockup_usd();
            available_btc -= stat.lockup_btc();
            acc_current_funds += stat;

            if stat.interestingness() >= interesting::Interestingness::Take
                && stat.order_size().is_positive()
            {
                asks_to_make.push(stat.corresponding_ask());
            }

            // Once we're out of money no point in continuing to loop through bids
            if available_usd == Price::ZERO || available_btc == bitcoin::Amount::ZERO {
                break;
            }
        }

        // Once we've looped through the order book, log what we found.
        let mut ret_usd = Price::ZERO;
        let mut ret_btc = bitcoin::Amount::ZERO;
        if best_bid.order_size().is_positive() && acc.total_value() > yield_threshold {
            // Log the non-order-specific contract data.
            opt.log_option_data(
                ColorFormat::light_purple("Interesting contract: "),
                now,
                btc_price.btc_price,
            );

            if best_bid.total_value() > yield_threshold {
                opt.log_order_data(
                    "            Best Bid: ",
                    now,
                    btc_price.btc_price,
                    best_bid.order_price(),
                    Some(best_bid.order_size()),
                );
            }
            if best_bid != acc {
                opt.log_order_data(
                    "     Accum. Good Bid: ",
                    now,
                    btc_price.btc_price,
                    acc.order_price(),
                    Some(acc.order_size()),
                );
            }
            if acc_current_funds != acc {
                opt.log_order_data(
                    "With available funds: ",
                    now,
                    btc_price.btc_price,
                    acc_current_funds.order_price(),
                    Some(acc_current_funds.order_size()),
                );
            }
            for ask in asks_to_make {
                opt.log_order_data(
                    ColorFormat::white("     Selling to take: "),
                    now,
                    btc_price.btc_price,
                    ask.order_price(),
                    Some(ask.order_size()),
                );
                let order = CreateOrder::new_ask(c, ask.order_size(), ask.order_price());
                tx.send(crate::connect::Message::OpenOrder(order)).unwrap();
                ret_usd += ask.lockup_usd();
                ret_btc += ask.lockup_btc();
            }
        }
        (ret_usd, ret_btc)
    }

    /// Add a new contract to the tracker
    ///
    /// Some checks will be done as to whether this is an "interesting" option
    /// at the current price, and if so, we print a log message.
    pub fn add_contract(&mut self, c: Contract) {
        debug!("Add contract {}: {}", c.id(), c.label());
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
    pub fn insert_order(&mut self, order: datafeed::Order) -> OrderResponse {
        let (contract, book_state) = match self.contracts.get_mut(&order.contract_id) {
            Some(c) => (&mut c.0, &mut c.1),
            None => {
                debug!(
                    "Received order mid {} for unknown contract {}",
                    order.message_id, order.contract_id,
                );
                return OrderResponse::UnknownContract(order);
            }
        };
        if contract.underlying() != Underlying::Btc {
            debug!(
                "Ignoring order mid {} for non-BTC contract {}",
                order.message_id, order.contract_id,
            );
            return OrderResponse::OtherUntracked;
        }
        // Insert into order book
        debug!("Inserting into contract {}: {}", contract.id(), order);
        // Before doing anything else, track this if it's an own-order
        if order.customer_id.is_some() {
            book_state.insert_order(order.clone()); // line duplicated for borrowck
            if self
                .own_orders
                .insert_order(contract, order, self.price_ref)
            {
                OrderResponse::OursFilled
            } else {
                OrderResponse::OursOk
            }
        } else {
            book_state.insert_order(order); // line duplicated for borrowck
            OrderResponse::OtherTracked
        }
    }

    /// Deletes all open orders at the end of the day
    pub fn clear_orderbooks(&mut self) {
        self.contracts = HashMap::new();
    }

    /// Initializes the orderbook with the date from the book state API endpoint
    pub fn initialize_orderbooks(
        &mut self,
        data: json::BookStateMessage,
        timestamp: UtcTime,
        tx: &Sender<crate::connect::Message>,
    ) {
        // Delete existing data
        if let Some((contract, ref mut book_state)) = self.contracts.get_mut(&data.data.contract_id)
        {
            *book_state = BookState::new(contract.asset());
            if contract.asset() == Asset::Btc {
                // We don't use the LX orderbook as a price reference at all
                //self.price_ref.clear_book();
            }
        }
        for order in data.data.book_states {
            self.insert_order(datafeed::Order::from((order, timestamp)));
        }
        if let Some((c, book)) = self.contracts.get(&data.data.contract_id) {
            let (usd, btc) = self.log_interesting_contract(c, book, tx);
            // Pre-emptively dock our balances based on
            Self::preemptively_dock_balances(
                &mut self.available_usd,
                &mut self.available_btc,
                usd,
                btc,
            );
        }
    }
}
