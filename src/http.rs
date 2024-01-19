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
        req = req.with_header("Authorization", format!("JWT {key}"));
    }
    let resp = req
        .send()
        .with_context(|| format!("Request data from {url}"))?;

    info!(
        target: "lx_http_get",
        "{}: GET request to {} (api key {})",
        chrono::offset::Utc::now(),
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
    serde_json::from_slice(&bytes).with_context(|| format!("parsing json from {url}"))
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

pub fn post_to_prowl(data: &str) {
    let encoded = urlencoding::encode(&data);
    let body = format!(
        "apikey=71d4fa4bfa2a49c69ebb470594be2e079b05006d\
        &application=lx-trade-tracker\
        &event=filled-trade\
        &description={encoded}"
    );
    if let Err(e) = minreq::post("https://api.prowlapp.com/publicapi/add")
        .with_timeout(10)
        .with_header("Content-type", "application/x-www-form-urlencoded")
        .with_body(body.clone())
        .send()
    {
        warn!("Sending message to Prowl failed: {}", e);
        warn!("{}", body);
    }
}

/// Make a HTTP DELETE request to cancel all orders.
///
/// This is only used by the "cancel all orders" API endpoint which
/// takes an empty message, so we special case it.
pub fn lx_cancel_all_orders(api_key: &str) -> Result<(), anyhow::Error> {
    let url = "https://trade.ledgerx.com/api/orders";
    let req = minreq::delete(url)
        .with_header("Authorization", format!("JWT {api_key}"))
        .with_timeout(10);

    let resp = req
        .send()
        .with_context(|| format!("Request data from api/orders"))?;

    info!(
        target: "lx_http_get",
        "{}: DELETE request to {}",
        chrono::offset::Utc::now(),
        url,
    );
    if let Ok(s) = resp.as_str() {
        info!(target: "lx_http_get", "{}", s);
    } else {
        warn!(target: "lx_http_get", "Non-UTF8 reply: {}", hex::encode(resp.as_bytes()));
    }

    if resp.status_code == 200 {
        Ok(())
    } else {
        Err(anyhow::Error::msg(format!(
            "bad status code {} when cancelling orders",
            resp.status_code
        )))
    }
}
