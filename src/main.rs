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

pub mod csv;
pub mod http;
pub mod ledgerx;
pub mod local_bs;
pub mod logger;
pub mod lot;
pub mod option;
pub mod price;
pub mod terminal;
pub mod timemap;
pub mod transaction;
pub mod units;

use crate::ledgerx::LotId;
pub use crate::timemap::TimeMap;
use anyhow::Context;
use clap::Clap;
use log::{info, warn};
use rust_decimal::Decimal;
use std::{
    collections::HashMap,
    convert::TryInto,
    fs,
    io::{self, BufRead},
    path::PathBuf,
    str::FromStr,
    sync::mpsc::{channel, Receiver, Sender},
    thread,
};

use ledgerx::{datafeed, LedgerX};
use price::Historic;

/// Don't bother loading historical price data from before this date
const MIN_PRICE_DATE: &str = "2023";

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

#[derive(Clap)]
enum Command {
    /// Read a CSV file downloaded from Bitcoincharts, storing all its price data (at
    /// a ten-minute resolution rather than all of it)
    InitializePriceData {
        #[clap(name = "csv_file", parse(from_os_str))]
        csv: PathBuf,
    },
    /// Ping bitcoincharts in real time to get recent price data
    UpdatePriceData {
        #[clap(
            name = "url",
            default_value = "http://api.bitcoincharts.com/v1/trades.csv?symbol=bitstampUSD"
        )]
        url: String,
    },
    /// Return the latest stored price. Mainly useful as a test.
    LatestPrice {},
    /// Record a hex transaction that may be interesting to the software
    RecordTx {
        #[clap(name = "rawtx", about = "The hex-encoded raw transaction to record")]
        rawtx: String,
        #[clap(
            name = "timestamp",
            about = "Please provide the timestamp from the block header in which the transaction was confirmed"
        )]
        timestamp: i64,
    },
    /// Record a hex transaction that may be interesting to the software
    RecordLot {
        #[clap(name = "lot-id", about = "The lot ID to specify the timestamp for")]
        lot_id: LotId,
        #[clap(
            name = "timestamp",
            about = "Please provide the timestamp that you would like this lot to be sorted into the FIFO queue on"
        )]
        timestamp: i64,
    },
    /// Print a list of potential orders for a given option near a given volatility, at various
    /// prices
    Price {
        #[clap(name = "option")]
        option: option::Option,
        /// Specific volatility, if provided
        #[clap(long, short)]
        volatility: Option<f64>,
    },
    /// Print a list of potential orders for a given option near a given price
    Iv {
        #[clap(name = "option")]
        option: option::Option,
        /// Specific price, if provided
        #[clap(long, short)]
        price: Option<Decimal>,
    },
    /// Connect to LedgerX API and monitor activity in real-time
    Connect {
        #[clap(name = "token")]
        api_key: String,
    },
    /// Connect to LedgerX API and download complete transaction history, for a given year if
    /// supplied. Outputs in CSV.
    History {
        #[clap(name = "token")]
        api_key: String,
        #[clap(name = "year")]
        year: Option<i32>,
    },
    /// Connect to LedgerX API and attempt to recreate its tax CSV file for a given year
    TaxHistory {
        #[clap(name = "token")]
        api_key: String,
        #[clap(name = "year")]
        year: i32,
        #[clap(name = "mode", default_value = "both")]
        mode: TaxHistoryMode,
        #[clap(name = "lx_csv_file", parse(from_os_str))]
        lx_csv: Option<PathBuf>,
    },
}

/// Outputs a newline to stdout.
///
/// This is in its own function so I can easily grep for println! calls (there
/// should be none, except this one, because we should be using the log macros
/// instead.)
fn newline() {
    println!();
}

