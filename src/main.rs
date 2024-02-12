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

#![allow(clippy::manual_range_contains)] // this lint is bullshit

pub mod cli;
pub mod coinbase;
pub mod connect;
pub mod csv;
pub mod file;
pub mod http;
pub mod ledgerx;
pub mod local_bs;
pub mod logger;
pub mod option;
pub mod price;
pub mod terminal;
pub mod timemap;
pub mod transaction;
pub mod units;

use crate::cli::Command;
pub use crate::timemap::TimeMap;
use crate::units::UtcTime;
use anyhow::Context;
use bitcoin::hashes::{sha256, Hash};
use chrono::offset::Utc;
use chrono::Datelike as _;
use log::{info, warn};
use std::{fs, io, str::FromStr};

use price::Historic;

/// Don't bother loading historical price data from before this date
const TAX_PRICE_MIN_YEAR: &str = "2021";

/// Mode indicating how much/what data to output from the tax-history command
pub enum TaxHistoryMode {
    JustLxData,
    JustLotIds,
    Both,
}

impl FromStr for TaxHistoryMode {
    type Err = String;
    fn from_str(s: &str) -> Result<TaxHistoryMode, String> {
        match s {
            "just-lx-data" => Ok(TaxHistoryMode::JustLxData),
            "just-lot-ids" => Ok(TaxHistoryMode::JustLotIds),
            "both" => Ok(TaxHistoryMode::Both),
            x => Err(format!(
                "Invalid tax history mode {x}; allowed values: just-lx-data, just-lot-ids, both",
            )),
        }
    }
}

/// Outputs a newline to stdout.
///
/// This is in its own function so I can easily grep for println! calls (there
/// should be none, except this one, because we should be using the log macros
/// instead.)
fn newline() {
    println!();
}

fn initialize_logging(
    now: UtcTime,
    command: &Command,
) -> Result<Option<logger::LogFilenames>, anyhow::Error> {
    let ret = match command {
        // Commands that interact with the LX API should have full logging, including
        // debug logs and sending all json replies to log files.
        Command::Connect { .. } | Command::History { .. } | Command::TaxHistory { .. } => {
            let log_dir = format!("{}/log", env!("CARGO_MANIFEST_DIR"));
            if let Ok(metadata) = std::fs::metadata(&log_dir) {
                if !metadata.is_dir() {
                    return Err(anyhow::Error::msg(format!(
                        "Log directory {log_dir} alrready exists but is not a directory.",
                    )));
                }
            } else {
                std::fs::create_dir(&log_dir)
                    .with_context(|| format!("creating log directory {log_dir}"))?;
            }

            let log_name = command.log_name();
            let log_time = now.format("%F_%H-%M-%S");
            let filenames = logger::LogFilenames {
                coinbase_log: format!("{log_dir}/{log_name}-coinbase_{log_time}.log"),
                debug_log: format!("{log_dir}/{log_name}-debug_{log_time}.log"),
                datafeed_log: format!("{log_dir}/{log_name}-datafeed_{log_time}.log"),
                http_get_log: format!("{log_dir}/{log_name}-http_{log_time}.log"),
            };
            logger::Logger::init(&filenames).with_context(|| {
                format!(
                    "initializing logger (datafeed_log {}, debug log {}, http_get_log {})",
                    filenames.datafeed_log, filenames.debug_log, filenames.http_get_log,
                )
            })?;
            Some(filenames)
        }
        // "One-off" commands just dump everything to stdout
        Command::InitializePriceData { .. }
        | Command::UpdatePriceData { .. }
        | Command::LatestPrice {}
        | Command::Price { .. }
        | Command::Iv { .. } => {
            logger::Logger::init_stdout_only().context("initializing stdout logger")?;
            None
        }
    };

    info!("Trade tracker version {}", env!("CARGO_PKG_VERSION"));
    info!("Price data pulled from http://api.bitcoincharts.com/v1/trades.csv?symbol=bitstampUSD -- call `update-price-data` to update");
    newline();
    Ok(ret)
}

fn parse_config_file(
    config_file: &std::path::Path,
) -> Result<(sha256::Hash, ledgerx::history::Configuration), anyhow::Error> {
    // Parse config file
    let config_name = config_file.to_string_lossy();
    let input = fs::File::open(config_file)
        .with_context(|| format!("opening config file {config_name}"))?;
    let bufread = io::BufReader::new(input);
    let config: ledgerx::history::Configuration = serde_json::from_reader(bufread)
        .with_context(|| format!("parsing config file {config_name}"))?;
    // Read it again to get its hash
    let input = fs::File::open(config_file)
        .with_context(|| format!("opening config file {config_name}"))?;
    let mut bufread = io::BufReader::new(input);
    let mut hash_eng = sha256::Hash::engine();
    io::copy(&mut bufread, &mut hash_eng)
        .with_context(|| format!("copying {config_name} into hash engine"))?;

    Ok((sha256::Hash::from_engine(hash_eng), config))
}

