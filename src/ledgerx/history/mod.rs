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
use crate::units::{
    BudgetAsset, DepositAsset, Price, Quantity, TaxAsset, Underlying, UnknownQuantity,
};
use anyhow::Context;
use log::{debug, info, warn};
use serde::{de, Deserialize, Deserializer};
use std::collections::HashMap;
use std::fs;
use std::str::FromStr;
use time::OffsetDateTime;

pub mod config;
pub mod lot;
pub mod tax;

pub use self::config::Configuration;
pub use self::lot::Id as LotId;

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
    UsdDeposit {
        amount: Quantity,
    },
    BtcDeposit {
        amount: bitcoin::Amount,
        outpoint: bitcoin::OutPoint,
        lot_info: config::LotInfo,
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
        synthetic: bool,
    },
    Assignment {
        option: crate::option::Option,
        underlying: Underlying,
        size: Quantity,
        price_ref: Option<Price>,
    },
    Expiry {
        option: crate::option::Option,
        underlying: Underlying,
        size: Quantity,
    },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct History {
    year: i32,
    lot_db: HashMap<LotId, config::LotInfo>,
    transaction_db: crate::transaction::Database,
    lx_price_ref: HashMap<OffsetDateTime, Price>,
    config_hash: bitcoin::hashes::sha256::Hash,
    events: crate::TimeMap<Event>,
}

impl History {
    /// Construct a new empty history
    pub fn new(
        config: &Configuration,
        config_hash: bitcoin::hashes::sha256::Hash,
    ) -> anyhow::Result<Self> {
        // Extract price reference from LX CSV lines
        let mut lx_price_ref = HashMap::new();
        for line in config.lx_csv() {
            let data = crate::ledgerx::csv::CsvLine::from_str(line).map_err(anyhow::Error::msg)?;
            for (time, price) in data.price_references() {
                debug!("At {} using LX-inferred price {}", time, price,);
                lx_price_ref.insert(time, price);
            }
        }
        // Extract transaction database from list of raw transactions
        let transaction_db = config
            .transaction_db()
            .context("extracting transaction database from config file")?;
        // Return
        Ok(History {
            year: config.year(),
            lot_db: config.lot_db().clone(),
            transaction_db,
            lx_price_ref,
            config_hash,
            events: Default::default(),
        })
    }

