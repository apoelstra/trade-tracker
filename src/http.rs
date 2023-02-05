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

//! HTTP
//!
//! Utility functions to make HTTP requests easier
//!

use anyhow::Context;
use log::{info, warn};

/// Make a HTTP GET request, optionally with a LX API key, which will be
/// used if provided, and return a byte vector.
pub fn get_bytes(url: &str, api_key: Option<&str>) -> Result<Vec<u8>, anyhow::Error> {
    let mut req = minreq::get(url).with_timeout(10);
    if let Some(key) = api_key {
        req = req.with_header("Authorization", format!("JWT {}", key));
    }
    let resp = req
        .send()
        .with_context(|| format!("Request data from {}", url))?;

    info!(
        target: "lx_http_get",
        "{}: request to {} (api key {})",
        time::OffsetDateTime::now_utc().lazy_format(time::Format::Rfc3339),
        url,
        api_key.is_some(),
    );
    if let Ok(s) = resp.as_str() {
        info!(target: "lx_http_get", "{}", s);
    } else {
        warn!(target: "lx_http_get", "Non-UTF8 reply: {}", hex::encode(resp.as_bytes()));
    }
    Ok(resp.into_bytes())
}

/// Make a HTTP GET request and JSON-parse the result
pub fn get_json<D: serde::de::DeserializeOwned>(
    url: &str,
    api_key: Option<&str>,
) -> Result<D, anyhow::Error> {
    let bytes = get_bytes(url, api_key)?;
    Ok(serde_json::from_slice(&bytes).with_context(|| format!("parsing json from {}", url))?)
}

/// Make a HTTP GET request and JSON-parse the result
pub fn get_json_from_data_field<D: serde::de::DeserializeOwned>(
    url: &str,
    api_key: Option<&str>,
) -> Result<D, anyhow::Error> {
    #[derive(serde::Deserialize)]
    struct Response<U> {
        data: U,
    }
    let bytes = get_bytes(url, api_key)?;
    let json: Response<D> =
        serde_json::from_slice(&bytes).context("parsing json inside a .data field")?;
    Ok(json.data)
}
