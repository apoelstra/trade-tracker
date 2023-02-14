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

//! Data Feed
//!
//! Streaming data from the data feed
//!

use super::{json, Contract, ContractId};
use crate::units::{Price, UnknownQuantity};
use serde::Deserialize;
use std::fmt;
use time::OffsetDateTime;

/// ID of a customer; provided only for own trades
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct CustomerId(usize);

impl fmt::Display for CustomerId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// ID of a specific message, which is the same across an order submission/edit/cancel/etc
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct MessageId([u8; 16]);

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        bitcoin::hashes::hex::format_hex(&self.0, f)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum BidAsk {
    Bid,
    Ask,
}
pub use BidAsk::{Ask, Bid};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Order {
    /// Whether the order is a bid or an ask
    pub bid_ask: BidAsk,
    /// Number of contracts
    pub size: i64,
    /// Number of contracts filled
    pub filled_size: UnknownQuantity,
    /// Limit price
    pub price: Price,
    /// ID of the contract being bid/ask on
    pub contract_id: ContractId,
    /// ID of the customer, if provided (only provided for own trades)
    pub customer_id: Option<CustomerId>,
    /// ID of the manifest
    pub message_id: MessageId,
    /// Timestamp that the order occured on
    pub timestamp: OffsetDateTime,
    /// Timestamp that the order was last updated on
    pub updated_timestamp: OffsetDateTime,
}

impl From<(json::BookState, OffsetDateTime)> for Order {
    fn from(data: (json::BookState, OffsetDateTime)) -> Self {
        Order {
            bid_ask: if data.0.is_ask { Ask } else { Bid },
            size: data.0.size,
            filled_size: UnknownQuantity::from(0), // not provided for book states, assume 0
            contract_id: data.0.contract_id,
            price: data.0.price,
            customer_id: None, // not provided for book states
            message_id: MessageId(data.0.mid),
            updated_timestamp: data.1,
            timestamp: data.1,
        }
    }
}

/// Object from the data stream
#[derive(Clone, PartialEq, Eq, Hash, Debug, Deserialize)]
#[serde(from = "json::DataFeedObject")]
pub enum Object {
    /// A customer limit order
    Order(Order),
    BookTop {
        contract_id: ContractId,
        ask: Price,
        ask_size: i64,
        bid: Price,
        bid_size: i64,
    },
    AvailableBalances {
        btc: bitcoin::Amount,
        usd: Price,
    },
    ContractAdded(Contract),
    ContractRemoved(ContractId),
    Other,
}

impl From<json::DataFeedObject> for Object {
    fn from(js: json::DataFeedObject) -> Self {
        match js {
            json::DataFeedObject::ActionReport {
                contract_id,
                price,
                size,
                filled_size,
                is_ask,
                cid,
                mid,
                timestamp,
                updated_time,
                ..
            } => Object::Order(Order {
                contract_id,
                customer_id: cid.map(CustomerId),
                message_id: MessageId(mid),
                size,
                filled_size: UnknownQuantity::from(filled_size),
                price,
                bid_ask: if is_ask { Ask } else { Bid },
                timestamp,
                updated_timestamp: updated_time,
            }),
            json::DataFeedObject::BookTop {
                contract_id,
                ask,
                ask_size,
                bid,
                bid_size,
                ..
            } => Object::BookTop {
                contract_id,
                ask,
                ask_size,
                bid,
                bid_size,
            },
            json::DataFeedObject::CollateralBalanceUpdate { collateral } => {
                Object::AvailableBalances {
                    btc: collateral.available_balances.btc,
                    usd: collateral.available_balances.usd,
                }
            }
            json::DataFeedObject::ContractAdded { data } => Object::ContractAdded(data),
            json::DataFeedObject::ContractRemoved { data } => Object::ContractRemoved(data.id()),
            _ => Object::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ask() {
        let order_s = "{\"canceled_size\": 52, \"updated_time\": 1674839748016616735, \"original_size\": 52, \"mid\": \"014aa5ad13564272a793c0582a776000\", \"vwap\": 0, \"timestamp\": 1674839748016616735, \"filled_size\": 0, \"status_reason\": 0, \"ticks\": 1674839748016616735, \"clock\": 173827, \"filled_price\": 0, \"type\": \"action_report\", \"order_type\": \"customer_limit_order\", \"inserted_price\": 0, \"original_price\": 126400, \"inserted_size\": 0, \"size\": 0, \"is_ask\": true, \"open_interest\": 248, \"price\": 0, \"inserted_time\": 1674834303810514441, \"is_volatile\": true, \"status_type\": 203, \"contract_id\": 22256362}";
        let obj: Object = serde_json::from_str(order_s).unwrap();

        assert_eq!(
            obj,
            Object::Order(Order {
                bid_ask: Ask,
                size: 0,
                price: Price::ZERO,
                contract_id: ContractId::from(22256362),
                customer_id: None,
                message_id: MessageId([
                    0x01, 0x4a, 0xa5, 0xad, 0x13, 0x56, 0x42, 0x72, 0xa7, 0x93, 0xc0, 0x58, 0x2a,
                    0x77, 0x60, 0x00,
                ]),
                timestamp: OffsetDateTime::from_unix_timestamp_nanos(1674839748016616735),
            })
        );
    }
}
