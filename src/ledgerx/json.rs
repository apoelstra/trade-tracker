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

//! Json stuff
//!
//! Some utility methods for parsing json from the LX API
//!

use crate::units::{Price, Quantity, Underlying, UtcTime};
use serde::{de, Deserialize, Deserializer, Serialize};
use std::convert::TryFrom;
use std::fmt;

fn deserialize_datetime<'de, D>(deser: D) -> Result<Option<UtcTime>, D::Error>
where
    D: Deserializer<'de>,
{
    use chrono::DateTime;

    let s: Option<&str> = Deserialize::deserialize(deser)?;
    match s {
        Some(s) => Ok(Some(
            DateTime::parse_from_str(s, "%F %T%z")
                .map_err(|_| {
                    de::Error::invalid_value(
                        de::Unexpected::Str(s),
                        &"a datetime in %F %T%z format",
                    )
                })?
                .into(),
        )),
        None => Ok(None),
    }
}

fn deserialize_timestamp<'de, D>(deser: D) -> Result<UtcTime, D::Error>
where
    D: Deserializer<'de>,
{
    let s: i64 = Deserialize::deserialize(deser)?;
    UtcTime::from_unix_nanos_i64(s).map_err(|_| {
        de::Error::invalid_value(
            de::Unexpected::Signed(s),
            &"a timestamp in range for the datetime type",
        )
    })
}

/// The type of the derivative
#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum DerivativeType {
    DayAheadSwap,
    FutureContract,
    OptionsContract,
}

/// Value of the "type" field
#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Type {
    Put,
    Call,
}

/// From <https://docs.ledgerx.com/reference/action-report-status-codes>
#[derive(Deserialize, Debug)]
#[serde(try_from = "usize")]
pub enum StatusType {
    Inserted,
    CrossTrade,
    NotFilled,
    Cancelled,
    CancelledAndReplaced,
    MessageAcknowledged,
    ContractNotFound,
    OrderIdNotFound,
    OrderIdInvalid,
    OrderRejected,
    InsufficientCollateral,
    ContractExpired,
    PriceThresholdExceeded,
    ContractNotActive,
    InvalidBlockSize,
}