fn initialize_logging(now: time::OffsetDateTime, command: &Command) -> Result<(), anyhow::Error> {
    match command {
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

            let filenames = logger::LogFilenames {
                debug_log: format!("{}/debug_{}.log", log_dir, now.lazy_format("%F_%H%M%S")),
                datafeed_log: format!("{}/datafeed_{}.log", log_dir, now.lazy_format("%F_%H%M%S")),
                http_get_log: format!("{}/http_get_{}.log", log_dir, now.lazy_format("%F_%H%M%S")),
            };
            logger::Logger::init(&filenames).with_context(|| {
                format!(
                    "initializing logger (datafeed_log {}, debug log {}, http_get_log {})",
                    filenames.datafeed_log, filenames.debug_log, filenames.http_get_log,
                )
            })?;
        }
        // "One-off" commands just dump everything to stdout
        Command::InitializePriceData { .. }
        | Command::UpdatePriceData { .. }
        | Command::LatestPrice {}
        | Command::RecordLot { .. }
        | Command::RecordTx { .. }
        | Command::Price { .. }
        | Command::Iv { .. } => {
            logger::Logger::init_stdout_only().context("initializing stdout logger")?;
        }
    }

    info!("Trade tracker version {}", env!("CARGO_PKG_VERSION"));
    info!("Price data pulled from http://api.bitcoincharts.com/v1/trades.csv?symbol=bitstampUSD -- call `update-price-data` to update");
    newline();
    Ok(())
}

