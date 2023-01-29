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

use super::json;
use rust_decimal::Decimal;

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum BidAsk {
    Bid,
    Ask,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Order {
    /// Whether the order is a bid or an ask
    pub bid_ask: BidAsk,
    /// Number of contracts
    pub size: u64,
    /// Limit price
    pub price: Decimal,
    /// ID of the contract being bid/ask on
    pub contract_id: usize,
}

impl Order {
    /// Parse a contract from the JSON output from the LX /contracts API
    ///
    /// May return an error if we can't parse data; will return None if the given
    /// Json is not an order.
    pub fn from_json(json_: &serde_json::Value) -> Result<Option<Self>, String> {
        let json = match json_ {
            serde_json::Value::Object(json) => json,
            _ => return Err(format!("order json was not an object: {json_}")),
        };

        if json.get("order_type").and_then(|js| js.as_str()) != Some("customer_limit_order") {
            return Ok(None);
        }
        let ba = match json.get("is_ask") {
            Some(serde_json::Value::Bool(false)) => BidAsk::Bid,
            Some(serde_json::Value::Bool(true)) => BidAsk::Ask,
            Some(_) => {
                return Err(format!(
                    "Could not parse `is_ask` field in order stream {}",
                    json_
                ))
            }
            None => return Err(format!("No `is_ask` field in order stream {}", json_)),
        };

        Ok(Some(Order {
            bid_ask: ba,
            size: json::parse_num(&json, "size")?,
            price: Decimal::from(json::parse_num(&json, "price")?) / Decimal::from(100),
            contract_id: json::parse_num(&json, "contract_id")? as usize,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ask() {
        let order_s = "{\"canceled_size\": 52, \"updated_time\": 1674839748016616735, \"original_size\": 52, \"mid\": \"014aa5ad13564272a793c0582a776000\", \"vwap\": 0, \"timestamp\": 1674839748016616735, \"filled_size\": 0, \"status_reason\": 0, \"ticks\": 1674839748016616735, \"clock\": 173827, \"filled_price\": 0, \"type\": \"action_report\", \"order_type\": \"customer_limit_order\", \"inserted_price\": 0, \"original_price\": 126400, \"inserted_size\": 0, \"size\": 0, \"is_ask\": true, \"open_interest\": 248, \"price\": 0, \"inserted_time\": 1674834303810514441, \"is_volatile\": true, \"status_type\": 203, \"contract_id\": 22256362}";
        let order_json: serde_json::Value = serde_json::from_str(order_s).unwrap();
        let order = Order::from_json(&order_json).unwrap();

        assert_eq!(
            order,
            Some(Order {
                bid_ask: BidAsk::Ask,
                size: 0,
                price: Decimal::from(0),
                contract_id: 22256362,
            })
        );
    }
}
