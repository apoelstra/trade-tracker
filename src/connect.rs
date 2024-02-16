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
use crate::price::BitcoinPrice;
use crate::units::{Price, Quantity, Underlying, UtcTime};
use anyhow::Context as _;
use log::{info, warn};
use std::sync::mpsc::{channel, Sender};
use std::thread;

// Because of DST we can't be super precise about when the market is actually
// open, without importing a timezone database and doing a bunch of crap. So
// we just swag that it's open from 1300 to 2100.
fn market_is_open(now: UtcTime) -> bool {
    let nyt = now.new_york_time();
    let open = chrono::NaiveTime::from_hms_opt(9, 30, 0).unwrap();
    let close = chrono::NaiveTime::from_hms_opt(16, 0, 0).unwrap();
    nyt >= open && nyt < close
}

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
    PriceReference(BitcoinPrice),
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

/// Helper function to construct an initial LX tracker with all current contracts
fn recreate_tracker(
    initial_price: BitcoinPrice,
    contract_thread_tx: &Sender<ledgerx::ContractId>,
) -> LedgerX {
    let all_contracts: Vec<ledgerx::Contract> =
        http::get_json_from_data_field("https://api.ledgerx.com/trading/contracts", None)
            .context("looking up list of contracts")
            .expect("retrieving and parsing json from contract endpoint");
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
    tracker
}

/// Helper function to attempt cancelling all orders, sending a text
/// and panicking if this fails.
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
pub fn main_loop(api_key: String, history: Option<ledgerx::history::History>) -> ! {
    let (tx, rx) = channel();
    let initial_time = UtcTime::now();

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
                        "Failed to connect to LedgerX. Will wait 5 minutes. Error: {}",
                        e
                    );
                }
            }
            thread::sleep(std::time::Duration::from_secs(300));
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
        thread::sleep(std::time::Duration::from_secs(120 * 60));
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

    // Get history to determine past BTC transactions. We attempt to "undo" any
    // BTC sales by selling puts at a discount, and we use this history to
    // determine how best to do that.
    //
    // This is not rational in terms of total account value (which optimally
    // would require a memoryless strategy), but is rational if our goal is to
    // minimize the amount if time we spend holding fewer bitcoins than we
    // started with.
    //
    let mut net_btc = bitcoin::SignedAmount::ZERO;
    let mut net_usd = Price::ZERO;
    let mut recent_net_btc = bitcoin::SignedAmount::ZERO;
    let mut recent_net_usd = Price::ZERO;
    let mut min_average_price = Price::MAX;
    if let Some(hist) = history {
        for (time, event) in hist.events() {
            use crate::ledgerx::history::Event;
            use crate::units::BudgetAsset;

            let delta_btc;
            let delta_usd;
            match event {
                Event::Trade {
                    asset, price, size, ..
                } => {
                    delta_usd = -*price * *size;
                    if BudgetAsset::from(*asset) == BudgetAsset::Btc {
                        delta_btc = size.btc_equivalent();
                    } else {
                        delta_btc = bitcoin::SignedAmount::ZERO;
                    }
                }
                Event::Assignment {
                    option,
                    underlying,
                    size,
                    ..
                } => {
                    if *underlying == Underlying::Btc {
                        match option.pc {
                            crate::option::PutCall::Call => {
                                delta_usd = option.strike * *size;
                                delta_btc = size.btc_equivalent() * -1;
                            }
                            crate::option::PutCall::Put => {
                                delta_usd = -option.strike * *size;
                                delta_btc = size.btc_equivalent();
                            }
                        }
                    } else {
                        delta_usd = Price::ZERO;
                        delta_btc = bitcoin::SignedAmount::ZERO;
                    }
                }
                _ => {
                    delta_usd = Price::ZERO;
                    delta_btc = bitcoin::SignedAmount::ZERO;
                }
            }
            net_usd += delta_usd;
            net_btc += delta_btc;
            if initial_time - time < chrono::Duration::days(500) {
                recent_net_usd += delta_usd;
                recent_net_btc += delta_btc;
            }

            if net_btc != bitcoin::SignedAmount::ZERO {
                let average = net_usd / Quantity::from(net_btc).abs();
                if average < min_average_price {
                    info!(
                        "At {} sold {} for {} (average price {})",
                        time, net_btc, net_usd, average
                    );
                    min_average_price = average;
                }
            }
        }
    }

    if net_btc != bitcoin::SignedAmount::ZERO {
        info!(
            "History: sold a total of {} for {} (average price {})",
            net_btc,
            net_usd,
            net_usd / Quantity::from(net_btc).abs()
        );
    }
    if recent_net_btc != bitcoin::SignedAmount::ZERO {
        info!(
            "History: in last 500 days sold {} for {} (average price {})",
            recent_net_btc,
            recent_net_usd,
            recent_net_usd / Quantity::from(recent_net_btc).abs()
        );
    }

    // ...and output

    // Setup
    let mut last_heartbeat_time = initial_time - chrono::Duration::hours(48);
    let mut last_market_open = market_is_open(initial_time);
    let mut heartbeat_price_ref = initial_price;
    let mut current_price = initial_price;

    let mut tracker = recreate_tracker(initial_price, &contract_thread_tx);

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
        if market_is_open(now) && !last_market_open {
            tracker = recreate_tracker(current_price, &contract_thread_tx);
        }
        last_market_open = market_is_open(now);

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
                tracker.initialize_orderbooks(book_state, now, &tx);
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

                // Update balances to make sure we're in sync with LX
                let balances: ledgerx::json::GetBalancesResponse = http::get_json_from_data_field(
                    "https://api.ledgerx.com/funds/balances",
                    Some(&api_key),
                )
                .context("looking up current balances")
                .expect("retrieving and parsing json from contract endpoint");
                info!(
                    "Balance details (available/position locked/settlement locked/deliverable locked): {}/{}/{}/{}, {}/{}/{}/{}",
                    balances.usd.available_balance,
                    balances.usd.position_locked,
                    balances.usd.settlement_locked,
                    balances.usd.deliverable_locked,
                    balances.btc.available_balance,
                    balances.btc.position_locked,
                    balances.btc.settlement_locked,
                    balances.btc.deliverable_locked,
                );
                tracker.set_balances(
                    balances.usd.available_balance,
                    balances.btc.available_balance,
                );

                if market_is_open(now) {
                    tracker.log_open_orders();
                    tracker.log_interesting_contracts(&tx);
                    cancel_all_orders(&api_key);
                    // THIS LINE is currently the entirety of my trading algo. It
                    // may push "open order" requests onto the message queue, which
                    // we execute obediently.
                    tracker.open_standing_orders(&tx);
                } else {
                    info!("Market closed.");
                    tracker.clear_orderbooks();
                }
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
