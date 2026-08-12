//! Lbank API Protocol Definitions
//!
//! Based on reversed engineered API from browser HAR analysis.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Lbank API Response wrapper
#[derive(Debug, Clone, Deserialize)]
pub struct LbankResponse<T> {
    pub code: i32,
    pub msg: Option<String>,
    pub data: Option<T>,
}

impl<T> LbankResponse<T> {
    pub fn is_success(&self) -> bool {
        self.code == 200
    }

    pub fn into_result(self) -> Result<T, LbankError> {
        if self.is_success() {
            self.data.ok_or(LbankError::EmptyResponse)
        } else {
            Err(LbankError::ApiError {
                code: self.code,
                message: self.msg.unwrap_or_default(),
            })
        }
    }
}

/// API Error types
#[derive(Debug, thiserror::Error)]
pub enum LbankError {
    #[error("API returned error: code={code}, message={message}")]
    ApiError { code: i32, message: String },

    #[error("Empty response data")]
    EmptyResponse,

    #[error("Authentication failed: {0}")]
    AuthError(String),

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Invalid response format: {0}")]
    InvalidResponse(String),
}

/// Product group types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductGroup {
    SwapU,  // USDT perpetual
    Swap,   // Coin-based perpetual (maybe)
}

impl ProductGroup {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SwapU => "SwapU",
            Self::Swap => "Swap",
        }
    }
}

/// Trade direction (Lbank format)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeDirection {
    Long = 0,  // Buy / 做多
    Short = 1, // Sell / 做空
}

impl TradeDirection {
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "0" => Some(Self::Long),
            "1" => Some(Self::Short),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Long => "0",
            Self::Short => "1",
        }
    }
}

/// Offset flag (open/close)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetFlag {
    Open = 0,
    Close = 1,
}

impl OffsetFlag {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "0",
            Self::Close => "1",
        }
    }
}

/// Order price type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderPriceType {
    Limit = 1,    // 限价单
    Market = 2,   // 市价单
    Trigger = 4, // 计划委托
}

impl OrderPriceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Limit => "1",
            Self::Market => "2",
            Self::Trigger => "4",
        }
    }
}

// ============================================================================
// REST API Request/Response Types
// ============================================================================

/// WebSocket Token Request
#[derive(Debug, Serialize)]
pub struct WsTokenRequest {}

// WebSocket Token Response
#[derive(Debug, Deserialize)]
pub struct WsTokenResponse {
    #[serde(rename = "code")]
    pub code: i32,
    #[serde(rename = "data")]
    pub data: Option<String>,
}

/// Order Insert Request (下单)
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderInsertRequest {
    pub instrument_id: String,
    pub exchange_id: String,
    pub direction: String,      // "0" = Long, "1" = Short
    pub offset_flag: String,    // "0" = Open, "1" = Close
    pub order_price_type: String, // "1" = Limit, "2" = Market
    pub order_type: String,     // "1" = Limit, "2" = Market
    pub volume: f64,
    pub price: Option<f64>,
    #[serde(rename = "orderProportion")]
    pub order_proportion: String,
}

impl OrderInsertRequest {
    pub fn new_market_order(
        symbol: &str,
        direction: TradeDirection,
        offset: OffsetFlag,
        volume: f64,
    ) -> Self {
        Self {
            instrument_id: symbol.to_string(),
            exchange_id: "Exchange".to_string(),
            direction: direction.as_str().to_string(),
            offset_flag: offset.as_str().to_string(),
            order_price_type: OrderPriceType::Market.as_str().to_string(),
            order_type: "2".to_string(), // Market
            volume,
            price: None,
            order_proportion: "0.0000".to_string(),
        }
    }

    pub fn new_limit_order(
        symbol: &str,
        direction: TradeDirection,
        offset: OffsetFlag,
        volume: f64,
        price: f64,
    ) -> Self {
        Self {
            instrument_id: symbol.to_string(),
            exchange_id: "Exchange".to_string(),
            direction: direction.as_str().to_string(),
            offset_flag: offset.as_str().to_string(),
            order_price_type: OrderPriceType::Limit.as_str().to_string(),
            order_type: "1".to_string(), // Limit
            volume,
            price: Some(price),
            order_proportion: "0.0000".to_string(),
        }
    }
}

