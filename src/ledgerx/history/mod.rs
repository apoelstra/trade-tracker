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
    BudgetAsset, DepositAsset, Price, Quantity, TaxAsset, Underlying, UnknownQuantity, UtcTime,
};
use anyhow::Context;
use chrono::{Datelike as _, Timelike as _};
use log::{debug, info, warn};
use serde::Deserialize;
use std::collections::{hash_map, BTreeMap, HashMap};
use std::str::FromStr;

pub mod config;
pub mod lot;
pub mod tax;

pub use self::config::Configuration;
pub use self::lot::Id as LotId;

#[derive(Deserialize, Debug)]
struct Meta {
    #[serde(default)]
    next: Option<String>,
}

#[derive(Deserialize, Debug)]
struct Deposit {
    amount: UnknownQuantity,
    asset: DepositAsset,
    address: String,
    created_at: UtcTime,
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
    asset: DepositAsset,
    created_at: UtcTime,
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
    execution_time: UtcTime,
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
        asset: TaxAsset,
        price: Price,
        size: Quantity,
        fee: Price,
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
    user_id: usize,
    years: BTreeMap<i32, tax::LotSelectionStrategy>,
    lot_db: HashMap<LotId, config::LotInfo>,
    transaction_db: crate::transaction::Database,
    lx_price_ref: HashMap<UtcTime, Price>,
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
            match crate::ledgerx::csv::price_ref(line) {
                Err(e) => Err(anyhow::Error::msg(e))
                    .with_context(|| format!("Parsing CSV line {line}"))?,
                Ok(Some((date, price))) => {
                    debug!("At {} using LX-inferred price {}", date, price,);
                    lx_price_ref.insert(date, price);
                }
                Ok(None) => {} // no price ref
            }
        }
        // Extract transaction database from list of raw transactions
        let transaction_db = config
            .transaction_db()
            .context("extracting transaction database from config file")?;
        // Return
        Ok(History {
            user_id: config.user,
            years: config.years().clone(),
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
    fn import_deposits(&mut self, deposits: &Deposits) -> anyhow::Result<()> {
        for dep in &deposits.data {
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
                    let addr = bitcoin::Address::from_str(&dep.address)
                        .with_context(|| format!("parsing BTC address {}", dep.address))?;

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
    fn import_withdrawals(&mut self, withdrawals: &Withdrawals) {
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
    fn import_trades(
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
                    asset: contract
                        .tax_asset()
                        .with_context(|| format!("getting tax asset for {contract}"))?,
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
    fn import_positions(&mut self, positions: &Positions) {
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

            // LedgerX's data has the time forced to 22:00 even when DST makes this wrong
            let price_ref_date = if option.expiry.year() == 2021 {
                assert_eq!(option.expiry.minute(), 0);
                assert_eq!(option.expiry.second(), 0);
                assert_eq!(option.expiry.nanosecond(), 0);
                option.expiry.with_hour(22).unwrap().into()
            } else {
                option.expiry + chrono::Duration::hours(1)
            };

            // Insert the expiry event, if any (in 2021 this is BEFORE assignment, in 2022 AFTER)
            if option.expiry.year() == 2021 && expired != 0 {
                self.events.insert(
                    price_ref_date,
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
                self.events.insert(
                    price_ref_date,
                    Event::Assignment {
                        option,
                        underlying: pos.contract.underlying(),
                        size: n_assigned,
                        price_ref: self.lx_price_ref.get(&price_ref_date).copied(),
                    },
                );
            }
            // Insert the expiry event, if any (in 2021 this is BEFORE assignment, in 2022 AFTER)
            if option.expiry.year() > 2021 && expired != 0 {
                self.events.insert(
                    price_ref_date,
                    Event::Expiry {
                        option,
                        underlying: pos.contract.underlying(),
                        size: UnknownQuantity::from(expired).with_asset(pos.contract.asset()),
                    },
                );
            }
        }
    }

    /// Dump the contents of the history in CSV format
    pub fn print_csv(&self, price_history: &crate::price::Historic) {
        for (date, event) in &self.events {
            // Skip years that we haven't set a tax strategy for
            if !self.years.contains_key(&date.year()) {
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
                Event::Trade {
                    asset, price, size, ..
                } => (
                    "Trade",
                    date_fmt,
                    BudgetAsset::from(*asset),
                    (Some(*price), *size),
                    match asset {
                        TaxAsset::Bitcoin | TaxAsset::NextDay { .. } => (btc_price, None, None),
                        TaxAsset::Option { option, .. } => (
                            btc_price,
                            Some(csv::Iv(option.bs_iv(date, btc_price, *price))),
                            Some(csv::Arr(option.arr(date, btc_price, *price))),
                        ),
                    },
                ),
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
    pub fn print_tax_csv(
        &self,
        dir_path: &str,
        price_history: &crate::price::Historic,
    ) -> anyhow::Result<()> {
        // Write out metadata, in part to make sure we can create files before
        // we do too much heavy lifting.
        let mut metadata = create_text_file(
            format!("{dir_path}/metadata.txt"),
            "with metadata about this run.",
        )?;
        writeln!(
            metadata,
            "Started on: {}",
            chrono::offset::Utc::now().format("%F %H:%M:%S UTC")
        )?;
        writeln!(metadata, "Configuration file hash: {}", self.config_hash)?;

        let mut tracker = tax::PositionTracker::new();
        for (date, event) in &self.events {
            debug!("Processing event {:?}", event);
            if let Some(strat) = self.years.get(&date.year()) {
                tracker.set_bitcoin_lot_strategy(*strat);
            } else {
                warn!(
                    "Have no tax strategy for year {}. Stopping here.",
                    date.year()
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
                    let lot =
                        lot::Lot::from_deposit(*outpoint, lot_info.price, *amount, lot_info.date);
                    tracker.push_lot(date.into(), lot);
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
                    asset,
                    price,
                    size,
                    fee,
                } => {
                    debug!("[trade] \"{}\" {} @ {}; fee {}", asset, size, price, fee,);

                    let adj_price = *price + *fee / *size; // nb `unit_fee` is a signed quantity

                    tracker
                        .push_trade(*asset, *size, adj_price, date.into())
                        .with_context(|| format!("pushing trade of {asset} size {size}"))?;
                }
                // Expiries are a simple tax event (a straight gain)
                Event::Expiry {
                    option,
                    underlying,
                    size,
                } => {
                    debug!("[expiry] {} {} expired {}", underlying, option, size);
                    tracker
                        .push_expiry(*option, *underlying, *size)
                        .with_context(|| format!("expiring option {option} n {size}"))?;
                }
                // Assignments are less simple because we need a price reference to compute
                // the intrinsic price (with expiries this is assumed to be zero)
                Event::Assignment {
                    option,
                    underlying,
                    size,
                    price_ref,
                } => {
                    debug!(
                        "[expiry] {} {} assigned {} at date {}",
                        underlying, option, size, date
                    );
                    let btc_price = match price_ref {
                        Some(price) => *price,
                        None => {
                            // We allow this because otherwise we can't possibly produce
                            // files until LX gives us their shit, which they take
                            // forever to do. But arguably it should be a hard error
                            // because the result will not be so easily justifiable to
                            // the IRS.
                            let btc_price = price_history.price_at(date);
                            warn!(
                                "Do not have LX price reference for {}; using price {}",
                                date, btc_price
                            );
                            writeln!(
                                metadata,
                                "WARNING: used non-official price reference of {} on {} for calculating \
                                 assignment loss (strike {} size {})",
                                btc_price.btc_price, date, option.strike, size,
                            )?;
                            btc_price.btc_price
                        }
                    };

                    tracker
                        .push_assignment(*option, *underlying, *size, btc_price)
                        .with_context(|| format!("assignment option {option} n {size}"))?;
                }
            };
        }
        tracker.lx_sort_events();

        for (year, strat) in &self.years {
            writeln!(metadata)?;
            writeln!(metadata, "Year: {year}")?;
            writeln!(metadata, "    Lot selection strategy: {strat}")?;
            let mut n_events = 0;
            let mut total_1256 = Price::ZERO;
            let mut total_st = Price::ZERO;
            let mut total_lt = Price::ZERO;
            for ev in tracker.events().iter().filter(|ev| ev.date.year() == *year) {
                n_events += 1;
                if let tax::OpenClose::Close(ref close) = ev.open_close {
                    match close.gain_loss_type() {
                        tax::GainType::Option1256 => total_1256 += close.gain_loss(),
                        tax::GainType::ShortTerm => total_st += close.gain_loss(),
                        tax::GainType::LongTerm => total_lt += close.gain_loss(),
                    }
                }
            }
            writeln!(metadata, "    Number of events: {n_events}")?;
            writeln!(metadata, "    Total LT gain/loss: {total_lt}")?;
            writeln!(metadata, "    Total ST gain/loss: {total_st}")?;
            writeln!(metadata, "    Total 1256 gain/loss: {total_1256}")?;
            let lt = total_lt + total_1256.sixty();
            let st = total_st + total_1256.forty();
            writeln!(metadata, "    After 60/40 splitting {lt} LT {st} ST")?;
            if st < Price::ZERO {
                let total = lt + st;
                if total >= Price::ZERO {
                    // ST losses can cancel LT gains
                    writeln!(metadata, "    Cancelling, total liability is {total} LT")?;
                } else {
                    // ...though once all the LT gains are cancelled, what's left is a ST loss
                    writeln!(metadata, "    Cancelling, total liability is {total} ST")?;
                }
            }
        }

        let mut reports_lx = HashMap::new();
        let mut reports_full = HashMap::new();
        for event in tracker.events() {
            let year = event.date.year();
            debug!("WRITING OUT date {} event: {:?}", event.date, event);
            // Open LX file for this year
            if let hash_map::Entry::Vacant(e) = reports_lx.entry(year) {
                let mut new_lx = create_text_file(
                    format!("{dir_path}/{year}-ledgerx.csv"),
                    "which should match the LX-provided CSV.",
                )?;
                writeln!(
                    new_lx,
                    "Reference,Description,Date Acquired,Date Sold or Disposed of,\
                     Proceeds,Cost or other basis,Gain/(Loss),Short-term/Long-term,,,\
                     Note that column C and column F reflect * where cost basis could not be obtained."
                )?;
                e.insert(new_lx);
            }
            let report_lx = reports_lx.get_mut(&year).unwrap();
            // Open full report file for this year
            if let hash_map::Entry::Vacant(e) = reports_full.entry(year) {
                let mut new_full = create_text_file(
                    format!("{dir_path}/{year}-full.csv"),
                    "which should provide a full tax accounting, matching LX's totals",
                )?;
                writeln!(
                    new_full,
                    "Event,Date,Quantity,Asset,Price,Lot ID,Old Lot Size,Old Lot Basis,\
                     New Lot Size,New Lot Basis,Basis,Proceeds,Gain/Loss,Gain/Loss Type"
                )?;
                e.insert(new_full);
            }
            let report_full = reports_full.get_mut(&year).unwrap();

            match event.open_close {
                tax::OpenClose::Open(ref lot) => {
                    writeln!(report_full, "{}", lot.csv_printer())?;
                }
                tax::OpenClose::Close(ref close) => {
                    let lx = close.csv_printer(event.asset, self.user_id, lot::PrintMode::LedgerX);
                    //let lx_alt = close.csv_printer(event.asset, lot::PrintMode::LedgerXAnnotated);
                    let full = close.csv_printer(event.asset, self.user_id, lot::PrintMode::Full);
                    debug!("report_lx: {}", lx);
                    debug!("report_full: {}", full);
                    writeln!(report_lx, "{lx}")?;
                    writeln!(report_full, "{full}")?;
                }
            }
        }
        Ok(())
    }
}
