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
use crate::units::Price;
use serde::Deserialize;
use std::fmt;
use time::OffsetDateTime;

/// ID of a specific "manifest", which is the same across an order submission/edit/cancel/etc
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ManifestId(pub [u8; 16]);

impl fmt::Display for ManifestId {
    #[rustfmt::skip]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}",
            self.0[0], self.0[1], self.0[2], self.0[3],
            self.0[4], self.0[5], self.0[6], self.0[7],
            self.0[8], self.0[9], self.0[10], self.0[11],
            self.0[12], self.0[13], self.0[14], self.0[15],
        )
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
    /// Limit price
    pub price: Price,
    /// ID of the contract being bid/ask on
    pub contract_id: ContractId,
    /// ID of the manifest
    pub manifest_id: ManifestId,
    /// Timestamp that the order occured on
    pub timestamp: OffsetDateTime,
}

impl From<(json::BookState, OffsetDateTime)> for Order {
    fn from(data: (json::BookState, OffsetDateTime)) -> Self {
        Order {
            bid_ask: if data.0.is_ask { Ask } else { Bid },
            size: data.0.size,
            contract_id: data.0.contract_id,
            price: data.0.price,
            manifest_id: ManifestId(data.0.mid),
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
                is_ask,
                mid,
                timestamp,
                ..
            } => Object::Order(Order {
                contract_id,
                manifest_id: ManifestId(mid),
                size,
                price,
                bid_ask: if is_ask { Ask } else { Bid },
                timestamp,
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
                manifest_id: ManifestId([
                    0x01, 0x4a, 0xa5, 0xad, 0x13, 0x56, 0x42, 0x72, 0xa7, 0x93, 0xc0, 0x58, 0x2a,
                    0x77, 0x60, 0x00,
                ]),
                timestamp: OffsetDateTime::from_unix_timestamp_nanos(1674839748016616735),
            })
        );
    }
}
