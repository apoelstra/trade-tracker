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

use crate::price::BitcoinPrice;
use crate::units::UtcTime;
use log::info;
use serde::{de, Deserialize, Deserializer};
use std::sync::mpsc::Sender;
use std::thread;

fn deserialize_datetime<'de, D>(deser: D) -> Result<UtcTime, D::Error>
where
    D: Deserializer<'de>,
{
    let s: &str = Deserialize::deserialize(deser)?;
    UtcTime::parse_coinbase(s).map_err(|_| {
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
        time: UtcTime,
    },
    Subscriptions {
        channels: Vec<SubscriptionChannel>,
    },
}
//{"type":"subscriptions","channels":[{"name":"ticker","product_ids":["BTC-USD"]}]}

pub fn spawn_ticker_thread(tx: Sender<crate::connect::Message>) {
    thread::spawn(move || loop {
        let mut coinbase_sock = tungstenite::client::connect("wss://ws-feed.exchange.coinbase.com")
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

        // We maintain a "shutdown price reference" which is updated whenever the price
        // moves by more than 5% in either direction. If such a movement happens too
        // quickly then we do an emergency shutdown.
        //
        // This algorithm is not great: it allows, for example, the price to drop 4% (not
        // triggering an update to the reference) and then increase 8% (staying within 5%
        // of the reference despite actually moving much more). However, the goal of this
        // is mainly to detect bad data from the ticker, which should show up as a massive
        // instantaneous price movement. Natural volatility, as long as it doesn't go
        // wildly out of range, is fine and probably even good for us.
        let mut shutdown_price_ref: Option<BitcoinPrice> = None;
        while let Ok(tungstenite::protocol::Message::Text(msg)) = coinbase_sock.0.read_message() {
            info!(target: "cb_datafeed", "{}", msg);
            match serde_json::from_str(&msg).unwrap() {
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
                    let new_price = BitcoinPrice {
                        btc_price: mid,
                        timestamp: time,
                    };

                    let ref_price = shutdown_price_ref.unwrap_or(new_price);
                    let ratio = new_price.btc_price / ref_price.btc_price;
                    // 5% in 5 minutes is an "emergency shutdown" situation. Either the
                    // price feed has glitched out or the price is doing something wild
                    // and we don't want to be automatically trading anyway.
                    if ratio < 0.95 || ratio > 1.05 {
                        if new_price.timestamp - ref_price.timestamp
                            > chrono::Duration::seconds(300)
                        {
                            tx.send(crate::connect::Message::EmergencyShutdown {
                                msg: format!(
                                    "Rapid price movement: from {ref_price} to {new_price}"
                                ),
                            })
                            .unwrap();
                        }
                        shutdown_price_ref = Some(new_price);
                    }
                    tx.send(crate::connect::Message::PriceReference(new_price))
                        .unwrap();
                }
            }
        }
        info!("Restarting connection to coinbase.");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datetime_parse() {
        let cb_datetime = "2024-01-08T04:07:11.750237Z";
        let parsed = UtcTime::parse_coinbase(cb_datetime).unwrap();
        assert_eq!(parsed.to_string(), "2024-01-08 04:07:11.750237 UTC",);
    }
}
