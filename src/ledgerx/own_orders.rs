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

use crate::ledgerx::{datafeed::Order, Contract, CustomerId, MessageId};
use crate::units::UnknownQuantity;
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

    pub fn insert_order(&mut self, contract: &Contract, order: Order) {
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

        if order.size == UnknownQuantity::from(0) {
            // Is this a deletion?
            self.map.remove(&order.message_id);
            info!(
                "Deleted order: id {} contract {}",
                order.message_id, contract,
            );
        } else if let Some(existing) = self.map.get(&order.message_id) {
            // Or an update?
            if existing.updated_timestamp != order.updated_timestamp {
                let filled_size = order.filled_size.with_asset(contract.asset());
                if existing.size != order.size || existing.price != order.price {
                    info!(
                        "Updated order: id {} contract {}, size {}, price {}",
                        order.message_id,
                        contract,
                        order.size.with_asset(contract.asset()),
                        order.price,
                    );
                }
                if filled_size.is_nonzero() {
                    info!(
                        "Filled order: id {} contract {}, filled size {}, price {}",
                        order.message_id, contract, filled_size, order.price,
                    );
                }
            }
            self.map.insert(order.message_id, order);
        } else {
            // Or a new order?
            info!(
                "Created order: id {} contract {}, size {}, price {}",
                order.message_id,
                contract,
                order.size.with_asset(contract.asset()),
                order.price,
            );
            self.map.insert(order.message_id, order);
        }
    }
}
