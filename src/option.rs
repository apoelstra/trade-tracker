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

//! Options
//!
//! Data structure representing a single option
//!

use crate::terminal::{format_color, format_redgreen};
use log::info;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::{fmt, str};
use time::OffsetDateTime;

/// Whether an option is a put or a call
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum PutCall {
    /// A call
    Call,
    /// A put
    Put,
}
/// Re-export these so users don't have to keep writing PutCall::Call
pub use PutCall::{Call, Put};

impl PutCall {
    fn to_char(self) -> char {
        match self {
            Call => 'C',
            Put => 'P',
        }
    }
}

/// An option
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Option {
    /// Whether this is a put or a call
    pub pc: PutCall,
    /// Strike price
    pub strike: Decimal,
    /// Expiry date
    pub expiry: OffsetDateTime,
}

impl fmt::Display for Option {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = format!("{}{}{}", self.expiry.date(), self.pc.to_char(), self.strike,);
        f.pad(&s)
    }
}

impl str::FromStr for Option {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // e.g. 2023-01-27C10000
        if !s.is_ascii() {
            return Err(format!("option {} is not ASCII", s));
        }
        if s.len() < 12 {
            return Err(format!("option {} is too short (len {})", s, s.len()));
        }

        let expiry = time::Date::parse(&s[0..10], "%F")
            .map(|d| d.with_time(time::time!(21:00)))
            .map(|dt| dt.assume_utc().to_offset(time::UtcOffset::UTC))
            .map_err(|e| format!("Parsing time in option {}: {}", s, e))?;
        let pc = match s.as_bytes()[10] {
            b'C' | b'c' => Call,
            b'P' | b'p' => Put,
            x => return Err(format!("Unknown put/call symbol {} in {}", x, s)),
        };
        let strike = Decimal::from_str(&s[11..])
            .map_err(|e| format!("Parsing strike in option {}: {}", s, e))?;
        // return
        Ok(Option { pc, strike, expiry })
    }
}

impl Option {
    /// Construct a new call option
    pub fn new_call(strike: Decimal, expiry: OffsetDateTime) -> Self {
        Option {
            pc: Call,
            strike,
            expiry,
        }
    }

    /// Construct a new put option
    pub fn new_put(strike: Decimal, expiry: OffsetDateTime) -> Self {
        Option {
            pc: Put,
            strike,
            expiry,
        }
    }

    /// Outputs a tuple suitable for copy/pasting into Andrew's Excel sheet
    pub fn csv_tuple(
        &self,
    ) -> (
        std::option::Option<crate::csv::DateOnly>,
        &'static str,
        std::option::Option<Decimal>,
    ) {
        (
            Some(crate::csv::DateOnly(self.expiry)),
            match self.pc {
                crate::option::Call => "C",
                crate::option::Put => "P",
            },
            Some(self.strike),
        )
    }

    /// Compute the number of years to expiry, as a float, given current time
    pub fn years_to_expiry(&self, now: OffsetDateTime) -> f64 {
        (self.expiry - now) / time::Duration::days(365)
    }

    /// Whether the option is ITM or not. Considers options exactly at the money
    /// to be "in the money".
    ///
    /// If you need a more nuanced notion of OTM/ATM/ITM you will need to manually
    /// do it using the `strike` field.
    pub fn in_the_money(&self, btc_price: Decimal) -> bool {
        match self.pc {
            Call => self.strike <= btc_price,
            Put => self.strike >= btc_price,
        }
    }

    /// Computes the "annualized rate of return" of an OTM short option modeled as a loan
    ///
    /// Assuming that we were to sell an option which would then expire worthless, we
    /// can model this as a loan (from the seller's POV; the buyer can model this as
    /// a money fire). That is, for the remaining option lifetime, the seller must lock
    /// up some amount of collateral, and in exchange they receive some premium, or
    /// "interest".
    pub fn arr(&self, now: OffsetDateTime, btc_price: Decimal, self_price: Decimal) -> f64 {
        let yte = self.years_to_expiry(now);
        assert!(yte > 0.0);
        match self.pc {
            Put => {
                // For 100 put contracts, we lock up strike_price much cash
                // and receive self_price much cash. Easy.
                (Decimal::from(1) + self_price / self.strike)
                    .to_f64()
                    .unwrap()
                    .powf(1.0 / yte)
                    - 1.0
            }
            Call => {
                // For a call, we lock up 1 BTC at current price and receive
                // self_price much cash.
                (Decimal::from(1) + self_price / btc_price)
                    .to_f64()
                    .unwrap()
                    .powf(1.0 / yte)
                    - 1.0
            }
        }
    }

