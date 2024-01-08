// Trade Tracker
// Written in 2024 by
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

//! Coinbase
//!
//! Data Structures etc for the Coinbase Websockets API

use serde::{de, Deserialize, Deserializer};
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use time::OffsetDateTime;

fn deserialize_datetime<'de, D>(deser: D) -> Result<OffsetDateTime, D::Error>
where
    D: Deserializer<'de>,
{
    let s: &str = Deserialize::deserialize(deser)?;
    OffsetDateTime::parse(s, "%FT%T%z").map_err(|_| {
        de::Error::invalid_value(de::Unexpected::Str(s), &"a datetime in %FT%T%z format")
    })
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "type")]
struct SubscriptionChannel {
    name: String,
    product_ids: Vec<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case", tag = "type")]
enum CoinbaseMsg {
    Ticker {
        #[serde(deserialize_with = "crate::units::deserialize_dollars")]
        best_bid: crate::units::Price,
        #[serde(deserialize_with = "crate::units::deserialize_dollars")]
        best_ask: crate::units::Price,
        #[serde(deserialize_with = "deserialize_datetime")]
        time: time::OffsetDateTime,
    },
    Subscriptions {
        channels: Vec<SubscriptionChannel>,
    },
}
//{"type":"subscriptions","channels":[{"name":"ticker","product_ids":["BTC-USD"]}]}

pub fn spawn_ticker_thread() -> Receiver<crate::price::BitcoinPrice> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let mut coinbase_sock =
            tungstenite::client::connect(format!("wss://ws-feed.exchange.coinbase.com"))
                .expect("failed to connect to Coinbase");
        // Subscribe to public BTC-USD ticker. This is not an authenticated socket
        // and the Coinbase docs suggest that if you are being serious that you
        // should instead use the "level2" channel, which does require authentication
        // (it is still free, but requires a Coinbase account).
        //
        // In our case we will just do some sanity checks, and if they fail, we will
        // just cancel all orders and kill the bot TODO.
        coinbase_sock.0.write_message(tungstenite::protocol::Message::Text(
            "{\"type\":\"subscribe\",\"product_ids\": [\"BTC-USD\"],\"channels\": [\"ticker\"]}".to_string()
        )).unwrap();

        //{"type":"ticker","sequence":70942923614,"product_id":"BTC-USD","price":"43974.45","open_24h":"43907.27","volume_24h":"5439.41242986","low_24h":"43724.94","high_24h":"44510.01","volume_30d":"382489.32783775","best_bid":"43974.44","best_bid_size":"0.00743054","best_ask":"43974.45","best_ask_size":"0.21670607","side":"buy","time":"2024-01-07T22:55:34.347040Z","trade_id":592171029,"last_size":"0.00002997"}

        while let Ok(tungstenite::protocol::Message::Text(msg)) = coinbase_sock.0.read_message() {
            let decoded: CoinbaseMsg = serde_json::from_str(&msg).unwrap();
            println!("decoded to: {:?}", decoded);
            match decoded {
                CoinbaseMsg::Subscriptions { channels } => {
                    assert_eq!(channels.len(), 1);
                    assert_eq!(channels[0].name, "ticker");
                    assert_eq!(channels[0].product_ids, ["BTC-USD"]);
                }
                CoinbaseMsg::Ticker {
                    best_bid,
                    best_ask,
                    time,
                } => {
                    let mid = best_bid.half() + best_ask.half();
                    tx.send(crate::price::BitcoinPrice {
                        btc_price: mid,
                        timestamp: time,
                    })
                    .unwrap();
                }
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datetime_parse() {
        let cb_datetime = "2024-01-08T04:07:11.750237Z";
        OffsetDateTime::parse(cb_datetime, "%FT%T").unwrap();
    }
}
