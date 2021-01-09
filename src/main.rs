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

//! Trade Tracker
//!
//! Personal-use barely-maintained tool for keeping track of trades
//!

mod price;
mod trade;

use anyhow::Context;
use clap::Clap;
use std::{fs, path::PathBuf};

use price::Historic;

#[derive(Clap)]
enum Command {
    InitializePriceData {
        #[clap(name = "csv_file", parse(from_os_str))]
        csv: PathBuf,
    },
    UpdatePriceData {
        #[clap(
            name = "url",
            default_value = "http://api.bitcoincharts.com/v1/trades.csv?symbol=bitstampUSD"
        )]
        url: String,
    },
    LatestPrice {},
}

fn main() -> Result<(), anyhow::Error> {
    let mut data_path = dirs::data_dir().context("getting XDG config directory")?;
    data_path.push("trade-tracker");

    match Command::parse() {
        Command::InitializePriceData { csv } => {
            let mut history = Historic::default();
            let csv_name = csv.to_string_lossy();

            let input =
                fs::File::open(&csv).with_context(|| format!("opening price data {}", csv_name))?;
            history
                .read_csv(input)
                .with_context(|| format!("decoding CSV data from {}", csv_name))?;

            data_path.push("pricedata");
            history.write_out(&mut data_path).with_context(|| {
                format!(
                    "writing out price history to {}",
                    data_path.to_string_lossy()
                )
            })?;
            data_path.pop();
        }
        Command::UpdatePriceData { url } => {
            let mut history = Historic::default();
            let data = reqwest::blocking::get(&url)
                .with_context(|| format!("getting data from {}", url))?;
            history
                .read_csv(data)
                .with_context(|| format!("decoding CSV data from {}", url))?;

            data_path.push("pricedata");
            history
                .write_out(&mut data_path)
                .context("writing out price history")?;
            data_path.pop();
        }
        Command::LatestPrice {} => {
            data_path.push("pricedata");
            let history = Historic::read_json(&data_path).context("reading price history")?;
            data_path.pop();

            let now = time::OffsetDateTime::now_utc();
            println!("{}", history.price_at(now));
        }
    }

    Ok(())
}
