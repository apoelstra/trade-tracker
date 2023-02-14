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

//! Assets
//!
//! The different asset types that are supported by this library.
//!

use serde::{Deserialize, Deserializer};
use std::fmt;

/// The primary "asset" type which covers every kind of asset supported by
/// the software.
///
/// It does not directly support deserialization or serialization, because
/// there are many different ways this data is serialized throughout LX.
/// You must instead convert it to a more specific type, such as
/// [DepositAsset] or [TaxAsset].
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub enum Asset {
    /// Bitcoin, or a day-ahead swap (these are treated identically) for
    /// unit-tracking purposes.
    Btc,
    /// Ethereum
    Eth,
    /// US Dollars
    Usd,
    /// A put or call option
    Option {
        underlying: Underlying,
        option: crate::option::Option,
    },
    /// A future
    Future {
        underlying: Underlying,
        expiry: time::OffsetDateTime,
    },
}

/// A kind of asset that can be deposited or withdrawn
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash, Deserialize)]
pub enum DepositAsset {
    /// Bitcoin
    #[serde(rename = "CBTC")]
    Btc,
    /// Ethereum
    #[serde(rename = "ETH")]
    Eth,
    /// US Dollars
    #[serde(rename = "USD")]
    Usd,
}

impl From<DepositAsset> for Asset {
    fn from(dep: DepositAsset) -> Asset {
        match dep {
            DepositAsset::Btc => Asset::Btc,
            DepositAsset::Eth => Asset::Btc,
            DepositAsset::Usd => Asset::Usd,
        }
    }
}

/// A kind of asset which is reflected in the end-of-year tax CSVs
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub enum TaxAsset {
    /// Bitcoin
    Btc,
    /// A put or call option
    Option {
        underlying: Underlying,
        option: crate::option::Option,
    },
}

impl From<TaxAsset> for Asset {
    fn from(dep: TaxAsset) -> Asset {
        match dep {
            TaxAsset::Btc => Asset::Btc,
            TaxAsset::Option { underlying, option } => Asset::Option { underlying, option },
        }
    }
}

impl fmt::Display for TaxAsset {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            TaxAsset::Btc => f.write_str("BTC"),
            TaxAsset::Option { underlying, option } => {
                write!(
                    f,
                    "{} Mini {} {} {:#}",
                    underlying,
                    option.expiry.lazy_format("%F"),
                    option.pc.as_str(),
                    option.strike,
                )
            }
        }
    }
}

/// A kind of asset which is reflected in my budget spreadsheet
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub enum BudgetAsset {
    /// Bitcoin
    Btc,
    /// Ethereum (lol)
    Eth,
    /// US Dollars
    Usd,
    /// A put or call option
    Option {
        underlying: Underlying,
        option: crate::option::Option,
    },
}

impl From<DepositAsset> for BudgetAsset {
    fn from(dep: DepositAsset) -> BudgetAsset {
        match dep {
            DepositAsset::Btc => BudgetAsset::Btc,
            DepositAsset::Usd => BudgetAsset::Usd,
            DepositAsset::Eth => BudgetAsset::Eth,
        }
    }
}

impl From<BudgetAsset> for Asset {
    fn from(dep: BudgetAsset) -> Asset {
        match dep {
            BudgetAsset::Btc => Asset::Btc,
            BudgetAsset::Eth => Asset::Eth,
            BudgetAsset::Usd => Asset::Usd,
            BudgetAsset::Option { underlying, option } => Asset::Option { underlying, option },
        }
    }
}

/// A kind of asset which may be the "underlying" for a put or call option
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash, Deserialize)]
pub enum Underlying {
    /// Bitcoin
    #[serde(rename = "CBTC")]
    Btc,
    /// Ethereum
    #[serde(rename = "ETH")]
    Eth,
}

impl From<Underlying> for Asset {
    fn from(dep: Underlying) -> Asset {
        match dep {
            Underlying::Btc => Asset::Btc,
            Underlying::Eth => Asset::Eth,
        }
    }
}

impl fmt::Display for Underlying {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Underlying::Btc => f.write_str("BTC"),
            Underlying::Eth => f.write_str("ETH"),
        }
    }
}

/// Deserialize a deposit address which is contained within a "name" field for some reason
pub fn deserialize_name_deposit_asset<'de, D>(deser: D) -> Result<DepositAsset, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct WrappedAsset {
        name: DepositAsset,
    }
    let wrapped: WrappedAsset = Deserialize::deserialize(deser)?;
    Ok(wrapped.name)
}
