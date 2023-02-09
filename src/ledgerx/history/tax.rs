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

use crate::csv;
use log::debug;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    cmp,
    collections::BTreeMap,
    collections::HashMap,
    convert::TryFrom,
    fmt, mem, str,
    sync::atomic::{AtomicI64, AtomicUsize, Ordering},
};

/// Used to give every lot a unique ID
static LOT_INDEX: AtomicUsize = AtomicUsize::new(0);

/// Used to skew dates so that they all get inserted into BTreeSets separately
static DATE_OFFSET: AtomicI64 = AtomicI64::new(0);

/// Newtype for unique lot IDs
#[derive(Clone, PartialEq, Eq, Debug, Hash, Deserialize, Serialize)]
pub struct LotId(String);
impl csv::PrintCsv for LotId {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.0 == "" {
            unreachable!("tried to print invalid lot ID");
        }
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
        LotId(format!("lx-btc-{:03}", idx))
    }

    /// Constructor for the next LX-generated BTC option ID
    fn next_opt() -> LotId {
        let idx = LOT_INDEX.fetch_add(1, Ordering::SeqCst);
        LotId(format!("lx-opt-{:03}", idx))
    }

    /// Constructor for a lot ID that comes from a UTXO
    fn from_outpoint(outpoint: bitcoin::OutPoint) -> LotId {
        LotId(format!("{:.6}-{:03}", outpoint.txid, outpoint.vout))
    }

    /// Constructor for an "invalid" lot ID that should never actually be a lot
    fn invalid() -> LotId {
        LotId("".into())
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
        use crate::ledgerx::contract::Type;

        let s = match contract.ty() {
            Type::Option { opt, .. } => {
                let strike_int = opt.strike.to_i64().unwrap();
                assert_eq!(Decimal::from(strike_int), opt.strike); // check no cents
                assert!(strike_int >= 1000);
                assert!(strike_int < 1000000); // so I can be lazy about comma placement
                format!(
                    "{} Mini {} {} ${},{:03}.00",
                    contract.underlying(),
                    opt.expiry.lazy_format("%F"),
                    match opt.pc {
                        crate::option::Call => "Call",
                        crate::option::Put => "Put",
                    },
                    strike_int / 1000,
                    strike_int % 1000,
                )
            }
            Type::NextDay { .. } => "BTC".into(),
            Type::Future { .. } => unimplemented!("future tax label"),
        };
        Label(s)
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

/// If a position is open, what direction it is going in
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Direction {
    Short,
    Long,
}
impl Direction {
    /// Turns a positive number into "long" and a negative into "short"
    fn from_size(s: i64) -> Self {
        if s < 0 {
            Direction::Short
        } else {
            Direction::Long
        }
    }
}

/// Event that creates or enlarges a position
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Lot {
    id: LotId,
    /// If this is used to close a position rather than create a new lot
    close_ty: CloseType,
    direction: Direction,
    quantity: u64,
    price: Decimal,
    date: TaxDate,
}

impl fmt::Display for Lot {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {{ {:?}, date: {}, price: {}, qty: {} }}",
            self.id.0,
            self.direction,
            self.date.0.lazy_format("%FT%T"),
            self.price,
            self.quantity,
        )
    }
}

impl Lot {
    /// Accessor for the ID
    pub fn id(&self) -> &LotId {
        &self.id
    }

    /// Constructs an `Lot` object from a deposit
    pub fn from_deposit_utxo(
        outpoint: bitcoin::OutPoint,
        price: Decimal,
        size_sat: u64,
        date: time::OffsetDateTime,
    ) -> Lot {
        debug!(
            "Lot::from_deposit_utxo price {} size {}sat date {}",
            price, size_sat, date
        );
        Lot {
            id: LotId::from_outpoint(outpoint),
            close_ty: CloseType::TxFee, // lol a deposit better not close a position..
            direction: Direction::Long,
            quantity: size_sat,
            price: price,
            date: TaxDate(date),
        }
    }

