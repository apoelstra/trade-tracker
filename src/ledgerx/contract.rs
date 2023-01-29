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

//! LedgerX
//!
//! Data Structures etc for the LedgerX API
//!

use crate::{ledgerx::json, option};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::convert::TryFrom;
use time::OffsetDateTime;

/// Type of contract
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Type {
    /// A put or call option
    Option {
        /// Time at which exercise choice must be made
        exercise_date: OffsetDateTime,
        /// The underlying option
        opt: option::Option,
    },
    /// A next-day swap
    NextDay {
        /// The next day
        expiry: OffsetDateTime,
    },
    /// A future
    Future {
        /// Date at which the future expires
        expiry: OffsetDateTime,
    },
}

/// Structure representing a contract
#[derive(Clone, PartialEq, Eq, Hash, Debug, Deserialize)]
#[serde(try_from = "json::Contract")]
pub struct Contract {
    /// Contract ID
    pub id: usize,
    /// Whether the contract is a put or a call
    pub ty: Type,
    /// Underlying physical asset
    pub underlying: super::Asset,
    /// Human-readable label
    pub label: String,
    /// Multiplier (100 for BTC options, 10 for ETH options)
    pub multiplier: usize,
}

impl TryFrom<json::Contract> for Contract {
    type Error = &'static str;
    fn try_from(js: json::Contract) -> Result<Contract, &'static str> {
        let expiry = js.date_expires.ok_or("missing field 'date_expires'")?;
        let ty = match js.derivative_type {
            json::DerivativeType::OptionsContract => Type::Option {
                exercise_date: js.date_exercise.ok_or("missing field 'date_exercise'")?,
                opt: match js.ty {
                    Some(json::Type::Call) => option::Option::new_call(
                        js.strike_price.ok_or("missing field 'strike_price'")? / Decimal::from(100),
                        expiry,
                    ),
                    Some(json::Type::Put) => option::Option::new_put(
                        js.strike_price.ok_or("missing field 'strike_price'")? / Decimal::from(100),
                        expiry,
                    ),
                    None => return Err("missing field 'type'"),
                },
            },
            json::DerivativeType::FutureContract => Type::Future { expiry },
            json::DerivativeType::DayAheadSwap => Type::NextDay { expiry },
        };
        Ok(Contract {
            id: js.id,
            ty,
            underlying: js.underlying_asset,
            multiplier: js.multiplier,
            label: js.label,
        })
    }
}

impl Contract {
    /// Parse a contract from the JSON output from the LX /contracts API
    pub fn from_json(json: &serde_json::Value) -> Result<Self, String> {
        let json = match json {
            serde_json::Value::Object(json) => json,
            _ => return Err(format!("contract json was not an object: {json}")),
        };

        let expiry = json::parse_datetime(&json, "date_expires")?;
        let ty = match (
            json.get("derivative_type").and_then(|js| js.as_str()),
            json.get("is_call"),
        ) {
            (Some("options_contract"), Some(serde_json::Value::Bool(true))) => Type::Option {
                exercise_date: json::parse_datetime(&json, "date_exercise")?,
                opt: option::Option::new_call(
                    Decimal::from(json::parse_num(&json, "strike_price")?) / Decimal::from(100),
                    expiry,
                ),
            },
            (Some("options_contract"), Some(serde_json::Value::Bool(false))) => Type::Option {
                exercise_date: json::parse_datetime(&json, "date_exercise")?,
                opt: option::Option::new_put(
                    Decimal::from(json::parse_num(&json, "strike_price")?) / Decimal::from(100),
                    expiry,
                ),
            },
            (Some("day_ahead_swap"), Some(serde_json::Value::Null)) => Type::NextDay { expiry },
            (Some("future_contract"), Some(serde_json::Value::Null)) => Type::Future { expiry },
            _ => {
                return Err(format!(
                    "Could not make sense of contract derivative_type/is_call fields {:?} {:?}",
                    json.get("derivative_type"),
                    json.get("is_call"),
                ))
            }
        };

        Ok(Contract {
            id: json::parse_num(&json, "id")? as usize,
            ty,
            underlying: json::parse_asset(&json, "underlying_asset")?,
            multiplier: json::parse_num(&json, "multiplier")? as usize,
            label: json::parse_string(&json, "label")?,
        })
    }