    /// Construct a new history by calling the LX API
    pub fn from_api(
        api_key: &str,
        config: &Configuration,
        config_hash: bitcoin::hashes::sha256::Hash,
    ) -> anyhow::Result<Self> {
        let mut ret = History::new(config, config_hash)?;
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

            ret.import_deposits(&deposits)
                .context("importing deposits")?;
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
    pub fn import_deposits(&mut self, deposits: &Deposits) -> anyhow::Result<()> {
        for dep in &deposits.data {
            assert_eq!(
                dep.asset, dep.deposit_address.asset,
                "lol lx fucked up here pretty good",
            );
            let amount = dep.amount.with_asset(dep.asset.into());
            match dep.asset {
                // ETH deposits are easy
                DepositAsset::Eth => unimplemented!("we do not support eth deposits"),
                // USD deposits almost as easy
                DepositAsset::Usd => {
                    self.events
                        .insert(dep.created_at, Event::UsdDeposit { amount });
                }
                // BTC deposits are much more involved, as we need to sort out lots
                DepositAsset::Btc => {
                    let total_btc = dep.amount.as_sats().to_unsigned().with_context(|| {
                        format!("negative deposit amount {}", dep.amount.as_sats())
                    })?;
                    let addr = bitcoin::Address::from_str(&dep.deposit_address.address)
                        .with_context(|| {
                            format!("parsing BTC address {}", dep.deposit_address.address)
                        })?;

                    // Look up transaction based on address. If we can't find one, error out.
                    let (tx, vout) = self
                        .transaction_db
                        .find_tx_for_deposit(&addr, total_btc)
                        .with_context(|| {
                            format!("no txout matched address/amount {addr}/{total_btc}")
                        })?;

                    if tx.output.len() == 1 {
                        debug!(
                            "Assuming that a single-output deposit is from Andrew's wallet \
                                and that every input UXTO is a separate lot."
                        );
                        let mut total_btc = total_btc;
                        for outpoint in tx.input.iter().map(|inp| inp.previous_output) {
                            let txout =
                                self.transaction_db.find_txout(outpoint).with_context(|| {
                                    format!("config file did not have tx data for {outpoint}")
                                })?;
                            let id = LotId::from_outpoint(outpoint);
                            let lot_info = self
                                .lot_db
                                .get(&id)
                                .with_context(|| {
                                    format!("config file did not have info for lot {id}")
                                })?
                                .clone();
                            debug!(
                                "Lot {}: price {} date {}",
                                id, lot_info.price, lot_info.date
                            );
                            // Take fees away from the last input(s). We consider this a
                            // partial loss of the lot corresponding to the input
                            //
                            // A future iteration may consider this to be a taxable loss but this
                            // won't affect anything downstream, basically it'll just add an extra
                            // log line. FIXME implement this.
                            let mut amount = bitcoin::Amount::from_sat(txout.value);
                            if amount > total_btc {
                                amount = total_btc;
                            };
                            total_btc -= amount;
                            self.events.insert(
                                dep.created_at,
                                Event::BtcDeposit {
                                    amount,
                                    outpoint,
                                    lot_info,
                                },
                            );
                        }
                    } else {
                        debug!("Assuming that a multi-output deposit is constitutes a single lot.");
                        let outpoint = bitcoin::OutPoint {
                            txid: tx.txid(),
                            vout,
                        };
                        let id = LotId::from_outpoint(outpoint);
                        let lot_info = self
                            .lot_db
                            .get(&id)
                            .with_context(|| format!("config file did not have info for lot {id}"))?
                            .clone();
                        debug!(
                            "Lot {}: price {} date {}",
                            id, lot_info.price, lot_info.date
                        );
                        self.events.insert(
                            dep.created_at,
                            Event::BtcDeposit {
                                amount: total_btc,
                                outpoint: bitcoin::OutPoint {
                                    txid: tx.txid(),
                                    vout,
                                },
                                lot_info,
                            },
                        );
                    }
                }
            }
        }
        Ok(())
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
                    synthetic: false,
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
            // Non-options are "expired" in a trivial sense (and are taxed at the time
            // of sale, as a sale) and never assigned, so ignore them here.
            let option = match pos.contract.as_option() {
                Some(opt) => opt,
                None => continue,
            };

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

            // Insert the expiry event, if any
            if expired != 0 {
                self.events.insert(
                    option.expiry,
                    Event::Expiry {
                        option,
                        underlying: pos.contract.underlying(),
                        size: UnknownQuantity::from(expired).with_asset(pos.contract.asset()),
                    },
                );
            }
            // Insert the assignment event, if any
            if assigned != 0 {
                let n_assigned = UnknownQuantity::from(assigned).with_asset(pos.contract.asset());
                // LedgerX's data has the time forced to 22:00 even when DST makes this wrong
                let price_ref_date = option
                    .expiry
                    .date()
                    .with_time(time::time!(22:00))
                    .assume_utc();

                self.events.insert(
                    option.expiry,
                    Event::Assignment {
                        option,
                        underlying: pos.contract.underlying(),
                        size: n_assigned,
                        price_ref: self.lx_price_ref.get(&price_ref_date).copied(),
                    },
                );

                // Assignments also cause synthetic trades to happen, which have the
                // same tax consequences as any other trade.
                debug!("Because of assignment inserting a synthetic BTC trade");
                let contracts_in_btc = Quantity::from(n_assigned.btc_equivalent());
                self.events.insert(
                    option.expiry,
                    Event::Trade {
                        contract: pos.contract.clone(),
                        price: option.strike,
                        size: match option.pc {
                            crate::option::Call => -contracts_in_btc,
                            crate::option::Put => contracts_in_btc,
                        },
                        fee: Price::ZERO,
                        synthetic: true,
                    },
                );
            }
        }
    }

