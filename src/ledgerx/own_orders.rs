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
use crate::units::{Price, Quantity, UnknownQuantity, UtcTime};
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

    pub fn insert_order(&mut self, contract: &Contract, order: Order, price_ref: (Price, UtcTime)) {
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
                    contract, filled_size, order.filled_price, order.message_id, price_ref.0,
                );
                let encoded = urlencoding::encode(&message);
                let body = format!(
                    "apikey=71d4fa4bfa2a49c69ebb470594be2e079b05006d\
                    &application=lx-trade-tracker\
                    &event=filled-trade\
                    &description={encoded}"
                );
                if let Err(e) = minreq::post("https://api.prowlapp.com/publicapi/add")
                    .with_timeout(10)
                    .with_header("Content-type", "application/x-www-form-urlencoded")
                    .with_body(body.clone())
                    .send()
                {
                    warn!("Sending message to Prowl failed: {}", e);
                    warn!("{}", body);
                }
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
                    opt.log_option_data(msg, price_ref.1, price_ref.0);
                    opt.log_order_data(msg, price_ref.1, price_ref.0, price, Some(size));
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
    }

    /// Get an iterator over all open orders
    pub fn open_order_iter(&self) -> impl Iterator<Item = &Order> {
        self.map.values()
    }
}
