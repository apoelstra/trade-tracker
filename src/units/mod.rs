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

//! Units
//!
//! Data structures representing the various fundamental units used throughout
//! the codebase. In general, any use of bare `Decimal` or `u64` is a code
//! smell and should be replaced by one of these.
//!

mod price;

pub use price::Price;