    /// Dump the contents of the history in CSV format
    pub fn print_csv(&self, price_history: &crate::price::Historic) {
        for (date, event) in &self.events {
            // lol we could be smarter about this, e.g. not even fetching old data
            if self.year != date.year() {
                continue;
            }

            let btc_price = price_history.price_at(date);
            let btc_price = btc_price.btc_price; // just discard exact price timestamp
            let date_fmt = csv::DateTime(date);

            // First accumulate the CSV into tuples (between 0 and 2 of them). We do
            // it this way to ensure that every branch outputs the same type of data,
            // which is a basic sanity check.
            let csv = match event {
                Event::UsdDeposit { amount, .. } => (
                    "Deposit",
                    date_fmt,
                    BudgetAsset::Usd,
                    (None, *amount),
                    (btc_price, None, None),
                ),
                Event::BtcDeposit { amount, .. } => (
                    "Deposit",
                    date_fmt,
                    BudgetAsset::Btc,
                    (None, (*amount).into()),
                    (btc_price, None, None),
                ),
                Event::Withdrawal { asset, amount } => (
                    "Withdraw",
                    date_fmt,
                    BudgetAsset::from(*asset),
                    (None, *amount),
                    (btc_price, None, None),
                ),
                // Ignore synthetic trades for spreadsheeting purposes
                Event::Trade { synthetic, .. } if *synthetic == true => continue,
                Event::Trade {
                    contract,
                    price,
                    size,
                    ..
                } => match contract.ty() {
                    super::contract::Type::Option { opt, .. } => (
                        "Trade",
                        date_fmt,
                        contract.budget_asset().unwrap(),
                        (Some(*price), *size),
                        (
                            btc_price,
                            Some(csv::Iv(opt.bs_iv(date, btc_price, *price))),
                            Some(csv::Arr(opt.arr(date, btc_price, *price))),
                        ),
                    ),
                    super::contract::Type::NextDay { .. } => (
                        "Trade",
                        date_fmt,
                        contract.budget_asset().unwrap(),
                        (Some(*price), *size),
                        (btc_price, None, None),
                    ),
                    super::contract::Type::Future { .. } => {
                        unimplemented!("futures trading")
                    }
                },
                // FIXME use LX btc price
                Event::Expiry {
                    option,
                    underlying,
                    size,
                }
                | Event::Assignment {
                    option,
                    underlying,
                    size,
                    ..
                } => (
                    if let Event::Expiry { .. } = event {
                        "Expiry"
                    } else {
                        "Assignment"
                    },
                    date_fmt,
                    BudgetAsset::Option {
                        underlying: *underlying,
                        option: *option,
                    },
                    (None, *size),
                    (btc_price, None, None),
                ),
            };

            // ...then output it
            println!("{}", CsvPrinter(csv));
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
    pub fn print_tax_csv(&self, price_history: &crate::price::Historic) -> anyhow::Result<()> {
        // 0. Attempt to create output directory
        let now = OffsetDateTime::now_utc();
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
        writeln!(metadata, "Tax year: {}", self.year)?;
        writeln!(metadata, "Configuration file hash: {}", self.config_hash)?;
        writeln!(
            metadata,
            "Events in this year: {}",
            self.events
                .iter()
                .filter(|(d, _)| d.year() == self.year)
                .count()
        )?;
        drop(metadata);

        let mut tracker = tax::PositionTracker::new();
        for (date, event) in &self.events {
            debug!("Processing event {:?}", event);
            if date.year() > self.year {
                debug!(
                    "Encountered event with date {}, stopping as our tax year is {}",
                    date, self.year
                );
                break;
            }

            match event {
                // USD deposits are not tax-relevant
                Event::UsdDeposit { .. } => continue,
                // Deposits of BTC cause lots to be become accessible to our tax optimizer
                Event::BtcDeposit {
                    amount,
                    outpoint,
                    lot_info,
                } => {
                    debug!("[deposit] \"BTC\" {} outpoint {}", amount, outpoint);
                    let open = tax::Lot::from_deposit_utxo(
                        *outpoint,
                        lot_info.price,
                        *amount,
                        lot_info.date,
                    );
                    // Assert that deposits do not close any positions (since we cannot have
                    // a short BTC position)
                    assert_eq!(tracker.push_lot(TaxAsset::Btc, open, lot_info.date), 0);
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
                    synthetic,
                } => {
                    debug!(
                        "[trade] \"{}\" {} @ {}; fee {}; synthetic: {}",
                        contract, price, size, fee, synthetic
                    );
                    let asset = contract
                        .tax_asset()
                        .with_context(|| format!("asset of {contract} not supported (taxes)"))?;
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
                        tracker.push_lot(asset, open, date);
                    } else {
                        let open = tax::Lot::from_trade_opt(*price, *size, *fee, tax_date);
                        tracker.push_lot(asset, open, date);
                    }
                }
                // Expiries are a simple tax event (a straight gain)
                Event::Expiry {
                    option,
                    underlying,
                    size,
                } => {
                    let asset = TaxAsset::Option {
                        underlying: *underlying,
                        option: *option,
                    };
                    debug!("[expiry] {} expired {}", asset, size);
                    // Only do something if this is an option expiry -- dayaheads and futures
                    // also expire, but dayaheads we treat as sales at the time of sale, and
                    // futures we don't support.
                    let open = tax::Lot::from_expiry(option, *size);
                    tracker.push_lot(asset, open, date);
                }
                // Assignments are less simple
                Event::Assignment {
                    option,
                    underlying,
                    size,
                    price_ref,
                } => {
                    let asset = TaxAsset::Option {
                        underlying: *underlying,
                        option: *option,
                    };
                    debug!("[expiry] {} assigned {}", asset, size);
                    // see "seriously WTF" doccomment
                    let expiry = option
                        .expiry
                        .date()
                        .with_time(time::time!(22:00))
                        .assume_utc();

                    let btc_price = match price_ref {
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

                    let open = tax::Lot::from_assignment(option, *size, btc_price);
                    tracker.push_lot(asset, open, date);
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
            if self.year != event.date.0.year() {
                continue;
            }
            //let date = event.date.0.lazy_format("%F %H:%M:%S.%NZ");

            match event.open_close {
                tax::OpenClose::Open(..) => {}
                tax::OpenClose::Close(ref close) => {
                    let lx = close.csv_printer(event.asset, tax::PrintMode::LedgerX);
                    let lx_alt = close.csv_printer(event.asset, tax::PrintMode::LedgerXAnnotated);
                    writeln!(lx_file, "{}", lx)?;
                    writeln!(lx_alt_file, "{}", lx_alt)?;
                }
            }
        }
        Ok(())
    }
}
