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

//! "Connect" Command (Main Loop)
//!
//! When calling `trade-tracker connect` the tool will run indefinitely,
//! talking to LX and to other services. This is its main loop.
//!

use crate::http;
use crate::ledgerx::{self, datafeed, LedgerX};
use crate::units::{Underlying, UtcTime};
use anyhow::Context as _;
use log::{info, warn};
use std::sync::mpsc::channel;
use std::thread;

/// A message to the main loop
#[derive(Debug)]
pub enum Message {
    /// A new message from the LX websocket
    LedgerX(datafeed::Object),
    /// A request to open an order.
    OpenOrder(ledgerx::json::CreateOrder),
    /// A new book state has been retrieved from the contract lookup thread.
    BookState(ledgerx::json::BookStateMessage),
    /// An update from a price reference websocket
    PriceReference(crate::price::BitcoinPrice),
    /// "Heartbeat" to wakes up the main thread for housekeeping
    Heartbeat,
    /// If heartbeats come in too quickly they are accumulated into a "delayed
    /// heartbeat", as a rate-limiting mechanism. This is because heartbeats
    /// happen on a timer but are also triggered by orderbook actions.
    DelayedHeartbeat { delay_til: UtcTime, ready: bool },
    /// Something bad has happened elsewhere in the program and we need to
    /// cancel all open orders and shut down.
    EmergencyShutdown { msg: String },
}

// Helper function to attempt cancelling all orders, sending a text
// and panicking if this fails.
fn cancel_all_orders(api_key: &str) {
    if let Err(e) = http::lx_cancel_all_orders(api_key) {
        http::post_to_prowl(&format!("Tried to cancel all orders and failed: {e}"));
        panic!("Tried to cancel all orders and failed: {}", e);
    }
}

