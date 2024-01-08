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

pub mod cli;
pub mod coinbase;
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
use crate::units::{Underlying, UtcTime};
use anyhow::Context;
use bitcoin::hashes::{sha256, Hash};
use chrono::offset::Utc;
use chrono::Datelike as _;
use log::{info, warn};
use std::{
    fs, io,
    str::FromStr,
    sync::mpsc::{channel, Receiver, Sender},
    thread,
};

use ledgerx::{datafeed, LedgerX};
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
        Command::Connect { .. }
        | Command::History { .. }
        | Command::TaxHistory { .. }
        | Command::TmpAndrew => {
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
                debug_log: format!("{log_dir}/{log_name}_{log_time}_debug.log"),
                datafeed_log: format!("{log_dir}/{log_name}_{log_time}_datafeed.log"),
                http_get_log: format!("{log_dir}/{log_name}_{log_time}_http.log"),
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
        Command::InitializePriceData { .. } => Ok(Historic::default()),
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
        Command::Connect { api_key } => {
            let all_contracts: Vec<ledgerx::Contract> =
                http::get_json_from_data_field("https://api.ledgerx.com/trading/contracts", None)?;

            let current_price = history.price_at(now);
            info!("BTC price: {}", current_price);
            info!("Risk-free rate: 4% (assumed)");

            let mut tracker = LedgerX::new(current_price);
            let (cid_tx, book_state_rx) = spawn_contract_lookup_thread(api_key.clone());
            for contr in all_contracts {
                // For expired or non-BTC options, fetch the full book. Otherwise
                // just record the contract's existence.
                if contr.active() && contr.underlying() == Underlying::Btc {
                    cid_tx
                        .send(contr.id())
                        .expect("book-states endpoint thread has not panicked");
                }
                tracker.add_contract(contr);
            }
            info!("Loaded contracts. Watching feed.");

            let mut last_update = now;
            let mut last_price = current_price.btc_price;
            loop {
                let mut sock = tungstenite::client::connect(format!(
                    "wss://api.ledgerx.com/ws?token={api_key}",
                ))?;
                while let Ok(tungstenite::protocol::Message::Text(msg)) = sock.0.read_message() {
                    let current_time = UtcTime::now();
                    let current_price = tracker.current_price().0;
                    info!(target: "lx_datafeed", "{}", msg);
                    info!(target: "lx_btcprice", "{}", current_price);

                    let obj: datafeed::Object = serde_json::from_str(&msg)
                        .with_context(|| "parsing json from trading/contracts endpoint")?;
                    match obj {
                        datafeed::Object::Other => { /* ignore */ }
                        datafeed::Object::BookTop { .. } => { /* ignore */ }
                        datafeed::Object::Order(order) => {
                            if let ledgerx::UpdateResponse::UnknownContract(order) =
                                tracker.insert_order(order)
                            {
                                warn!("unknown contract ID {}", order.contract_id);
                                warn!("full msg {}", msg);
                            }
                        }
                        datafeed::Object::AvailableBalances { usd, btc } => {
                            tracker.set_balances(usd, btc);
                        }
                        datafeed::Object::ContractAdded(contr) => {
                            cid_tx
                                .send(contr.id())
                                .expect("book-states endpoint thread has not panicked");
                            tracker.add_contract(contr);
                        }
                        datafeed::Object::ContractRemoved(cid) => {
                            tracker.remove_contract(cid);
                        }
                        datafeed::Object::ChatMessage {
                            message,
                            initiator,
                            counterparty,
                            chat_id,
                        } => {
                            info!(
                                "New message (chat {}) between {} and {}: {}",
                                chat_id, initiator, counterparty, message
                            );
                        }
                    }

                    // Initialize any pending contracts
                    while let Ok(reply) = book_state_rx.try_recv() {
                        tracker.initialize_orderbooks(reply, current_time);
                    }

                    // Log the "standing" data every 6 hours or whenever the price moves a lot
                    if current_time - last_update > chrono::Duration::hours(6)
                        || current_price < last_price.scale_approx(0.98)
                        || current_price > last_price.scale_approx(1.02)
                    {
                        tracker.log_open_orders();
                        tracker.log_interesting_contracts();
                        last_update = current_time;
                        last_price = current_price;
                    }
                } // while let
            } // loop
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
            drop(bufread);
            // Query LX to get all historic trade data
            let hist = ledgerx::history::History::from_api(
                api_key,
                &config,
                sha256::Hash::from_engine(hash_eng),
            )
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
        Command::TmpAndrew => {
            let rx = crate::coinbase::spawn_ticker_thread();
            loop {
                let price_update = rx.recv().unwrap();
                // TODO use the return value
            }
        }
    }

    Ok(())
}

fn spawn_contract_lookup_thread(
    api_key: String,
) -> (
    Sender<ledgerx::ContractId>,
    Receiver<ledgerx::json::BookStateMessage>,
) {
    let (tx_cid, rx_cid) = channel();
    let (tx_resp, rx_resp) = channel();
    thread::spawn(move || {
        for contract_id in rx_cid.iter() {
            let reply: ledgerx::json::BookStateMessage = http::get_json(
                &format!("https://trade.ledgerx.com/api/book-states/{contract_id}"),
                Some(&api_key),
            )
            .context("getting data from trading/contracts endpoint")
            .expect("parsing json from book-states endpoint");
            tx_resp.send(reply).unwrap();
        }
    });
    (tx_cid, rx_resp)
}
