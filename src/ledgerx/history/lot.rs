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
use crate::ledgerx::history::tax::{GainType, TaxDate};
use crate::option::{Call, Put};
use crate::units::{Price, Quantity, TaxAsset, TaxAsset2022};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    fmt, mem, str,
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
            self.id, self.open_ty, self.quantity, self.asset, self.price, self.date,
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
            sort_date: date.bare_time(),
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
            date: date.into(),
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
        synthetic: Option<crate::option::PutCall>,
    ) -> anyhow::Result<(Close, Option<Self>)> {
        if self.quantity.has_same_sign(quantity) {
            return Err(anyhow::Error::msg(format!(
                "Tried to close {self} with quantity {quantity} of same sign"
            )));
        }

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
                synthetic,
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

/// CSV printer for a lot
///
/// Outputs data consistent with the "full" CSV output for closes.
pub struct LotCsv<'lot> {
    lot: &'lot Lot,
}

impl<'lot> csv::PrintCsv for LotCsv<'lot> {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let csv = (
            self.lot.open_ty,
            self.lot.date,
            self.lot.quantity,
            self.lot.asset,
            self.lot.price,
            &self.lot.id,
            "", // old lot size
            "", // old lot basis
            self.lot.quantity,
            self.lot.price * self.lot.quantity,
            "", // basis
            "", // proceeds
            "", // gain/loss
            "", // gain/loss type
        );
        csv.print(f)
    }
}

/// The nature of a taxable "open position" event
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum OpenType {
    BuyToOpen,
    SellToOpen,
    Deposit,
    /// Only used for expiries and assignments which are events
    /// that can't produce new lots, but our functions require
    /// an `OpenType` nonetheless.
    Unknown,
}
impl fmt::Display for OpenType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        csv::PrintCsv::print(self, f)
    }
}
impl csv::PrintCsv for OpenType {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            OpenType::BuyToOpen => f.write_str("Buy To Open"),
            OpenType::SellToOpen => f.write_str("Sell To Open"),
            OpenType::Deposit => f.write_str("Deposit"),
            OpenType::Unknown => f.write_str("UNKNOWN THIS IS A BUG"),
        }
    }
}

/// The nature of a taxable "close position" event
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum CloseType {
    BuyBack,
    Sell,
    Expiry,
    Exercise,
    TxFee,
}
impl fmt::Display for CloseType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        csv::PrintCsv::print(self, f)
    }
}
impl csv::PrintCsv for CloseType {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CloseType::BuyBack => f.write_str("Buy Back"),
            CloseType::Sell => f.write_str("Sell"),
            CloseType::Expiry => f.write_str("Expired"),
            CloseType::Exercise => f.write_str("Exercised"),
            CloseType::TxFee => f.write_str("Transaction Fee"),
        }
    }
}

/// Data structure representing the closing of a lot
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Close {
    ty: CloseType,
    synthetic: Option<crate::option::PutCall>,
    open_id: Id,
    open_original_quantity: Quantity,
    open_price: Price,
    open_date: TaxDate,
    close_price: Price,
    close_date: TaxDate,
    asset: TaxAsset,
    quantity: Quantity,
}

impl fmt::Display for Close {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {{ {:?}, date: {}, asset: {}, price: {}, qty: {} }}",
            self.open_id, self.ty, self.close_date, self.asset, self.close_price, self.quantity,
        )
    }
}

impl Close {
    /// Type of the close (sell, buy, expiry, assignment)
    pub fn ty(&self) -> CloseType {
        self.ty
    }

    /// The size of the lot prior to this close
    pub fn old_lot_size(&self) -> Quantity {
        self.open_original_quantity
    }

    /// The basis of the lot at its size prior to this close
    pub fn old_lot_basis(&self) -> Price {
        self.open_price * self.open_original_quantity
    }

    /// The size of the lot *after* this close
    pub fn new_lot_size(&self) -> Quantity {
        self.open_original_quantity + self.quantity
    }