    /// Constructs an `Lot` object from a transaction fee
    pub fn from_tx_fee(size_sat: u64, date: time::OffsetDateTime) -> Lot {
        Lot {
            id: LotId::invalid(),
            close_ty: CloseType::TxFee, // lol a fee better *had* close a position..
            direction: Direction::Short,
            quantity: size_sat,
            price: Decimal::ZERO,
            date: TaxDate(date),
        }
    }

    /// Constructs an `Lot` object from a trade event
    pub fn from_trade(
        price: Decimal,
        size: i64,
        fee: Decimal,
        date: time::OffsetDateTime,
        is_btc: bool,
    ) -> Lot {
        debug!(
            "Lot::from_trade price {} size {} fee {} date {}",
            price, size, fee, date
        );
        let unit_fee = if is_btc {
            fee / Decimal::new(size, 8)
        } else {
            fee / Decimal::new(size, 2)
        };
        let adj_price = price + unit_fee; // nb `unit_fee` is a signed quantity
        Lot {
            id: if is_btc {
                LotId::next_btc()
            } else {
                LotId::next_opt()
            },
            close_ty: if size > 0 {
                CloseType::BuyBack
            } else {
                CloseType::Sell
            },
            direction: Direction::from_size(size),
            quantity: size.unsigned_abs(),
            price: adj_price,
            date: TaxDate(date),
        }
    }

    /// Constructs an `Lot` object from an expiration event
    pub fn from_expiry(opt: &crate::option::Option, n_expired: i64) -> Lot {
        debug!("Lot::from_expiry opt {} n {}", opt, n_expired);
        // seriously WTF -- the time is always fixed at 22:00 even though this
        // is 5PM in winter and 6PM in summer, neither of which are 4PM when
        // these options actually expire.
        let expiry = opt.expiry.date().with_time(time::time!(22:00)).assume_utc();
        Lot {
            id: LotId::invalid(),
            close_ty: CloseType::Expiry,
            direction: Direction::from_size(n_expired),
            quantity: n_expired.unsigned_abs(),
            price: Decimal::ZERO,
            date: TaxDate(expiry),
        }
    }