/// Starts the main loop and a couple utility threads. Returns a single `Sender`
/// for control messages.
///
/// # Panics
///
/// Will panic if anything goes wrong during startup.
pub fn main_loop(api_key: String) -> ! {
    let (tx, rx) = channel();

    // Before doing anything else, connect to a price reference and
    // get an initial price. Otherwise we can't initialize our trade
    // tracker etc.
    crate::coinbase::spawn_ticker_thread(tx.clone());
    let initial_price = match rx.recv() {
        Ok(Message::PriceReference(price)) => price,
        Ok(_) => unreachable!(),
        Err(e) => panic!("Failed to get initial price reference: {}", e),
    };
    info!(target: "lx_btcprice", "{}", initial_price);
    info!("BTC price: {}", initial_price);
    info!("Risk-free rate: 4% (assumed)");

    // LedgerX websocket thread
    let lx_tx = tx.clone();
    let lx_api_key = api_key.clone();
    thread::spawn(move || loop {
        let mut sock = loop {
            match tungstenite::client::connect(format!(
                "wss://api.ledgerx.com/ws?token={lx_api_key}",
            )) {
                Ok(sock) => break sock,
                Err(e) => {
                    warn!(
                        "Failed to connect to LedgerX. Will wait 30 seconds. Error: {}",
                        e
                    );
                }
            }
            thread::sleep(std::time::Duration::from_secs(30));
        };
        while let Ok(tungstenite::protocol::Message::Text(msg)) = sock.0.read_message() {
            info!(target: "lx_datafeed", "{}", msg);
            let obj: datafeed::Object = match serde_json::from_str(&msg) {
                Ok(obj) => obj,
                Err(e) => {
                    warn!("Received malformed message from LX: {}", msg);
                    warn!("JSON error: {}", e);
                    warn!("Disconnecting.");
                    break;
                }
            };
            lx_tx.send(Message::LedgerX(obj)).unwrap();
        }
    });

    // Clock thread
    let heartbeat_tx = tx.clone();
    thread::spawn(move || loop {
        thread::sleep(std::time::Duration::from_secs(30 * 60));
        heartbeat_tx.send(Message::Heartbeat).unwrap();
    });

    // Contract lookup thread
    let contract_tx = tx.clone();
    let contract_tx_api_key = api_key.clone();
    let (contract_thread_tx, contract_thread_rx) = channel();
    thread::spawn(move || {
        for contract_id in contract_thread_rx.iter() {
            let reply: ledgerx::json::BookStateMessage = http::get_json(
                &format!("https://trade.ledgerx.com/api/book-states/{contract_id}"),
                Some(&contract_tx_api_key),
            )
            .context("getting data from trading/contracts endpoint")
            .expect("retreiving and parsing json from book-states endpoint");
            contract_tx.send(Message::BookState(reply)).unwrap();
        }
    });

    // Setup
    let all_contracts: Vec<ledgerx::Contract> =
        http::get_json_from_data_field("https://api.ledgerx.com/trading/contracts", None)
            .context("looking up list of contracts")
            .expect("retrieving and parsing json from contract endpoint");

    let mut last_heartbeat_time = UtcTime::now() - chrono::Duration::hours(48);
    let mut heartbeat_price_ref = initial_price;
    let mut current_price = initial_price;

    let mut tracker = LedgerX::new(initial_price);
    for contr in all_contracts {
        // For expired or non-BTC options, fetch the full book. Otherwise
        // just record the contract's existence.
        if contr.active() && contr.underlying() == Underlying::Btc {
            contract_thread_tx
                .send(contr.id())
                .expect("book-states endpoint thread has not panicked");
        }
        tracker.add_contract(contr);
    }
    info!("Loaded contracts. Watching feed.");

    // Wait 30 seconds for LX to pile up some messages (in particular,
    // the balances) and for the contract lookup thread to finish all
    // initial lookups. Then push an initial heartbeat message, and
    // start the main loop to process everything in order.
    //
    // In theory we could be DoSsed here if LX or Coinbase or whatever
    // floods us with messages and causes our message queue to use a
    // ton of memory. But this happens only at startup so it's ok.
    thread::sleep(std::time::Duration::from_secs(30));
    tx.send(Message::Heartbeat).unwrap();

    // Main thread
    for msg in rx.iter() {
        let now = UtcTime::now();
        match msg {
            Message::LedgerX(obj) => {
                match obj {
                    datafeed::Object::Other => { /* ignore */ }
                    datafeed::Object::BookTop { .. } => { /* ignore */ }
                    datafeed::Object::Order(order) => {
                        match tracker.insert_order(order) {
                            ledgerx::OrderResponse::OursOk
                            | ledgerx::OrderResponse::OtherTracked
                            | ledgerx::OrderResponse::OtherUntracked => {
                                // Don't do anything
                            }
                            ledgerx::OrderResponse::OursFilled => {
                                info!("Triggering heartbeat since an order was filled.");
                                tx.send(Message::Heartbeat).unwrap();
                            }
                            ledgerx::OrderResponse::UnknownContract(order) => {
                                warn!("unknown contract ID {}", order.contract_id);
                                warn!("full order data {}", order);
                            }
                        }
                    }
                    datafeed::Object::AvailableBalances { usd, btc } => {
                        tracker.set_balances(usd, btc);
                    }
                    datafeed::Object::ContractAdded(contr) => {
                        contract_thread_tx
                            .send(contr.id())
                            .expect("book-states endpoint thread has not panicked");
                        tracker.add_contract(contr);
                    }
                    datafeed::Object::ContractRemoved(cid) => {
                        tracker.remove_contract(cid);
                    }
                    datafeed::Object::ChatMessage {
                        message,
                        initiator,
                        counterparty,
                        chat_id,
                    } => {
                        info!(
                            "New message (chat {}) between {} and {}: {}",
                            chat_id, initiator, counterparty, message
                        );
                    }
                }
            }
            Message::OpenOrder(order) => {
                if let Err(e) =
                    http::post_json("https://trade.ledgerx.com/api/orders", &api_key, &order)
                {
                    // A failed order open is just a warning; all our orders
                    // are asks at not-quite-reasonable prices and if we fail
                    // to open one it's maybe a lost profit opportunity but
                    // not an emergency.
                    warn!("Failed to open order {}: {}", order, e);
                }
            }
            Message::BookState(book_state) => {
                tracker.initialize_orderbooks(book_state, now);
            }
            Message::PriceReference(price) => {
                info!(target: "lx_btcprice", "{}", price);
                tracker.set_current_price(price);
                current_price = price;

                // If the price has drifted by 1% since the last heartbeat,
                // then force a heartbeat so that we reprice our orders.
                let ratio = (current_price.btc_price.to_approx_f64())
                    / (heartbeat_price_ref.btc_price.to_approx_f64());
                if ratio < 0.99 || ratio > 1.01 {
                    tx.send(Message::Heartbeat).unwrap();
                }
            }
            Message::Heartbeat | Message::DelayedHeartbeat { ready: true, .. } => {
                info!("[heartbeat {:?}]", msg);
                if now - last_heartbeat_time < chrono::Duration::minutes(1) {
                    // If a delayed heartbeat comes in too rapidly, we just drop
                    // it. If a normal heartbeat comes in too quickly, we drop it
                    // but queue a delayed heartbeat in 75 seconds.
                    if let Message::Heartbeat = msg {
                        let delay_til = now + chrono::Duration::seconds(75);
                        tx.send(Message::DelayedHeartbeat {
                            delay_til,
                            ready: false,
                        })
                        .unwrap();
                    }
                    continue;
                }
                last_heartbeat_time = now;

                heartbeat_price_ref = current_price;
                tracker.log_open_orders();
                tracker.log_interesting_contracts();
                cancel_all_orders(&api_key);
                // THIS LINE is currently the entirety of my trading algo. It
                // may push "open order" requests onto the message queue, which
                // we execute obediently.
                tracker.open_standing_orders(&tx);
            }
            Message::DelayedHeartbeat { delay_til, .. } => {
                thread::sleep(std::time::Duration::from_millis(250));
                tx.send(Message::DelayedHeartbeat {
                    delay_til,
                    ready: now > delay_til,
                })
                .unwrap();
            }
            Message::EmergencyShutdown { msg } => {
                http::post_to_prowl(&format!("Emergency shutdown: {msg}"));
                cancel_all_orders(&api_key);
                panic!("Emergency shutdown: {}", msg);
            }
        }
    }

    http::post_to_prowl("Main loop stopped receiving messages; shutting down.");
    cancel_all_orders(&api_key);
    panic!("Main loop stopped receiving messages.");
}
