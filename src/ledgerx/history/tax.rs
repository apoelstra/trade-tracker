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
    ledgerx::history::lot::{Close, CloseType, Lot, OpenType},
    units::{Price, Quantity, TaxAsset, Underlying},
};
use anyhow::Context;
use log::debug;
use std::{cmp, collections::HashMap, fmt, ops};

/// Wrapper around a date that will output time to the nearest second in 3339 format
#[derive(Copy, Clone, PartialOrd, Ord, PartialEq, Eq, Debug)]
pub struct TaxDate(time::OffsetDateTime);

impl TaxDate {
    /// Accessor for the internal timestamp
    pub fn bare_time(&self) -> time::OffsetDateTime {
        self.0
    }

    /// Year of this tax date
    pub fn year(&self) -> i32 {
        self.0.year()
    }
}

impl ops::Sub for TaxDate {
    type Output = time::Duration;
    fn sub(self, other: TaxDate) -> Self::Output {
        self.0 - other.0
    }
}

impl fmt::Display for TaxDate {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        csv::PrintCsv::print(self, f)
    }
}

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

impl From<time::OffsetDateTime> for TaxDate {
    fn from(t: time::OffsetDateTime) -> Self {
        TaxDate(t)
    }
}

/// Whether cap gains are short or long term, or 1256 (60% long / 40% short)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum GainType {
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

/// A position in a specific asset, represented by a FIFO queue of opening events
#[derive(Clone, Debug)]
pub struct Position {
    asset: TaxAsset,
    queue: crate::TimeMap<Lot>,
}

impl Position {
    /// Creates a new empty position
    pub fn new(asset: TaxAsset) -> Self {
        Position {
            asset,
            queue: Default::default(),
        }
    }

    /// Given a quantity, returns whether this position is open in the same direction,
    /// or is empty (so is "open" in both directions)
    pub fn has_same_direction(&self, quantity: Quantity) -> bool {
        let direction = self
            .queue
            .values()
            .next()
            .map(Lot::quantity)
            .unwrap_or(Quantity::Zero);
        direction.has_same_sign(quantity)
    }

    /// Sums over everything in the queue
    fn total_size(&self) -> Quantity {
        self.queue.values().map(|lot| lot.quantity()).sum()
    }

    /// Adds a given quantity to the position
    ///
    /// If the quantity is in the same direction as the existing position,
    /// creates a new lot at the back of the FIFO queue. If it is in an
    /// opposite direction, closes out existing lots. If it runs out of lots
    /// to close, opens a new position with the remainder.
    ///
    /// Returns a vector of close events, if any, and a copy of the new lot,
    /// if any.
    fn add(
        &mut self,
        mut quantity: Quantity,
        price: Price,
        date: TaxDate,
        open_ty: OpenType,
        close_ty: CloseType,
    ) -> anyhow::Result<(Vec<Close>, Option<Lot>)> {
        if self.has_same_direction(quantity) {
            let new_lot = Lot::new(self.asset, quantity, price, date, open_ty);
            self.queue.insert(new_lot.sort_date(), new_lot.clone());
            Ok((vec![], Some(new_lot)))
        } else {
            let mut closes = vec![];
            // FIXME choose pop strategy intelligently
            while let Some((existing_date, existing_lot)) = self.queue.pop_first() {
                let existing_qty = existing_lot.quantity();
                let (close, partial) = existing_lot
                    .close(quantity, price, date, close_ty)
                    .with_context(|| {
                        format!(
                            "Closing {} lot, qty {quantity} price {price} date {date}",
                            self.asset,
                        )
                    })?;
                closes.push(close);
                if let Some(partial_lot) = partial {
                    // Put back any partial fills
                    self.queue.insert(existing_date, partial_lot);
                    return Ok((closes, None));
                } else {
                    quantity += existing_qty;
                    if !quantity.is_nonzero() {
                        return Ok((closes, None));
                    }
                }
            }
            // If we get to this point we ran out of things to close, so create
            // a new lot and return.
            if quantity.is_nonzero() {
                let new_lot = Lot::new(self.asset, quantity, price, date, open_ty);
                self.queue.insert(new_lot.sort_date(), new_lot.clone());
                Ok((closes, Some(new_lot)))
            } else {
                Ok((closes, None))
            }
        }
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
    pub asset: TaxAsset,
    pub open_close: OpenClose,
}

/// Tracks positions in multiple assets, recording tax events
#[derive(Clone, Debug, Default)]
pub struct PositionTracker {
    positions: HashMap<TaxAsset, Position>,
    events: Vec<Event>,
}

impl PositionTracker {
    /// Constructs a new empty position tracker
    pub fn new() -> Self {
        Default::default()
    }

    /// Helper function to log a set of closes and opens
    ///
    /// Returns the number of loses
    fn push_events(&mut self, log_str: &str, closes: Vec<Close>, open: Option<Lot>) -> usize {
        let n_ret = closes.len();
        // ...then log it
        for close in closes {
            debug!("{}: close {}", log_str, close);
            self.events.push(Event {
                date: close.close_date(),
                asset: close.asset(),
                open_close: OpenClose::Close(close),
            });
        }
        if let Some(lot) = open {
            debug!("{}: new lot {}", log_str, lot);
            self.events.push(Event {
                date: lot.date(),
                asset: lot.asset(),
                open_close: OpenClose::Open(lot),
            });
        }
        // Return the number of closes that happened
        n_ret
    }

