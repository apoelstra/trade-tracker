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
use std::{collections::VecDeque, convert::TryFrom, fmt, mem};

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
}
impl csv::PrintCsv for CloseType {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CloseType::BuyBack => f.write_str("Buy Back"),
            CloseType::Sell => f.write_str("Sell"),
            CloseType::Expiry => f.write_str("Expired"),
            CloseType::Exercise => f.write_str("Exercised"),
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
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Lot {
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
            "Lot({}, {:?}, {:?}, {} @ {})",
            self.date.0.lazy_format("%FT%T"),
            self.direction,
            self.close_ty,
            self.quantity,
            self.price,
        )
    }
}

impl Lot {
    /// Constructs an `Lot` object from a trade event
    pub fn from_trade(price: Decimal, size: i64, fee: Decimal, date: time::OffsetDateTime) -> Lot {
        debug!(
            "Lot::from_trade price {} size {} fee {} date {}",
            price, size, fee, date
        );
        let unit_fee = fee / Decimal::new(size, 2);
        let adj_price = price + unit_fee; // nb `unit_fee` is a signed quantity
        Lot {
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
            close_ty: CloseType::Exercise,
            direction: Direction::from_size(n_assigned),
            quantity: n_assigned.unsigned_abs(),
            price: opt.intrinsic_value(btc_price),
            date: TaxDate(expiry),
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Close {
    ty: CloseType,
    gain_ty: GainType,
    open_direction: Direction,
    open_price: Decimal,
    open_date: TaxDate,
    close_price: Decimal,
    close_date: TaxDate,
    quantity: u64,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct CloseCsv<'label, 'close> {
    label: &'label Label,
    close: &'close Close,
}

impl<'label, 'close> csv::PrintCsv for CloseCsv<'label, 'close> {
    fn print(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let i_qty = i64::try_from(self.close.quantity).unwrap();
        let mut proceeds = Decimal::new(i_qty, 2) * self.close.close_price;
        let mut basis = Decimal::new(i_qty, 2) * self.close.open_price;
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
            format!("{}, {}", Decimal::new(i_qty, 2), self.label)
        } else {
            format!("{}, {}", Decimal::new(i_qty, 0), self.label)
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
            .print(f)
    }
}

impl Close {
    /// Constructs a CSV outputter for this close
    pub fn csv_printer<'label, 'close>(
        &'close self,
        label: &'label Label,
    ) -> csv::CsvPrinter<CloseCsv<'label, 'close>> {
        csv::CsvPrinter(CloseCsv { label, close: self })
    }
}

/// A position in a specific asset, represented by a FIFO queue of opening events
#[derive(Default)]
pub struct Position {
    fifo: VecDeque<Lot>,
}

impl Position {
    /// Creates a new empty position
    pub fn new() -> Self {
        Position {
            fifo: Default::default(),
        }
    }

    fn total_size(&self) -> u64 {
        self.fifo.iter().map(|open| open.quantity).sum()
    }

    fn direction(&self) -> Option<Direction> {
        self.fifo.front().map(|open| open.direction)
    }

    /// Modifies the position based on an event.
    ///
    /// If this results in closing previously opened positions, return a list of
    /// `Close`s with their type set to the `close_type` argument.
    pub fn push_event(&mut self, mut open: Lot, is_1256: bool) -> Vec<Close> {
        // If the position is empty, then just push
        if self.fifo.is_empty() {
            debug!("Create new position with open {}", open);
            self.fifo.push_back(open);
            return vec![];
        }
        // If there is an open position, in the same direction as the new open,
        // add the new open to the FIFO.
        if self.fifo.front().unwrap().direction == open.direction {
            debug!(
                "Increasing position ({:?}, qty {}) with open {}",
                self.direction(),
                self.total_size(),
                open
            );
            self.fifo.push_back(open);
            return vec![];
        }
        // Otherwise, we must close the position
        let mut ret = vec![];
        while open.quantity > 0 {
            debug!(
                "Closing position ({:?}, qty {}) with open {}",
                self.direction(),
                self.total_size(),
                open
            );
            let mut front = match self.fifo.pop_front() {
                Some(front) => front,
                None => {
                    // If we've closed out everything and still have an open, then
                    // we're opening a new position in the opposite direction.
                    self.fifo.push_back(open);
                    return ret;
                }
            };
            assert_ne!(open.direction, front.direction);

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
                self.fifo.push_front(front); // put incompletely closed position back
            } else {
                // Full close
                close.quantity = front.quantity;
                open.quantity -= front.quantity;
            }
            ret.push(close);
        }

        return ret;
    }
}
