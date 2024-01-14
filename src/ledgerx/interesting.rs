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

//! Interestingness
//!
//! Algorithms which decide whether a certain option is "interesting" to make
//! a bid/ask on, or whether a certain standing order is worth taking
//!

use crate::ledgerx::{Contract, Underlying};
use crate::option;
use crate::price::BitcoinPrice;
use crate::units::{Price, Quantity, UtcTime};
use log::warn;
use std::marker::PhantomData;
use std::{cmp, fmt, ops};

pub trait OrderType: Eq + fmt::Debug + Copy {}
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum Bid {}
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum Ask {}

impl OrderType for Bid {}
impl OrderType for Ask {}

#[derive(PartialEq, Eq, Copy, Clone, Debug)]
enum Moneyness {
    Itm,
    Atm,
    Otm,
}

impl Moneyness {
    /// Determine the moneyness of the option. Our price reference is not
    /// necessarily very precise so we consider +/- 1% to be "at the money"
    /// rather than making a determination of moneyness.
    fn from_option(btc_price: Price, option: &option::Option) -> Self {
        let ratio = btc_price / option.strike;
        match option.pc {
            option::PutCall::Call => {
                if ratio < 0.99 {
                    Moneyness::Otm
                } else if ratio < 1.01 {
                    Moneyness::Atm
                } else {
                    Moneyness::Itm
                }
            }
            option::PutCall::Put => {
                if ratio < 0.99 {
                    Moneyness::Itm
                } else if ratio < 1.01 {
                    Moneyness::Atm
                } else {
                    Moneyness::Otm
                }
            }
        }
    }
}

/// Utility function which sanity checks that a price reference is not too old.
fn check_price_ref(now: UtcTime, btc_price: BitcoinPrice) -> bool {
    if now - btc_price.timestamp > chrono::Duration::minutes(5) {
        warn!(
            "Price reference {} is more than 5 minutes old ({})",
            btc_price,
            UtcTime::now() - btc_price.timestamp,
        );
        false
    } else {
        true
    }
}

/// The degree to which an order is interesting.
///
/// Ranked in order of how much we want to be a counterparty. The lowest level
/// is therefore "match", meaning that we might want to open our own order at
/// the same price. The highest level is "take", meaning that if somebody else
/// had opened this order, we'd want to take it.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Interestingness {
    /// The order is interesting enough that we should open our own matching
    /// order.
    Match,
    /// The order is interesting enough to log (and maybe manually match),
    /// but not interesting enough to do anything autonomously.
    LogMatch,
    /// The order is not interesting at all.
    No,
    /// The order is interesting enough to log (and maybe manually take),
    /// but not interesting enough to do anything autonomously.
    LogTake,
    /// The order is interesting enough that we should take the other side.
    Take,
}

impl Interestingness {
    /// Inverts the interestingness (swapping "take" and "match")
    pub fn invert(self) -> Self {
        match self {
            Interestingness::Match => Interestingness::Take,
            Interestingness::LogMatch => Interestingness::LogTake,
            Interestingness::No => Interestingness::No,
            Interestingness::LogTake => Interestingness::LogMatch,
            Interestingness::Take => Interestingness::Match,
        }
    }
}

pub fn extract_option(contract: &Contract, btc_price: BitcoinPrice) -> Option<option::Option> {
    let now = UtcTime::now();

    // Immediately reject non-BTC contracts
    if contract.underlying() != Underlying::Btc {
        return None;
    }
    // Immediately reject non-options; we are not trying to directly trade the underlying.
    let opt = contract.as_option()?;
    // Immediately reject expired options.
    if opt.expiry <= now {
        return None;
    }
    // Only consider OTM options (for now)
    let moneyness = Moneyness::from_option(btc_price.btc_price, &opt);
    if moneyness != Moneyness::Otm {
        return None;
    }
    // Reject if our price reference is stale (but do this last to try to reduce log spam)
    if !check_price_ref(now, btc_price) {
        warn!(
            "Price reference {} is more than 5 minutes old ({})",
            btc_price,
            UtcTime::now() - btc_price.timestamp,
        );
        return None;
    }

    Some(opt)
}

/// Statistics about an order that tell us whether it is worth making or matching.
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub struct OrderStats<T: OrderType> {
    order_type: PhantomData<T>,
    /// The underlying option
    option: option::Option,
    /// Bitcoin price reference
    btc_price: BitcoinPrice,
    /// Price of the order in question
    order_price: Price,
    /// Size of the order in question
    order_size: Quantity,
}

pub type BidStats = OrderStats<Bid>;
pub type AskStats = OrderStats<Ask>;

