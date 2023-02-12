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

//! LedgerX *Tax* History
//!
//! Data structures related to history of tax events, with the specific goal of
//! reproducing LX's weird CSV fies.
//!

use crate::{
    csv,
    units::{Price, Quantity},
};
use log::debug;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    cmp,
    collections::HashMap,
    fmt, mem, str,
    sync::atomic::{AtomicUsize, Ordering},
};

/// Used to give every lot a unique ID
static LOT_INDEX: AtomicUsize = AtomicUsize::new(1);

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

/// Newtype for unique lot IDs
#[derive(Clone, PartialEq, Eq, Debug, Hash, Deserialize, Serialize)]
pub struct LotId(String);
impl csv::PrintCsv for LotId {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.print(f)
    }
}

impl str::FromStr for LotId {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(LotId(s.into()))
    }
}

impl fmt::Display for LotId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl LotId {
    /// Constructor for the next LX-generated BTC lot ID
    fn next_btc() -> LotId {
        let idx = LOT_INDEX.fetch_add(1, Ordering::SeqCst);
        LotId(format!("lx-btc-{idx:04}"))
    }

    /// Constructor for the next LX-generated BTC option ID
    fn next_opt() -> LotId {
        let idx = LOT_INDEX.fetch_add(1, Ordering::SeqCst);
        LotId(format!("lx-opt-{idx:04}"))
    }

    /// Constructor for a lot ID that comes from a UTXO
    fn from_outpoint(outpoint: bitcoin::OutPoint) -> LotId {
        LotId(format!("{:.8}-{:02}", outpoint.txid, outpoint.vout))
    }
}

/// Wrapper around a date that will output time to the nearest second in 3339 format
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct TaxDate(pub time::OffsetDateTime);
impl csv::PrintCsv for TaxDate {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut date_utc = self.0.to_offset(time::UtcOffset::UTC);
        // Our date library seems to always round seconds down, while LX does
        // nearest-int rounding.
        if date_utc.microsecond() > 500_000 {
            date_utc += time::Duration::seconds(1);
        }
        write!(f, "{}Z", date_utc.lazy_format("%FT%H:%M:%S"),)
    }
}

/// A contract label, as formatted in LX's end-of-year CSV files
#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub struct Label(String);
impl csv::PrintCsv for Label {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.print(f)
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl Label {
    /// Creates the tax label for Bitcoin
    pub fn btc() -> Label {
        Label("BTC".into())
    }

    /// Whether or not this label represents Bitcoin
    pub fn is_btc(&self) -> bool {
        self.0 == "BTC"
    }

    /// Creates a tax label for a contract
    pub fn from_contract(contract: &crate::ledgerx::Contract) -> Label {
        let tax_asset = contract
            .tax_asset()
            .expect("asset is not something we know how to label for tax purposes");
        Label(tax_asset.to_string())
    }
}

/// Whether cap gains are short or long term, or 1256 (60% long / 40% short)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum GainType {
    ShortTerm,
    LongTerm,
    Option1256,
}
impl csv::PrintCsv for GainType {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            GainType::ShortTerm => f.write_str("Short-term"),
            GainType::LongTerm => f.write_str("Long-term"),
            GainType::Option1256 => f.write_str("-1256-"),
        }
    }
}

/// The nature of a taxable "close position" event
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum CloseType {
    BuyBack,
    Sell,
    Expiry,
    Exercise,
    TxFee,
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

/// Event that creates or enlarges a position
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Lot<ID> {
    id: ID,
    /// If this is used to close a position rather than create a new lot
    close_ty: CloseType,
    quantity: Quantity,
    price: Price,
    date: TaxDate,
}

impl From<UnknownOptId> for LotId {
    fn from(_: UnknownOptId) -> Self {
        LotId::next_opt()
    }
}

impl From<UnknownBtcId> for LotId {
    fn from(_: UnknownBtcId) -> Self {
        LotId::next_btc()
    }
}

impl<ID: fmt::Display> fmt::Display for Lot<ID> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {{ {} @ {} on {} }}",
            self.id,
            self.quantity,
            self.price,
            self.date.0.lazy_format("%FT%T"),
        )
    }
}

impl<ID> Lot<ID> {
    /// Accessor for the ID
    pub fn id(&self) -> &ID {
        &self.id
    }
}

