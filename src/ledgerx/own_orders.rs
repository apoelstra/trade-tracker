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

//! LedgerX Own-Orders
//!
//! Data about orders that belong to us
//!

use crate::ledgerx::{contract, datafeed::Order, Contract, CustomerId, MessageId};
use crate::price::BitcoinPrice;
use crate::units::{Price, Quantity, UnknownQuantity};
use log::{info, warn};
use std::collections::HashMap;

/// Own-order tracker
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Tracker {
    my_id: Option<CustomerId>,
    map: HashMap<MessageId, Order>,
}

impl Tracker {
    /// Create a new empty order tracker
    pub fn new() -> Self {
        Default::default()
    }

    /// Inserts the order into the own-order tracker.
    ///
    /// Returns a boolean indicating whether this was an order fill (true) or
    /// something else (false).
    pub fn insert_order(
        &mut self,
        contract: &Contract,
        order: Order,
        price_ref: BitcoinPrice,
    ) -> bool {
        // First log anything interesting about the CID.
        match (self.my_id, order.customer_id) {
            (_, None) => {
                warn!("Recevied \"own order\" without customer ID. This is a bug.");
            }
            (None, Some(them)) => {
                info!(
                    "Setting my customer ID to {} based on first own-order.",
                    them
                );
                self.my_id = Some(them);
            }
            (Some(me), Some(them)) => {
                if me != them {
                    warn!(
                        "Received \"own order\" for customer ID {}, but our ID is {}.",
                        them, me
                    );
                }
            }
        }

        let mut ret = false;
        let mid = order.message_id;
        let (msg, size, price) = if order.size == UnknownQuantity::from(0) {
            // A deletion or fill?
            let filled_size = order.filled_size.with_asset_trade(contract.asset());
            if filled_size.is_nonzero() {
                // For fills specifically send a text
                let message = &format!(
                    "LedgerX filled order\n\
                    {}: {} @ {}\n\
                    ID {}\n\
                    BTC Price {}",
                    contract,
                    filled_size,
                    order.filled_price,
                    order.message_id,
                    price_ref.btc_price,
                );
                crate::http::post_to_prowl(&message);
                ret = true;
                ("Filled ", filled_size, order.filled_price)
            } else if let Some(old_order) = self.map.remove(&order.message_id) {
                (
                    "Deleted ",
                    old_order.size.with_asset_trade(contract.asset()),
                    old_order.filled_price,
                )
            } else {
                warn!(
                    "Deleted order {} for {} which we weren't tracking.",
                    order.message_id, contract
                );
                ("", Quantity::Zero, Price::ZERO)
            }
        } else if let Some(existing) = self.map.get(&order.message_id) {
            // Or an update?
            let data = if existing.updated_timestamp != order.updated_timestamp {
                (
                    "Updated ",
                    order.size.with_asset_trade(contract.asset()),
                    order.price,
                )
            } else {
                ("", Quantity::Zero, Price::ZERO)
            };
            self.map.insert(order.message_id, order);
            data
        } else {
            // Or a new order?
            let data = (
                "Created ",
                order.size.with_asset_trade(contract.asset()),
                order.price,
            );
            self.map.insert(order.message_id, order);
            data
        };

        // Log it
        if !msg.is_empty() {
            match contract.ty() {
                contract::Type::Option { opt, .. } => {
                    info!("{}order {}", msg, mid);
                    opt.log_option_data(msg, price_ref.timestamp, price_ref.btc_price);
                    opt.log_order_data(
                        msg,
                        price_ref.timestamp,
                        price_ref.btc_price,
                        price,
                        Some(size),
                    );
                    info!("");
                }
                contract::Type::NextDay { .. } => {
                    info!("{} order {}: {} BTC @ {}", msg, mid, size, price);
                }
                contract::Type::Future { .. } => {
                    info!("{} order {}: {} future?? @ {}", msg, mid, size, price);
                }
            }
        }
        ret
    }

    /// Get an iterator over all open orders
    pub fn open_order_iter(&self) -> impl Iterator<Item = &Order> {
        self.map.values()
    }
}
