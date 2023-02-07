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
use anyhow::Context;
use log::{debug, info, warn};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{de, Deserialize, Deserializer};
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use time::OffsetDateTime;

pub mod tax;

// Note that this is *not* the same as the equivalent function in ledgerx/json.rs
// For some reason LX returns timestamps in like a dozen different formats.
fn deserialize_datetime<'de, D>(deser: D) -> Result<OffsetDateTime, D::Error>
where
    D: Deserializer<'de>,
{
    let s: &str = Deserialize::deserialize(deser)?;
    OffsetDateTime::parse(s, time::Format::Rfc3339).map_err(|_| {
        de::Error::invalid_value(de::Unexpected::Str(&s), &"a datetime in RFC 3339 format")
    })
}

#[derive(Deserialize, Debug)]
struct Meta {
    #[serde(default)]
    next: Option<String>,
}

#[derive(Deserialize, Debug)]
struct Asset {
    name: super::Asset,
}

#[derive(Deserialize, Debug)]
struct DepositAddress {
    address: String,
    asset: super::Asset,
}

#[derive(Deserialize, Debug)]
struct Deposit {
    amount: i64,
    asset: Asset,
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
        self.meta
            .as_ref()
            .and_then(|meta| meta.next.as_ref().map(|s| s.clone()))
    }
}

