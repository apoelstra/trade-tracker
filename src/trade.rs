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

//! Trades
//!
//! Data structures which represent trades
//!

use crate::price::BitcoinPrice;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Assets that I care to trade
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Asset {
    /// BTC
    Bitcoin,
    /// BTC Call option
    BitcoinCall {
        /// Strike price
        strike: Decimal,
        /// Expiry time
        #[serde(with = "time::serde::timestamp")]
        expiry: time::OffsetDateTime,
    },
    /// BTC Put option
    BitcoinPut {
        /// Strike price
        strike: Decimal,
        /// Expiry time
        #[serde(with = "time::serde::timestamp")]
        expiry: time::OffsetDateTime,
    },
    /// Canadian Dollars
    Cad,
    /// US Dollars
    Usd,
    /// Combination of multiple assets
    Synthetic { underlying: Vec<Asset> },
}

/// Data about an individual trade
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Deserialize, Serialize)]
struct Trade {
    /// Asset that was transferred away from me
    sale_asset: Asset,
    /// Asset that was transferred to me
    buy_asset: Asset,
    /// Price, in units of `sale_asset` per units of `buy_asset`, to 12 decimals
    price: Decimal,
    /// Price, in units of `buy_asset` per units of `sale_asset`, to 12 decimals
    inv_price: Decimal,
    /// Date that the trade took place
    #[serde(with = "time::serde::timestamp")]
    date: time::OffsetDateTime,
    /// Bitcoin price at the time of sale (this will not be very high resolution
    /// and is essentially only used for historic record-keeping)
    btc_price: BitcoinPrice,
}
