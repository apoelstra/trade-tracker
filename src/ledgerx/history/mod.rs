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

//! LedgerX History
//!
//! Data structures related to trading history for copying into Excel
//!

use crate::csv::{self, CsvPrinter};
use crate::file::create_text_file;
use crate::units::{BudgetAsset, DepositAsset, Price, Quantity, Underlying, UnknownQuantity};
use anyhow::Context;
use log::{debug, info, warn};
use serde::{de, Deserialize, Deserializer};
use std::collections::HashMap;
use std::fs;
use std::str::FromStr;
use time::OffsetDateTime;

pub mod config;
pub mod tax;

pub use self::config::Configuration;
pub use self::tax::LotId;

// Note that this is *not* the same as the equivalent function in ledgerx/json.rs
// For some reason LX returns timestamps in like a dozen different formats.
fn deserialize_datetime<'de, D>(deser: D) -> Result<OffsetDateTime, D::Error>
where
    D: Deserializer<'de>,
{
    let s: &str = Deserialize::deserialize(deser)?;
    OffsetDateTime::parse(s, time::Format::Rfc3339).map_err(|_| {
        de::Error::invalid_value(de::Unexpected::Str(s), &"a datetime in RFC 3339 format")
    })
}

#[derive(Deserialize, Debug)]
struct Meta {
    #[serde(default)]
    next: Option<String>,
}

#[derive(Deserialize, Debug)]
struct DepositAddress {
    address: String,
    asset: DepositAsset,
}

#[derive(Deserialize, Debug)]
struct Deposit {
    amount: UnknownQuantity,
    #[serde(deserialize_with = "crate::units::deserialize_name_deposit_asset")]
    asset: DepositAsset,
    deposit_address: DepositAddress,
    #[serde(deserialize_with = "deserialize_datetime")]
    created_at: OffsetDateTime,
}

/// Opaque structure representing the deposits list returned by the funds/deposits endpoint
#[derive(Deserialize, Debug)]
pub struct Deposits {
    data: Vec<Deposit>,
    #[serde(default)]
    meta: Option<Meta>,
}

impl Deposits {
    /// Returns the next URL, if any, to fetch
    pub fn next_url(&self) -> Option<String> {
        self.meta.as_ref().and_then(|meta| meta.next.clone())
    }
}

#[derive(Deserialize, Debug)]
struct Withdrawal {
    amount: UnknownQuantity,
    // Note: withdrawals don't have the extra "name" indirection for some reason
    asset: DepositAsset,
    #[serde(deserialize_with = "deserialize_datetime")]
    created_at: OffsetDateTime,
}

/// Opaque structure representing the withdrawals list returned by the funds/withdrawals endpoint
#[derive(Deserialize, Debug)]
pub struct Withdrawals {
    data: Vec<Withdrawal>,
    #[serde(default)]
    meta: Option<Meta>,
}

