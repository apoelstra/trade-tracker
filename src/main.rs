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

//! Trade Tracker
//!
//! Personal-use barely-maintained tool for keeping track of trades
//!

pub mod ledgerx;
pub mod local_bs;
pub mod option;
pub mod price;
pub mod terminal;
pub mod trade;

use anyhow::Context;
use clap::Clap;
use rust_decimal::Decimal;
use std::{convert::TryInto, fs, path::PathBuf};

use ledgerx::{datafeed, LedgerX};
use price::Historic;

/// Don't bother loading historical price data from before this date
const MIN_PRICE_DATE: &str = "2023";

#[derive(Clap)]
enum Command {
    InitializePriceData {
        #[clap(name = "csv_file", parse(from_os_str))]
        csv: PathBuf,
    },
    UpdatePriceData {
        #[clap(
            name = "url",
            default_value = "http://api.bitcoincharts.com/v1/trades.csv?symbol=bitstampUSD"
        )]
        url: String,
    },
    LatestPrice {},
    Price {
        #[clap(name = "option")]
        option: option::Option,
        /// Specific volatility, if provided
        #[clap(long, short)]
        volatility: Option<f64>,
    },
    Iv {
        #[clap(name = "option")]
        option: option::Option,
        /// Specific price, if provided
        #[clap(long, short)]
        price: Option<Decimal>,
    },
    Connect {
        #[clap(name = "token")]
        api_key: String,
    },
}

fn main() -> Result<(), anyhow::Error> {
    let mut data_path = dirs::data_dir().context("getting XDG config directory")?;
    data_path.push("trade-tracker");
    data_path.push("pricedata");
    let data_path = data_path; // drop mut

    println!("Trade tracker version [whatever]");
    println!("Price data pulled from http://api.bitcoincharts.com/v1/trades.csv?symbol=bitstampUSD -- call `update-price-data` to update");
    println!("");

    let command = Command::parse();
    let history = if let Command::InitializePriceData { .. } = command {
        // unused when initializing price data, just pick something
        Historic::default()
    } else {
        let history = Historic::read_json_from(&data_path, MIN_PRICE_DATE)
            .context("reading price history")?;
        history
    };
    let now = time::OffsetDateTime::now_utc();

    match Command::parse() {
        Command::InitializePriceData { csv } => {
            let mut history = Historic::default();
            let csv_name = csv.to_string_lossy();

            let input =
                fs::File::open(&csv).with_context(|| format!("opening price data {}", csv_name))?;
            history
                .read_csv(input)
                .with_context(|| format!("decoding CSV data from {}", csv_name))?;

            history.write_out(&data_path).with_context(|| {
                format!(
                    "writing out price history to {}",
                    data_path.to_string_lossy()
                )
            })?;
        }
        Command::UpdatePriceData { url } => {
            let mut history = history; // lol rust
            let data = minreq::get(&url)
                .with_timeout(10)
                .send()
                .with_context(|| format!("getting data from {}", url))?
                .into_bytes();
            history
                .read_csv(&data[..])
                .with_context(|| format!("decoding CSV data from {}", url))?;

            history
                .write_out(&data_path)
                .context("writing out price history")?;
        }
        Command::LatestPrice {} => {
            println!("{}", history.price_at(now));
        }
        Command::Price { option, volatility } => {
            let yte = option.years_to_expiry(now);
            let current_price = history.price_at(now);
            println!("BTC price: {}", current_price);
            println!("Risk-free rate: 4% (assumed)");
            println!(
                "Option: {} (years to expiry: {:7.6} or 1/{:7.6})",
                option,
                yte,
                1.0 / yte
            );
            println!("");
            for vol in 0..51 {
                let vol = volatility.unwrap_or(0.5) + 0.02 * (vol as f64);
                println!(
                    "Vol: {:3.2}   Price ($): {:8.2}   Theta ($): {:5.2}  DDel: {:3.2}%  Del: {:3.2}%",
                    vol,
                    option.bs_price(now, current_price.btc_price, vol),
                    option.bs_theta(now, current_price.btc_price, vol),
                    option.bs_dual_delta(now, current_price.btc_price, vol) * 100.0,
                    option.bs_delta(now, current_price.btc_price, vol) * 100.0,
                );
            }
        }
        Command::Iv { option, price } => {
            let current_price = history.price_at(now);
            println!("BTC price: {}", current_price);
            println!("Risk-free rate: 4% (assumed)");
            option.print_option_data(now, current_price.btc_price);
            println!("");

            let center = match price {
                Some(price) => price,
                None => option
                    .bs_price(now, current_price.btc_price, 0.75)
                    .try_into()
                    .unwrap_or(Decimal::from(0)),
            };
            let mut price = center / Decimal::from(2);
            while price - center <= center / Decimal::from(2) {
                option.print_order_data(now, current_price.btc_price, price, 1);
                price += center / Decimal::from(40);
            }
        }
        Command::Connect { api_key } => {
            let data = minreq::get("https://api.ledgerx.com/trading/contracts")
                .with_timeout(10)
                .send()
                .with_context(|| "getting data from trading/contracts endpoint")?
                .into_bytes();

            let all_contracts: Vec<ledgerx::Contract> = ledgerx::from_json_dot_data(&data)
                .with_context(|| "parsing contract list from json")?;

            let current_price = history.price_at(now);
            println!("BTC price: {}", current_price);
            println!("Risk-free rate: 4% (assumed)");

            let mut bscount = 0;
            let mut tracker = LedgerX::new(current_price);
            for contr in all_contracts {
                // Ignore expired and non-BTC options
                if !contr.active() || contr.underlying() != ledgerx::Asset::Btc {
                    continue;
                }

                let id = contr.id(); // save ID before moving into tracker
                tracker.add_contract(contr);
                let book_state =
                    minreq::get(format!("https://trade.ledgerx.com/api/book-states/{}", id))
                        .with_header("Authorization", format!("JWT {}", api_key))
                        .with_timeout(10)
                        .send()
                        .with_context(|| "getting data from trading/contracts endpoint")?
                        .into_bytes();
                let reply: ledgerx::json::BookStateMessage = serde_json::from_slice(&book_state)
                    .with_context(|| "parsing book state from json")?;
                tracker.initialize_orderbooks(reply, now);
                bscount += 1;
            }
            tracker.log_interesting_contracts();
            println!(
                "Loaded contracts ({} calls to book-states endpoint). Watching feed.",
                bscount
            );

            let mut last_update = now;
            loop {
                let mut sock = tungstenite::client::connect(format!(
                    "wss://api.ledgerx.com/ws?token={}",
                    api_key
                ))?;
                while let Ok(tungstenite::protocol::Message::Text(msg)) = sock.0.read_message() {
                    //println!("{}", msg);
                    let obj: datafeed::Object = serde_json::from_str(&msg)
                        .with_context(|| "parsing json from trading/contracts endpoint")?;
                    match obj {
                        datafeed::Object::Other => { /* ignore */ }
                        datafeed::Object::BookTop { .. } => { /* ignore */ }
                        datafeed::Object::Order(order) => {
                            tracker.insert_order(order);
                        }
                    }

                    let update_time = time::OffsetDateTime::now_utc();
                    if update_time - last_update > time::Duration::hours(1) {
                        tracker.log_interesting_contracts();
                        last_update = update_time;
                    }
                } // while let
            } // loop
        }
    }

    Ok(())
}