    /// The "intrinsic value" of the option, which is what it would be worth if
    /// it expired instantly at the current price
    pub fn intrinsic_value(&self, btc_price: Decimal) -> Decimal {
        match self.pc {
            Call => btc_price - self.strike,
            Put => self.strike - btc_price,
        }
    }

    /// Given a certain amount of BTC and USD, determine how many of this option
    /// we could short on LX without running out of cash/collateral.
    ///
    /// Assumes a fee on puts of $25/100 contracts. Returns the number of contracts
    /// that could be sold along with the cost in USD of each
    pub fn max_sale(
        &self,
        sale_price: Decimal,
        available_usd: Decimal,
        available_btc: Decimal,
    ) -> (u64, Decimal) {
        match self.pc {
            // For a call, we can sell as many as we have BTC to support
            Call => (
                (available_btc * Decimal::from(100)).to_u64().unwrap(),
                Decimal::from(0),
            ),
            // For a put it's a little more involved
            Put => {
                if sale_price > self.strike {
                    // This is a trollish situation where somebody is offering a put for more
                    // than the strike price, so any buyers would be buying the right to get
                    // some (but not all) of their money back in exchange for a coin. To avoid
                    // it causing us grief we just return 0s rather than computing crazy numbers.
                    return (0, Decimal::from(0));
                }
                let locked_per_100 = self.strike - sale_price + Decimal::from(25);
                let locked_per_1 = locked_per_100 / Decimal::from(100);
                (
                    (available_usd / locked_per_1).to_u64().unwrap(),
                    locked_per_1,
                )
            }
        }
    }

    /// Compute the price of the option at a given volatility
    pub fn bs_price(&self, now: OffsetDateTime, btc_price: Decimal, volatility: f64) -> f64 {
        match self.pc {
            Call => black_scholes::call(
                btc_price.to_f64().unwrap(),
                self.strike.to_f64().unwrap(),
                0.04f64, // risk free rate
                volatility,
                self.years_to_expiry(now),
            ),
            Put => black_scholes::put(
                btc_price.to_f64().unwrap(),
                self.strike.to_f64().unwrap(),
                0.04f64, // risk free rate
                volatility,
                self.years_to_expiry(now),
            ),
        }
    }

    /// Compute the IV of the option at a given price
    pub fn bs_iv(
        &self,
        now: OffsetDateTime,
        btc_price: Decimal,
        price: Decimal,
    ) -> Result<f64, f64> {
        match self.pc {
            Call => black_scholes::call_iv(
                price.to_f64().unwrap(),
                btc_price.to_f64().unwrap(),
                self.strike.to_f64().unwrap(),
                0.04f64, // risk free rate
                self.years_to_expiry(now),
            ),
            Put => black_scholes::put_iv(
                price.to_f64().unwrap(),
                btc_price.to_f64().unwrap(),
                self.strike.to_f64().unwrap(),
                0.04f64, // risk free rate
                self.years_to_expiry(now),
            ),
        }
    }

    /// Compute the theta of the option at a given price
    pub fn bs_theta(&self, now: OffsetDateTime, btc_price: Decimal, vol: f64) -> f64 {
        match self.pc {
            Call => {
                black_scholes::call_theta(
                    btc_price.to_f64().unwrap(),
                    self.strike.to_f64().unwrap(),
                    0.04f64, // risk free rate
                    vol,
                    self.years_to_expiry(now),
                ) / 365.0
            }
            Put => {
                black_scholes::put_theta(
                    btc_price.to_f64().unwrap(),
                    self.strike.to_f64().unwrap(),
                    0.04f64, // risk free rate
                    vol,
                    self.years_to_expiry(now),
                ) / 365.0
            }
        }
    }

    /// Compute the dual delta of the option at a given price
    pub fn bs_dual_delta(&self, now: OffsetDateTime, btc_price: Decimal, vol: f64) -> f64 {
        match self.pc {
            Call => {
                crate::local_bs::call_dual_delta(
                    btc_price.to_f64().unwrap(),
                    self.strike.to_f64().unwrap(),
                    0.04f64, // risk free rate
                    vol,
                    self.years_to_expiry(now),
                )
            }
            Put => {
                crate::local_bs::put_dual_delta(
                    btc_price.to_f64().unwrap(),
                    self.strike.to_f64().unwrap(),
                    0.04f64, // risk free rate
                    vol,
                    self.years_to_expiry(now),
                )
            }
        }
    }

