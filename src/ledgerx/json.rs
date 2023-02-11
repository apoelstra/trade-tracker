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

//! Json stuff
//!
//! Some utility methods for parsing json from the LX API
//!

use rust_decimal::Decimal;
use serde::{de, Deserialize, Deserializer};
use serde_json::{Map, Value};
use std::convert::TryFrom;
use time::OffsetDateTime;

pub fn parse_num(obj: &Map<String, Value>, field: &str) -> Result<u64, String> {
    obj.get(field)
        .ok_or(format!("missing field {field} in contract"))?
        .as_u64()
        .ok_or(format!("field {field} in contract is not number"))
}

pub fn deserialize_datetime<'de, D>(deser: D) -> Result<Option<OffsetDateTime>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: Option<&str> = Deserialize::deserialize(deser)?;
    match s {
        Some(s) => Ok(Some(OffsetDateTime::parse(s, "%F %T%z").map_err(|_| {
            de::Error::invalid_value(de::Unexpected::Str(s), &"a datetime in %F %T%z format")
        })?)),
        None => Ok(None),
    }
}

pub fn deserialize_timestamp<'de, D>(deser: D) -> Result<OffsetDateTime, D::Error>
where
    D: Deserializer<'de>,
{
    let s: i64 = Deserialize::deserialize(deser)?;
    Ok(OffsetDateTime::from_unix_timestamp_nanos(s.into()))
    /*
        .map_err(|_| {
        de::Error::invalid_value(de::Unexpected::Signed(s as i64), &"nanoseconds since the epoch")
    })
        */
}

/// The type of the derivative
#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum DerivativeType {
    DayAheadSwap,
    FutureContract,
    OptionsContract,
}

/// Value of the "type" field
#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Type {
    Put,
    Call,
}

/// From <https://docs.ledgerx.com/reference/action-report-status-codes>
#[derive(Deserialize, Debug)]
#[serde(try_from = "usize")]
pub enum StatusType {
    Inserted,
    CrossTrade,
    NotFilled,
    Cancelled,
    CancelledAndReplaced,
    MessageAcknowledged,
    ContractNotFound,
    OrderIdNotFound,
    OrderIdInvalid,
    OrderRejected,
    InsufficientCollateral,
    ContractExpired,
    PriceThresholdExceeded,
    ContractNotActive,
    InvalidBlockSize,
}