#[derive(Deserialize, Debug)]
struct Withdrawal {
    amount: i64,
    // Note: withdrawals don't have the extra "name" indirection for some reason
    asset: super::Asset,
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
        self.meta
            .as_ref()
            .and_then(|meta| meta.next.as_ref().map(|s| s.clone()))
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
    filled_price: i64,
    filled_size: i64,
    side: Side,
    fee: i64,
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
                    &format!("https://api.ledgerx.com/trading/contracts/{}", id),
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
        self.meta
            .as_ref()
            .and_then(|meta| meta.next.as_ref().map(|s| s.clone()))
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
        self.meta
            .as_ref()
            .and_then(|meta| meta.next.as_ref().map(|s| s.clone()))
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum Event {
    Deposit {
        amount: Decimal,
        address: bitcoin::Address,
        asset: super::Asset,
    },
    Withdrawal {
        amount: Decimal,
        asset: super::Asset,
    },
    Trade {
        contract: super::Contract,
        price: Decimal,
        size: i64,
        fee: Decimal,
    },
    Expiry {
        contract: super::Contract,
        assigned_size: i64,
        expired_size: i64,
    },
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct History {
    events: BTreeMap<OffsetDateTime, Event>,
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
                dep.asset.name, dep.deposit_address.asset,
                "lol lx fucked up here pretty good",
            );
            self.events.insert(
                dep.created_at,
                Event::Deposit {
                    amount: match dep.asset.name {
                        super::Asset::Btc => Decimal::new(dep.amount, 8),
                        super::Asset::Usd => Decimal::new(dep.amount, 2),
                        super::Asset::Eth => unimplemented!("ethereum deposits"),
                    },
                    address: bitcoin::Address::from_str(&dep.deposit_address.address)
                        .expect("bitcoin address from LX was not a valid BTC address"),
                    asset: dep.asset.name,
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
                    amount: match withd.asset {
                        super::Asset::Btc => Decimal::new(withd.amount, 8),
                        super::Asset::Usd => Decimal::new(withd.amount, 2),
                        super::Asset::Eth => unimplemented!("ethereum withdrawals"),
                    },
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
            self.events.insert(
                trade.execution_time,
                Event::Trade {
                    contract: match contracts.get(&trade.contract_id) {
                        Some(contract) => contract.clone(),
                        None => {
                            return Err(anyhow::Error::msg(format!(
                                "Unknown contract ID {}",
                                trade.contract_id
                            )))
                        }
                    },
                    price: Decimal::new(trade.filled_price, 2),
                    size: match trade.side {
                        Side::Bid => trade.filled_size,
                        Side::Ask => -trade.filled_size,
                    },
                    fee: Decimal::new(trade.fee, 2),
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
            assert_eq!(assigned + expired, -pos.size, "{:?}", pos);

            self.events.insert(
                pos.contract.unique_expiry_date(),
                Event::Expiry {
                    contract: pos.contract.clone(),
                    assigned_size: assigned,
                    expired_size: expired,
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

            let btc_price = price_history.price_at(*date);
            let btc_price = btc_price.btc_price; // just discard exact price timestamp
            let date_fmt = csv::DateTime(*date);

            // First accumulate the CSV into tuples (between 0 and 2 of them). We do
            // it this way to ensure that every branch outputs the same type of data,
            // which is a basic sanity check.
            let csv = match event {
                Event::Deposit { asset, amount, .. } => (
                    Some((
                        "Deposit",
                        date_fmt,
                        (None, asset.as_str(), None),
                        (None, *amount),
                        (btc_price, None, None),
                    )),
                    None,
                ),
                Event::Withdrawal { asset, amount } => (
                    Some((
                        "Withdraw",
                        date_fmt,
                        (None, asset.as_str(), None),
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
                            opt.csv_tuple(),
                            (Some(*price), Decimal::from(*size)),
                            (
                                btc_price,
                                Some(csv::Iv(opt.bs_iv(*date, btc_price, *price))),
                                Some(csv::Arr(opt.arr(*date, btc_price, *price))),
                            ),
                        )),
                        None,
                    ),
                    super::contract::Type::NextDay { .. } => (
                        Some((
                            "Trade",
                            date_fmt,
                            (None, contract.underlying().as_str(), None),
                            (Some(*price), Decimal::from(*size)),
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
                    super::contract::Type::Option { opt, .. } => {
                        let csv = (
                            "X",
                            date_fmt,
                            opt.csv_tuple(),
                            (None, Decimal::ZERO),
                            (btc_price, None, None),
                        );
                        let mut expiry_csv = None;
                        if *expired_size != 0 {
                            let mut csv_copy = csv;
                            csv_copy.0 = "Expiry";
                            csv_copy.3 .1 = Decimal::from(*expired_size);
                            expiry_csv = Some(csv_copy);
                        }
                        let mut assign_csv = None;
                        if *assigned_size != 0 {
                            let mut csv_copy = csv;
                            csv_copy.0 = "Assignment";
                            csv_copy.3 .1 = Decimal::from(*assigned_size);
                            assign_csv = Some(csv_copy);
                        }
                        (expiry_csv, assign_csv)
                    }
                    // NextDays don't expire, they are "assigned". We don't log this as a distinct
                    // event because we consider the originating trade to be the actual event.
                    super::contract::Type::NextDay { .. } => {
                        assert_eq!(*expired_size, 0);
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
        year: i32,
        mode: crate::TaxHistoryMode,
        price_history: &crate::price::Historic,
        transaction_db: &crate::transaction::Database,
    ) {
        let btc_label = tax::Label::btc();
        let mut tracker = tax::PositionTracker::new();
        for (date, event) in &self.events {
            debug!("Processing event {:?}", event);
            match event {
                // Deposits and withdrawals are not taxable events
                Event::Deposit {
                    amount,
                    asset,
                    address,
                } => {
                    let btc_price = price_history.price_at(*date);
                    debug!(
                        "Looked up BTC price for deposit at {}, got {} ({})",
                        date, btc_price.btc_price, btc_price.timestamp,
                    );
                    let btc_price = btc_price.btc_price;
                    // sanity check asset
                    match *asset {
                        super::Asset::Btc => {}        // ok
                        super::Asset::Usd => continue, // USD deposits are not tax-relevant
                        super::Asset::Eth => unimplemented!("we do not support eth deposits"),
                    }
                    debug!("[deposit] \"BTC\" {} @ {}", btc_price, amount);

                    let mut amount_sat = (amount * Decimal::from(100_000_000)).to_u64().unwrap();
                    let mut just_make_something_up = false;
                    let mut deposit_outpoint = bitcoin::OutPoint::default();
                    if let Some((tx, vout)) =
                        transaction_db.find_tx_for_deposit(address, amount_sat)
                    {
                        deposit_outpoint = bitcoin::OutPoint {
                            txid: tx.txid(),
                            vout: vout,
                        };
                        if tx.output.len() == 1 {
                            debug!(
                                "Assuming that a single-output deposit is from Andrew's wallet \
                                    and that every input UXTO is a separate lot."
                            );
                            for input in &tx.input {
                                let op = input.previous_output;
                                if let Some((txout, txout_date)) =
                                    transaction_db.find_txout(op.txid, op.vout)
                                {
                                    let price = price_history.price_at(txout_date);
                                    debug!(
                                        "Looked up BTC price for input {}:{} at {}, got {} ({})",
                                        op.txid,
                                        op.vout,
                                        txout_date,
                                        price.btc_price,
                                        price.timestamp,
                                    );
                                    let open = tax::Lot::from_deposit_utxo(
                                        op,
                                        price.btc_price,
                                        txout.value,
                                        txout_date,
                                    );
                                    assert_eq!(tracker.push_lot(&btc_label, open), 0);
                                    // Take fees away from the last input(s). We consider this a
                                    // partial loss of the lot corresponding to the input
                                    if txout.value > amount_sat {
                                        let open = tax::Lot::from_tx_fee(
                                            txout.value - amount_sat,
                                            *date, // date is now, not txout_date
                                        );
                                        assert_eq!(tracker.push_lot(&btc_label, open), 1);
                                        amount_sat = 0;
                                    } else {
                                        amount_sat -= txout.value;
                                    }
                                } else {
                                    warn!(
                                        "Please import txdata for {}. For now assuming CB of {}",
                                        op.txid, btc_price,
                                    );
                                    just_make_something_up = true;
                                }
                            }
                        } else {
                            debug!(
                                "Assuming that a multi-output deposit came from some shared \
                                    exchange or something; treating the deposit as a single lot \
                                    and ignoring the inputs."
                            );
                            just_make_something_up = true;
                        }
                    } else {
                        warn!(
                            "No transaction found for deposit of size {} to {} on {}. Assuming CB of {}.",
                            amount,
                            address,
                            date.lazy_format("%F %T.%N"),
                            btc_price,
                        );
                        just_make_something_up = true;
                    }

                    // "Just make something up" is a little strong. What it means is that we treat
                    // the deposit as a lot in the deposit amount on the deposit date, at the
                    // prevailing BTC price.
                    if just_make_something_up {
                        let open = tax::Lot::from_deposit_utxo(
                            deposit_outpoint,
                            btc_price,
                            amount_sat,
                            *date,
                        );
                        assert_eq!(tracker.push_lot(&btc_label, open), 0);
                    }
                }
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
                            (*date, false)
                        };
                    let adj_size = if is_btc { *size * 1_000_000 } else { *size };
                    let open = tax::Lot::from_trade(*price, adj_size, *fee, tax_date, is_btc);
                    tracker.push_lot(&label, open);
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
                        if *expired_size != 0 {
                            let open = tax::Lot::from_expiry(&opt, *expired_size);
                            tracker.push_lot(&label, open);
                        }

                        // An assignment is also a trade
                        if *assigned_size != 0 {
                            let btc_price = price_history.price_at(*date);
                            debug!(
                                "Looked up BTC price at {}, got {} ({})",
                                date, btc_price.btc_price, btc_price.timestamp,
                            );
                            let btc_price = btc_price.btc_price;

                            let open = tax::Lot::from_assignment(&opt, *assigned_size, btc_price);
                            tracker.push_lot(&label, open);

                            debug!("Because of assignment inserting a synthetic BTC trade");
                            assert_eq!(contract.underlying(), super::Asset::Btc);
                            // see "seriously WTF" comment
                            let expiry =
                                opt.expiry.date().with_time(time::time!(22:00)).assume_utc();
                            let open = tax::Lot::from_trade(
                                btc_price, // notice the basis is NOT the strike price but the
                                // actual market price.
                                match opt.pc {
                                    crate::option::Call => *assigned_size * -1_000_000,
                                    crate::option::Put => *assigned_size * 1_000_000,
                                },
                                Decimal::ZERO,
                                expiry,
                                true, // is_btc
                            );
                            tracker.push_lot(&btc_label, open);
                        }
                    }
                }
            };
        }

        tracker.lx_sort_events();

        for event in tracker.events() {
            // Unlike with the Excel reports, we actually need to generate data for every
            // year, and we only dismiss non-current data now, when we're logging.
            if year != event.date.0.year() {
                continue;
            }
            let date = event.date.0.lazy_format("%F %H:%M:%S.%NZ");

            match event.open_close {
                tax::OpenClose::Open(ref open) => {
                    if let crate::TaxHistoryMode::JustLotIds = mode {
                        println!("{:35}: {}:  open lot {}", event.label, date, open);
                    }
                }
                tax::OpenClose::Close(ref close) => match mode {
                    crate::TaxHistoryMode::JustLxData => {
                        info!("{}", close.csv_printer(&event.label, false));
                    }
                    crate::TaxHistoryMode::JustLotIds => {
                        println!("{:35}: {}: close lot {}", event.label, date, close);
                    }
                    crate::TaxHistoryMode::Both => {
                        info!("{}", close.csv_printer(&event.label, true));
                    }
                },
            }
        }
    }
}
