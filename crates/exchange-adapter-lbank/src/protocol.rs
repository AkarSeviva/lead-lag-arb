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

/// Order price type - 源码确认 (文档4.8)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderPriceType {
    Limit = 0,    // 限价单
    Any = 1,      // 任价
    Market = 4,    // 市价单
}

impl OrderPriceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Limit => "0",
            Self::Any => "1",
            Self::Market => "4",
        }
    }
}

/// Offset flag - 源码确认 (文档4.7)
/// 注意: 实际API市价平仓用"5"(CloseAll)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetFlag {
    Open = 0,           // 开仓
    Close = 1,          // 平仓 (枚举值)
    ForceClose = 2,     // 强平
    CloseToday = 3,     // 平今
    CloseYesterday = 4,  // 平昨
    CloseAll = 5,       // 全平 (实际API用这个平仓!)
    CloseAppointOrder = 6,  // 止盈止损平仓
    CloseAppointTrade = 7,  // 触发交易平仓
    CloseMax = 8,       // 触发单平仓
}

impl OffsetFlag {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "0",
            Self::Close => "1",  // 平仓枚举值 (实际API用"5"平仓)
            Self::ForceClose => "2",
            Self::CloseToday => "3",
            Self::CloseYesterday => "4",
            Self::CloseAll => "5",  // 全平 (实际API平仓用这个)
            Self::CloseAppointOrder => "6",
            Self::CloseAppointTrade => "7",
            Self::CloseMax => "8",
        }
    }
}

/// Trigger order type - 源码确认 (文档4.9)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerOrderType {
    PositionStopProfitLoss = 1,  // 持仓止盈止损
    OrderStopProfitLoss = 2,     // 订单止盈止损
    Plan = 3,                   // 计划委托
}

impl TriggerOrderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PositionStopProfitLoss => "1",
            Self::OrderStopProfitLoss => "2",
            Self::Plan => "3",
        }
    }
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

/// Order Insert Request (下单) - 文档3.4/3.5/4.1确认
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderInsertRequest {
    #[serde(rename = "InstrumentID")]
    pub instrument_id: String,
    #[serde(rename = "ExchangeID")]
    pub exchange_id: String,
    pub direction: String,      // "0" = Long, "1" = Short
    #[serde(rename = "OffsetFlag")]
    pub offset_flag: String,    // "0" = Open, "5" = Close
    #[serde(rename = "OrderPriceType")]
    pub order_price_type: String, // "0" = Limit, "4" = Market
    #[serde(rename = "OrderType")]
    pub order_type: String,     // "0" = Limit, "1" = Market
    pub volume: f64,
    pub price: Option<f64>,
    #[serde(rename = "orderProportion")]
    pub order_proportion: String,
    #[serde(rename = "TradeUnitID", skip_serializing_if = "Option::is_none")]
    pub trade_unit_id: Option<String>, // 平仓必需 (文档3.5)
}

impl OrderInsertRequest {
    /// 市价开仓 - 文档3.4
    pub fn new_market_open(
        symbol: &str,
        direction: TradeDirection,
        volume: f64,
    ) -> Self {
        Self {
            instrument_id: symbol.to_string(),
            exchange_id: "Exchange".to_string(),
            direction: direction.as_str().to_string(),
            offset_flag: OffsetFlag::Open.as_str().to_string(),
            order_price_type: OrderPriceType::Market.as_str().to_string(),
            order_type: "1".to_string(),
            volume,
            price: None,
            order_proportion: "0.0000".to_string(),
            trade_unit_id: None,
        }
    }

    /// 市价平仓 - 文档3.5
    pub fn new_market_close(
        symbol: &str,
        direction: TradeDirection,
        volume: f64,
        trade_unit_id: &str,
    ) -> Self {
        Self {
            instrument_id: symbol.to_string(),
            exchange_id: "Exchange".to_string(),
            direction: direction.as_str().to_string(),
            offset_flag: OffsetFlag::CloseAll.as_str().to_string(),
            order_price_type: OrderPriceType::Market.as_str().to_string(),
            order_type: "1".to_string(),
            volume,
            price: None,
            order_proportion: "0.0000".to_string(),
            trade_unit_id: Some(trade_unit_id.to_string()),
        }
    }

    /// 限价开仓 - 文档4.1
    pub fn new_limit_open(
        symbol: &str,
        direction: TradeDirection,
        volume: f64,
        price: f64,
    ) -> Self {
        Self {
            instrument_id: symbol.to_string(),
            exchange_id: "Exchange".to_string(),
            direction: direction.as_str().to_string(),
            offset_flag: OffsetFlag::Open.as_str().to_string(),
            order_price_type: OrderPriceType::Limit.as_str().to_string(),
            order_type: "0".to_string(),
            volume,
            price: Some(price),
            order_proportion: "0.0000".to_string(),
            trade_unit_id: None,
        }
    }