fn main() -> Result<(), anyhow::Error> {
    // Parse command-line args
    let command = Command::from_args();
    // Get data path
    let mut data_path = dirs::data_dir().context("getting XDG config directory")?;
    data_path.push("trade-tracker");
    data_path.push("pricedata");

    // Read price data history
    let history = match command {
        // unused when initializing price data, just pick something
        // Also unused for Connect, which uses a real-time ticker feed
        Command::InitializePriceData { .. } | Command::Connect { .. } => Ok(Historic::default()),
        // For tax stuff we have to load historic data going back a bit
        Command::History { .. } | Command::TaxHistory { .. } => {
            Historic::read_json_from(&data_path, TAX_PRICE_MIN_YEAR)
        }
        // For most everything else we can just use the current year
        _ => Historic::read_json_from(&data_path, &Utc::now().year().to_string()),
    }
    .context("reading price history")?;

    data_path.pop(); // "pricedata"

    // Turn on logging
    let now = UtcTime::now();
    let log_filenames = initialize_logging(now, &command).context("initializing logging")?;

    // Go
    match command {
        Command::InitializePriceData { csv } => {
            let mut history = Historic::default();
            let csv_name = csv.to_string_lossy();

            let input =
                fs::File::open(&csv).with_context(|| format!("opening price data {csv_name}"))?;
            history
                .read_csv(input)
                .with_context(|| format!("decoding CSV data from {csv_name}"))?;

            history.write_out(&data_path).with_context(|| {
                format!(
                    "writing out price history to {}",
                    data_path.to_string_lossy()
                )
            })?;
        }
        Command::UpdatePriceData { url } => {
            let mut history = history; // lol rust
            let data = http::get_bytes(&url, None)?;
            history
                .read_csv(&data[..])
                .with_context(|| format!("decoding CSV data from {url}"))?;

            data_path.push("pricedata");
            history
                .write_out(&data_path)
                .context("writing out price history")?;
            data_path.pop();
        }
        Command::LatestPrice {} => {
            info!("{}", history.price_at(now));
        }
        Command::Price { option, volatility } => {
            let yte = option.years_to_expiry(now);
            let current_price = history.price_at(now);
            info!("BTC price: {}", current_price);
            info!("Risk-free rate: 4% (assumed)");
            info!(
                "Option: {} (years to expiry: {:7.6} or 1/{:7.6})",
                option,
                yte,
                1.0 / yte
            );
            newline();
            for vol in 0..51 {
                let vol = volatility.unwrap_or(0.5) + 0.02 * (vol as f64);
                info!(
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
            info!("BTC price: {}", current_price);
            info!("Risk-free rate: 4% (assumed)");
            option.log_option_data("", now, current_price.btc_price);
            newline();

            let center = match price {
                Some(price) => price,
                None => option.bs_price(now, current_price.btc_price, 0.75),
            };
            let mut price = center.half();
            while price - center <= center.half() {
                option.log_order_data(
                    if price == center { "→" } else { " " },
                    now,
                    current_price.btc_price,
                    price,
                    None,
                );
                price += center.scale_approx(1.0 / 40.0);
            }
        }
        Command::Connect {
            api_key,
            config_file,
        } => {
            // Parse config file
            if let Some(config_file) = config_file {
                let (config_hash, config) = parse_config_file(&config_file)?;
                let hist = ledgerx::history::History::from_api(&api_key, &config, config_hash)
                    .context("getting history from LX API")?;
                connect::main_loop(api_key, Some(hist));
            } else {
                warn!("No configuration file passed; assuming fresh account/no history.");
                connect::main_loop(api_key, None);
            }
        }
        Command::History {
            ref api_key,
            ref config_file,
        }
        | Command::TaxHistory {
            ref api_key,
            ref config_file,
        } => {
            // Assert we have the log filenames before doing anything complex
            // If this unwrap fails it's a bug.
            let log_filenames = log_filenames.unwrap();
            // Parse config file
            let (config_hash, config) = parse_config_file(config_file)?;
            // Query LX to get all historic trade data
            let hist = ledgerx::history::History::from_api(api_key, &config, config_hash)
                .context("getting history from LX API")?;
            // ...and output
            if let Command::History { .. } = command {
                hist.print_csv(&history);
            } else {
                let dir_path = format!("lx_tax_output_{}", now.format("%F-%H%M"));
                if fs::metadata(&dir_path).is_ok() {
                    return Err(anyhow::Error::msg(format!(
                        "Output directory {dir_path} exists. Refusing to run."
                    )));
                }
                fs::create_dir(&dir_path).with_context(|| {
                    format!("Creating directory {dir_path} to put tax output into")
                })?;
                info!("Creating directory {} to hold output.", dir_path);
                let config_name = config_file.to_string_lossy();
                file::copy_file(&config_name, &format!("{dir_path}/configuration.json"))?;
                hist.print_tax_csv(&dir_path, &history)
                    .context("printing tax CSV")?;
                file::copy_file(&log_filenames.debug_log, &format!("{dir_path}/debug.log"))?;
                file::copy_file(
                    &log_filenames.http_get_log,
                    &format!("{dir_path}/http_get.log"),
                )?;
            }
        }
    }

    Ok(())
}