fn main() -> Result<(), anyhow::Error> {
    // Parse command-line args
    let command = Command::parse();
    // Get data path
    let mut data_path = dirs::data_dir().context("getting XDG config directory")?;
    data_path.push("trade-tracker");
    data_path.push("pricedata");

    // Read price data history
    let history = match command {
        // unused when initializing price data, just pick something
        Command::InitializePriceData { .. } => Ok(Historic::default()),
        Command::History { year, .. } => {
            Historic::read_json_from(&data_path, &year.unwrap_or(2020).to_string())
        }
        Command::TaxHistory { .. } => Historic::read_json_from(&data_path, "2017"),
        _ => Historic::read_json_from(&data_path, MIN_PRICE_DATE),
    }
    .context("reading price history")?;

    data_path.pop(); // "pricedata"

    // Turn on logging
    let now = time::OffsetDateTime::now_utc();
    initialize_logging(now, &command).context("initializing logging")?;

    // Go
    match Command::parse() {
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
        Command::RecordLot { lot_id, timestamp } => {
            // Basic sanity checks on the timestamp.
            if timestamp < 1231006505 {
                return Err(anyhow::Error::msg(
                    "Timestamp appears to be invalid (predates the genesis block)",
                ));
            } else if timestamp > 4102444800 {
                return Err(anyhow::Error::msg(
                    "Timestamp appears to be invalid (or you are in the year 2100, \
                     a time so far in the future that the earth men of 2023 could \
                     not imagine it.",
                ));
            }

            data_path.push("lots.json");
            let mut db = match lot::Database::load(&data_path) {
                Ok(db) => {
                    info!("Loaded lot database from disk.");
                    db
                }
                Err(e) => {
                    info!("Failed to load lot database: {}. Creating a new one.", e);
                    lot::Database::new()
                }
            };

            if let Some(existing) = db.insert_lot(lot_id, timestamp) {
                if time::OffsetDateTime::from_unix_timestamp(timestamp) == existing {
                    info!("Already had lot.");
                } else {
                    info!("Overwriting timestamp {} with {}.", existing, timestamp);
                }
            }
            db.save(&data_path).context("saving lot database")?;
            data_path.pop(); // "lots.json"
            info!("Success.");
        }
        Command::RecordTx { rawtx, timestamp } => {
            let bytes: Vec<u8> = hex::decode(rawtx).context("decoding rawtx as hex")?;
            let tx: bitcoin::Transaction =
                bitcoin::consensus::deserialize(&bytes).context("decoding rawtx as transaction")?;

            // Basic sanity checks on the timestamp.
            if timestamp < 1231006505 {
                return Err(anyhow::Error::msg(
                    "Timestamp appears to be invalid (predates the genesis block)",
                ));
            } else if timestamp > 4102444800 {
                return Err(anyhow::Error::msg(
                    "Timestamp appears to be invalid (or you are in the year 2100, \
                     a time so far in the future that the earth men of 2023 could \
                     not imagine it.",
                ));
            }

            data_path.push("transactions.json");
            let mut db = match transaction::Database::load(&data_path) {
                Ok(db) => {
                    info!("Loaded transaction database from disk.");
                    db
                }
                Err(e) => {
                    info!(
                        "Failed to load transaction database: {}. Creating a new one.",
                        e
                    );
                    transaction::Database::new()
                }
            };

            info!("Saving transaction with txid {}", tx.txid());
            if let Some(existing) = db.insert_tx(tx, timestamp) {
                if timestamp == existing {
                    info!("Already had transaction.");
                } else {
                    info!("Overwriting timestamp {} with {}.", existing, timestamp);
                }
            }
            db.save(&data_path).context("saving transaction database")?;
            data_path.pop(); // "transactions.json"
            info!("Success.");
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
                None => option
                    .bs_price(now, current_price.btc_price, 0.75)
                    .try_into()
                    .unwrap_or(Decimal::from(0)),
            };
            let mut price = center / Decimal::from(2);
            while price - center <= center / Decimal::from(2) {
                option.log_order_data(
                    if price == center { "→" } else { " " },
                    now,
                    current_price.btc_price,
                    price,
                    None,
                );
                price += center / Decimal::from(40);
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
                if contr.active() && contr.underlying() == ledgerx::Asset::Btc {
                    cid_tx
                        .send(contr.id())
                        .expect("book-states endpoint thread has not panicked");
                }
                tracker.add_contract(contr);
            }
            info!("Loaded contracts. Watching feed.");

            let mut last_update = now;
            loop {
                let mut sock = tungstenite::client::connect(format!(
                    "wss://api.ledgerx.com/ws?token={api_key}",
                ))?;
                while let Ok(tungstenite::protocol::Message::Text(msg)) = sock.0.read_message() {
                    let current_time = time::OffsetDateTime::now_utc();
                    info!(target: "lx_datafeed", "{}", msg);
                    info!(target: "lx_btcprice", "{}", tracker.current_price().0);

                    let obj: datafeed::Object = serde_json::from_str(&msg)
                        .with_context(|| "parsing json from trading/contracts endpoint")?;
                    match obj {
                        datafeed::Object::Other => { /* ignore */ }
                        datafeed::Object::BookTop { .. } => { /* ignore */ }
                        datafeed::Object::Order(order) => {
                            if let ledgerx::UpdateResponse::UnknownContract(order) =
                                tracker.insert_order(order)
                            {
                                warn!("unknown CID {}", order.contract_id);
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
                    }

                    // Initialize any pending contracts
                    while let Ok(reply) = book_state_rx.try_recv() {
                        tracker.initialize_orderbooks(reply, current_time);
                    }

                    if current_time - last_update > time::Duration::seconds(180) {
                        tracker.log_interesting_contracts();
                        last_update = current_time;
                    }
                } // while let
            } // loop
        }
        Command::History { api_key, year } => {
            let hist = ledgerx::history::History::from_api(&api_key)
                .context("getting history from LX API")?;
            hist.print_csv(year, &history);
        }
        Command::TaxHistory {
            api_key,
            year,
            mode,
            lx_csv,
        } => {
            let mut lx_price_ref = HashMap::new();
            if let Some(csv) = lx_csv {
                let csv_name = csv.to_string_lossy();
                let input =
                    fs::File::open(&csv).with_context(|| format!("opening tax data {csv_name}"))?;
                let bufread = io::BufReader::new(input);
                for line in bufread.lines().skip(1) {
                    let line = line.with_context(|| format!("parsing line from csv {csv_name}"))?;
                    let data =
                        ledgerx::csv::CsvLine::from_str(&line).map_err(anyhow::Error::msg)?;
                    for (time, price) in data.price_references() {
                        info!(
                            "At {} inferred price {} (reference price {})",
                            time,
                            price,
                            history.price_at(time)
                        );
                        lx_price_ref.insert(time, price);
                    }
                }
            }

            data_path.push("transactions.json");
            let db = transaction::Database::load(&data_path).context(
                "loading transaction database -- create one with the record-tx command. \
                          You will need to record every deposit transaction and its inputs. If \
                          you have made no BTC deposits, just record some random transaction.",
            )?;
            data_path.pop();
            data_path.push("lots.json");
            let lot_db = lot::Database::load(&data_path).ok();
            let hist = ledgerx::history::History::from_api(&api_key)
                .context("getting history from LX API")?;
            hist.print_tax_csv(year, mode, &history, &db, lot_db.as_ref(), &lx_price_ref);
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