    /// Compute the "loss 80" which is the probability that the option will wind up
    /// so far ITM that even the premium is lost
    pub fn bs_loss80(&self, now: OffsetDateTime, btc_price: Decimal, self_price: Decimal) -> f64 {
        let vol = 0.8;
        match self.pc {
            Call => {
                crate::local_bs::call_dual_delta(
                    btc_price.to_f64().unwrap(),
                    self.strike.to_f64().unwrap() + self_price.to_f64().unwrap(),
                    0.04f64, // risk free rate
                    vol,
                    self.years_to_expiry(now),
                )
            }
            Put => {
                crate::local_bs::put_dual_delta(
                    btc_price.to_f64().unwrap(),
                    self.strike.to_f64().unwrap() - self_price.to_f64().unwrap(),
                    0.04f64, // risk free rate
                    vol,
                    self.years_to_expiry(now),
                )
            }
        }
    }

    /// Compute the dual delta of the option at a given price
    pub fn bs_delta(&self, now: OffsetDateTime, btc_price: Decimal, vol: f64) -> f64 {
        match self.pc {
            Call => {
                black_scholes::call_delta(
                    btc_price.to_f64().unwrap(),
                    self.strike.to_f64().unwrap(),
                    0.04f64, // risk free rate
                    vol,
                    self.years_to_expiry(now),
                )
            }
            Put => {
                black_scholes::put_delta(
                    btc_price.to_f64().unwrap(),
                    self.strike.to_f64().unwrap(),
                    0.04f64, // risk free rate
                    vol,
                    self.years_to_expiry(now),
                )
            }
        }
    }

    /// Print option data
    pub fn log_option_data<D: fmt::Display>(
        &self,
        prefix: D,
        now: OffsetDateTime,
        btc_price: Decimal,
    ) {
        let dte = self.years_to_expiry(now) * 365.0;
        let dd80 = self.bs_dual_delta(now, btc_price, 0.80).abs();
        let intrinsic_str = if self.in_the_money(btc_price) {
            format!("{:5.2}", self.intrinsic_value(btc_price))
        } else {
            " OTM ".into()
        };

        info!(
            "{}{}  dte: {}  BTC: {:8.2}  intrinsic: {}  dd80: {}",
            prefix,
            format_color(format_args!("{:17}", self), 64, 192, 255),
            format_redgreen(format_args!("{:6.3}", dte), dte, 90.0, 0.0),
            btc_price,
            intrinsic_str,
            format_redgreen(format_args!("{:5.3}%", dd80 * 100.0), dd80, 0.15, 0.0),
        );
    }

    /// Print black-scholes data
    pub fn log_order_data<D: fmt::Display>(
        &self,
        prefix: D,
        now: OffsetDateTime,
        btc_price: Decimal,
        self_price: Decimal,
        size: std::option::Option<u64>,
    ) {
        let (vol_str, theta_str) = if let Ok(vol) = self.bs_iv(now, btc_price, self_price) {
            let theta = self.bs_theta(now, btc_price, vol);
            (
                format_redgreen(format_args!("{:3.2}", vol * 100.0), vol, 0.5, 1.2),
                format_redgreen(
                    format_args!("{:6.2}", theta),
                    theta,
                    0.0,
                    -self_price.to_f64().unwrap(),
                ),
            )
        } else {
            ("XXX".into(), "XXX".into())
        };
        let arr = self.arr(now, btc_price, self_price);
        // The "loss 80" is the likelihood that the option will end so far ITM that
        // even with preimum, it's a net loss, at an assumed 80% volatility
        let loss80 = self.bs_loss80(now, btc_price, self_price).abs();
        info!(
            "{}${}{}  sigma: {}%  loss80: {}  ARR: {}%, Theta: {}",
            prefix,
            format_redgreen(
                format_args!("{:8.2}", self_price),
                self_price.to_f64().unwrap().log10(),
                1.0,
                3.0
            ),
            if let Some(size) = size {
                let total = self_price * Decimal::new(size as i64, 2);
                format!(
                    " × {} = {}",
                    format_redgreen(format_args!("{:4}", size), (size as f64).log10(), 1.0, 4.0),
                    format_redgreen(
                        format_args!("{:8.2}", total),
                        total.to_f64().unwrap().log10(),
                        1.5,
                        5.0
                    )
                )
            } else {
                "".into()
            },
            vol_str,
            format_redgreen(format_args!("{:5.3}%", loss80 * 100.0), loss80, 0.15, 0.0),
            if arr > 10.0 {
                format_color(">1000%", 130, 220, 130)
            } else {
                format_redgreen(format_args!("{:4.2}", arr * 100.0), arr, 0.0, 0.2)
            },
            theta_str,
        );
    }
}
