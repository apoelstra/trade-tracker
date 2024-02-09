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
use serde::{de, Deserialize, Deserializer};

#[derive(Debug)]
pub enum Error {
    Parse(ParseError),
    ParseNum(num::ParseIntError),
    UnixTimeOutOfRange(i64),
}

impl From<ParseError> for Error {
    fn from(e: ParseError) -> Error {
        Error::Parse(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::Parse(ref e) => e.fmt(f),
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
            Error::Parse(ref e) => Some(e),
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

    /// Returns the current time in New York
    pub fn new_york_time(&self) -> chrono::NaiveTime {
        // Rather than dealing with a bunch of "2AM on the second sunday" bullshit,
        // we just assume that DST happens at midnight UTC (which is 9 or 10PM in
        // New York so the market is never open) and just fix the dates. Hopefully
        // by the time this table runs out we have dropped DST.

        // The following table was obtained from ChatGPT. I hand-compared it to
        // a computation in gnumeric using the table copied on 024-02-09 from
        // https://en.wikipedia.org/wiki/Daylight_saving_time_in_the_United_States
        // which only went to 2027, but let me sanity-check the pattern.
        let est_tz = chrono::offset::FixedOffset::west_opt(5 * 3600).unwrap();
        let edt_tz = chrono::offset::FixedOffset::west_opt(4 * 3600).unwrap();
        let tz = match self.inner.year() {
            2024 if self.inner.ordinal0() < 69 || self.inner.ordinal0() >= 307 => est_tz,
            2025 if self.inner.ordinal0() < 67 || self.inner.ordinal0() >= 305 => est_tz,
            2026 if self.inner.ordinal0() < 66 || self.inner.ordinal0() >= 304 => est_tz,
            2027 if self.inner.ordinal0() < 72 || self.inner.ordinal0() >= 310 => est_tz,
            2028 if self.inner.ordinal0() < 71 || self.inner.ordinal0() >= 309 => est_tz,
            2029 if self.inner.ordinal0() < 69 || self.inner.ordinal0() >= 307 => est_tz,
            2030 if self.inner.ordinal0() < 68 || self.inner.ordinal0() >= 306 => est_tz,
            2031 if self.inner.ordinal0() < 67 || self.inner.ordinal0() >= 305 => est_tz,
            2032 if self.inner.ordinal0() < 73 || self.inner.ordinal0() >= 311 => est_tz,
            2033 if self.inner.ordinal0() < 71 || self.inner.ordinal0() >= 309 => est_tz,
            2034 if self.inner.ordinal0() < 70 || self.inner.ordinal0() >= 308 => est_tz,
            2035 if self.inner.ordinal0() < 69 || self.inner.ordinal0() >= 307 => est_tz,
            2036 if self.inner.ordinal0() < 68 || self.inner.ordinal0() >= 306 => est_tz,
            2037 if self.inner.ordinal0() < 66 || self.inner.ordinal0() >= 304 => est_tz,
            2038 if self.inner.ordinal0() < 72 || self.inner.ordinal0() >= 310 => est_tz,
            2040 if self.inner.ordinal0() < 70 || self.inner.ordinal0() >= 308 => est_tz,
            2041 if self.inner.ordinal0() < 68 || self.inner.ordinal0() >= 306 => est_tz,
            2042 if self.inner.ordinal0() < 67 || self.inner.ordinal0() >= 305 => est_tz,
            2043 if self.inner.ordinal0() < 66 || self.inner.ordinal0() >= 304 => est_tz,
            2044 if self.inner.ordinal0() < 72 || self.inner.ordinal0() >= 310 => est_tz,
            2045 if self.inner.ordinal0() < 70 || self.inner.ordinal0() >= 308 => est_tz,
            2046 if self.inner.ordinal0() < 69 || self.inner.ordinal0() >= 307 => est_tz,
            2047 if self.inner.ordinal0() < 68 || self.inner.ordinal0() >= 306 => est_tz,
            2048 if self.inner.ordinal0() < 67 || self.inner.ordinal0() >= 305 => est_tz,
            2049 => panic!("you need to update the DST table in src/units/utc_time.rs"),
            2050 => panic!("you need to update the DST table in src/units/utc_time.rs"),
            2051 => panic!("you need to update the DST table in src/units/utc_time.rs"),
            _ => edt_tz,
        };
        self.inner.with_timezone(&tz).time()
    }

    /// Finds the most recent Friday to the given date.
    ///
    /// On Friday, returns a week ago..
    pub fn last_friday(&self) -> Self {
        let offset = match self.inner.weekday() {
            chrono::Weekday::Sat => 1,
            chrono::Weekday::Sun => 2,
            chrono::Weekday::Mon => 3,
            chrono::Weekday::Tue => 4,
            chrono::Weekday::Wed => 5,
            chrono::Weekday::Thu => 6,
            chrono::Weekday::Fri => 7,
        };
        UtcTime {
            inner: self.inner - chrono::Duration::days(offset.into()),
        }
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

pub fn deserialize_datetime<'de, D>(deser: D) -> Result<UtcTime, D::Error>
where
    D: Deserializer<'de>,
{
    let s: &str = Deserialize::deserialize(deser)?;
    UtcTime::parse_coinbase(s).map_err(|_| {
        de::Error::invalid_value(de::Unexpected::Str(s), &"a datetime in %FT%T%z format")
    })
}

pub mod serde_ts_seconds {
    use super::*;

    use serde::{Serialize, Serializer};

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
