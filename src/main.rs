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

pub mod option;
pub mod price;
pub mod trade;

use anyhow::Context;
use clap::Clap;
use rust_decimal::Decimal;
use std::{convert::TryInto, fs, path::PathBuf};

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
            let yte = option.years_to_expiry(&now);
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
                    "Vol: {:3.2}   Price ($): {:8.2}   Theta ($): {:5.2}",
                    vol,
                    option.bs_price(&now, current_price.btc_price, vol),
                    option.bs_theta(&now, current_price.btc_price, vol),
                );
            }
        }
        Command::Iv { option, price } => {
            let yte = option.years_to_expiry(&now);
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

            let center = match price {
                Some(price) => price,
                None => option
                    .bs_price(&now, current_price.btc_price, 0.75)
                    .try_into()
                    .unwrap_or(Decimal::from(0)),
            };
            let mut price = center / Decimal::from(2);
            while price - center <= center / Decimal::from(2) {
                match option.bs_iv(&now, current_price.btc_price, price) {
                    Ok(vol) => println!(
                        "Price ($) {:8.2}   Vol: {:5.4}   Theta ($): {:5.2}",
                        price,
                        vol,
                        option.bs_theta(&now, current_price.btc_price, vol),
                    ),
                    Err(vol) => println!(
                        "EE Price ($) {:8.2}   Vol: {:3.2}   Theta ($): {:5.2}",
                        price,
                        vol,
                        option.bs_theta(&now, current_price.btc_price, vol),
                    ),
                }
                price += center / Decimal::from(40);
            }
        }
    }

    Ok(())
}
