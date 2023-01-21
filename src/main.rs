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
use clipboard::x11_clipboard::{Clipboard, X11ClipboardContext};
use clipboard::ClipboardProvider;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::{fs, path::PathBuf, thread, time::Duration};

use price::{BitcoinPrice, Historic};

fn from_date(s: &str) -> Result<time::Date, anyhow::Error> {
    Ok(time::Date::parse(s, "%F")?)
}

fn from_datetime(s: &str) -> Result<time::PrimitiveDateTime, anyhow::Error> {
    time::PrimitiveDateTime::parse(s, "%F %T")
        .or(from_date(s).map(|d| d.with_time(time::time!(21:00))))
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
        volatility: f64,
    },
    ImpliedVolatilityCall {
        #[clap(long, short)]
        /// Strike price of the proposed call option
        strike: Decimal,
        /// Expiry date of the proposed call option
        #[clap(long, short, parse(try_from_str = from_offset_datetime))]
        expiry: time::OffsetDateTime,
        /// Price
        #[clap(long, short)]
        price: Decimal,
        /// Bitcoin Price, if provided
        #[clap(long, short)]
        btc_price: Option<Decimal>,
    },
    PricePut {
        #[clap(long, short)]
        /// Strike price of the proposed call option
        strike: Decimal,
        /// Expiry date of the proposed call option
        #[clap(long, short, parse(try_from_str = from_offset_datetime))]
        expiry: time::OffsetDateTime,
        /// Assumed BTC price volatility
        #[clap(long, short)]
        volatility: f64,
    },
    ImpliedVolatilityPut {
        #[clap(long, short)]
        /// Strike price of the proposed call option
        strike: Decimal,
        /// Expiry date of the proposed call option
        #[clap(long, short, parse(try_from_str = from_offset_datetime))]
        expiry: time::OffsetDateTime,
        /// Price
        #[clap(long, short)]
        price: Decimal,
        /// Bitcoin Price, if provided
        #[clap(long, short)]
        btc_price: Option<Decimal>,
    },
}