impl Withdrawals {
    /// Returns the next URL, if any, to fetch
    pub fn next_url(&self) -> Option<String> {
        self.meta.as_ref().and_then(|meta| meta.next.clone())
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
enum Side {
    Bid,
    Ask,
}

#[derive(Deserialize, Debug)]
struct Trade {
    contract_id: String,
    #[serde(deserialize_with = "deserialize_datetime")]
    execution_time: OffsetDateTime,
    #[serde(deserialize_with = "crate::units::deserialize_cents")]
    filled_price: Price,
    filled_size: UnknownQuantity,
    side: Side,
    #[serde(deserialize_with = "crate::units::deserialize_cents")]
    fee: Price,
}

#[derive(Deserialize, Debug)]
pub struct Trades {
    data: Vec<Trade>,
    #[serde(default)]
    meta: Option<Meta>,
}

impl Trades {
    /// Request contract data for every unknown contract ID from LX
    pub fn fetch_contract_ids(
        &self,
        map: &mut HashMap<String, super::Contract>,
    ) -> Result<(), anyhow::Error> {
        for trade in &self.data {
            let id = trade.contract_id.clone();
            if map.get(&id).is_none() {
                let contract = crate::http::get_json_from_data_field(
                    &format!("https://api.ledgerx.com/trading/contracts/{id}"),
                    None,
                )
                .context("lookup contract for trade history")?;
                map.insert(id, contract);
            }
        }
        Ok(())
    }

    /// Returns the next URL, if any, to fetch
    pub fn next_url(&self) -> Option<String> {
        self.meta.as_ref().and_then(|meta| meta.next.clone())
    }
}

#[derive(Deserialize, Debug)]
pub struct Position {
    size: i64,
    assigned_size: i64,
    contract: super::Contract,
    has_settled: bool,
}

#[derive(Deserialize, Debug)]
pub struct Positions {
    data: Vec<Position>,
    #[serde(default)]
    meta: Option<Meta>,
}

impl Positions {
    /// Position data, weirdly, contains full contract information. So store this to speed up
    /// trade lookups.
    pub fn store_contract_ids(&self, map: &mut HashMap<String, super::Contract>) {
        for pos in &self.data {
            map.insert(pos.contract.id().to_string(), pos.contract.clone());
        }
    }

    /// Returns the next URL, if any, to fetch
    pub fn next_url(&self) -> Option<String> {
        self.meta.as_ref().and_then(|meta| meta.next.clone())
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum Event {
    Deposit {
        amount: Quantity,
        address: bitcoin::Address,
        asset: DepositAsset,
    },
    Withdrawal {
        amount: Quantity,
        asset: DepositAsset,
    },
    Trade {
        contract: super::Contract,
        price: Price,
        size: Quantity,
        fee: Price,
    },
    Expiry {
        contract: super::Contract,
        assigned_size: Quantity,
        expired_size: Quantity,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct History {
    events: crate::TimeMap<Event>,
}

impl History {
    /// Construct a new empty history
    pub fn new() -> Self {
        Default::default()
    }

    /// Construct a new history by calling the LX API
    pub fn from_api(api_key: &str) -> anyhow::Result<Self> {
        let mut ret = History::new();
        let mut contracts = HashMap::new();

        let mut next_url = Some("https://api.ledgerx.com/trading/positions?limit=200".to_string());
        while let Some(url) = next_url {
            info!(
                "Fetching positions .. have {} contracts cached.",
                contracts.len()
            );
            let positions: Positions = crate::http::get_json(&url, Some(api_key))
                .context("getting positions from LX API")?;
            positions.store_contract_ids(&mut contracts);

            ret.import_positions(&positions);
            next_url = positions.next_url();
        }

        let mut next_url = Some("https://api.ledgerx.com/funds/deposits?limit=200".to_string());
        while let Some(url) = next_url {
            info!("Fetching deposits");
            let deposits: Deposits = crate::http::get_json(&url, Some(api_key))
                .context("getting deposits from LX API")?;

            ret.import_deposits(&deposits);
            next_url = deposits.next_url();
        }

        let mut next_url = Some("https://api.ledgerx.com/funds/withdrawals?limit=200".to_string());
        while let Some(url) = next_url {
            info!("Fetching withdrawals");
            let withdrawals: Withdrawals = crate::http::get_json(&url, Some(api_key))
                .context("getting withdrawals from LX API")?;

            ret.import_withdrawals(&withdrawals);
            next_url = withdrawals.next_url();
        }

        let mut next_url = Some("https://api.ledgerx.com/trading/trades?limit=200".to_string());
        while let Some(url) = next_url {
            info!(
                "Fetching trades .. have {} contracts cached.",
                contracts.len()
            );
            let trades: Trades =
                crate::http::get_json(&url, Some(api_key)).context("getting trades from LX API")?;
            trades
                .fetch_contract_ids(&mut contracts)
                .with_context(|| "getting contract IDs")?;

            ret.import_trades(&trades, &contracts)
                .with_context(|| "importing trades")?;
            next_url = trades.next_url();
        }
        Ok(ret)
    }

    /// Import a list of deposits into the history
    pub fn import_deposits(&mut self, deposits: &Deposits) {
        for dep in &deposits.data {
            assert_eq!(
                dep.asset, dep.deposit_address.asset,
                "lol lx fucked up here pretty good",
            );
            self.events.insert(
                dep.created_at,
                Event::Deposit {
                    amount: dep.amount.with_asset(dep.asset.into()),
                    address: bitcoin::Address::from_str(&dep.deposit_address.address)
                        .expect("bitcoin address from LX was not a valid BTC address"),
                    asset: dep.asset,
                },
            );
        }
    }

    /// Import a list of withdrawals into the history
    pub fn import_withdrawals(&mut self, withdrawals: &Withdrawals) {
        for withd in &withdrawals.data {
            self.events.insert(
                withd.created_at,
                Event::Withdrawal {
                    amount: withd.amount.with_asset(withd.asset.into()),
                    asset: withd.asset,
                },
            );
        }
    }

    /// Import a list of trades into the history
    pub fn import_trades(
        &mut self,
        trades: &Trades,
        contracts: &HashMap<String, super::Contract>,
    ) -> Result<(), anyhow::Error> {
        for trade in &trades.data {
            let contract = match contracts.get(&trade.contract_id) {
                Some(contract) => contract.clone(),
                None => {
                    return Err(anyhow::Error::msg(format!(
                        "Unknown contract ID {}",
                        trade.contract_id
                    )))
                }
            };
            let asset = contract.asset();
            self.events.insert(
                trade.execution_time,
                Event::Trade {
                    contract,
                    price: trade.filled_price,
                    size: match trade.side {
                        Side::Bid => trade.filled_size.with_asset_trade(asset),
                        Side::Ask => -trade.filled_size.with_asset_trade(asset),
                    },
                    fee: trade.fee,
                },
            );
        }
        Ok(())
    }

    /// Import a list of positions into the history
    pub fn import_positions(&mut self, positions: &Positions) {
        for pos in &positions.data {
            // Unsettled positions don't have any trade logs associated with them
            if !pos.has_settled {
                continue;
            }

            // We do a bit of goofy sign-mangling here; the idea is that the assigned
            // and expipred "sizes" represent the net change in number of contracts
            // held, such that after expiry we have net 0. So for long positions,
            // both numbers will be negative.
            //
            // On the input front, pos.size will be positive or negative according to
            // whether we are long or short; assigned_size is always positive; and
            // the expired size is not encoded. So arguably it's LX that's mangling
            // signs in weird ways, and we're just unmangling them.
            let (assigned, expired) = if pos.size > 0 {
                // long positions
                (-pos.assigned_size, -pos.size + pos.assigned_size)
            } else {
                // short positions
                (pos.assigned_size, -pos.size - pos.assigned_size)
            };
            // This assertion maybe makes it clearer what we're doing.
            assert_eq!(assigned + expired, -pos.size, "{pos:?}");

            self.events.insert(
                pos.contract.expiry(),
                Event::Expiry {
                    contract: pos.contract.clone(),
                    assigned_size: UnknownQuantity::from(assigned).with_asset(pos.contract.asset()),
                    expired_size: UnknownQuantity::from(expired).with_asset(pos.contract.asset()),
                },
            );
        }
    }

    /// Dump the contents of the history in CSV format
    pub fn print_csv(&self, year: Option<i32>, price_history: &crate::price::Historic) {
        for (date, event) in &self.events {
            // lol we could be smarter about this, e.g. not even fetching old data
            if year.is_some() && year != Some(date.year()) {
                continue;
            }

            let btc_price = price_history.price_at(date);
            let btc_price = btc_price.btc_price; // just discard exact price timestamp
            let date_fmt = csv::DateTime(date);

            // First accumulate the CSV into tuples (between 0 and 2 of them). We do
            // it this way to ensure that every branch outputs the same type of data,
            // which is a basic sanity check.
            let csv = match event {
                Event::Deposit { asset, amount, .. } => (
                    Some((
                        "Deposit",
                        date_fmt,
                        BudgetAsset::from(*asset),
                        (None, *amount),
                        (btc_price, None, None),
                    )),
                    None,
                ),
                Event::Withdrawal { asset, amount } => (
                    Some((
                        "Withdraw",
                        date_fmt,
                        BudgetAsset::from(*asset),
                        (None, *amount),
                        (btc_price, None, None),
                    )),
                    None,
                ),
                Event::Trade {
                    contract,
                    price,
                    size,
                    ..
                } => match contract.ty() {
                    super::contract::Type::Option { opt, .. } => (
                        Some((
                            "Trade",
                            date_fmt,
                            contract.budget_asset().unwrap(),
                            (Some(*price), *size),
                            (
                                btc_price,
                                Some(csv::Iv(opt.bs_iv(date, btc_price, *price))),
                                Some(csv::Arr(opt.arr(date, btc_price, *price))),
                            ),
                        )),
                        None,
                    ),
                    super::contract::Type::NextDay { .. } => (
                        Some((
                            "Trade",
                            date_fmt,
                            contract.budget_asset().unwrap(),
                            (Some(*price), *size),
                            (btc_price, None, None),
                        )),
                        None,
                    ),
                    super::contract::Type::Future { .. } => {
                        unimplemented!("futures trading")
                    }
                },
                Event::Expiry {
                    contract,
                    assigned_size,
                    expired_size,
                } => match contract.ty() {
                    super::contract::Type::Option { .. } => {
                        let csv = (
                            "X",
                            date_fmt,
                            contract.budget_asset().unwrap(),
                            (None, Quantity::Zero),
                            (btc_price, None, None),
                        );
                        let mut expiry_csv = None;
                        if expired_size.is_nonzero() {
                            let mut csv_copy = csv;
                            csv_copy.0 = "Expiry";
                            csv_copy.3 .1 = *expired_size;
                            expiry_csv = Some(csv_copy);
                        }
                        let mut assign_csv = None;
                        if assigned_size.is_nonzero() {
                            let mut csv_copy = csv;
                            csv_copy.0 = "Assignment";
                            csv_copy.3 .1 = *assigned_size;
                            assign_csv = Some(csv_copy);
                        }
                        (expiry_csv, assign_csv)
                    }
                    // NextDays don't expire, they are "assigned". We don't log this as a distinct
                    // event because we consider the originating trade to be the actual event.
                    super::contract::Type::NextDay { .. } => {
                        assert!(expired_size.is_zero());
                        (None, None)
                    }
                    // TBH I don't know what happens with futures
                    super::contract::Type::Future { .. } => unreachable!(),
                },
            };

            // ...then output it
            if let Some(first) = csv.0 {
                println!("{}", CsvPrinter(first));
            }
            if let Some(second) = csv.1 {
                println!("{}", CsvPrinter(second));
            }
        }
    }

    /// Dump the contents of the history in CSV format, attempting to match the end-of-year
    /// 1099 support files that LX sends out
    ///
    /// These are in kinda a weird format. Note that "Date Acquired" and "Date Disposed of"
    /// are swapped relative to the claimed headings.
    ///
    /// The "proceeds" column seems to have an absolute value function applied to it.
    ///
    /// For trades, "Proceeds" and "basis" seem to be switched. As a consequence the gain/loss
    /// column is consistently negated.
    ///
    /// For short expires, "proceeds" means how much the options were worth and "basis" means 0.
    ///
    /// For expiries of long positions, "Date Acquired" and "Date sold or disposed of" are swapped
    ///
    /// There are also two empty columns I don't know the purpose of.
    ///
    /// The expiry timestamps are always UTC 22:00, which is 5PM in the winter but 6PM in the
    /// summer in new york. The assignment timestamps are always UTC 21:00.
    pub fn print_tax_csv(
        &self,
        config: &Configuration,
        config_hash: bitcoin::hashes::sha256::Hash,
        price_history: &crate::price::Historic,
    ) -> anyhow::Result<()> {
        // 0. Attempt to create output directory
        let now = time::OffsetDateTime::now_utc();
        let dir_path = format!("lx_tax_output_{}", now.lazy_format("%F-%H%M"));
        if fs::metadata(&dir_path).is_ok() {
            return Err(anyhow::Error::msg(format!(
                "Output directory {dir_path} exists. Refusing to run."
            )));
        }
        fs::create_dir(&dir_path)
            .with_context(|| format!("Creating directory {dir_path} to put tax output into"))?;
        info!("Creating directory {} to hold output.", dir_path);
        // Write out metadata, in part to make sure we can create files before
        // we do too much heavy lifting.
        let mut metadata = create_text_file(
            format!("{dir_path}/metadata"),
            "with metadata about this run.",
        )?;
        writeln!(metadata, "Started on: {now}")?;
        writeln!(metadata, "Tax year: {}", config.year())?;
        writeln!(metadata, "Configuration file hash: {}", config_hash)?;
        writeln!(
            metadata,
            "Events in this year: {}",
            self.events
                .iter()
                .filter(|(d, _)| d.year() == config.year())
                .count()
        )?;
        drop(metadata);

        // 1. Construct price reference from LX CSV
        let mut lx_price_ref = HashMap::new();
        for line in config.lx_csv() {
            let data = crate::ledgerx::csv::CsvLine::from_str(line).map_err(anyhow::Error::msg)?;
            for (time, price) in data.price_references() {
                debug!(
                    "At {} using LX-inferred price {} (our price feed gives {})",
                    time,
                    price,
                    price_history.price_at(time)
                );
                lx_price_ref.insert(time, price);
            }
        }
        // 2. Parse the transaction map as a transaction database
        let transaction_db = config
            .transaction_db()
            .context("extracting transaction database from config file")?;
        let lot_db = config.lot_db();
        // 3. Do the deed
        let btc_label = tax::Label::btc();
        let mut tracker = tax::PositionTracker::new();
        for (date, event) in &self.events {
            debug!("Processing event {:?}", event);
            if date.year() > config.year() {
                debug!(
                    "Encountered event with date {}, stopping as our tax year is {}",
                    date,
                    config.year()
                );
                break;
            }

            match event {
                // Deposits are not taxable events, but deposits of BTC cause lots to be
                // created (or at least, become accessible to our tax optimizer)
                Event::Deposit {
                    amount,
                    asset,
                    address,
                } => {
                    // sanity check asset
                    match *asset {
                        DepositAsset::Btc => {}        // ok
                        DepositAsset::Usd => continue, // USD deposits are not tax-relevant
                        DepositAsset::Eth => unimplemented!("we do not support eth deposits"),
                    }
                    debug!("[deposit] \"BTC\" {}", amount);

                    // sanity check amount
                    let mut amount_sat = match amount {
                        Quantity::Zero => 0,
                        Quantity::Bitcoin(btc) => btc.to_sat() as u64,
                        Quantity::Contracts(_) => unreachable!("deposit of so many contracts"),
                        Quantity::Cents(_) => unreachable!("USD deposits not supported yet"),
                    };

                    // Look up transaction based on address. If we can't find one, error out.
                    let (tx, vout) = transaction_db
                        .find_tx_for_deposit(address, amount_sat)
                        .with_context(|| {
                            format!("no txout matched address/amount {address}/{amount_sat}")
                        })?;

                    let outpoint_iter: Box<dyn Iterator<Item = bitcoin::OutPoint>>;
                    if tx.output.len() == 1 {
                        debug!(
                            "Assuming that a single-output deposit is from Andrew's wallet \
                                and that every input UXTO is a separate lot."
                        );
                        outpoint_iter = Box::new(tx.input.iter().map(|inp| inp.previous_output));
                    } else {
                        debug!("Assuming that a multi-output deposit is constitutes a single lot.");
                        outpoint_iter = Box::new(std::iter::once(bitcoin::OutPoint {
                            txid: tx.txid(),
                            vout,
                        }));
                    }

                    for op in outpoint_iter {
                        let id = LotId::from_outpoint(op);
                        let txout = transaction_db.find_txout(op).with_context(|| {
                            format!("config file did not have tx data for {op}")
                        })?;
                        let lot_data = lot_db.get(&id).with_context(|| {
                            format!("config file did not have info for lot {id}")
                        })?;

                        debug!("Using BTC price {} for lot {}", lot_data.price, id,);
                        let mut open = tax::Lot::from_deposit_utxo(
                            op,
                            lot_data.price,
                            bitcoin::Amount::from_sat(txout.value),
                            lot_data.date,
                        );
                        // Take fees away from the last input(s). We consider this a
                        // partial loss of the lot corresponding to the input
                        //
                        // A future iteration may consider this to be a taxable loss but this
                        // won't affect anything downstream, basically it'll just add an extra
                        // log line. FIXME implement this.
                        if txout.value > amount_sat {
                            open.dock_fee(bitcoin::Amount::from_sat(txout.value - amount_sat));
                            amount_sat = 0;
                        } else {
                            amount_sat -= txout.value;
                        }
                        // Assert that deposits do not close any positions (since we cannot have
                        // a short BTC position)
                        assert_eq!(tracker.push_lot(&btc_label, open, lot_data.date), 0);
                    }
                }
                // Withdrawals of any kind are not taxable events.
                //
                // FIXME BTC withdrawals should take lots out of commission. Not sure how to
                // choose this. Probably should make the user decide in config file.
                Event::Withdrawal { .. } => {
                    debug!("Ignore withdrawal");
                }
                // Trades may be
                Event::Trade {
                    contract,
                    price,
                    size,
                    fee,
                } => {
                    let label = tax::Label::from_contract(contract);
                    debug!("[trade] \"{}\" {} @ {}; fee {}", label, price, size, fee);
                    let (tax_date, is_btc) =
                        if let super::contract::Type::NextDay { .. } = contract.ty() {
                            // BTC longs don't happen until the following day...also ofc LX fucks
                            // up the date and fixes the time to 21:00
                            (
                                contract
                                    .expiry()
                                    .date()
                                    .with_time(time::time!(21:00))
                                    .assume_utc(),
                                true,
                            )
                        } else {
                            (date, false)
                        };
                    if is_btc {
                        let open = tax::Lot::from_trade_btc(*price, *size, *fee, tax_date);
                        tracker.push_lot(&label, open, date);
                    } else {
                        let open = tax::Lot::from_trade_opt(*price, *size, *fee, tax_date);
                        tracker.push_lot(&label, open, date);
                    }
                }
                // Both expiries and assignments may be taxable
                Event::Expiry {
                    contract,
                    assigned_size,
                    expired_size,
                } => {
                    let label = tax::Label::from_contract(contract);
                    debug!(
                        "[expiry] {} assigned {} expired {}",
                        label, assigned_size, expired_size
                    );
                    // Only do something if this is an option expiry -- dayaheads and futures
                    // also expire, but dayaheads we treat as sales at the time of sale, and
                    // futures we don't support.
                    if let Some(opt) = contract.as_option() {
                        if expired_size.is_nonzero() {
                            let open = tax::Lot::from_expiry(&opt, *expired_size);
                            tracker.push_lot(&label, open, date);
                        }

                        // An assignment is also a trade
                        if assigned_size.is_nonzero() {
                            // see "seriously WTF" doccomment
                            let expiry =
                                opt.expiry.date().with_time(time::time!(22:00)).assume_utc();

                            let btc_price = match lx_price_ref.get(&expiry) {
                                Some(price) => *price,
                                None => {
                                    // We allow this because otherwise we can't possibly produce
                                    // files until LX gives us their shit, which they take
                                    // forever to do. But arguably it should be a hard error
                                    // because the result will not be so easily justifiable to
                                    // the IRS.
                                    let btc_price = price_history.price_at(expiry);
                                    warn!(
                                        "Do not have LX price reference for {}; using price {}",
                                        expiry, btc_price
                                    );
                                    btc_price.btc_price
                                }
                            };

                            let open = tax::Lot::from_assignment(&opt, *assigned_size, btc_price);
                            tracker.push_lot(&label, open, date);

                            debug!("Because of assignment inserting a synthetic BTC trade");
                            assert_eq!(contract.underlying(), Underlying::Btc);
                            let contracts_in_btc = Quantity::from(assigned_size.btc_equivalent());
                            let open = tax::Lot::from_trade_btc(
                                btc_price, // notice the basis is NOT the strike price but the
                                // actual market price.
                                match opt.pc {
                                    crate::option::Call => -contracts_in_btc,
                                    crate::option::Put => contracts_in_btc,
                                },
                                Price::ZERO,
                                expiry,
                            );
                            tracker.push_lot(&btc_label, open, date);
                        }
                    }
                }
            };
        }
        tracker.lx_sort_events();

        let mut lx_file = create_text_file(
            format!("{dir_path}/ledgerx-sim.csv"),
            "which should match the LX-provided CSV.",
        )?;
        let mut lx_alt_file = create_text_file(
            format!("{dir_path}/ledgerx-sim-annotated.csv"),
            "which should match the LX-provided CSV, with one extra column for lot IDs",
        )?;
        writeln!(lx_file, "Reference,Description,Date Acquired,Date Sold or Disposed of,Proceeds,Cost or other basis,Gain/(Loss),Short-term/Long-term,,,Note that column C and column F reflect * where cost basis could not be obtained.")?;
        writeln!(lx_alt_file, "Reference,Description,Date Acquired,Date Sold or Disposed of,Proceeds,Cost or other basis,Gain/(Loss),Short-term/Long-term,,,Note that column C and column F reflect * where cost basis could not be obtained.,Lot ID")?;
        for event in tracker.events() {
            // Unlike with the Excel reports, we actually need to generate data for every
            // year, and we only dismiss non-current data now, when we're logging.
            if config.year() != event.date.0.year() {
                continue;
            }
            //let date = event.date.0.lazy_format("%F %H:%M:%S.%NZ");

            match event.open_close {
                tax::OpenClose::Open(..) => {}
                tax::OpenClose::Close(ref close) => {
                    let lx = close.csv_printer(&event.label, tax::PrintMode::LedgerX);
                    let lx_alt = close.csv_printer(&event.label, tax::PrintMode::LedgerXAnnotated);
                    writeln!(lx_file, "{}", lx)?;
                    writeln!(lx_alt_file, "{}", lx_alt)?;
                }
            }
        }
        Ok(())
    }
}