    /// For a put or a call, return the option
    pub fn as_option(&self) -> Option<option::Option> {
        match self.ty {
            Type::Option { opt, .. } => Some(opt),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn parse_contract_put() {
        let contract_s = "{ \"id\": 22256321, \"name\": null, \"is_call\": false, \"strike_price\": 400000, \"min_increment\": 10, \"date_live\": \"2023-01-12 05:00:00+0000\", \"date_expires\": \"2023-12-29 21:00:00+0000\", \"date_exercise\": \"2023-12-29 22:00:00+0000\", \"derivative_type\": \"options_contract\", \"open_interest\": null, \"multiplier\": 10, \"label\": \"ETH-29DEC2023-4000-Put\", \"active\": true, \"is_next_day\": false, \"is_ecp_only\": false, \"underlying_asset\": \"ETH\", \"collateral_asset\": \"USD\", \"type\": \"put\" }";
        let contract: Contract = serde_json::from_str(contract_s).unwrap();
        assert_eq!(
            contract,
            Contract {
                id: 22256321,
                ty: Type::Option {
                    exercise_date: OffsetDateTime::parse("2023-12-29 22:00:00+0000", "%F %T%z")
                        .unwrap(),
                    opt: option::Option {
                        pc: option::PutCall::Put,
                        strike: Decimal::from_str("4000.00").unwrap(),
                        expiry: OffsetDateTime::parse("2023-12-29 21:00:00+0000", "%F %T%z")
                            .unwrap(),
                    },
                },
                underlying: crate::ledgerx::Asset::Eth,
                multiplier: 10,
                label: "ETH-29DEC2023-4000-Put".into(),
            },
        );
    }

    #[test]
    fn parse_contract_call() {
        let contract_s = "{ \"id\": 22256298, \"name\": null, \"is_call\": true, \"strike_price\": 2500000, \"min_increment\": 100, \"date_live\": \"2023-01-12 05:00:00+0000\", \"date_expires\": \"2023-12-29 21:00:00+0000\", \"date_exercise\": \"2023-12-29 22:00:00+0000\", \"derivative_type\": \"options_contract\", \"open_interest\": 674, \"multiplier\": 100, \"label\": \"BTC-Mini-29DEC2023-25000-Call\", \"active\": true, \"is_next_day\": false, \"is_ecp_only\": false, \"underlying_asset\": \"CBTC\", \"collateral_asset\": \"CBTC\", \"type\": \"call\" }";
        let contract: Contract = serde_json::from_str(contract_s).unwrap();
        assert_eq!(
            contract,
            Contract {
                id: 22256298,
                ty: Type::Option {
                    exercise_date: OffsetDateTime::parse("2023-12-29 22:00:00+0000", "%F %T%z")
                        .unwrap(),
                    opt: option::Option {
                        pc: option::PutCall::Call,
                        strike: Decimal::from_str("25000.00").unwrap(),
                        expiry: OffsetDateTime::parse("2023-12-29 21:00:00+0000", "%F %T%z")
                            .unwrap(),
                    },
                },
                underlying: crate::ledgerx::Asset::Btc,
                multiplier: 100,
                label: "BTC-Mini-29DEC2023-25000-Call".into(),
            },
        );
    }

    #[test]
    fn parse_contract_nextday() {
        let contract_s = "{ \"id\": 22256348, \"name\": null, \"is_call\": null, \"strike_price\": null, \"min_increment\": 100, \"date_live\": \"2023-02-13 21:00:00+0000\", \"date_expires\": \"2023-02-14 21:00:00+0000\", \"date_exercise\": \"2023-02-14 21:00:00+0000\", \"derivative_type\": \"day_ahead_swap\", \"open_interest\": null, \"multiplier\": 100, \"label\": \"BTC-Mini-14FEB2023-NextDay\", \"active\": false, \"is_next_day\": true, \"is_ecp_only\": false, \"underlying_asset\": \"CBTC\", \"collateral_asset\": \"CBTC\" }";

        let contract: Contract = serde_json::from_str(contract_s).unwrap();
        assert_eq!(
            contract,
            Contract {
                id: 22256348,
                ty: Type::NextDay {
                    expiry: OffsetDateTime::parse("2023-02-14 21:00:00+0000", "%F %T%z").unwrap(),
                },
                underlying: crate::ledgerx::Asset::Btc,
                multiplier: 100,
                label: "BTC-Mini-14FEB2023-NextDay".into(),
            },
        );
    }

    #[test]
    fn parse_future() {
        let contract_s = "{\"active\":true,\"collateral_asset\":\"CBTC\",\"date_exercise\":null,\"date_expires\":\"2023-03-31 21:00:00+0000\",\"date_live\":\"2023-01-27 05:00:00+0000\",\"derivative_type\":\"future_contract\",\"id\":22256410,\"is_call\":null,\"is_ecp_only\":false,\"is_next_day\":false,\"label\":\"BTC-Mini-31MAR2023-Future\",\"min_increment\":100,\"multiplier\":100,\"name\":null,\"open_interest\":null,\"strike_price\":null,\"underlying_asset\":\"CBTC\"}";

        let contract: Contract = serde_json::from_str(contract_s).unwrap();
        assert_eq!(
            contract,
            Contract {
                id: 22256410,
                ty: Type::Future {
                    expiry: OffsetDateTime::parse("2023-03-31 21:00:00+0000", "%F %T%z").unwrap(),
                },
                underlying: crate::ledgerx::Asset::Btc,
                multiplier: 100,
                label: "BTC-Mini-31MAR2023-Future".into(),
            },
        );
    }
}