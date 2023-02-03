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

//! Logging
//!
//! Log infrastructure. This uses the traits and macros from the log 0.4 crate.
//!
//! Will write INFO and more urgent messages to stdout; will also log everthing
//! DEBUG and up to a debug log (with more precise timestamp/severity information),
//! and also routes LX data feed messages to its own logs.
//!
//! Any errors related to writing are simply dropped and the messages won't be
//! logged. Errors related to initially opening the files should kill the program.
//!

use std::fs::File;
use std::io::Write;
use std::sync::Mutex;

/// Internal marker structure used to indicate that we only log to stdout
struct StdoutOnly;

impl log::Log for StdoutOnly {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Debug
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            println!("{}", record.args());
        }
    }

    fn flush(&self) {}
}

/// Actual logging structure
pub struct Logger {
    /// Most recent time that we logged something to stdout
    last_stdout_time: Mutex<time::OffsetDateTime>,
    /// Log for general output (excluding json-encoded data)
    ///
    /// Info and greater logs will also be put to stderr
    debug_log: Mutex<File>,
    /// Log to just dump websocket messages to
    msg_log: Mutex<File>,
    /// Latest Bitcoin price
    price: Mutex<String>,
}

impl Logger {
    /// Initialize a global logger
    pub fn init(debug_log: &str, msg_log: &str) -> Result<(), anyhow::Error> {
        log::set_max_level(log::LevelFilter::Debug);
        log::set_boxed_logger(Box::new(Logger {
            last_stdout_time: Mutex::new(time::OffsetDateTime::now_utc()),
            debug_log: Mutex::new(File::create(debug_log)?),
            msg_log: Mutex::new(File::create(msg_log)?),
            price: Mutex::new("".into()),
        }))
        .map_err(From::from)
    }

    /// Initialize a global logger (without extra files)
    pub fn init_stdout_only() -> Result<(), log::SetLoggerError> {
        log::set_max_level(log::LevelFilter::Debug);
        log::set_logger(&StdoutOnly)
    }
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Debug
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            if record.target() == "lx_datafeed" {
                // Messages targeted for the datafeed go to the datafeed log with no
                // additional processing (no timestamps etc)
                let _ = write!(self.msg_log.lock().unwrap(), "{}\n", record.args());
            } else if record.target() == "lx_btcprice" {
                // TODO maybe we should log the price somewhere as a personal price reference?
                *self.price.lock().unwrap() = format!("{}", record.args());
            } else {
                let now = time::OffsetDateTime::now_utc();

                // If it's more important than info, log to stdout
                if record.level() <= log::Level::Info {
                    let mut last_time_lock = self.last_stdout_time.lock().unwrap();
                    if now - *last_time_lock > time::Duration::minutes(10) {
                        println!("");
                    }
                    if now - *last_time_lock > time::Duration::seconds(30) {
                        println!("");
                    }
                    if now - *last_time_lock > time::Duration::seconds(1) {
                        println!("");
                        println!(
                            "{}",
                            crate::terminal::format_color(
                                format_args!(
                                    "Time: {}  BTC Price: {}",
                                    now.lazy_format("%F %T%z"),
                                    self.price.lock().unwrap(),
                                ),
                                255,
                                255,
                                180,
                            ),
                        );
                        *last_time_lock = now;
                    }
                    println!("{}", record.args());
                }
                // Regardless, log to debug log with more precise timestamp and log level
                let _ = write!(
                    self.debug_log.lock().unwrap(),
                    "{} [{}] {}\n",
                    now.lazy_format("%F %T%N%z"),
                    record.level(),
                    record.args(),
                );
            }
        }
    }

    fn flush(&self) {
        let _ = self.debug_log.lock().unwrap().flush();
        let _ = self.msg_log.lock().unwrap().flush();
    }
}