/// Order Insert Response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderInsertResponse {
    pub offset_flag: String,
    pub order_type: String,
    pub fee: String,
    pub frozen_fee: String,
    pub user_id: String,
    pub exchange_id: String,
    pub account_id: String,
    pub order_sys_id: String,
    pub volume_remain: String,
    pub price: String,
}

/// Position Query Request
#[derive(Debug, Serialize)]
pub struct PositionQueryRequest {
    pub product_group: String,
    pub valid: i32,
    pub page_index: i32,
    pub page_size: i32,
}

/// Position Query Response
#[derive(Debug, Deserialize)]
pub struct PositionResponse {
    #[serde(rename = "instrumentId")]
    pub instrument_id: String,
    #[serde(rename = "exchangeId")]
    pub exchange_id: String,
    pub direction: Option<String>,
    pub position_volume: Option<String>,
    pub position_cost: Option<String>,
    pub position_profit: Option<String>,
    #[serde(rename = "useVolume")]
    pub use_volume: Option<String>,
    pub frozen_volume: Option<String>,
    pub can_close_volume: Option<String>,
}

/// Order Query Response
#[derive(Debug, Deserialize)]
pub struct OrderResponse {
    #[serde(rename = "orderSysID")]
    pub order_sys_id: String,
    #[serde(rename = "instrumentId")]
    pub instrument_id: String,
    pub direction: Option<String>,
    pub offset_flag: Option<String>,
    pub order_type: Option<String>,
    pub order_status: Option<String>,
    pub volume: Option<String>,
    pub price: Option<String>,
    #[serde(rename = "tradedVolume")]
    pub traded_volume: Option<String>,
    #[serde(rename = "avgPrice")]
    pub avg_price: Option<String>,
    #[serde(rename = "orderInsertTime")]
    pub order_insert_time: Option<i64>,
}

/// Market Order Book (深度)
#[derive(Debug, Deserialize)]
pub struct MarketOrderBook {
    pub table: String,
    pub data: Vec<MarketOrderItem>,
}

#[derive(Debug, Deserialize)]
pub struct MarketOrderItem {
    pub orders: String,
    pub price: String,
    pub volume: String,
    #[serde(rename = "Direction")]
    pub direction: String, // "0" = Ask, "1" = Bid
}

/// 24hr Ticker
#[derive(Debug, Deserialize)]
pub struct Ticker24hr {
    #[serde(rename = "instrumentId")]
    pub instrument_id: String,
    pub last_price: String,
    pub open_price: String,
    pub high_price: String,
    pub low_price: String,
    pub volume: String,
    pub quote_volume: String,
    #[serde(rename = "change24h")]
    pub change_24h: Option<String>,
    #[serde(rename = "changePercent24h")]
    pub change_percent_24h: Option<String>,
}

/// Instrument Info (合约品种)
#[derive(Debug, Deserialize)]
pub struct InstrumentInfo {
    #[serde(rename = "instrumentId")]
    pub instrument_id: String,
    #[serde(rename = "exchangeId")]
    pub exchange_id: String,
    #[serde(rename = "productGroup")]
    pub product_group: String,
    #[serde(rename = "baseCurrency")]
    pub base_currency: String,
    #[serde(rename = "quoteCurrency")]
    pub quote_currency: String,
    #[serde(rename = "tickSize")]
    pub tick_size: String,
    #[serde(rename = "lotSize")]
    pub lot_size: String,
    #[serde(rename = "minOrderVolume")]
    pub min_order_volume: String,
    #[serde(rename = "maxOrderVolume")]
    pub max_order_volume: String,
    #[serde(rename = "maxLeverage")]
    pub max_leverage: String,
    #[serde(rename = "state")]
    pub state: i32,
}

/// Fee Rate Response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeRateResponse {
    #[serde(rename = "makerOpenFeeRate")]
    pub maker_open_fee_rate: String,
    #[serde(rename = "makerCloseFeeRate")]
    pub maker_close_fee_rate: String,
    #[serde(rename = "takerOpenFeeRate")]
    pub taker_open_fee_rate: String,
    #[serde(rename = "takerCloseFeeRate")]
    pub taker_close_fee_rate: String,
}