impl Lot<LotId> {
    /// Constructs an `Lot` object from a deposit
    pub fn from_deposit_utxo(
        outpoint: bitcoin::OutPoint,
        price: Price,
        size: bitcoin::Amount,
        date: time::OffsetDateTime,
    ) -> Self {
        debug!(
            "Lot::from_deposit_utxo price {} size {} date {}",
            price, size, date
        );
        Lot {
            id: LotId::from_outpoint(outpoint),
            close_ty: CloseType::TxFee, // lol a deposit better not close a position..
            quantity: size.into(),
            price,
            date: TaxDate(date),
        }
    }

    /// Reduces a lot by a transaction fee amount
    pub fn dock_fee(&mut self, n: bitcoin::Amount) {
        if let Quantity::Bitcoin(ref mut btc) = self.quantity {
            *btc -= n.to_signed().expect("bitcoin amount overflow");
        } else {
            panic!("{}: tried to dock fees but this is a non-bitcoin lot", self)
        }
    }
}

impl Lot<UnknownBtcId> {
    /// Constructs an `Lot` object from a transaction fee
    pub fn from_tx_fee(size: bitcoin::Amount, date: time::OffsetDateTime) -> Self {
        Lot {
            id: UnknownBtcId,
            close_ty: CloseType::TxFee, // lol a fee better *had* close a position..
            quantity: -Quantity::from(size),
            price: Price::ZERO,
            date: TaxDate(date),
        }
    }

    /// Constructs an `Lot` object from a trade event
    pub fn from_trade_btc(
        price: Price,
        size: Quantity,
        fee: Price,
        date: time::OffsetDateTime,
    ) -> Self {
        debug!(
            "Lot::from_trade_btc price {} size {} fee {} date {}",
            price, size, fee, date
        );
        let unit_fee = fee / size;
        let adj_price = price + unit_fee; // nb `unit_fee` is a signed quantity
        Lot {
            id: UnknownBtcId,
            close_ty: if size.is_nonnegative() {
                CloseType::BuyBack
            } else {
                CloseType::Sell
            },
            quantity: size,
            price: adj_price,
            date: TaxDate(date),
        }
    }
}

impl Lot<UnknownOptId> {
    /// Constructs an `Lot` object from a trade event
    pub fn from_trade_opt(
        price: Price,
        size: Quantity,
        fee: Price,
        date: time::OffsetDateTime,
    ) -> Self {
        debug!(
            "Lot::from_trade_opt price {} size {} fee {} date {}",
            price, size, fee, date
        );
        let unit_fee = fee / size;
        let adj_price = price + unit_fee; // nb `unit_fee` is a signed quantity
        Lot {
            id: UnknownOptId,
            close_ty: if size.is_nonnegative() {
                CloseType::BuyBack
            } else {
                CloseType::Sell
            },
            quantity: size,
            price: adj_price,
            date: TaxDate(date),
        }
    }
}

impl Lot<UnknownOptId> {
    /// Constructs an `Lot` object from an expiration event
    pub fn from_expiry(opt: &crate::option::Option, n_expired: i64) -> Self {
        debug!("Lot::from_expiry opt {} n {}", opt, n_expired);
        // seriously WTF -- the time is always fixed at 22:00 even though this
        // is 5PM in winter and 6PM in summer, neither of which are 4PM when
        // these options actually expire.
        let expiry = opt.expiry.date().with_time(time::time!(22:00)).assume_utc();
        Lot {
            id: UnknownOptId,
            close_ty: CloseType::Expiry,
            quantity: Quantity::from_contracts(n_expired),
            price: Price::ZERO,
            date: TaxDate(expiry),
        }
    }

