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

pub mod price;
pub mod trade;

use anyhow::Context;
use clap::Clap;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::{fs, path::PathBuf};

use price::Historic;

fn from_date(s: &str) -> Result<time::Date, anyhow::Error> {
    Ok(time::Date::parse(s, "%F")?)
}

fn from_datetime(s: &str) -> Result<time::PrimitiveDateTime, anyhow::Error> {
    time::PrimitiveDateTime::parse(s, "%F %T").or(from_date(s).map(|d| d.midnight()))
}

fn from_offset_datetime(s: &str) -> Result<time::OffsetDateTime, anyhow::Error> {
    time::OffsetDateTime::parse(s, "%F %T%z")
        .or(from_datetime(s).map(|dt| dt.assume_utc().to_offset(time::UtcOffset::UTC)))
}

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
    PriceCall {
        #[clap(long, short)]
        /// Strike price of the proposed call option
        strike: Decimal,
        /// Expiry date of the proposed call option
        #[clap(long, short, parse(try_from_str = from_offset_datetime))]
        expiry: time::OffsetDateTime,
        /// Assumed BTC price volatility
        #[clap(long, short)]
        volatility: f32,
    },
}

fn main() -> Result<(), anyhow::Error> {
    let mut data_path = dirs::data_dir().context("getting XDG config directory")?;
    data_path.push("trade-tracker");

    match Command::parse() {
        Command::InitializePriceData { csv } => {
            let mut history = Historic::default();
            let csv_name = csv.to_string_lossy();

            let input =
                fs::File::open(&csv).with_context(|| format!("opening price data {}", csv_name))?;
            history
                .read_csv(input)
                .with_context(|| format!("decoding CSV data from {}", csv_name))?;

            data_path.push("pricedata");
            history.write_out(&mut data_path).with_context(|| {
                format!(
                    "writing out price history to {}",
                    data_path.to_string_lossy()
                )
            })?;
            data_path.pop();
        }
        Command::UpdatePriceData { url } => {
            let mut history =
                Historic::read_json_from(&data_path, "2020").context("reading price history")?;
            let data = reqwest::blocking::get(&url)
                .with_context(|| format!("getting data from {}", url))?;
            history
                .read_csv(data)
                .with_context(|| format!("decoding CSV data from {}", url))?;

            data_path.push("pricedata");
            history
                .write_out(&mut data_path)
                .context("writing out price history")?;
            data_path.pop();
        }
        Command::LatestPrice {} => {
            data_path.push("pricedata");
            let history =
                Historic::read_json_from(&data_path, "2021").context("reading price history")?;
            data_path.pop();

            let now = time::OffsetDateTime::now_utc();
            println!("{}", history.price_at(now));
        }
        Command::PriceCall {
            strike,
            expiry,
            volatility,
        } => {
            data_path.push("pricedata");
            let history =
                Historic::read_json_from(&data_path, "2021").context("reading price history")?;
            data_path.pop();

            let now = time::OffsetDateTime::now_utc();
            let current_price = history.price_at(now);
            println!("Using price: {}", current_price);

            let years: f64 = (expiry - now) / time::Duration::days(365);
            let price = black_scholes_pricer::bs_single::bs_price(
                black_scholes_pricer::OptionDir::CALL,
                current_price.btc_price.to_f32().unwrap(),
                strike.to_f32().unwrap(),
                years as f32,
                1.04f32, // risk free rate
                volatility,
                0.0, // lol bitcoin
            );
            println!("Price: {}", price);
        }
    }

    Ok(())
}