impl TryFrom<usize> for StatusType {
    type Error = String;
    fn try_from(x: usize) -> Result<Self, Self::Error> {
        match x {
            200 => Ok(StatusType::Inserted),
            201 => Ok(StatusType::CrossTrade),
            202 => Ok(StatusType::NotFilled),
            203 => Ok(StatusType::Cancelled),
            204 => Ok(StatusType::CancelledAndReplaced),
            300 => Ok(StatusType::MessageAcknowledged),
            600 => Ok(StatusType::ContractNotFound),
            601 => Ok(StatusType::OrderIdNotFound),
            602 => Ok(StatusType::OrderIdInvalid),
            607 => Ok(StatusType::OrderRejected),
            609 => Ok(StatusType::InsufficientCollateral),
            610 => Ok(StatusType::ContractExpired),
            613 => Ok(StatusType::PriceThresholdExceeded),
            614 => Ok(StatusType::ContractNotActive),
            616 => Ok(StatusType::InvalidBlockSize),
            _ => Err(format!("unknown status type {x}")),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(try_from = "usize")]
pub enum StatusReason {
    NoReason,
    FullFill,
    CancelledByExchange,
}

impl TryFrom<usize> for StatusReason {
    type Error = String;
    fn try_from(x: usize) -> Result<Self, Self::Error> {
        match x {
            0 => Ok(StatusReason::NoReason),
            52 => Ok(StatusReason::FullFill),
            53 => Ok(StatusReason::CancelledByExchange),
            _ => Err(format!("unknown status reason {x}")),
        }
    }
}

/// Copy of the "contract" as returned from the /contracts endpoint
#[derive(Deserialize, Debug)]
pub struct Contract {
    pub id: usize,
    pub active: bool,
    pub underlying_asset: Underlying,
    #[serde(default, deserialize_with = "deserialize_datetime")]
    pub date_exercise: Option<UtcTime>,
    #[serde(default, deserialize_with = "deserialize_datetime")]
    pub date_expires: Option<UtcTime>,
    #[serde(default, deserialize_with = "deserialize_datetime")]
    pub date_live: Option<UtcTime>,
    pub is_call: Option<bool>,
    pub is_next_day: Option<bool>,
    pub is_ecp_only: Option<bool>,
    pub derivative_type: DerivativeType,
    #[serde(default, deserialize_with = "crate::units::deserialize_cents_opt")]
    pub strike_price: Option<Price>,
    pub min_increment: usize,
    #[serde(default)]
    pub open_interest: Option<usize>,
    pub multiplier: usize,
    pub label: String,
    #[serde(rename = "type")]
    pub ty: Option<Type>,
    pub name: Option<String>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct DataFeedMeta {
    index: usize,
    max_index: usize,
    #[serde(deserialize_with = "hex::serde::deserialize")]
    manifest_id: [u8; 16],
}

#[derive(Deserialize, Debug)]
pub struct UsdBalanceDetails {
    #[serde(deserialize_with = "crate::units::deserialize_cents")]
    pub available_balance: Price,
    #[serde(deserialize_with = "crate::units::deserialize_cents")]
    pub position_locked: Price,
    #[serde(deserialize_with = "crate::units::deserialize_cents")]
    pub settlement_locked: Price,
    #[serde(deserialize_with = "crate::units::deserialize_cents")]
    pub deliverable_locked: Price,
}

#[derive(Deserialize, Debug)]
pub struct BtcBalanceDetails {
    #[serde(with = "bitcoin::amount::serde::as_sat")]
    pub available_balance: bitcoin::Amount,
    #[serde(with = "bitcoin::amount::serde::as_sat")]
    pub position_locked: bitcoin::Amount,
    #[serde(with = "bitcoin::amount::serde::as_sat")]
    pub settlement_locked: bitcoin::Amount,
    #[serde(with = "bitcoin::amount::serde::as_sat")]
    pub deliverable_locked: bitcoin::Amount,
}

#[derive(Deserialize, Debug)]
pub struct GetBalancesResponse {
    #[serde(rename = "USD")]
    pub usd: UsdBalanceDetails,
    #[serde(rename = "BTC")]
    pub btc: BtcBalanceDetails,
}

#[derive(Deserialize, Debug)]
pub struct Balances {
    #[serde(rename = "USD", deserialize_with = "crate::units::deserialize_cents")]
    pub usd: Price,
    #[serde(rename = "BTC", with = "bitcoin::amount::serde::as_sat")]
    pub btc: bitcoin::Amount,
}

#[derive(Deserialize, Debug)]
pub struct AllBalances {
    pub available_balances: Balances,
    pub deliverable_locked_balances: Balances,
    pub position_locked_balances: Balances,
}

#[derive(Deserialize, Debug)]
pub struct ChatCounterparty {
    pub chat_username: String,
    pub is_online: bool,
}

#[derive(Deserialize, Debug)]
pub struct MessageInner {
    pub message: String,
    pub counterparty: ChatCounterparty,
    pub initiator: ChatCounterparty,
}

#[derive(Deserialize, Debug)]
pub struct MessageData {
    pub message: MessageInner,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum DataFeedObject {
    ActionReport {
        contract_id: super::ContractId,
        open_interest: usize,
        #[serde(deserialize_with = "hex::serde::deserialize")]
        mid: [u8; 16],
        /// Will always be `customer_limit_order`
        order_type: String,
        #[serde(deserialize_with = "crate::units::deserialize_cents")]
        price: Price,
        size: i64,
        #[serde(deserialize_with = "crate::units::deserialize_cents")]
        inserted_price: Price,
        inserted_size: i64,
        #[serde(deserialize_with = "crate::units::deserialize_cents")]
        filled_price: Price,
        filled_size: i64,
        #[serde(deserialize_with = "crate::units::deserialize_cents")]
        original_price: Price,
        original_size: i64,
        is_ask: bool,
        /// Whether the order auto-cancels at 4PM
        is_volatile: bool,
        #[serde(default)]
        cid: Option<usize>,
        #[serde(default)]
        mpid: Option<usize>,
        status_type: StatusType,
        #[serde(default)]
        status_reason: Option<StatusReason>,
        /// "The current clock for the entire contract"
        clock: u64,
        #[serde(deserialize_with = "deserialize_timestamp")]
        timestamp: UtcTime,
        #[serde(deserialize_with = "deserialize_timestamp")]
        inserted_time: UtcTime,
        #[serde(deserialize_with = "deserialize_timestamp")]
        updated_time: UtcTime,
        #[serde(default)]
        _meta: Option<DataFeedMeta>,
    },
    UnauthSuccess {},
    AuthSuccess {},
    ContractAdded {
        data: crate::ledgerx::Contract,
    },
    ContractRemoved {
        data: crate::ledgerx::Contract,
    },
    TradeBusted {},
    Meta {},
    OpenPositionsUpdate {},
    CollateralBalanceUpdate {
        collateral: AllBalances,
    },
    ExposureReports {},
    ContactAdded {},
    ContactRemoved {},
    ContactConnected {},
    ContactDisconnected {},
    ConversationNewMessage {
        data: MessageData,
        conversation_id: usize,
    },
    StateManifest {},
    BookTop {
        contract_id: super::ContractId,
        #[serde(default, deserialize_with = "crate::units::deserialize_cents")]
        ask: Price,
        ask_size: i64,
        #[serde(default, deserialize_with = "crate::units::deserialize_cents")]
        bid: Price,
        bid_size: i64,
        /// "The current clock for the entire contract"
        clock: u64,
    },
    /// Lol AFAICT this one is just undocumented
    Heartbeat {},
}

#[derive(Deserialize, Debug)]
pub struct BookStateMessage {
    pub data: BookStateData,
}
#[derive(Deserialize, Debug)]
pub struct BookStateData {
    pub contract_id: super::ContractId,
    pub book_states: Vec<BookState>,
}
#[derive(Deserialize, Debug)]
pub struct BookState {
    pub clock: u64,
    pub contract_id: super::ContractId,
    #[serde(deserialize_with = "hex::serde::deserialize")]
    pub mid: [u8; 16],
    pub is_ask: bool,
    #[serde(default, deserialize_with = "crate::units::deserialize_cents")]
    pub price: Price,
    pub size: i64,
}

/// A "create order" API call
#[derive(PartialEq, Eq, Serialize, Debug)]
pub struct CreateOrder {
    /// Order type; must always be "limit"
    order_type: &'static str,
    /// ID of the contract to trade
    contract_id: super::ContractId,
    /// Whether this is a bid or ask
    is_ask: bool,
    /// Swap purpose; either "bf_hedge", "non_bf_hedge" or "undisclosed".
    ///
    /// Best to always set this to "undisclosed". See CFTC guidance on what this means at
    /// https://www.cftc.gov/IndustryOversight/MarketSurveillance/SpeculativeLimits/index.htm
    ///
    /// From that page, it appears that "bona fide hedgers" may be exempt from some
    /// trading limits the CFTC has in place to reduce speculation. But all the limits,
    /// as of January 2024, are for real assets, not Bitcoin. So making a claim here
    /// either way seems like it'd just encourage scrutiny without gaining you anything.
    swap_purpose: &'static str,
    /// Size of the order, in contracts
    size: i64,
    /// Price of the order, in cents
    price: i64,
}

impl CreateOrder {
    /// Constructs a new bid with the given price, rounded down to the nearest dollar.
    ///
    /// # Panics
    ///
    /// Panics if the contract is not an option (futures I never intend to trade, and
    /// BTC I don't currently intend to trade automatically and am uncertain how to
    /// specify the quantity in the JSON API), or if the quantity is inconsistent
    /// with the contract (meaning: it is neither Zero nor a number of contracts).
    pub fn new_bid(contract: &super::Contract, qty: Quantity, price: Price) -> Self {
        let price = price.round_down();
        Self::new_internal(contract, qty, price, false)
    }

    /// Constructs a new ask with the given price, rounded up to the nearest dollar.
    ///
    /// # Panics
    ///
    /// Panics if the contract is not an option (futures I never intend to trade, and
    /// BTC I don't currently intend to trade automatically and am uncertain how to
    /// specify the quantity in the JSON API), or if the quantity is inconsistent
    /// with the contract (meaning: it is neither Zero nor a number of contracts).
    pub fn new_ask(contract: &super::Contract, qty: Quantity, price: Price) -> Self {
        let price = price.round_up();
        Self::new_internal(contract, qty, price, true)
    }

    fn new_internal(contract: &super::Contract, qty: Quantity, price: Price, is_ask: bool) -> Self {
        if !matches!(contract.ty(), super::contract::Type::Option { .. }) {
            panic!("Tried to create bid for non-option contract {}", contract);
        }
        let size = match qty {
            Quantity::Contracts(n) => n,
            _ => panic!(
                "Tried to create option bid with invalid quantity type {}",
                qty
            ),
        };
        CreateOrder {
            order_type: "limit",
            contract_id: contract.id(),
            is_ask,
            swap_purpose: "undisclosed",
            size,
            price: price.to_cents(),
        }
    }
}

impl fmt::Display for CreateOrder {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "[ {}: {} @ {} ]",
            self.contract_id, self.size, self.price,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io, io::BufRead};

    #[test]
    fn fixed_option_order() {
        let contract: crate::ledgerx::Contract = serde_json::from_str(
            "{\"active\":true,\"collateral_asset\":\"USD\",\"date_exercise\":\"2023-12-29 22:00:00+0000\",\"date_expires\":\"2023-12-29 21:00:00+0000\",\"date_live\":\"2023-01-12 05:00:00+0000\",\"derivative_type\":\"options_contract\",\"id\":22256323,\"is_call\":false,\"is_ecp_only\":false,\"is_next_day\":false,\"label\":\"ETH-29DEC2023-5000-Put\",\"min_increment\":10,\"multiplier\":10,\"name\":null,\"open_interest\":null,\"strike_price\":500000,\"type\":\"put\",\"underlying_asset\":\"ETH\"}",
        ).expect("parsing contract");

        CreateOrder::new_bid(&contract, Quantity::Contracts(100), Price::ONE_HUNDRED);
        assert_eq!(
            CreateOrder::new_ask(&contract, Quantity::Contracts(100), Price::ONE_HUNDRED),
            CreateOrder {
                order_type: "limit",
                contract_id: contract.id(),
                is_ask: true,
                swap_purpose: "undisclosed",
                size: 100,
                price: 10000,
            },
        );
    }

    #[test]
    fn fixed_vector_contracts() {
        let vecs = vec![
            "{\"active\":true,\"collateral_asset\":\"USD\",\"date_exercise\":\"2023-12-29 22:00:00+0000\",\"date_expires\":\"2023-12-29 21:00:00+0000\",\"date_live\":\"2023-01-12 05:00:00+0000\",\"derivative_type\":\"options_contract\",\"id\":22256323,\"is_call\":false,\"is_ecp_only\":false,\"is_next_day\":false,\"label\":\"ETH-29DEC2023-5000-Put\",\"min_increment\":10,\"multiplier\":10,\"name\":null,\"open_interest\":null,\"strike_price\":500000,\"type\":\"put\",\"underlying_asset\":\"ETH\"}",
        ];

        for v in vecs {
            let _des: Contract = serde_json::from_str(v).expect("successful parse");
        }
    }

    #[test]
    fn fixed_vector_datafeed() {
        let fh = fs::File::open("src/ledgerx/test-datafeed.json").unwrap();
        let fh = io::BufReader::new(fh);
        for json in fh.lines() {
            let json = json.unwrap();
            serde_json::from_str::<DataFeedObject>(&json).expect("successful parse");
        }
    }
}