impl<T: OrderType> ops::AddAssign for OrderStats<T> {
    fn add_assign(&mut self, other: Self) {
        if self.option != other.option {
            panic!("Tried to add two OrderStats structs associated to different options");
        }

        // Average BTC prices. We do a weighted average using the order
        // size as a weight, in order that our "addition" be associative.
        self.btc_price.timestamp = cmp::min(self.btc_price.timestamp, other.btc_price.timestamp);
        self.btc_price.btc_price = self.btc_price.btc_price.average(
            self.order_size,
            other.btc_price.btc_price,
            other.order_size,
        );
        self.order_price =
            self.order_price
                .average(self.order_size, other.order_price, other.order_size);
        self.order_size += other.order_size;
    }
}

impl<T: OrderType> OrderStats<T> {
    /// Creates an order statistics from an order and some context
    pub fn from_order(
        btc_price: BitcoinPrice,
        contract: &Contract,
        order_price: Price,
        order_size: Quantity,
    ) -> Option<Self> {
        let opt = extract_option(contract, btc_price)?;

        Some(OrderStats {
            order_type: PhantomData,
            option: opt,
            btc_price,
            order_price,
            order_size,
        })
    }

    /// Annualized rate of return on collateral of a short option, assuming
    /// the option expires worthless
    pub fn arr(&self) -> f64 {
        let now = UtcTime::now();
        assert!(
            check_price_ref(now, self.btc_price),
            "bitcoin price is not fresh",
        );
        self.option
            .arr(now, self.btc_price.btc_price, self.order_price)
    }

    /// Assuming the Black-Scholes model with 80% volatility, the probability that
    /// this order's option will end so far in the money that the short side of the
    /// order will lose money
    pub fn loss80(&self) -> f64 {
        let now = UtcTime::now();
        assert!(
            check_price_ref(now, self.btc_price),
            "bitcoin price is not fresh",
        );
        self.option
            .bs_loss80(now, self.btc_price.btc_price, self.order_price)
    }

    /// The implied volatility of the underlying option at the price of the order
    pub fn iv(&self) -> f64 {
        let now = UtcTime::now();
        assert!(
            check_price_ref(now, self.btc_price),
            "bitcoin price is not fresh",
        );
        // An IV calculation can fail, but only for "free money" options, which are
        // ITM options being sold for a lower price than their intrinsic value.
        //
        // We ignore such options at least for now, because claiming the free money
        // is a bit of a PITA on LX which has low liquidity for BTC.
        self.option
            .bs_iv(now, self.btc_price.btc_price, self.order_price)
            .expect("computing IV for ITM option in place where OTM is assumed")
    }

    // TODO add cash yield and BTC yield accessors, and "constrain to" modifier
    // for low available funds

    /// Accessor for the total value of the order
    pub fn total_value(&self) -> Price {
        self.order_price * self.order_size
    }

    /// Accessor for the order size
    pub fn order_size(&self) -> Quantity {
        self.order_size
    }

    /// Accessor for the order price
    pub fn order_price(&self) -> Price {
        self.order_price
    }
}

impl OrderStats<Bid> {
    /// The interestingness of a bid.
    ///
    /// Since our current strategy exclusivly involves selling options, this
    /// will range from "No" to "Take" but we will never considered matching
    /// a bid.
    ///
    /// Our criteria to take an order are a low loss80 (likelihood of getting
    /// run over) and a high IV. For puts we also consider the ARR.
    pub fn interestingness(&self) -> Interestingness {
        // If the order has crappy stats, it's not interesting
        if self.loss80() > 0.1 || self.iv() < 0.7 {
            return Interestingness::No;
        }
        if self.option.pc == option::PutCall::Put && self.arr() < 0.04 {
            return Interestingness::No;
        }
        // If the order has very good stats, we want to take it
        if self.loss80() < 0.05 && self.iv() > 0.85 {
            if self.option.pc == option::PutCall::Call || self.arr() > 0.05 {
                return Interestingness::Take;
            }
        }
        // Otherwise it's a "log"
        Interestingness::LogTake
    }
}

impl OrderStats<Ask> {
    /// The interestingness of a bid.
    ///
    /// Since our current strategy exclusivly involves selling options, this
    /// will range from "Match" to "No" but we will never considered taking
    /// an ask.
    pub fn interestingness(&self) -> Interestingness {
        // We just pass through the interestingness check on the equivalent
        // bid and invert it.
        let equiv_bid: OrderStats<Bid> = OrderStats {
            btc_price: self.btc_price,
            option: self.option,
            order_price: self.order_price,
            order_size: self.order_size,
            order_type: PhantomData,
        };
        equiv_bid.interestingness().invert()
    }
}