    /// 限价平仓
    pub fn new_limit_close(
        symbol: &str,
        direction: TradeDirection,
        volume: f64,
        price: f64,
        trade_unit_id: &str,
    ) -> Self {
        Self {
            instrument_id: symbol.to_string(),
            exchange_id: "Exchange".to_string(),
            direction: direction.as_str().to_string(),
            offset_flag: OffsetFlag::CloseAll.as_str().to_string(),
            order_price_type: OrderPriceType::Limit.as_str().to_string(),
            order_type: "0".to_string(),
            volume,
            price: Some(price),
            order_proportion: "0.0000".to_string(),
            trade_unit_id: Some(trade_unit_id.to_string()),
        }
    }
}

/// 止盈止损下单请求 - 文档4.1
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseOrderInsertRequest {
    #[serde(rename = "InstrumentID")]
    pub instrument_id: String,
    #[serde(rename = "ExchangeID")]
    pub exchange_id: String,
    pub direction: String,
    #[serde(rename = "OffsetFlag")]
    pub offset_flag: String,
    #[serde(rename = "OrderPriceType")]
    pub order_price_type: String,
    #[serde(rename = "OrderType")]
    pub order_type: String,
    pub price: f64,
    pub volume: f64,
    #[serde(rename = "CloseSLTriggerPrice")]
    pub close_sl_trigger_price: String,
    #[serde(rename = "CloseSLTriggerPriceType")]
    pub close_sl_trigger_price_type: String,
    #[serde(rename = "CloseTPTriggerPrice")]
    pub close_tp_trigger_price: String,
    #[serde(rename = "CloseTPTriggerPriceType")]
    pub close_tp_trigger_price_type: String,
    #[serde(rename = "TriggerOrderType")]
    pub trigger_order_type: String,
}