    /// The basis of the lot at its size *after* this close
    pub fn new_lot_basis(&self) -> Price {
        self.old_lot_basis() + self.basis()
    }

    /// The basis of the closed quantity of the original lot
    ///
    /// In other words, the difference between [Self::new_lot_basis] and
    /// [Self::old_lot_basis].
    pub fn basis(&self) -> Price {
        self.open_price * -self.quantity
    }

    /// The amount the closed quantity actually closed for
    pub fn proceeds(&self) -> Price {
        self.close_price * -self.quantity
    }

    /// The gain/loss caused by this closure
    pub fn gain_loss(&self) -> Price {
        self.proceeds() - self.basis()
    }

    /// The gain/loss caused by this closure
    pub fn gain_loss_type(&self) -> GainType {
        if self.asset.is_1256() {
            GainType::Option1256
        } else if self.close_date - self.open_date <= time::Duration::days(365) {
            GainType::ShortTerm
        } else {
            GainType::LongTerm
        }
    }

    /// The date the closed lot was created
    pub fn open_date(&self) -> TaxDate {
        self.open_date
    }

    /// The date the lot was (partially) closed
    pub fn close_date(&self) -> TaxDate {
        self.close_date
    }

    /// The asset of the closed lot
    pub fn asset(&self) -> TaxAsset {
        self.asset
    }

    /// Constructs a CSV outputter for this close
    pub fn csv_printer(
        &self,
        asset: TaxAsset,
        user_id: usize,
        mode: PrintMode,
    ) -> csv::CsvPrinter<CloseCsv> {
        csv::CsvPrinter(CloseCsv {
            user_id,
            asset,
            close: self,
            mode,
        })
    }
}

/// Output style
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum PrintMode {
    /// Try to exactly match the LX output, so you can use 'diff' to confirm
    /// that we're interpreting the same data in the same way
    LedgerX,
    /// LedgerX format, but annotated with lot IDs.
    ///
    /// The idea here is that we can output LX's view with this format, easily
    /// check that it matches their CSV output (by importing into excel, deleting
    /// a column, and diffing), and then see wtf they're thinking.
    ///
    /// Then we can output our own view in this format, and by diffing we can see
    /// that the only changes were due to changes in choice of BTC lots.
    LedgerXAnnotated,
    /// A sane format that we could provide as evidence for our history.
    ///
    /// Hard to diff this against any of the above formats but at least it will
    /// end up at the same total number. Will also show where the lots come from,
    /// data which is conspicuously missing from the other formats.
    Full,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct CloseCsv<'close> {
    user_id: usize,
    asset: TaxAsset,
    close: &'close Close,
    mode: PrintMode,
}

