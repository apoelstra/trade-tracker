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

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::{fmt, str};

/// Whether an option is a put or a call
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum PutCall {
    /// A call
    Call,
    /// A put
    Put,
}

impl PutCall {
    fn to_char(self) -> char {
        match self {
            PutCall::Call => 'C',
            PutCall::Put => 'P',
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
    pub expiry: time::OffsetDateTime,
}

impl fmt::Display for Option {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}{}{}",
            self.expiry.date(),
            self.pc.to_char(),
            self.strike,
        )
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
            b'C' | b'c' => PutCall::Call,
            b'P' | b'p' => PutCall::Put,
            x => return Err(format!("Unknown put/call symbol {} in {}", x, s)),
        };
        let strike = Decimal::from_str(&s[11..])
            .map_err(|e| format!("Parsing strike in option {}: {}", s, e))?;
        // return
        Ok(Option { pc, strike, expiry })
    }
}

impl Option {
    /// Compute the number of years to expiry, as a float, given current time
    pub fn years_to_expiry(&self, now: &time::OffsetDateTime) -> f64 {
        (self.expiry - *now) / time::Duration::days(365)
    }

    /// Compute the price of the option at a given volatility
    pub fn bs_price(&self, now: &time::OffsetDateTime, btc_price: Decimal, volatility: f64) -> f64 {
        match self.pc {
            PutCall::Call => black_scholes::call(
                btc_price.to_f64().unwrap(),
                self.strike.to_f64().unwrap(),
                0.04f64, // risk free rate
                volatility,
                self.years_to_expiry(now),
            ),
            PutCall::Put => black_scholes::put(
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
        now: &time::OffsetDateTime,
        btc_price: Decimal,
        price: Decimal,
    ) -> Result<f64, f64> {
        match self.pc {
            PutCall::Call => black_scholes::call_iv(
                price.to_f64().unwrap(),
                btc_price.to_f64().unwrap(),
                self.strike.to_f64().unwrap(),
                0.04f64, // risk free rate
                self.years_to_expiry(now),
            ),
            PutCall::Put => black_scholes::put_iv(
                price.to_f64().unwrap(),
                btc_price.to_f64().unwrap(),
                self.strike.to_f64().unwrap(),
                0.04f64, // risk free rate
                self.years_to_expiry(now),
            ),
        }
    }

    /// Compute the theta of the option at a given price
    pub fn bs_theta(&self, now: &time::OffsetDateTime, btc_price: Decimal, vol: f64) -> f64 {
        match self.pc {
            PutCall::Call => {
                black_scholes::call_theta(
                    btc_price.to_f64().unwrap(),
                    self.strike.to_f64().unwrap(),
                    0.04f64, // risk free rate
                    vol,
                    self.years_to_expiry(now),
                ) / 365.0
            }
            PutCall::Put => {
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
    pub fn bs_dual_delta(&self, now: &time::OffsetDateTime, btc_price: Decimal, vol: f64) -> f64 {
        match self.pc {
            PutCall::Call => {
                crate::local_bs::call_dual_delta(
                    btc_price.to_f64().unwrap(),
                    self.strike.to_f64().unwrap(),
                    0.04f64, // risk free rate
                    vol,
                    self.years_to_expiry(now),
                )
            }
            PutCall::Put => {
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
}