/// Aggregate Info (持仓限制等)
#[derive(Debug, Deserialize)]
pub struct AggregateInfo {
    #[serde(rename = "isMarketAcount")]
    pub is_market_account: i32,
    #[serde(rename = "longMaxVolume")]
    pub long_max_volume: String,
    #[serde(rename = "shortMaxVolume")]
    pub short_max_volume: String,
    #[serde(rename = "longMaxLeverage")]
    pub long_max_leverage: i32,
    #[serde(rename = "shortMaxLeverage")]
    pub short_max_leverage: i32,
    #[serde(rename = "markedPrice")]
    pub marked_price: String,
    #[serde(rename = "isOnlyClose")]
    pub is_only_close: i32,
    pub state: i32,
}

/// Margin Rate Response
#[derive(Debug, Deserialize)]
pub struct MarginRateResponse {
    #[serde(rename = "maintenanceMarginRate")]
    pub maintenance_margin_rate: String,
    #[serde(rename = "riskLimit")]
    pub risk_limit: String,
    #[serde(rename = "initialMarginRate")]
    pub initial_margin_rate: String,
    #[serde(rename = "maxLeverage")]
    pub max_leverage: String,
}

// ============================================================================
// WebSocket Message Types
// ============================================================================

/// WebSocket subscription request
#[derive(Debug, Serialize)]
pub struct WsSubscribeRequest {
    pub type_: String,
    pub topic: String,
    #[serde(rename = "reqNo")]
    req_no: Option<String>,
}

/// WebSocket push message
#[derive(Debug, Deserialize)]
pub struct WsPushMessage {
    pub table: Option<String>,
    pub topic: Option<String>,
    pub data: Option<serde_json::Value>,
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
}

/// WebSocket order book update
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WsOrderBookUpdate {
    pub instrument_id: String,
    pub exchange_id: String,
    #[serde(rename = "lastPrice")]
    pub last_price: String,
    #[serde(rename = "lastVolume")]
    pub last_volume: String,
    pub bids: Option<Vec<PriceLevel>>,
    pub asks: Option<Vec<PriceLevel>>,
    #[serde(rename = "timestamp")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct PriceLevel {
    pub price: String,
    pub volume: String,
}

/// Order update event from WebSocket
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WsOrderUpdate {
    #[serde(rename = "orderSysID")]
    pub order_sys_id: String,
    #[serde(rename = "instrumentId")]
    pub instrument_id: String,
    pub direction: Option<String>,
    pub offset_flag: Option<String>,
    pub order_status: Option<String>,
    pub volume: Option<String>,
    pub traded_volume: Option<String>,
    pub price: Option<String>,
    #[serde(rename = "orderInsertTime")]
    pub order_insert_time: Option<i64>,
    pub fee: Option<String>,
}

// ============================================================================
// Normalized Market Data Types (Internal)
// ============================================================================

/// Normalized price level for order book
#[derive(Debug, Clone)]
pub struct NormalizedPriceLevel {
    pub price: Decimal,
    pub volume: Decimal,
    pub orders: u32, // Number of orders at this level
}

/// Normalized order book snapshot
#[derive(Debug, Clone)]
pub struct NormalizedOrderBook {
    pub symbol: String,
    pub timestamp: i64,
    pub last_price: Decimal,
    pub bids: Vec<NormalizedPriceLevel>, // Sorted descending
    pub asks: Vec<NormalizedPriceLevel>, // Sorted ascending
    pub seq_id: Option<u64>,
}

impl NormalizedOrderBook {
    pub fn best_bid(&self) -> Option<Decimal> {
        self.bids.first().map(|l| l.price)
    }

    pub fn best_ask(&self) -> Option<Decimal> {
        self.asks.first().map(|l| l.price)
    }

    pub fn spread(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some(ask - bid),
            _ => None,
        }
    }

    pub fn mid_price(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some((bid + ask) / Decimal::from(2)),
            _ => None,
        }
    }
}

/// Convert Lbank market order to normalized format
impl From<&MarketOrderItem> for NormalizedPriceLevel {
    fn from(item: &MarketOrderItem) -> Self {
        Self {
            price: item.price.parse().unwrap_or_default(),
            volume: item.volume.parse().unwrap_or_default(),
            orders: item.orders.parse().unwrap_or(1),
        }
    }
}
