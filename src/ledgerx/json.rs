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

use super::Asset;
use rust_decimal::Decimal;
use serde::{de, Deserialize, Deserializer};
use serde_json::{Map, Value};
use time::OffsetDateTime;

pub fn parse_num(obj: &Map<String, Value>, field: &str) -> Result<u64, String> {
    Ok(obj
        .get(field)
        .ok_or(format!("missing field {} in contract", field))?
        .as_u64()
        .ok_or(format!("field {} in contract is not number", field))?)
}

pub fn parse_string(obj: &Map<String, Value>, field: &str) -> Result<String, String> {
    Ok(obj
        .get(field)
        .ok_or(format!("missing field {} in contract", field))?
        .as_str()
        .ok_or(format!("field {} in contract is not string", field))?
        .into())
}

pub fn parse_asset(obj: &Map<String, Value>, field: &str) -> Result<Asset, String> {
    match obj.get(field) {
        Some(Value::String(s)) => match &s[..] {
            "CBTC" => Ok(Asset::Btc),
            "ETH" => Ok(Asset::Eth),
            "USD" => Ok(Asset::Usd),
            x => Err(format!(
                "unknown asset type {} in contract field {}",
                x, field
            )),
        },
        Some(x) => Err(format!(
            "non-string asset {} in contract field {}",
            x, field
        )),
        None => Err(format!("missing field {} in contract", field)),
    }
}

pub fn parse_datetime(obj: &Map<String, Value>, field: &str) -> Result<OffsetDateTime, String> {
    match obj.get(field) {
        Some(Value::String(s)) => OffsetDateTime::parse(&s, "%F %T%z").map_err(|e| e.to_string()),
        Some(x) => Err(format!(
            "non-string datetime {} in contract field {}",
            x, field
        )),
        None => Err(format!("missing field {} in contract", field)),
    }
}

pub fn deserialize_datetime<'de, D>(deser: D) -> Result<Option<OffsetDateTime>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: Option<&str> = Deserialize::deserialize(deser)?;
    match s {
        Some(s) => Ok(Some(OffsetDateTime::parse(s, "%F %T%z").map_err(|_| {
            de::Error::invalid_value(de::Unexpected::Str(&s), &"a datetime in %F %T%z format")
        })?)),
        None => Ok(None),
    }
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
    pub strike_price: Option<Decimal>,
    pub min_increment: usize,
    #[serde(default)]
    pub open_interest: Option<usize>,
    pub multiplier: usize,
    pub label: String,
    #[serde(rename = "type")]
    pub ty: Option<Type>,
    pub name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_vector_contracts() {
        let vecs = vec![
            "{\"active\":true,\"collateral_asset\":\"USD\",\"date_exercise\":\"2023-12-29 22:00:00+0000\",\"date_expires\":\"2023-12-29 21:00:00+0000\",\"date_live\":\"2023-01-12 05:00:00+0000\",\"derivative_type\":\"options_contract\",\"id\":22256323,\"is_call\":false,\"is_ecp_only\":false,\"is_next_day\":false,\"label\":\"ETH-29DEC2023-5000-Put\",\"min_increment\":10,\"multiplier\":10,\"name\":null,\"open_interest\":null,\"strike_price\":500000,\"type\":\"put\",\"underlying_asset\":\"ETH\"}",
        ];

        for v in vecs {
            let _des: Contract = serde_json::from_str(v).expect("successful parse");
        }
    }
}
