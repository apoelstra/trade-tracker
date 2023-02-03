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

use anyhow::Context;
use rust_decimal::Decimal;
use serde::{de, Deserialize, Deserializer};
use std::collections::{BTreeMap, HashMap};
use time::OffsetDateTime;

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
struct Deposit {
    amount: i64,
    asset: Asset,
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
        #[derive(Deserialize)]
        struct Response {
            data: super::Contract,
        }

        for trade in &self.data {
            let id = trade.contract_id.clone();
            if map.get(&id).is_none() {
                let resp =
                    minreq::get(&format!("https://api.ledgerx.com/trading/contracts/{}", id))
                        .with_timeout(10)
                        .send()
                        .with_context(|| format!("requesting contract data for {}", id))?;
                let data: Response = serde_json::from_slice(&resp.into_bytes())?;
                map.insert(id, data.data);
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

    /// Import a list of deposits into the history
    pub fn import_deposits(&mut self, deposits: &Deposits) {
        for dep in &deposits.data {
            self.events.insert(
                dep.created_at,
                Event::Deposit {
                    amount: match dep.asset.name {
                        super::Asset::Btc => Decimal::new(dep.amount, 8),
                        super::Asset::Usd => Decimal::new(dep.amount, 2),
                        super::Asset::Eth => unimplemented!("ethereum deposits"),
                    },
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
            match event {
                Event::Deposit { asset, amount } => {
                    println!(
                        "Deposit,{},,{},,{},{}",
                        // It took a ton of experimenting to get a format that gnumeric
                        // will recognize and parse correctly..
                        date.to_offset(time::UtcOffset::UTC).format("%FT%T.%NZ"),
                        asset,
                        amount,
                        btc_price,
                    );
                }
                Event::Withdrawal { asset, amount } => {
                    println!(
                        "Withdraw,{},,{},,{},{}",
                        date.to_offset(time::UtcOffset::UTC).format("%FT%T.%NZ"),
                        asset,
                        amount,
                        btc_price,
                    );
                }
                Event::Trade {
                    contract,
                    price,
                    size,
                } => match contract.ty() {
                    super::contract::Type::Option { opt, .. } => {
                        println!(
                            "Trade,{},{},{},{},{},{},{},{},{}",
                            date.to_offset(time::UtcOffset::UTC).format("%FT%T.%NZ"),
                            opt.expiry.format("%F"),
                            match opt.pc {
                                crate::option::Call => "C",
                                crate::option::Put => "P",
                            },
                            opt.strike,
                            price,
                            size,
                            btc_price,
                            match opt.bs_iv(*date, btc_price, *price) {
                                Ok(iv) => iv.to_string(),
                                Err(_) => "free money".into(),
                            },
                            match opt.arr(*date, btc_price, *price) {
                                // don't encode ARRs greater than 10000%, it's silly and fucks up
                                // the display
                                x if x < 100.0 => x.to_string(),
                                _ => String::new(),
                            }
                        );
                    }
                    super::contract::Type::NextDay { .. } => {
                        println!(
                            "Trade,{},,{},,{},{},{}",
                            date.to_offset(time::UtcOffset::UTC).format("%FT%T.%NZ"),
                            contract.underlying(),
                            price,
                            size,
                            btc_price,
                        );
                    }
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
                        if *expired_size != 0 {
                            println!(
                                "Expiry,{},{},{},{},,{},",
                                date.to_offset(time::UtcOffset::UTC).format("%FT%T.%NZ"),
                                opt.expiry.format("%F"),
                                match opt.pc {
                                    crate::option::Call => "C",
                                    crate::option::Put => "P",
                                },
                                opt.strike,
                                expired_size,
                            );
                        }
                        if *assigned_size != 0 {
                            println!(
                                "Assignment,{},{},{},{},,{},",
                                date.to_offset(time::UtcOffset::UTC).format("%FT%T.%NZ"),
                                opt.expiry.format("%F"),
                                match opt.pc {
                                    crate::option::Call => "C",
                                    crate::option::Put => "P",
                                },
                                opt.strike,
                                assigned_size,
                            );
                        }
                    }
                    // NextDays don't expire, they are "assigned". We don't log this as a distinct
                    // event because we consider the originating trade to be the actual event.
                    super::contract::Type::NextDay { .. } => {
                        assert_eq!(*expired_size, 0);
                    }
                    // TBH I don't know what happens with futures
                    super::contract::Type::Future { .. } => unreachable!(),
                },
            }
        }
    }
}