impl TryFrom<usize> for StatusType {
    type Error = String;
    fn try_from(x: usize) -> Result<Self, Self::Error> {
        match x {
            200 => Ok(StatusType::Inserted),
            201 => Ok(StatusType::CrossTrade),
            202 => Ok(StatusType::NotFilled),
            203 => Ok(StatusType::Cancelled),
            204 => Ok(StatusType::CancelledAndReplaced),
            300 => Ok(StatusType::MessageAcknowledged),
            600 => Ok(StatusType::ContractNotFound),
            601 => Ok(StatusType::OrderIdNotFound),
            602 => Ok(StatusType::OrderIdInvalid),
            607 => Ok(StatusType::OrderRejected),
            609 => Ok(StatusType::InsufficientCollateral),
            610 => Ok(StatusType::ContractExpired),
            613 => Ok(StatusType::PriceThresholdExceeded),
            614 => Ok(StatusType::ContractNotActive),
            616 => Ok(StatusType::InvalidBlockSize),
            _ => Err(format!("unknown status type {x}")),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(try_from = "usize")]
pub enum StatusReason {
    NoReason,
    FullFill,
    CancelledByExchange,
}

impl TryFrom<usize> for StatusReason {
    type Error = String;
    fn try_from(x: usize) -> Result<Self, Self::Error> {
        match x {
            0 => Ok(StatusReason::NoReason),
            52 => Ok(StatusReason::FullFill),
            53 => Ok(StatusReason::CancelledByExchange),
            _ => Err(format!("unknown status reason {x}")),
        }
    }
}

/// Copy of the "contract" as returned from the /contracts endpoint
#[derive(Deserialize, Debug)]
pub struct Contract {
    pub id: usize,
    pub active: bool,
    pub collateral_asset: super::Asset,
    pub underlying_asset: super::Asset,
    #[serde(default, deserialize_with = "deserialize_datetime")]
    pub date_exercise: Option<OffsetDateTime>,
    #[serde(default, deserialize_with = "deserialize_datetime")]
    pub date_expires: Option<OffsetDateTime>,
    #[serde(default, deserialize_with = "deserialize_datetime")]
    pub date_live: Option<OffsetDateTime>,
    pub is_call: Option<bool>,
    pub is_next_day: Option<bool>,
    pub is_ecp_only: Option<bool>,
    pub derivative_type: DerivativeType,
    pub strike_price: Option<i64>,
    pub min_increment: usize,
    #[serde(default)]
    pub open_interest: Option<usize>,
    pub multiplier: usize,
    pub label: String,
    #[serde(rename = "type")]
    pub ty: Option<Type>,
    pub name: Option<String>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct DataFeedMeta {
    index: usize,
    max_index: usize,
    #[serde(deserialize_with = "hex::serde::deserialize")]
    manifest_id: [u8; 16],
}

#[derive(Deserialize, Debug)]
pub struct Balances {
    #[serde(rename = "USD")]
    pub usd: Decimal,
    #[serde(rename = "CBTC")]
    pub btc: Decimal,
}

#[derive(Deserialize, Debug)]
pub struct AllBalances {
    pub available_balances: Balances,
    pub deliverable_locked_balances: Balances,
    pub position_locked_balances: Balances,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum DataFeedObject {
    ActionReport {
        contract_id: usize,
        open_interest: usize,
        #[serde(deserialize_with = "hex::serde::deserialize")]
        mid: [u8; 16],
        /// Will always be `customer_limit_order`
        order_type: String,
        price: Decimal,
        size: u64,
        inserted_price: Decimal,
        inserted_size: u64,
        filled_price: Decimal,
        filled_size: u64,
        original_price: Decimal,
        original_size: u64,
        /// volume-weighted average price the order has been filled at
        vwap: Decimal,
        is_ask: bool,
        /// Whether the order auto-cancels at 4PM
        is_volatile: bool,
        #[serde(default)]
        cid: Option<usize>,
        #[serde(default)]
        mpid: Option<usize>,
        status_type: StatusType,
        #[serde(default)]
        status_reason: Option<StatusReason>,
        /// "The current clock for the entire contract"
        clock: u64,
        #[serde(deserialize_with = "deserialize_timestamp")]
        timestamp: OffsetDateTime,
        #[serde(deserialize_with = "deserialize_timestamp")]
        inserted_time: OffsetDateTime,
        #[serde(deserialize_with = "deserialize_timestamp")]
        updated_time: OffsetDateTime,
        #[serde(default)]
        _meta: Option<DataFeedMeta>,
    },
    UnauthSuccess {},
    AuthSuccess {},
    ContractAdded {
        data: crate::ledgerx::Contract,
    },
    ContractRemoved {
        data: crate::ledgerx::Contract,
    },
    TradeBusted {},
    Meta {},
    OpenPositionsUpdate {},
    CollateralBalanceUpdate {
        collateral: AllBalances,
    },
    ExposureReports {},
    ContactAdded {},
    ContactRemoved {},
    ContactConnected {},
    ContactDisconnected {},
    StateManifest {},
    BookTop {
        contract_id: usize,
        ask: Decimal,
        ask_size: u64,
        bid: Decimal,
        bid_size: u64,
        /// "The current clock for the entire contract"
        clock: u64,
    },
    /// Lol AFAICT this one is just undocumented
    Heartbeat {},
}

#[derive(Deserialize, Debug)]
pub struct BookStateMessage {
    pub data: BookStateData,
}
#[derive(Deserialize, Debug)]
pub struct BookStateData {
    pub contract_id: usize,
    pub book_states: Vec<BookState>,
}
#[derive(Deserialize, Debug)]
pub struct BookState {
    pub clock: u64,
    pub contract_id: usize,
    #[serde(deserialize_with = "hex::serde::deserialize")]
    pub mid: [u8; 16],
    pub is_ask: bool,
    pub price: i64,
    pub size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io, io::BufRead};

    #[test]
    fn fixed_vector_contracts() {
        let vecs = vec![
            "{\"active\":true,\"collateral_asset\":\"USD\",\"date_exercise\":\"2023-12-29 22:00:00+0000\",\"date_expires\":\"2023-12-29 21:00:00+0000\",\"date_live\":\"2023-01-12 05:00:00+0000\",\"derivative_type\":\"options_contract\",\"id\":22256323,\"is_call\":false,\"is_ecp_only\":false,\"is_next_day\":false,\"label\":\"ETH-29DEC2023-5000-Put\",\"min_increment\":10,\"multiplier\":10,\"name\":null,\"open_interest\":null,\"strike_price\":500000,\"type\":\"put\",\"underlying_asset\":\"ETH\"}",
        ];

        for v in vecs {
            let _des: Contract = serde_json::from_str(v).expect("successful parse");
        }
    }

    #[test]
    fn fixed_vector_datafeed() {
        let fh = fs::File::open("src/ledgerx/test-datafeed.json").unwrap();
        let fh = io::BufReader::new(fh);
        for json in fh.lines() {
            let json = json.unwrap();
            serde_json::from_str::<DataFeedObject>(&json).expect("successful parse");
        }
    }
}