impl<'close> csv::PrintCsv for CloseCsv<'close> {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.mode {
            PrintMode::LedgerX | PrintMode::LedgerXAnnotated => {
                let mut proceeds = self.close.proceeds();
                let mut basis = self.close.basis();

                let mut close_date = self.close.close_date;
                let mut open_date = self.close.open_date;

                if self.close.quantity.is_negative() {
                    // wtf
                    mem::swap(&mut close_date, &mut open_date);
                    mem::swap(&mut basis, &mut proceeds);
                }
                // also wtf
                proceeds = proceeds.abs();
                basis = basis.abs();

                if self.close.close_date.year() == 2021 {
                    let description = match self.close.quantity {
                        Quantity::Bitcoin(btc) => {
                            let real_amount = Decimal::new(btc.to_sat(), 8);
                            let round_amount = real_amount.round_dp(2);
                            // If we can, reduce to 2 decimal points. This will be the common case since LX
                            // will only let us trade in 1/100th of a bitcoin, and will let us better match
                            // their output.
                            if real_amount == round_amount {
                                format!("{}, {}", round_amount.abs(), self.asset)
                            } else {
                                format!("{}, {}", real_amount.abs(), self.asset)
                            }
                        }
                        Quantity::Contracts(n) => format!("{}, {}", n.abs(), self.asset),
                        Quantity::Cents(_) => {
                            panic!("tried to write out a sale of dollars as a tax event")
                        }
                        Quantity::Zero => "0".into(), // maybe we should just panic here
                    };

                    (
                        self.close.ty,
                        description,
                        close_date,
                        open_date,
                        basis,
                        proceeds,
                        basis - proceeds,
                        self.close.gain_loss_type(),
                        "",
                        "",
                        "",
                    )
                        .print(f)?;
                } else {
                    // Tax years not 2021
                    let ref_1 = if self.close.asset == TaxAsset::Bitcoin {
                        "Exercise"
                    } else {
                        match self.close.ty {
                            CloseType::BuyBack => "Buy to Close",
                            CloseType::Sell => "Sell to Close",
                            CloseType::Expiry => "Expire",
                            CloseType::Exercise => "Exercise",
                            CloseType::TxFee => "TX Fee",
                        }
                    };
                    let ref_2 = match self.close.synthetic {
                        Some(Call) => "1256 Option - Call",
                        Some(Put) => "1256 Option - Put",
                        None => match self.close.asset {
                            TaxAsset::Bitcoin => "Non-1256 - Future",
                            TaxAsset::NextDay { .. } => "Non-1256 - Future",
                            TaxAsset::Option { option, .. } => {
                                if self.close.ty == CloseType::Expiry
                                    || self.close.ty == CloseType::Exercise
                                {
                                    match option.pc {
                                        Call => "1256 Option - Call",
                                        Put => "1256 Option - Put",
                                    }
                                } else {
                                    "1256 Option"
                                }
                            }
                        },
                    };
                    let reference = format!("{ref_1} - {ref_2}");

                    let quantity = match self.close.quantity {
                        Quantity::Bitcoin(btc) => {
                            let real_amount = Decimal::new(btc.to_sat(), 8).abs();
                            let round_amount = real_amount.round_dp(2);
                            if real_amount == round_amount {
                                round_amount
                            } else {
                                real_amount
                            }
                        }
                        Quantity::Contracts(n) => Decimal::new(n.abs() * 100, 2),
                        Quantity::Cents(_) => {
                            panic!("tried to write out a sale of dollars as a tax event")
                        }
                        Quantity::Zero => Decimal::new(0, 2),
                    };

                    let close_date = format!(
                        "{}.{:03}Z",
                        close_date.bare_time().format("%FT%H:%M:%S"),
                        close_date.bare_time().millisecond(),
                    );
                    let open_date = format!(
                        "{}.{:03}Z",
                        open_date.bare_time().format("%FT%H:%M:%S"),
                        open_date.bare_time().millisecond(),
                    );
                    (
                        self.user_id,
                        reference,
                        quantity,
                        TaxAsset2022(self.asset),
                        close_date,
                        open_date,
                        // for prices, we use the alt format except we strip off the $
                        format!("{:#}", basis),
                        format!("{:#}", proceeds),
                        format!("{:#}", basis - proceeds),
                        match self.close.gain_loss_type() {
                            GainType::LongTerm => "Long-Term",
                            GainType::ShortTerm => "Short-Term",
                            GainType::Option1256 => "- 1256 - ", // notice trailing space
                        },
                    )
                        .print(f)?
                }

                if self.mode == PrintMode::LedgerXAnnotated {
                    f.write_str(",")?;
                    self.close.open_id.print(f)?;
                }
            }
            PrintMode::Full => {
                let csv = (
                    self.close.ty,
                    self.close.close_date,
                    self.close.quantity,
                    self.asset,
                    self.close.close_price,
                    &self.close.open_id,
                    self.close.old_lot_size(),
                    self.close.old_lot_basis(),
                    self.close.new_lot_size(),
                    self.close.new_lot_basis(),
                    self.close.basis(),
                    self.close.proceeds(),
                    self.close.gain_loss(),
                    self.close.gain_loss_type(),
                );
                csv.print(f)?;
            }
        }
        Ok(())
    }
}
