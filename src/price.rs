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

//! Price Data
//!
//! Functionality to keep track of historic price data
//!

use anyhow::Context;
use log::info;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt, fs,
    io::{self, BufRead},
    path::{Path, PathBuf},
    str::FromStr,
};

/// Price
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Deserialize, Serialize)]
pub struct BitcoinPrice {
    /// Timestamp that the price was recorded at
    #[serde(with = "time::serde::timestamp")]
    pub timestamp: time::OffsetDateTime,
    /// Price in USD, to 12 decimal places
    pub btc_price: Decimal,
}

impl BitcoinPrice {
    /// Turn a `Decimal` into a price at the current timestamp
    pub fn from_current(num: Decimal) -> BitcoinPrice {
        BitcoinPrice {
            timestamp: time::OffsetDateTime::now_utc(),
            btc_price: num,
        }
    }

    /// Parse a price from CSV data
    pub fn from_csv(data: &str) -> Result<BitcoinPrice, anyhow::Error> {
        let mut data = data.split(',');

        let date = match data.next() {
            Some(date) => time::OffsetDateTime::from_unix_timestamp(i64::from_str(&date)?),
            None => return Err(anyhow::Error::msg("CSV line had no timestamp")),
        };
        let price = match data.next() {
            Some(price) => rust_decimal::Decimal::from_str(&price)?,
            None => return Err(anyhow::Error::msg("CSV line had no price")),
        };
        // These checks aren't really necessary but are useful as sanity checks
        if data.next().is_none() {
            return Err(anyhow::Error::msg("CSV line had no volume"));
        }
        if data.next().is_some() {
            return Err(anyhow::Error::msg("CSV line had extra data"));
        }

        Ok(BitcoinPrice {
            timestamp: date,
            btc_price: price,
        })
    }
}

impl fmt::Display for BitcoinPrice {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:.2} @ {}", self.btc_price, self.timestamp)
    }
}

/// Historic price data
#[derive(Default)]
pub struct Historic {
    data: BTreeMap<time::OffsetDateTime, BitcoinPrice>,
}

impl Historic {
    /// Records a price
    pub fn record(&mut self, price: BitcoinPrice) {
        self.data.insert(price.timestamp, price);
    }

    /// Returns the most recent price as of a given time
    pub fn price_at(&self, time: time::OffsetDateTime) -> BitcoinPrice {
        log::debug!(
            "look up price at {} ... full range {} to {}, short range ends at {}",
            time,
            self.data.keys().next().unwrap(),
            self.data.keys().rev().next().unwrap(),
            *self.data.range(..time).rev().next().unwrap().0,
        );
        *self.data.range(..time).rev().next().unwrap().1
    }

    /// Number of price entries recorded
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Reads a bunch of price records from CSV data, keeping only the most
    /// recent entry as of each half-hour
    pub fn read_csv<R: io::Read>(&mut self, data: R) -> Result<(), anyhow::Error> {
        let mut last_half_hour = 0;
        let mut last_price = None;
        for (lineno, entry) in io::BufReader::new(data).lines().enumerate() {
            let entry = entry.with_context(|| format!("reading line {}", lineno))?;
            let price = BitcoinPrice::from_csv(&entry)
                .with_context(|| format!("decoding price \"{}\" at {}", entry, lineno))?;

            let half_hour =
                12 * price.timestamp.time().hour() + price.timestamp.time().minute() / 5;
            if last_half_hour != half_hour {
                last_half_hour = half_hour;
                self.record(price);
            }

            if lineno % 1_000_000 == 0 && lineno > 0 {
                info!(
                    "Read {}M lines, recorded {} datapoints. Last trade {}",
                    lineno / 1_000_000,
                    self.len(),
                    price
                );
            }

            last_price = Some(price);
        }

        // Also record most recent price, since the user is likely using
        // this in real-time to price an option
        if let Some(price) = last_price {
            self.record(price);
        }
        Ok(())
    }

    /// Reads all price records from cache
    pub fn read_json<P: AsRef<Path>>(datadir: P) -> Result<Self, anyhow::Error> {
        Historic::read_json_from(datadir, "")
    }

    /// Reads all price records from cache, starting from files
    /// whose name is >= the given `min_date``
    pub fn read_json_from<P: AsRef<Path>>(
        datadir: P,
        min_date: &str,
    ) -> Result<Self, anyhow::Error> {
        let mut new = Historic::default();
        for file in fs::read_dir(datadir).context("opening pricedata directory")? {
            let filepath = file.context("getting file path")?.path();
            let filename = filepath.to_string_lossy();

            if filename.rsplit('/').next() >= Some(min_date) {
                let input =
                    io::BufReader::new(fs::File::open(filepath).context("opening json file")?);
                let prices: Vec<BitcoinPrice> =
                    serde_json::from_reader(input).context("decoding json")?;
                for price in prices {
                    new.record(price);
                }
            }
        }
        Ok(new)
    }

    /// Writes out all price records
    pub fn write_out(&self, datadir: &PathBuf) -> Result<(), anyhow::Error> {
        let mut datadir = datadir.clone();
        let mut last_year_mo = 0;
        let mut mo_entries = vec![];
        fs::create_dir_all(&datadir).context("creating pricedata directory")?;
        for entry in self.data.values() {
            let year_mo =
                100 * entry.timestamp.date().year() + entry.timestamp.date().month() as i32;
            if last_year_mo != year_mo {
                if last_year_mo > 0 {
                    datadir.push(format!("{:06}.json", last_year_mo));
                    serde_json::to_writer(
                        io::BufWriter::new(
                            fs::File::create(&datadir).context("creating json file")?,
                        ),
                        &mo_entries,
                    )
                    .context("writing json")?;
                    datadir.pop();
                }
                mo_entries.clear();
                last_year_mo = year_mo;
            }
            mo_entries.push(entry);
        }

        // Record most recent month
        if last_year_mo > 0 {
            datadir.push(format!("{:06}.json", last_year_mo));
            serde_json::to_writer(
                io::BufWriter::new(fs::File::create(&datadir).context("creating json file")?),
                &mo_entries,
            )
            .context("writing json")?;
            datadir.pop();
        }

        Ok(())
    }
}
