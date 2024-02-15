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
use crate::units::{Price, UnknownQuantity, UtcTime};
use serde::Deserialize;
use std::fmt;

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
        use hex_conservative::DisplayHex as _;
        fmt::Display::fmt(&self.0.as_hex(), f)
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Order {
    /// Number of contracts (negative for asks, positive for bids)
    pub size: UnknownQuantity,
    /// Number of contracts filled
    pub filled_size: UnknownQuantity,
    /// Price at which fill happened
    pub filled_price: Price,
    /// Limit price
    pub price: Price,
    /// ID of the contract being bid/ask on
    pub contract_id: ContractId,
    /// ID of the customer, if provided (only provided for own trades)
    pub customer_id: Option<CustomerId>,
    /// ID of the manifest
    pub message_id: MessageId,
    /// Timestamp that the order occured on
    pub timestamp: UtcTime,
    /// Timestamp that the order was last updated on
    pub updated_timestamp: UtcTime,
}

impl fmt::Display for Order {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "order {} (contract {}",
            self.message_id, self.contract_id,
        )?;
        if self.size.is_nonzero() {
            write!(f, ", size {} @ {}", self.size, self.price,)?;
        }
        if self.filled_size.is_nonzero() {
            write!(f, ", filled {} @ {}", self.filled_size, self.filled_price,)?;
        }
        if let Some(cid) = self.customer_id {
            write!(f, ", {cid}")?;
        }
        write!(
            f,
            ", timestamp {} updated {})",
            self.timestamp.format("%FT%H:%M:%S.%f%z"),
            self.updated_timestamp.format("%FT%H:%M:%S.%f%%z"),
        )
    }
}

impl From<(json::BookState, UtcTime)> for Order {
    fn from(data: (json::BookState, UtcTime)) -> Self {
        let ba_mult = if data.0.is_ask { -1 } else { 1 };
        Order {
            size: UnknownQuantity::from(ba_mult * data.0.size),
            filled_size: UnknownQuantity::from(0), // not provided for book states, assume 0
            filled_price: Price::ZERO,             // not provided for book states, assume 0
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
    ChatMessage {
        message: String,
        initiator: String,
        counterparty: String,
        chat_id: usize,
    },
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
                filled_price,
                is_ask,
                cid,
                mid,
                timestamp,
                updated_time,
                ..
            } => {
                let ba_mult = if is_ask { -1 } else { 1 };
                Object::Order(Order {
                    contract_id,
                    customer_id: cid.map(CustomerId),
                    message_id: MessageId(mid),
                    size: UnknownQuantity::from(ba_mult * size),
                    filled_size: UnknownQuantity::from(ba_mult * filled_size),
                    filled_price,
                    price,
                    timestamp,
                    updated_timestamp: updated_time,
                })
            }
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
            json::DataFeedObject::ConversationNewMessage {
                data,
                conversation_id,
            } => Object::ChatMessage {
                message: data.message.message,
                initiator: data.message.initiator.chat_username,
                counterparty: data.message.counterparty.chat_username,
                chat_id: conversation_id,
            },
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
                filled_price: Price::ZERO,
                filled_size: UnknownQuantity::from(0),
                size: UnknownQuantity::from(0),
                price: Price::ZERO,
                contract_id: ContractId::from(22256362),
                customer_id: None,
                message_id: MessageId([
                    0x01, 0x4a, 0xa5, 0xad, 0x13, 0x56, 0x42, 0x72, 0xa7, 0x93, 0xc0, 0x58, 0x2a,
                    0x77, 0x60, 0x00,
                ]),
                timestamp: UtcTime::from_unix_nanos_i64(1674839748016616735).unwrap(),
                updated_timestamp: UtcTime::from_unix_nanos_i64(1674839748016616735).unwrap(),
            })
        );
    }
}