    pub fn from_assignment(
        opt: &crate::option::Option,
        n_assigned: i64,
        btc_price: Decimal,
    ) -> Lot {
        debug!("Lot::from_assignment opt {} n {}", opt, n_assigned);
        // seriously WTF -- the time is always fixed at 22:00 even though this
        // is 5PM in winter and 6PM in summer, neither of which are 4PM when
        // these options actually expire.
        let expiry = opt.expiry.date().with_time(time::time!(22:00)).assume_utc();
        Lot {
            id: LotId::invalid(),
            close_ty: CloseType::Exercise,
            direction: Direction::from_size(n_assigned),
            quantity: n_assigned.unsigned_abs(),
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
    open_direction: Direction,
    open_price: Decimal,
    open_date: TaxDate,
    close_price: Decimal,
    close_date: TaxDate,
    quantity: u64,
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
        let i_qty = i64::try_from(self.close.quantity).unwrap();
        let real_amount = if self.label.is_btc() {
            Decimal::new(i_qty, 8)
        } else {
            Decimal::new(i_qty, 2)
        };
        let mut proceeds = real_amount * self.close.close_price;
        let mut basis = real_amount * self.close.open_price;
        proceeds.rescale(2);
        basis.rescale(2);

        let mut close_date = self.close.close_date;
        let mut open_date = self.close.open_date;
        if self.close.open_direction == Direction::Long {
            // wtf
            mem::swap(&mut close_date, &mut open_date);
            mem::swap(&mut basis, &mut proceeds);
        }

        let i_qty = i64::try_from(self.close.quantity).unwrap();
        let description = if self.label.0 == "BTC" {
            // If we can, reduce to 2 decimal points. This will be the common case since LX
            // will only let us trade in 1/100th of a bitcoin, and will let us better match
            // their output.
            if real_amount == real_amount.round_dp(2) {
                let mut trunc = real_amount;
                trunc.rescale(2);
                format!("{}, {}", trunc, self.label)
            } else {
                format!("{}, {}", real_amount, self.label)
            }
        } else {
            format!("{}, {}", i_qty, self.label)
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
    fifo: BTreeMap<time::OffsetDateTime, Lot>,
}

impl Position {
    /// Creates a new empty position
    pub fn new() -> Self {
        Position {
            fifo: Default::default(),
        }
    }

    fn total_size(&self) -> u64 {
        self.fifo.values().map(|open| open.quantity).sum()
    }

    fn direction(&self) -> Option<Direction> {
        self.fifo.values().next().map(|open| open.direction)
    }

    /// Modifies the position based on an event.
    ///
    /// If this results in closing previously opened positions, return a list of
    /// `Close`s with their type set to the `close_type` argument. If it results
    /// in opening a position (which may happen in a "pure" open or may happen
    /// only after closing), then it also returns a created lot.
    fn push_event(
        &mut self,
        mut open: Lot,
        sort_date: time::OffsetDateTime,
        is_1256: bool,
    ) -> (Vec<Close>, Option<Lot>) {
        // If the position is empty, then just push
        if self.fifo.is_empty() {
            debug!(
                "Create new position with open {}; sort date {}",
                open, sort_date
            );
            self.fifo.insert(sort_date, open.clone());
            return (vec![], Some(open));
        }
        // If there is an open position, in the same direction as the new open,
        // add the new open to the FIFO.
        if self.fifo.values().next().unwrap().direction == open.direction {
            debug!(
                "Increasing position ({:?}, qty {}) with open {}; sort date {}",
                self.direction(),
                self.total_size(),
                open,
                sort_date,
            );
            self.fifo.insert(sort_date, open.clone());
            return (vec![], Some(open));
        }
        // Otherwise, we must close the position
        let mut ret = vec![];
        while open.quantity > 0 {
            let (front_date, mut front) = match self.fifo.pop_first() {
                Some(kv) => kv,
                None => {
                    debug!(
                        "fully closed out position, opening new one: {}; sort date {}",
                        open, sort_date
                    );
                    // If we've closed out everything and still have an open, then
                    // we're opening a new position in the opposite direction.
                    self.fifo.insert(sort_date, open.clone());
                    return (ret, Some(open));
                }
            };
            assert_ne!(open.direction, front.direction);
            debug!("closing lot {} with potential-lot {}", front, open);

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
                open_direction: front.direction,
                open_price: front.price,
                open_date: front.date,
                close_price: open.price,
                close_date: open.date,
                quantity: 0, // TBD
            };

            // ...figure out the quantity
            if front.quantity > open.quantity {
                // Partial close
                front.quantity -= open.quantity;
                close.quantity = open.quantity;
                open.quantity = 0;
                self.fifo.insert(front_date, front); // put incompletely closed position back
            } else {
                // Full close
                close.quantity = front.quantity;
                open.quantity -= front.quantity;
            }
            ret.push(close);
        }

        // If we made it here we consumed the whole initial lot and only closed things.
        return (ret, None);
    }
}

/// "anonymous" enum covering an open or a close
#[derive(Clone, Debug)]
pub enum OpenClose {
    Open(Lot),
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
    pub fn push_lot(&mut self, label: &Label, lot: Lot, sort_date: time::OffsetDateTime) -> usize {
        let date_offset = DATE_OFFSET.fetch_add(1, Ordering::SeqCst);
        let sort_date = sort_date + time::Duration::nanoseconds(date_offset);
        // Take the action...
        let pos = self.positions.entry(label.clone()).or_default();
        let (closes, open) = pos.push_event(lot.clone(), sort_date, !label.is_btc());
        let n_ret = closes.len();
        // ...then log it
        let date = lot.date;
        for close in closes {
            self.events.push(Event {
                date: date,
                label: label.clone(),
                open_close: OpenClose::Close(close),
            });
        }
        if let Some(open) = open {
            self.events.push(Event {
                date: date,
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
            if let (&C(ref cx), &C(ref cy)) = (&x.open_close, &y.open_close) {
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
