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

//! Command-line Argument Parsing
//!

use crate::{option, units::Price};
use std::{env, ffi::OsString, fmt, path::PathBuf, process, str::FromStr};

/// If no price feed URL is provided, use BitcoinCharts' CSV data.
///
/// From <https://bitcoincharts.com/about/markets-api/>:
///     * Delayed 15 minutes. No guarantees about accuracy. Do not trade on this (lol)
///     * Do not query more than once every 15 minutes!
static DEFAULT_PRICE_FEED_URL: &str =
    "http://api.bitcoincharts.com/v1/trades.csv?symbol=bitstampUSD";

/// Structure representing parsing of command-line options
pub enum Command {
    /// Read a CSV file downloaded from Bitcoincharts, storing all its price data (at
    /// a ten-minute resolution rather than all of it)
    InitializePriceData { csv: PathBuf },
    /// Ping bitcoincharts in real time to get recent price data
    UpdatePriceData { url: String },
    /// Return the latest stored price. Mainly useful as a test.
    LatestPrice {},
    /// Print a list of potential orders for a given option near a given volatility, at various
    /// prices
    Price {
        option: option::Option,
        /// Specific volatility, if provided
        volatility: Option<f64>,
    },
    /// Print a list of potential orders for a given option near a given price
    Iv {
        option: option::Option,
        /// Specific price, if provided
        price: Option<Price>,
    },
    /// Connect to LedgerX API and monitor activity in real-time
    Connect {
        api_key: String,
        config_file: Option<PathBuf>,
    },
    /// Connect to LedgerX API and download complete transaction history, for a given year if
    /// supplied. Outputs in CSV.
    History {
        api_key: String,
        config_file: PathBuf,
    },
    /// Connect to LedgerX API and attempt to recreate its tax CSV file for a given year
    TaxHistory {
        api_key: String,
        config_file: PathBuf,
    },
}

/// Master list of supported commands
#[allow(clippy::type_complexity)]
static COMMANDS: &[(&str, &str, fn(&str, env::ArgsOs) -> Command)] = &[
    (
        "initialize-price-data",
        "<csv filename>",
        initialize_price_data,
    ),
    (
        "update-price-data",
        "[URL (default: bitcoincharts)]",
        update_price_data,
    ),
    ("latest-price", "", latest_price),
    ("price", "<option> [-v <volatility>]", price),
    ("iv", "<option> [-p <price>]", iv),
    ("connect", "<api key>", connect),
    ("history", "<api key> <config file>", history),
    ("tax-history", "<api key> <config file>", tax_history),
];

/// Parse the "initialize-price-data" command
fn initialize_price_data(invocation: &str, mut args: env::ArgsOs) -> Command {
    match args.next() {
        Some(x) => Command::InitializePriceData { csv: x.into() },
        None => usage(invocation),
    }
}

/// Parse the "update-price-data" command
fn update_price_data(invocation: &str, mut args: env::ArgsOs) -> Command {
    match args.next().map(OsString::into_string) {
        Some(Ok(url)) => Command::UpdatePriceData { url },
        Some(Err(url)) => {
            eprintln!("Unable to parse non-UTF8 URL {}", url.to_string_lossy());
            usage(invocation);
        }
        None => Command::UpdatePriceData {
            url: DEFAULT_PRICE_FEED_URL.into(),
        },
    }
}

/// Parse the "latest-price" command
fn latest_price(_: &str, _: env::ArgsOs) -> Command {
    Command::LatestPrice {}
}

/// Parse the "price" command
fn price(invocation: &str, mut args: env::ArgsOs) -> Command {
    let option = parse_os_string_required(args.next(), "option ID", invocation);
    let vol = parse_os_string(args.next(), "-v flag", invocation).map(|dashv: DashOpt| {
        if dashv.0 == b'v' {
            parse_os_string_required(args.next(), "volatility", invocation)
        } else {
            eprintln!("Unrecognized flag -{}", char::from(dashv.0));
            usage(invocation);
        }
    });
    Command::Price {
        option,
        volatility: vol,
    }
}

/// Parse the "iv" command
fn iv(invocation: &str, mut args: env::ArgsOs) -> Command {
    let option = parse_os_string_required(args.next(), "option ID", invocation);
    let price = parse_os_string(args.next(), "-v flag", invocation).map(|dashv: DashOpt| {
        if dashv.0 == b'p' {
            parse_os_string_required(args.next(), "price", invocation)
        } else {
            eprintln!("Unrecognized flag -{}", char::from(dashv.0));
            usage(invocation);
        }
    });
    Command::Iv { option, price }
}

