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
use crate::ledgerx::history::tax::{Close, CloseType, GainType, OpenType, TaxDate};
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

/// Tax Lot
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Lot {
    id: Id,
    asset: TaxAsset,
    quantity: Quantity,
    price: Price,
    date: TaxDate,
    open_ty: OpenType,
    sort_date: time::OffsetDateTime,
}

impl fmt::Display for Lot {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} ({}): {} {} at {}; date {}",
            self.id, self.open_ty, self.quantity, self.asset, self.price, self.date.0,
        )
    }
}

impl Lot {
    /// Constructs a lot from a given asset/quantity/price/date data
    ///
    /// Will assign the lot a fresh ID. Don't use this for deposits!
    /// Instead use [Lot::from_deposit] which will assign an ID based
    /// on the outpoint of the deposit.
    pub fn new(
        asset: TaxAsset,
        quantity: Quantity,
        price: Price,
        date: TaxDate,
        open_ty: OpenType,
    ) -> Lot {
        Lot {
            id: match asset {
                TaxAsset::Bitcoin => Id::next_btc(),
                TaxAsset::NextDay { .. } => unreachable!(
                    "dayaheads should be converted to their underlying, and are not tracked as lots by themselves",
                ),
                TaxAsset::Option { .. } => Id::next_opt(),
            },
            asset,
            quantity,
            price,
            date,
            open_ty,
            sort_date: date.0,
        }
    }

    /// Directly constructs a lot from a deposit
    pub fn from_deposit(
        outpoint: bitcoin::OutPoint,
        price: Price,
        quantity: bitcoin::Amount,
        date: time::OffsetDateTime,
    ) -> Lot {
        Lot {
            id: Id::from_outpoint(outpoint),
            asset: TaxAsset::Bitcoin,
            quantity: quantity.into(),
            price,
            date: TaxDate(date),
            open_ty: OpenType::Deposit,
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

    /// Consume the lot by closing it. If this is a partial close, return
    /// the reduced-size log.
    pub fn close(
        mut self,
        quantity: Quantity,
        price: Price,
        date: TaxDate,
        ty: CloseType,
    ) -> anyhow::Result<(Close, Option<Self>)> {
        if self.quantity.has_same_sign(quantity) {
            return Err(anyhow::Error::msg(format!(
                "Tried to close {self} with quantity {quantity} of same sign"
            )));
        }

        let gain_ty = if self.asset.is_1256() {
            GainType::Option1256
        } else if date.0 - self.date.0 <= time::Duration::days(365) {
            GainType::ShortTerm
        } else {
            GainType::LongTerm
        };

        let open_original_quantity = self.quantity; // record for tax records

        let partial;
        let close_quantity;
        if self.quantity.abs() > quantity.abs() {
            // Partial close
            self.quantity += quantity;
            close_quantity = quantity;
            partial = true;
        } else {
            // Full close
            close_quantity = -self.quantity;
            partial = false;
        }

        Ok((
            Close {
                ty,
                gain_ty,
                open_id: self.id.clone(),
                open_original_quantity,
                open_price: self.price,
                open_date: self.date,
                close_price: price,
                close_date: date,
                asset: self.asset,
                quantity: close_quantity,
            },
            if partial { Some(self) } else { None },
        ))
    }

    pub fn csv_printer(&self) -> csv::CsvPrinter<LotCsv> {
        csv::CsvPrinter(LotCsv { lot: self })
    }
}

pub struct LotCsv<'lot> {
    lot: &'lot Lot,
}

impl<'lot> csv::PrintCsv for LotCsv<'lot> {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let csv = (
            self.lot.open_ty,
            self.lot.quantity,
            self.lot.asset,
            self.lot.price,
            &self.lot.id,
            "", // old lot size
            "", // old lot basis
            self.lot.quantity,
            self.lot.price * self.lot.quantity,
            "", // gain/loss
            "", // gain/loss type
        );
        csv.print(f)
    }
}