impl CloseOrderInsertRequest {
    /// 创建带止盈止损的限价单
    pub fn new(
        symbol: &str,
        direction: TradeDirection,
        volume: f64,
        price: f64,
        sl_trigger_price: &str,
        tp_trigger_price: &str,
        trigger_order_type: TriggerOrderType,
    ) -> Self {
        Self {
            instrument_id: symbol.to_string(),
            exchange_id: "Exchange".to_string(),
            direction: direction.as_str().to_string(),
            offset_flag: OffsetFlag::Open.as_str().to_string(),
            order_price_type: OrderPriceType::Limit.as_str().to_string(),
            order_type: "0".to_string(),
            price,
            volume,
            close_sl_trigger_price: sl_trigger_price.to_string(),
            close_sl_trigger_price_type: "0".to_string(),
            close_tp_trigger_price: tp_trigger_price.to_string(),
            close_tp_trigger_price_type: "0".to_string(),
            trigger_order_type: trigger_order_type.as_str().to_string(),
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

/// Position Query Response - 文档5.1确认
#[derive(Debug, Deserialize)]
pub struct PositionResponse {
    #[serde(rename = "positionID")]
    pub position_id: String,          // 持仓ID
    #[serde(rename = "TradeUnitID")]
    pub trade_unit_id: String,       // 交易单元ID (平仓必需)
    #[serde(rename = "instrumentId")]
    pub instrument_id: String,
    #[serde(rename = "exchangeId")]
    pub exchange_id: String,
    #[serde(rename = "posiDirection")]
    pub posi_direction: String,     // "0"=多, "1"=空
    pub position: String,            // 持仓数量
    #[serde(rename = "openPrice")]
    pub open_price: String,          // 开仓均价
    #[serde(rename = "positionCost")]
    pub position_cost: String,      // 持仓成本
    #[serde(rename = "useMargin")]
    pub use_margin: String,         // 已用保证金
    #[serde(rename = "leverage")]
    pub leverage: Option<i32>,      // 杠杆
    #[serde(rename = "estimateLiquidationPrice")]
    pub estimate_liquidation_price: Option<String>,  // 预估强平价
    pub direction: Option<String>,
    #[serde(rename = "positionVolume")]
    pub position_volume: Option<String>,
    pub position_profit: Option<String>,
    #[serde(rename = "useVolume")]
    pub use_volume: Option<String>,
    pub frozen_volume: Option<String>,
    #[serde(rename = "canCloseVolume")]
    pub can_close_volume: Option<String>,
}

/// Order Query Response - 文档5.3
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
    #[serde(rename = "closeProfit")]
    pub close_profit: Option<String>,  // 平仓盈亏
    pub fee: Option<String>,            // 手续费
    #[serde(rename = "volumeCancled")]
    pub volume_cancled: Option<String>, // 已撤数量
}

/// History Order Response (历史委托) - 文档5.3
#[derive(Debug, Deserialize)]
pub struct HistoryOrderResponse {
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
    #[serde(rename = "tradePrice")]
    pub trade_price: Option<String>,
    #[serde(rename = "openPrice")]
    pub open_price: Option<String>,
    #[serde(rename = "closeProfit")]
    pub close_profit: Option<String>,
    pub fee: Option<String>,
    #[serde(rename = "insertTime")]
    pub insert_time: Option<i64>,
    #[serde(rename = "volumeCancled")]
    pub volume_cancled: Option<String>,
}

/// Trade Response (历史成交) - 文档5.2
#[derive(Debug, Deserialize)]
pub struct TradeResponse {
    #[serde(rename = "tradeID")]
    pub trade_id: String,
    #[serde(rename = "orderSysID")]
    pub order_sys_id: String,
    #[serde(rename = "positionID")]
    pub position_id: String,
    pub direction: String,         // "0"=开仓, "1"=平仓
    pub offset_flag: String,       // "0"=开仓, "5"=平仓
    pub price: String,
    pub volume: String,
    pub fee: String,
    #[serde(rename = "closeProfit")]
    pub close_profit: Option<String>,  // 平仓盈亏
    #[serde(rename = "tradeTime")]
    pub trade_time: i64,
    #[serde(rename = "openPrice")]
    pub open_price: Option<String>,
    #[serde(rename = "useMargin")]
    pub use_margin: Option<String>,
    #[serde(rename = "instrumentId")]
    pub instrument_id: Option<String>,
}

/// Trigger Order Response (触发单) - 文档4.3
#[derive(Debug, Deserialize)]
pub struct TriggerOrderResponse {
    #[serde(rename = "orderSysID")]
    pub order_sys_id: String,
    #[serde(rename = "instrumentId")]
    pub instrument_id: String,
    pub direction: Option<String>,
    pub offset_flag: Option<String>,
    pub order_type: Option<String>,
    pub volume: Option<String>,
    pub price: Option<String>,
    #[serde(rename = "triggerPrice")]
    pub trigger_price: Option<String>,
    #[serde(rename = "triggerOrderType")]
    pub trigger_order_type: Option<String>,
    pub status: Option<String>,      // "0"=待触发, "1"=已触发, "2"=已撤销
    #[serde(rename = "triggeredPrice")]
    pub triggered_price: Option<String>,
    #[serde(rename = "insertTime")]
    pub insert_time: Option<i64>,
    #[serde(rename = "triggeredTime")]
    pub triggered_time: Option<i64>,
}

/// Cancel Order Response (撤单响应) - 文档4.4
#[derive(Debug, Deserialize)]
pub struct CancelOrderResponse {
    #[serde(rename = "orderSysID")]
    pub order_sys_id: String,
    #[serde(rename = "orderStatus")]
    pub order_status: String,  // "6"=已撤销
    #[serde(rename = "volumeCancled")]
    pub volume_cancled: String,
    #[serde(rename = "volumeRemain")]
    pub volume_remain: String,
}

/// Set Leverage Request - 文档3.3
#[derive(Debug, Serialize)]
pub struct SetLeverageRequest {
    #[serde(rename = "instrumentID")]
    pub instrument_id: String,
    #[serde(rename = "longLeverage")]
    pub long_leverage: i32,
    #[serde(rename = "shortLeverage")]
    pub short_leverage: i32,
}

/// Aggregate Info (完整版) - 文档3.2
#[derive(Debug, Deserialize)]
pub struct AggregateInfoResponse {
    #[serde(rename = "assetBalance")]
    pub asset_balance: Option<AssetBalance>,
    #[serde(rename = "markedPrice")]
    pub marked_price: String,
    #[serde(rename = "longLeverage")]
    pub long_leverage: Option<i32>,
    #[serde(rename = "shortLeverage")]
    pub short_leverage: Option<i32>,
    #[serde(rename = "longMaxLeverage")]
    pub long_max_leverage: Option<i32>,
    #[serde(rename = "shortMaxLeverage")]
    pub short_max_leverage: Option<i32>,
    #[serde(rename = "fundingRate")]
    pub funding_rate: Option<String>,
    #[serde(rename = "isMarketAcount")]
    pub is_market_account: Option<i32>,
    #[serde(rename = "isOnlyClose")]
    pub is_only_close: Option<i32>,
    pub state: Option<i32>,
}

/// Asset Balance - 文档3.2
#[derive(Debug, Deserialize)]
pub struct AssetBalance {
    #[serde(rename = "assets")]
    pub assets: String,
    pub available: String,
    pub balance: String,
    #[serde(rename = "realAvailable")]
    pub real_available: String,
    #[serde(rename = "frozenMargin")]
    pub frozen_margin: String,
    #[serde(rename = "totalCloseProfit")]
    pub total_close_profit: String,
    #[serde(rename = "crossMargin")]
    pub cross_margin: Option<String>,
    #[serde(rename = "marginAble")]
    pub margin_able: Option<String>,
}

/// Account Info Response - 文档3.2
#[derive(Debug, Deserialize)]
pub struct AccountInfoResponse {
    #[serde(rename = "parentId")]
    pub parent_id: Option<i64>,
    #[serde(rename = "uid")]
    pub uid: String,
    #[serde(rename = "list")]
    pub list: Option<Vec<AssetBalance>>,
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