/// Parse the "connect" command
fn connect(invocation: &str, mut args: env::ArgsOs) -> Command {
    Command::Connect {
        api_key: parse_os_string_required(args.next(), "API key", invocation),
        config_file: args.next().map(From::from),
    }
}

/// Parse the "history" command
fn history(invocation: &str, mut args: env::ArgsOs) -> Command {
    Command::History {
        api_key: parse_os_string_required(args.next(), "API key", invocation),
        config_file: match args.next() {
            Some(x) => x.into(),
            None => {
                eprintln!("Missing configuration filename");
                usage(invocation)
            }
        },
    }
}

/// Parse the "tax-history" command
fn tax_history(invocation: &str, mut args: env::ArgsOs) -> Command {
    Command::TaxHistory {
        api_key: parse_os_string_required(args.next(), "API key", invocation),
        config_file: match args.next() {
            Some(x) => x.into(),
            None => {
                eprintln!("Missing configuration filename");
                usage(invocation)
            }
        },
    }
}

impl Command {
    /// Parse the command-line arguments
    ///
    /// If this fails, it will output a usage text to stderr and then
    /// terminate the process. It should not be called once the program
    /// is "really" running.
    pub fn from_args() -> Self {
        let mut args = env::args_os();
        // Obtain name we were called with
        let invocation = match args.next().map(OsString::into_string) {
            Some(Ok(inv)) => inv,
            Some(Err(_)) => "non-utf8-command-name".into(),
            None => panic!("called with no arguments, not even a command-line name"),
        };

        // Obtain primary command
        match args.next().map(OsString::into_string) {
            Some(Ok(inv)) => {
                for (cmd, _, f) in COMMANDS {
                    if inv == *cmd {
                        return f(&invocation, args);
                    }
                }
                eprintln!("Unknown command {inv}");
                usage(&invocation);
            }
            Some(Err(inv)) => {
                eprintln!("Unknown non-UTF8 command {}", inv.to_string_lossy());
                usage(&invocation);
            }
            None => usage(&invocation),
        }
    }

    /// The name to prefix log files with
    pub fn log_name(&self) -> &'static str {
        match *self {
            Command::InitializePriceData { .. } => "init-price-data",
            Command::UpdatePriceData { .. } => "update-price-data",
            Command::LatestPrice { .. } => "latest-price",
            Command::Price { .. } => "price",
            Command::Iv { .. } => "iv",
            Command::Connect { .. } => "connect",
            Command::History { .. } => "history",
            Command::TaxHistory { .. } => "tax-history",
        }
    }
}

fn usage(invocation: &str) -> ! {
    eprintln!();
    eprintln!("Usage:");
    for (cmd, help, _) in COMMANDS {
        eprintln!("    {invocation} {cmd} {help}");
    }
    process::exit(1)
}

struct DashOpt(u8);
impl FromStr for DashOpt {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        if s.len() != 2 || !s.is_ascii() || s.as_bytes()[0] != b'-' {
            return Err(format!("malformed flag option {s}"));
        }
        Ok(DashOpt(s.as_bytes()[1]))
    }
}

/// Helper function to parse some string data from an OsString
fn parse_os_string<T>(iter_res: Option<OsString>, desc: &str, invocation: &str) -> Option<T>
where
    T: FromStr,
    <T as FromStr>::Err: fmt::Display,
{
    iter_res.map(|oss| match oss.into_string() {
        Ok(s) => match T::from_str(&s) {
            Ok(obj) => obj,
            Err(e) => {
                eprintln!("Unable to parse {desc}: {e}");
                usage(invocation);
            }
        },
        Err(s) => {
            eprintln!("Unable to non-UTF8 {desc} {}", s.to_string_lossy());
            usage(invocation);
        }
    })
}

/// Helper function to parse some string data from an OsString
fn parse_os_string_required<T>(iter_res: Option<OsString>, desc: &str, invocation: &str) -> T
where
    T: FromStr,
    <T as FromStr>::Err: fmt::Display,
{
    match parse_os_string(iter_res, desc, invocation) {
        Some(x) => x,
        None => {
            eprintln!("Missing required {desc}.");
            usage(invocation);
        }
    }
}
