// Trade Tracker
// Written in 2024 by
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

//! UTC Time
//!
//! UTC timestamps. This is a thin wrapper around `chrono::DateTime<chrono::offset::Utc>`.
//!

use chrono::offset::Utc;
use chrono::{DateTime, Datelike as _, ParseError, Timelike as _};
use core::str::FromStr as _;
use core::{fmt, num, ops};
use serde::Deserialize;

#[derive(Debug)]
pub enum Error {
    ParseError(ParseError),
    ParseNum(num::ParseIntError),
    UnixTimeOutOfRange(i64),
}

impl From<ParseError> for Error {
    fn from(e: ParseError) -> Error {
        Error::ParseError(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::ParseError(ref e) => e.fmt(f),
            Error::ParseNum(ref e) => e.fmt(f),
            Error::UnixTimeOutOfRange(n) => {
                write!(f, "timestamp {n} out of range for UNIX timestamp")
            }
        }
    }
}

impl std::error::Error for Error {
    fn cause(&self) -> Option<&dyn std::error::Error> {
        match *self {
            Error::ParseError(ref e) => Some(e),
            Error::ParseNum(ref e) => Some(e),
            Error::UnixTimeOutOfRange(_) => None,
        }
    }
}

/// A timestamp fixed to the UTC timezone. This is a thin wrapper around
/// `chrono::DateTime<Utc>`. If you find you need conversions from other
/// timezones please add an explicit conversion function.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, Deserialize)]
pub struct UtcTime {
    inner: DateTime<Utc>,
}

impl UtcTime {
    /// Returns the current time
    pub fn now() -> Self {
        UtcTime { inner: Utc::now() }
    }

    /// Parses a UNIX timestamp from an integer number of seconds
    pub fn from_unix_nanos_i64(n: i64) -> Result<Self, Error> {
        Ok(UtcTime {
            inner: chrono::DateTime::from_timestamp(n / 1_000_000_000, (n % 1_000_000_000) as u32)
                .ok_or(Error::UnixTimeOutOfRange(n / 1_000_000_000))?,
        })
    }

    /// Parses a UNIX timestamp from an integer number of seconds
    pub fn from_unix_i64(n: i64) -> Result<Self, Error> {
        Ok(UtcTime {
            inner: chrono::DateTime::from_timestamp(n, 0).ok_or(Error::UnixTimeOutOfRange(n))?,
        })
    }

    /// Parses the date embedded in an option expiry (e.g. 2024-01-24C50000)
    pub fn parse_option_expiry(s: &str) -> Result<Self, Error> {
        let expiry = chrono::NaiveDate::parse_from_str(&s[0..10], "%F")?
            .and_hms_opt(21, 0, 0)
            .unwrap()
            .and_utc();
        Ok(UtcTime { inner: expiry })
    }

    /// Parses the date from Coinbase API calls
    pub fn parse_coinbase(s: &str) -> Result<Self, Error> {
        Ok(UtcTime {
            inner: chrono::DateTime::parse_from_rfc3339(s)?.into(),
        })
    }

    /// Returns a copy of the given timestamp, with the time component set to a specific hour
    pub fn forced_to_hour(&self, n: u32) -> Self {
        UtcTime {
            inner: self
                .inner
                .with_hour(n)
                .unwrap()
                .with_minute(0)
                .unwrap()
                .with_second(0)
                .unwrap()
                .with_nanosecond(0)
                .unwrap(),
        }
    }

    /// Parses a UNIX timestamp from a decimal-string encoded number of seconds
    pub fn from_unix_str(n: &str) -> Result<Self, Error> {
        let i = i64::from_str(n).map_err(Error::ParseNum)?;
        Self::from_unix_i64(i)
    }

    /// Creates an object which can be given to a formatter
    pub fn format<'s>(&self, s: &'s str) -> impl fmt::Display + 's {
        self.inner.format(s)
    }

    /// Accessor for the year
    pub fn year(&self) -> i32 {
        self.inner.year()
    }

    /// Accessor for the month
    pub fn month(&self) -> u32 {
        self.inner.month()
    }

    /// Accessor for the day
    pub fn day(&self) -> u32 {
        self.inner.day()
    }

    /// Accessor for the hour
    pub fn hour(&self) -> u32 {
        self.inner.hour()
    }

    /// Accessor for the minute
    pub fn minute(&self) -> u32 {
        self.inner.minute()
    }

    /// Accessor for the second
    pub fn second(&self) -> u32 {
        self.inner.second()
    }

    /// Accessor for the sub-second part
    pub fn nanosecond(&self) -> u32 {
        self.inner.nanosecond()
    }
}

impl<T: Into<DateTime<Utc>>> From<T> for UtcTime {
    fn from(t: T) -> Self {
        UtcTime { inner: t.into() }
    }
}

impl fmt::Display for UtcTime {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl ops::Add<chrono::Duration> for UtcTime {
    type Output = Self;
    fn add(self, other: chrono::Duration) -> Self::Output {
        UtcTime {
            inner: self.inner + other,
        }
    }
}

impl ops::Sub<chrono::Duration> for UtcTime {
    type Output = Self;
    fn sub(self, other: chrono::Duration) -> Self::Output {
        UtcTime {
            inner: self.inner - other,
        }
    }
}

impl ops::AddAssign<chrono::Duration> for UtcTime {
    fn add_assign(&mut self, other: chrono::Duration) {
        self.inner += other;
    }
}

impl ops::Sub for UtcTime {
    type Output = chrono::Duration;
    fn sub(self, other: Self) -> Self::Output {
        self.inner - other.inner
    }
}

pub mod serde_ts_seconds {
    use super::*;

    use serde::{de, Deserializer, Serialize, Serializer};

    pub fn deserialize<'de, D>(deser: D) -> Result<UtcTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let n = Deserialize::deserialize(deser)?;
        UtcTime::from_unix_i64(n).map_err(|_| {
            de::Error::invalid_value(de::Unexpected::Signed(n), &"a valid UNIX timestamp")
        })
    }

    pub fn serialize<S>(obj: &UtcTime, ser: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Serialize::serialize(&obj.inner.timestamp(), ser)
    }
}