    /// Directly insert a lot without attempting any cancelation etc
    ///
    /// Panics if there is already an open position in the opposite direction
    /// of this lot.
    pub fn push_lot(&mut self, lot: Lot) {
        debug!(
            "[position-tracker] direct push of lot {} (sort date {})",
            lot,
            lot.sort_date()
        );
        // Assert that deposits do not close any positions (since we cannot have
        // a short BTC position)
        let pos = self
            .positions
            .entry(lot.asset())
            .or_insert(Position::new(TaxAsset::Bitcoin));
        assert!(
            pos.has_same_direction(lot.quantity()),
            "Tried to directly insert {} but had an opposing position open",
            lot,
        );
        pos.queue.insert(lot.sort_date(), lot);
    }

    /// Expire a bunch of some option. Returns the number of lots closed.
    pub fn push_expiry(
        &mut self,
        option: crate::option::Option,
        underlying: Underlying,
        size: Quantity,
    ) -> anyhow::Result<usize> {
        let asset = TaxAsset::Option { underlying, option };
        debug!("[position-tracker] expiry of asset {} size {}", asset, size);
        // Force expiry date to match LX goofiness
        let expiry = option
            .expiry
            .date()
            .with_time(time::time!(22:00))
            .assume_utc();
        let pos = match self.positions.get_mut(&asset) {
            Some(pos) => pos,
            None => {
                return Err(anyhow::Error::msg(format!(
                    "attempted expiry of asset {} but no position open",
                    asset
                )))
            }
        };

        // Do the expiry
        let (closes, open) = pos
            .add(
                size,
                Price::ZERO,
                expiry.into(),
                OpenType::Unknown,
                CloseType::Expiry,
            )
            .with_context(|| format!("Expiring {size} units of {asset}"))?;
        // Return an error if it wasn't a clean close
        if let Some(lot) = open {
            return Err(anyhow::Error::msg(format!(
                "attempted expiry of {asset} but had fewer; left over {lot}"
            )));
        }
        // If this was a complete expiry delete the asset
        if pos.queue.is_empty() {
            self.positions.remove(&asset);
        }

        // Return the number of closes that happened.
        Ok(self.push_events("push_expiry", closes, None))
    }

    /// Assign a bunch of some option. Returns the number of lots closed.
    pub fn push_assignment(
        &mut self,
        option: crate::option::Option,
        underlying: Underlying,
        size: Quantity,
        btc_price: Price,
    ) -> anyhow::Result<usize> {
        let asset = TaxAsset::Option { underlying, option };
        debug!(
            "[position-tracker] assignment of asset {} size {}",
            asset, size
        );
        // Force expiry date to match LX goofiness
        let expiry = option
            .expiry
            .date()
            .with_time(time::time!(22:00))
            .assume_utc();
        let pos = match self.positions.get_mut(&asset) {
            Some(pos) => pos,
            None => {
                return Err(anyhow::Error::msg(format!(
                    "attempted assignment of asset {} but no position open",
                    asset
                )))
            }
        };
        // Do the assignment. Note that the options go away but this is *not* a
        // "close at price 0" but a "close at intrinsic price". The provided
        // BTC price should come from the LX price reference.
        let price = option.intrinsic_value(btc_price);
        let (closes, open) = pos
            .add(
                size,
                price,
                expiry.into(),
                OpenType::Unknown,
                CloseType::Exercise,
            )
            .with_context(|| format!("Assigned on {size} units of {asset}"))?;
        // Return an error if it wasn't a clean close
        if let Some(lot) = open {
            return Err(anyhow::Error::msg(format!(
                "attempted assignment of {asset} but had fewer; left over {lot}"
            )));
        }
        // Assignments should happen after expiries, and should be total (i.e. there
        // should be nothing left over). This is essentially just a sanity check.
        if !pos.queue.is_empty() {
            return Err(anyhow::Error::msg(format!(
                "done assignment of {asset} but position not fully closed; remaining {}",
                pos.total_size()
            )));
        }
        self.positions.remove(&asset);

        // Return the number of closes that happened.
        Ok(self.push_events("push_assignment", closes, None))
    }

    /// Adds a trade of some asset to the tracker, adjusting positions as appropriate.
    ///
    /// The lot may add to a position, in which case it is an "open". Or it may shrink one
    /// or more existing lots, in which case it is a "close".
    ///
    /// Returns the number of lots closed.
    pub fn push_trade(
        &mut self,
        mut asset: TaxAsset,
        quantity: Quantity,
        price: Price,
        mut date: TaxDate,
    ) -> anyhow::Result<usize> {
        let (open_ty, close_ty) = if quantity.is_nonnegative() {
            (OpenType::BuyToOpen, CloseType::BuyBack)
        } else {
            (OpenType::SellToOpen, CloseType::Sell)
        };

        // Dayaheads we have to convert to bitcoin to ensure they are tracked correctly.
        // Furthermore, long positions we bump to the expiry date of the dayahead.
        if let TaxAsset::NextDay { underlying, expiry } = asset {
            assert_eq!(underlying, Underlying::Btc);
            // Lol, not the actual expiry date. The expiry date with its timestamp
            // munged to be equal to 21:00.
            //
            // Furthermore, note that the date is *always* forced, even for short positions,
            // even though in the LX trading interface, once you sell a next day, you
            // receive immediate cash.
            //
            // I believe the interpretation here is that the nextday trade actually has
            // zero tax consequence since it's an exchange of cash for a cash contract
            // of equal value. It is only at expiry, when bitcoin changes hands, that
            // a taxable event occurs.
            date = expiry
                .date()
                .with_time(time::time!(21:00))
                .assume_utc()
                .into();
            asset = TaxAsset::Bitcoin;
        }

        let pos = self.positions.entry(asset).or_insert(Position::new(asset));
        let (closes, open) = pos
            .add(quantity, price, date, open_ty, close_ty)
            .with_context(|| format!("adding {quantity} units of {asset} at {price} on {date}",))?;

        Ok(self.push_events("push_trade", closes, open))
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
                cx.open_date().cmp(&cy.open_date())
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
