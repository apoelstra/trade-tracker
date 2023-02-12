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

use crate::units::{Asset, BudgetAsset, TaxAsset, Underlying};
use crate::{ledgerx::json, option};
use serde::Deserialize;
use std::{convert::TryFrom, fmt};
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

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Deserialize)]
pub struct ContractId(usize);

impl From<usize> for ContractId {
    fn from(u: usize) -> Self {
        ContractId(u)
    }
}

impl fmt::Display for ContractId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Structure representing a contract
#[derive(Clone, PartialEq, Eq, Hash, Debug, Deserialize)]
#[serde(try_from = "json::Contract")]
pub struct Contract {
    /// Contract ID
    id: ContractId,
    /// Whether the contract is active
    active: bool,
    /// Whether the contract is a put or a call
    ty: Type,
    /// Underlying physical asset
    underlying: Underlying,
    /// Human-readable label
    label: String,
    /// Multiplier (100 for BTC options, 10 for ETH options)
    multiplier: usize,
    /// Most recent "interesting contract" log date
    pub last_log: Option<OffsetDateTime>,
}

impl Contract {
    /// Accessor for contract ID
    pub fn id(&self) -> ContractId {
        self.id
    }
    /// Whether the contract is active
    pub fn active(&self) -> bool {
        self.active
    }
    /// Type of the contract
    pub fn ty(&self) -> Type {
        self.ty
    }

    /// If this contract represents an asset we track for tax purposes, return it
    pub fn asset(&self) -> Asset {
        match self.ty {
            Type::Option { opt, .. } => Asset::Option {
                underlying: self.underlying,
                option: opt,
            },
            Type::NextDay { .. } => match self.underlying {
                Underlying::Btc => Asset::Btc,
                Underlying::Eth => Asset::Eth,
            },
            Type::Future { .. } => unimplemented!("futures"),
        }
    }

    /// If this contract represents an asset we track for tax purposes, return it
    pub fn tax_asset(&self) -> Option<TaxAsset> {
        match self.ty {
            Type::Option { opt, .. } => Some(TaxAsset::Option {
                underlying: self.underlying,
                option: opt,
            }),
            Type::NextDay { .. } => match self.underlying {
                Underlying::Btc => Some(TaxAsset::Btc),
                Underlying::Eth => None,
            },
            _ => None,
        }
    }

    /// If this contract represents an asset we track for budgeting purposes, return it
    pub fn budget_asset(&self) -> Option<BudgetAsset> {
        match self.ty {
            Type::Option { opt, .. } => Some(BudgetAsset::Option {
                underlying: self.underlying,
                option: opt,
            }),
            Type::NextDay { .. } => match self.underlying {
                Underlying::Btc => Some(BudgetAsset::Btc),
                Underlying::Eth => None,
            },
            _ => None,
        }
    }

    /// Underlying asset type
    pub fn underlying(&self) -> Underlying {
        self.underlying
    }
    /// Contract label
    pub fn label(&self) -> &str {
        &self.label
    }
    /// Multiplier
    pub fn multiplier(&self) -> usize {
        self.multiplier
    }

    /// Expiry date
    pub fn expiry(&self) -> OffsetDateTime {
        match self.ty {
            Type::Option { opt, .. } => opt.expiry,
            Type::NextDay { expiry, .. } => expiry,
            Type::Future { expiry, .. } => expiry,
        }
    }
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
                        js.strike_price.ok_or("missing field 'strike_price'")?,
                        expiry,
                    ),
                    Some(json::Type::Put) => option::Option::new_put(
                        js.strike_price.ok_or("missing field 'strike_price'")?,
                        expiry,
                    ),
                    None => return Err("missing field 'type'"),
                },
            },
            json::DerivativeType::FutureContract => Type::Future { expiry },
            json::DerivativeType::DayAheadSwap => Type::NextDay { expiry },
        };
        Ok(Contract {
            id: ContractId(js.id),
            active: js.active,
            ty,
            underlying: js.underlying_asset,
            multiplier: js.multiplier,
            label: js.label,
            last_log: None,
        })
    }
}

impl Contract {
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

    #[test]
    fn parse_contract_put() {
        let contract_s = "{ \"id\": 22256321, \"name\": null, \"is_call\": false, \"strike_price\": 400000, \"min_increment\": 10, \"date_live\": \"2023-01-12 05:00:00+0000\", \"date_expires\": \"2023-12-29 21:00:00+0000\", \"date_exercise\": \"2023-12-29 22:00:00+0000\", \"derivative_type\": \"options_contract\", \"open_interest\": null, \"multiplier\": 10, \"label\": \"ETH-29DEC2023-4000-Put\", \"active\": true, \"is_next_day\": false, \"is_ecp_only\": false, \"underlying_asset\": \"ETH\", \"collateral_asset\": \"USD\", \"type\": \"put\" }";
        let contract: Contract = serde_json::from_str(contract_s).unwrap();
        assert_eq!(
            contract,
            Contract {
                id: ContractId(22256321),
                active: true,
                ty: Type::Option {
                    exercise_date: OffsetDateTime::parse("2023-12-29 22:00:00+0000", "%F %T%z")
                        .unwrap(),
                    opt: option::Option {
                        pc: option::PutCall::Put,
                        strike: crate::price!(4000),
                        expiry: OffsetDateTime::parse("2023-12-29 21:00:00+0000", "%F %T%z")
                            .unwrap(),
                    },
                },
                underlying: Underlying::Eth,
                multiplier: 10,
                label: "ETH-29DEC2023-4000-Put".into(),
                last_log: None,
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
                id: ContractId(22256298),
                active: true,
                ty: Type::Option {
                    exercise_date: OffsetDateTime::parse("2023-12-29 22:00:00+0000", "%F %T%z")
                        .unwrap(),
                    opt: option::Option {
                        pc: option::PutCall::Call,
                        strike: crate::price!(25000),
                        expiry: OffsetDateTime::parse("2023-12-29 21:00:00+0000", "%F %T%z")
                            .unwrap(),
                    },
                },
                underlying: Underlying::Btc,
                multiplier: 100,
                label: "BTC-Mini-29DEC2023-25000-Call".into(),
                last_log: None,
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
                id: ContractId(22256348),
                active: false,
                ty: Type::NextDay {
                    expiry: OffsetDateTime::parse("2023-02-14 21:00:00+0000", "%F %T%z").unwrap(),
                },
                underlying: Underlying::Btc,
                multiplier: 100,
                label: "BTC-Mini-14FEB2023-NextDay".into(),
                last_log: None,
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
                id: ContractId(22256410),
                active: true,
                ty: Type::Future {
                    expiry: OffsetDateTime::parse("2023-03-31 21:00:00+0000", "%F %T%z").unwrap(),
                },
                underlying: Underlying::Btc,
                multiplier: 100,
                label: "BTC-Mini-31MAR2023-Future".into(),
                last_log: None,
            },
        );
    }
}