    pub fn from_assignment(opt: &crate::option::Option, n_assigned: i64, btc_price: Price) -> Self {
        debug!("Lot::from_assignment opt {} n {}", opt, n_assigned);
        // seriously WTF -- the time is always fixed at 22:00 even though this
        // is 5PM in winter and 6PM in summer, neither of which are 4PM when
        // these options actually expire.
        let expiry = opt.expiry.date().with_time(time::time!(22:00)).assume_utc();
        Lot {
            id: UnknownOptId,
            close_ty: CloseType::Exercise,
            quantity: Quantity::from_contracts(n_assigned),
            price: opt.intrinsic_value(btc_price),
            date: TaxDate(expiry),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Close {
    ty: CloseType,
    gain_ty: GainType,
    open_id: LotId,
    open_price: Price,
    open_date: TaxDate,
    close_price: Price,
    close_date: TaxDate,
    quantity: Quantity,
}

impl fmt::Display for Close {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {{ {:?}, date: {}, price: {}, qty: {} }}",
            self.open_id.0,
            self.ty,
            self.close_date.0.lazy_format("%FT%FT"),
            self.close_price,
            self.quantity,
        )
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct CloseCsv<'label, 'close> {
    label: &'label Label,
    close: &'close Close,
    print_lot_id: bool,
}

impl<'label, 'close> csv::PrintCsv for CloseCsv<'label, 'close> {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut proceeds = self.close.close_price * self.close.quantity;
        let mut basis = self.close.open_price * self.close.quantity;

        let mut close_date = self.close.close_date;
        let mut open_date = self.close.open_date;
        if !self.close.quantity.is_positive() {
            // wtf
            mem::swap(&mut close_date, &mut open_date);
            mem::swap(&mut basis, &mut proceeds);
        }
        proceeds = proceeds.abs();
        basis = basis.abs();

        let description = match self.close.quantity {
            Quantity::Bitcoin(btc) => {
                let real_amount = Decimal::new(btc.to_sat(), 8);
                let round_amount = real_amount.round_dp(2);
                // If we can, reduce to 2 decimal points. This will be the common case since LX
                // will only let us trade in 1/100th of a bitcoin, and will let us better match
                // their output.
                if real_amount == round_amount {
                    format!("{}, {}", round_amount.abs(), self.label)
                } else {
                    format!("{}, {}", real_amount.abs(), self.label)
                }
            }
            Quantity::Contracts(n) => format!("{}, {}", n.abs(), self.label),
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
            self.close.gain_ty,
            "",
            "",
            "",
        )
            .print(f)?;

        if self.print_lot_id {
            f.write_str(",")?;
            self.close.open_id.print(f)?;
        }
        Ok(())
    }
}

impl Close {
    /// Constructs a CSV outputter for this close
    pub fn csv_printer<'label, 'close>(
        &'close self,
        label: &'label Label,
        print_lot_id: bool,
    ) -> csv::CsvPrinter<CloseCsv<'label, 'close>> {
        csv::CsvPrinter(CloseCsv {
            label,
            close: self,
            print_lot_id,
        })
    }
}

/// A position in a specific asset, represented by a FIFO queue of opening events
#[derive(Clone, Default, Debug)]
pub struct Position {
    fifo: crate::TimeMap<Lot<LotId>>,
}

impl Position {
    /// Creates a new empty position
    pub fn new() -> Self {
        Position {
            fifo: Default::default(),
        }
    }

    fn total_size(&self) -> Quantity {
        self.fifo.values().map(|open| open.quantity).sum()
    }

    /// Assigns an ID to a lot and adds it to the position tracker
    fn insert_lot<ID: Into<LotId>>(
        &mut self,
        sort_date: time::OffsetDateTime,
        lot: Lot<ID>,
    ) -> Lot<LotId> {
        let new_lot = Lot {
            id: lot.id.into(),
            close_ty: lot.close_ty,
            quantity: lot.quantity,
            price: lot.price,
            date: lot.date,
        };
        self.fifo.insert(sort_date, new_lot.clone());
        new_lot
    }

    /// Modifies the position based on an event.
    ///
    /// If this results in closing previously opened positions, return a list of
    /// `Close`s with their type set to the `close_type` argument. If it results
    /// in opening a position (which may happen in a "pure" open or may happen
    /// only after closing), then it also returns a created lot.
    fn push_event<ID: fmt::Display + Into<LotId>>(
        &mut self,
        mut open: Lot<ID>,
        sort_date: time::OffsetDateTime,
        is_1256: bool,
    ) -> (Vec<Close>, Option<Lot<LotId>>) {
        // If the position is empty, then just push
        if self.fifo.is_empty() {
            debug!(
                "Create new position with open {}; sort date {}",
                open, sort_date
            );
            let lot = self.insert_lot(sort_date, open);
            return (vec![], Some(lot));
        }
        // If there is an open position, in the same direction as the new open,
        // add the new open to the FIFO.
        if self
            .fifo
            .values()
            .next()
            .unwrap()
            .quantity
            .has_same_sign(open.quantity)
        {
            debug!(
                "Increasing position (total qty {}) with open {}; sort date {}",
                self.total_size(),
                open,
                sort_date,
            );
            let lot = self.insert_lot(sort_date, open);
            return (vec![], Some(lot));
        }
        // Otherwise, we must close the position
        let mut ret = vec![];
        while open.quantity.is_nonzero() {
            let pre_pop_total = self.total_size();
            let to_match = if is_1256 {
                self.fifo.pop_first()
            } else {
                self.fifo.pop_max(|lot| lot.price)
            };
            let (front_date, mut front) = match to_match {
                Some(kv) => kv,
                None => {
                    debug!(
                        "fully closed out position, opening new one: {}; sort date {}",
                        open, sort_date
                    );
                    // If we've closed out everything and still have an open, then
                    // we're opening a new position in the opposite direction.
                    let lot = self.insert_lot(sort_date, open);
                    return (ret, Some(lot));
                }
            };
            debug!(
                "closing lot {} (total qty {}) with potential-lot {}",
                front, pre_pop_total, open
            );

            // Construct a close object with everything known but the quantity
            let mut close = Close {
                ty: open.close_ty,
                gain_ty: if is_1256 {
                    GainType::Option1256
                } else if open.date.0 - front.date.0 <= time::Duration::days(365) {
                    GainType::ShortTerm
                } else {
                    GainType::LongTerm
                },
                open_id: front.id.clone(),
                open_price: front.price,
                open_date: front.date,
                close_price: open.price,
                close_date: open.date,
                quantity: Quantity::Zero, // TBD below
            };

            // ...figure out the quantity
            assert!(
                !front.quantity.has_same_sign(open.quantity),
                "cancelling lot {} should have opposite sign of opened lot {}",
                open,
                front,
            );
            if front.quantity.abs() > open.quantity.abs() {
                // Partial close
                front.quantity += open.quantity;
                close.quantity = open.quantity;
                open.quantity = Quantity::Zero;
                self.fifo.insert(front_date, front); // put incompletely closed position back
            } else {
                // Full close
                close.quantity = -front.quantity;
                open.quantity += front.quantity;
            }
            debug!("push_event: pushing close {} onto ret", close);
            ret.push(close);
        }

        // If we made it here we consumed the whole initial lot and only closed things.
        (ret, None)
    }
}

/// "anonymous" enum covering an open or a close
#[derive(Clone, Debug)]
pub enum OpenClose {
    Open(Lot<LotId>),
    Close(Close),
}

/// Loggable "tax event"
#[derive(Clone, Debug)]
pub struct Event {
    pub date: TaxDate,
    pub label: Label,
    pub open_close: OpenClose,
}

/// Tracks positions in multiple assets, recording tax events
#[derive(Clone, Debug, Default)]
pub struct PositionTracker {
    positions: HashMap<Label, Position>,
    events: Vec<Event>,
}

impl PositionTracker {
    /// Constructs a new empty position tracker
    pub fn new() -> Self {
        Default::default()
    }

    /// Attempts to add a new lot to the tracker
    ///
    /// The lot may add to a position, in which case it is an "open". Or it may shrink one
    /// or more existing lots, in which case it is a "close".
    ///
    /// Returns the number of lots closed.
    pub fn push_lot<ID: fmt::Display + Into<LotId>>(
        &mut self,
        label: &Label,
        lot: Lot<ID>,
        sort_date: time::OffsetDateTime,
    ) -> usize {
        let date = lot.date;
        // Take the action...
        let pos = self.positions.entry(label.clone()).or_default();
        let (closes, open) = pos.push_event(lot, sort_date, !label.is_btc());
        let n_ret = closes.len();
        // ...then log it
        for close in closes {
            debug!("push_lot: logging close {} at date {}", close, date.0);
            self.events.push(Event {
                date,
                label: label.clone(),
                open_close: OpenClose::Close(close),
            });
        }
        if let Some(open) = open {
            debug!("push_lot: logging open {} at date {}", open, date.0);
            self.events.push(Event {
                date,
                label: label.clone(),
                open_close: OpenClose::Open(open),
            });
        }
        // Return the number of closes that happened
        n_ret
    }

    /// Sort the tax events to match LX's sort order
    ///
    /// We sort entries by their occurence -- which matches LX, but when things expire,
    /// many expiries may happen simultaneously. In this case we sort by the date of
    /// the expired position being *opened*
    pub fn lx_sort_events(&mut self) {
        // This is a stable sort, so to avoid reordering things we just return "equal"
        self.events.sort_by(|x, y| {
            use OpenClose::Close as C;
            if x.date != y.date {
                return cmp::Ordering::Equal;
            }
            if let (C(cx), C(cy)) = (&x.open_close, &y.open_close) {
                cx.open_date.0.cmp(&cy.open_date.0)
            } else {
                cmp::Ordering::Equal
            }
        });
    }

    /// Returns a list of all the tax events that have been recorded
    pub fn events(&self) -> &[Event] {
        &self.events
    }
}
