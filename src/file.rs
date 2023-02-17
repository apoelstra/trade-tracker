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

//! Files
//!
//! Wrappers around [std::fs::File] which basically just provide better
//! logging/error messages.
//!

use anyhow::Context;
use log::info;
use std::{fmt, fs, io};

/// A text file
pub struct TextFile {
    name: String,
    inner: io::BufWriter<fs::File>,
}

impl TextFile {
    /// Writes some formatted data to the text file
    ///
    /// This method should not be used directly. Use the write! or writeln!
    /// macros from std instead.
    pub fn write_fmt(&mut self, f: fmt::Arguments<'_>) -> anyhow::Result<()> {
        io::Write::write_fmt(&mut self.inner, f)
            .with_context(|| format!("writing {f} to {}", self.name))
    }
}

/// Helper function to create a file with logging etc
///
/// Takes the filename by owner
pub fn create_text_file(name: String, reason: &str) -> anyhow::Result<TextFile> {
    if fs::metadata(&name).is_ok() {
        return Err(anyhow::Error::msg(format!(
            "File {name} already exists. Refusing to overwrite."
        )));
    }
    info!("Creating file {} {}.", name, reason);
    let file = fs::File::create(&name).with_context(|| format!("Creating file {name}"))?;
    Ok(TextFile {
        name,
        inner: io::BufWriter::new(file),
    })
}

/// Helper function to copy a file with reasonable safety checks and logging
pub fn copy_file(source: &str, dest: &str) -> anyhow::Result<()> {
    info!("Copying {} to {}", source, dest);
    if fs::metadata(dest).is_ok() {
        return Err(anyhow::Error::msg(format!(
            "File {dest} already exists. Refusing to overwrite."
        )));
    }
    fs::copy(source, dest).with_context(|| format!("Copying {source} to {dest}"))?;
    Ok(())
}