fn main() -> Result<(), anyhow::Error> {
    let mut data_path = dirs::data_dir().context("getting XDG config directory")?;
    data_path.push("trade-tracker");

    println!("Trade tracker version [whatever]");
    println!("Price data pulled from http://api.bitcoincharts.com/v1/trades.csv?symbol=bitstampUSD -- call `update-price-data` to update");
    println!("");

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
            data_path.push("pricedata");
            let mut history =
                Historic::read_json_from(&data_path, "2020").context("reading price history")?;
            data_path.pop();
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
            println!("Using risk-free-rate: 4% (assumed)");

            let years: f64 = (expiry - now) / time::Duration::days(365);
            let price = black_scholes::call(
                current_price.btc_price.to_f64().unwrap(),
                strike.to_f64().unwrap(),
                0.04f64, // risk free rate
                volatility,
                years,
            );
            println!("Price: {}", price);
        }
        Command::ImpliedVolatilityCall {
            strike,
            expiry,
            price,
            btc_price,
        } => {
            data_path.push("pricedata");
            let history =
                Historic::read_json_from(&data_path, "2021").context("reading price history")?;
            data_path.pop();

            println!("Call: strike {} exp {} price {}", strike, expiry, price);
            println!("");

            let now = time::OffsetDateTime::now_utc();
            let current_price = btc_price
                .map(|d| BitcoinPrice::from_current(d))
                .unwrap_or(history.price_at(now));
            println!("Using price: {:.2}", current_price);
            println!("Using risk-free-rate: 4% (assumed)");

            let years: f64 = (expiry - now) / time::Duration::days(365);
            println!("Years to expiry: {:.8} (inv {:.6})", years, 1.0 / years);
            let vol = black_scholes::call_iv(
                price.to_f64().unwrap(),
                current_price.btc_price.to_f64().unwrap(),
                strike.to_f64().unwrap(),
                0.04f64, // risk free rate
                years,
            )
            .unwrap();
            println!("Implied volatility: {:.6} %", 100.0 * vol);
            let theta = black_scholes::call_theta(
                current_price.btc_price.to_f64().unwrap(),
                strike.to_f64().unwrap(),
                0.04f64, // risk free rate
                vol,
                years,
            ) / 365.0;
            println!("Theta: {:.6}", theta);
            println!(
                "Annualized return if expires worthless: {:.6} %",
                100.0
                    * (1.0f64
                        + price.to_f64().unwrap() / current_price.btc_price.to_f64().unwrap())
                    .powf(1.0f64 / years)
                    - 100.0
            );
            // Print data in excel form
            let clipboard = X11ClipboardContext::<Clipboard>::new();
            if let Ok(mut clipboard) = clipboard {
                let _ = clipboard.set_contents(format!(
                    "{:0.8}\t{:0.8}\t{:6.2}",
                    vol,
                    (1.0f64 + price.to_f64().unwrap() / current_price.btc_price.to_f64().unwrap())
                        .powf(1.0f64 / years)
                        - 1.0,
                    current_price.btc_price.to_f64().unwrap()
                ));
                thread::sleep(Duration::from_secs(30));
            }
        }
        Command::PricePut {
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
            println!("Using risk-free-rate: 4% (assumed)");

            let years: f64 = (expiry - now) / time::Duration::days(365);
            let price = black_scholes::put(
                current_price.btc_price.to_f64().unwrap(),
                strike.to_f64().unwrap(),
                0.04f64, // risk free rate
                volatility,
                years,
            );
            println!("Price: {}", price);
        }
        Command::ImpliedVolatilityPut {
            strike,
            expiry,
            price,
            btc_price,
        } => {
            data_path.push("pricedata");
            let history =
                Historic::read_json_from(&data_path, "2021").context("reading price history")?;
            data_path.pop();

            println!("Put: strike {} exp {} price {}", strike, expiry, price);
            println!("");

            let now = time::OffsetDateTime::now_utc();
            let current_price = btc_price
                .map(|d| BitcoinPrice::from_current(d))
                .unwrap_or(history.price_at(now));
            println!("Using price: {}", current_price);
            println!("Using risk-free-rate: 4% (assumed)");

            let years: f64 = (expiry - now) / time::Duration::days(365);
            println!("Years to expiry: {:.8} (inv {:.6})", years, 1.0 / years);
            let vol = black_scholes::put_iv(
                price.to_f64().unwrap(),
                current_price.btc_price.to_f64().unwrap(),
                strike.to_f64().unwrap(),
                0.04f64, // risk free rate
                years,
            );
            if let Ok(vol) = vol {
                println!("Implied volatility: {:.6} %", 100.0 * vol);
                let theta = black_scholes::put_theta(
                    current_price.btc_price.to_f64().unwrap(),
                    strike.to_f64().unwrap(),
                    0.04f64, // risk free rate
                    vol,
                    years,
                ) / 365.0;
                println!("Theta: {:.6}", theta);
            } else {
                println!("Implied volatility: (cannot compute) %");
                println!("Theta: (cannot compute) %");
            }
            println!(
                "Annualized return if expires worthless: {:.6} %",
                100.0
                    * (1.0f64 + price.to_f64().unwrap() / strike.to_f64().unwrap())
                        .powf(1.0f64 / years)
                    - 100.0
            );
            // Print data in excel form
            let clipboard = X11ClipboardContext::<Clipboard>::new();
            if let Ok(mut clipboard) = clipboard {
                let _ = clipboard.set_contents(format!(
                    "{:0.8}\t{:0.8}\t{:6.2}",
                    vol.unwrap_or(0.0),
                    (1.0f64 + price.to_f64().unwrap() / strike.to_f64().unwrap())
                        .powf(1.0f64 / years)
                        - 1.0,
                    current_price.btc_price.to_f64().unwrap()
                ));
                thread::sleep(Duration::from_secs(30));
            }
        }
    }

    Ok(())
}
