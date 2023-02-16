// Trade Tracker
// Written in 2023 by
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

//! Lot Management
//!
//! Data structures related to trading history for copying into Excel
//!

use crate::csv;
use crate::ledgerx::history::tax::TaxDate;
use crate::units::{Price, Quantity, TaxAsset};
use serde::{Deserialize, Serialize};
use std::{
    fmt, str,
    sync::atomic::{AtomicUsize, Ordering},
};

/// Used to give every lot a unique ID
static LOT_INDEX: AtomicUsize = AtomicUsize::new(1);

/// Newtype for unique lot IDs
#[derive(Clone, PartialEq, Eq, Debug, Hash, Deserialize, Serialize)]
pub struct Id(String);
impl csv::PrintCsv for Id {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.print(f)
    }
}

impl str::FromStr for Id {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Id(s.into()))
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl Id {
    /// Constructor for the next LX-generated BTC lot ID
    fn next_btc() -> Id {
        let idx = LOT_INDEX.fetch_add(1, Ordering::SeqCst);
        Id(format!("lx-btc-{idx:04}"))
    }

    /// Constructor for the next LX-generated BTC option ID
    fn next_opt() -> Id {
        let idx = LOT_INDEX.fetch_add(1, Ordering::SeqCst);
        Id(format!("lx-opt-{idx:04}"))
    }

    /// Constructor for a lot ID that comes from a UTXO
    ///
    /// This is the only constructor accessible from outside of
    /// this module, since it's the only stateless one, and we
    /// want to keep careful track of our state to ensure that
    /// our records have consistent lot IDs from year to year.
    pub fn from_outpoint(outpoint: bitcoin::OutPoint) -> Id {
        Id(format!("{:.8}-{:02}", outpoint.txid, outpoint.vout))
    }
}

/// Marker for "no lot ID"
#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub struct UnknownOptId;
impl fmt::Display for UnknownOptId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("<lx-option>")
    }
}

/// Marker for "no lot ID"
#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub struct UnknownBtcId;
impl fmt::Display for UnknownBtcId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("<lx-btc>")
    }
}

impl From<UnknownOptId> for Id {
    fn from(_: UnknownOptId) -> Self {
        Id::next_opt()
    }
}

impl From<UnknownBtcId> for Id {
    fn from(_: UnknownBtcId) -> Self {
        Id::next_btc()
    }
}

/// Tax Lot
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Lot {
    id: Id,
    asset: TaxAsset,
    quantity: Quantity,
    price: Price,
    date: TaxDate,
    sort_date: time::OffsetDateTime,
}

impl Lot {
    /// Directly constructs a lot from a deposit
    pub fn from_deposit(
        outpoint: bitcoin::OutPoint,
        price: Price,
        quantity: bitcoin::Amount,
        date: time::OffsetDateTime,
    ) -> Lot {
        Lot {
            id: Id::from_outpoint(outpoint),
            asset: TaxAsset::Btc,
            quantity: quantity.into(),
            price,
            date: TaxDate(date),
            sort_date: date + time::Duration::days(365 * 100),
        }
    }

    /// Accessor for the ID
    pub fn id(&self) -> &Id {
        &self.id
    }

    /// Accessor for the date
    pub fn asset(&self) -> TaxAsset {
        self.asset
    }

    /// Accessor for the date
    pub fn date(&self) -> TaxDate {
        self.date
    }

    /// Accessor for the sort date of the lot
    ///
    /// This is normally the same as the actual date, but for deposits,
    /// we bump it 100 years in the future so that the lot won't be used
    /// by the LX-style "FIFO" until after all other lots are used.
    ///
    /// Note that this returns a bare date, not a [TaxDate], which hopefully
    /// should avoid accidentally using this for anything "real"
    pub fn sort_date(&self) -> time::OffsetDateTime {
        self.sort_date
    }

    /// Accessor for the basis unit price
    ///
    /// This is NOT the basis; to get the basis multiply this value by
    /// the quantity.
    pub fn price(&self) -> Price {
        self.price
    }

    /// Accessor for the quantity
    pub fn quantity(&self) -> Quantity {
        self.quantity
    }
}
