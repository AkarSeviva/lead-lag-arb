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
    #[serde(rename = "instrumentID")]
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
    pub volume: String,
    #[serde(rename = "Price")]
    pub price: Option<String>,
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
        volume: String,
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
        volume: String,
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
        volume: String,
        price: String,
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
        volume: String,
        price: String,
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
    #[serde(rename = "instrumentID")]
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
///
/// 所有字段都使用 `#[serde(default)]` 提高容错：
/// - Lbank 在不同订单类型（市价/限价/带TPSL）返回的字段不完全一致
/// - 部分字段（如 `minVolume`、`tips`）只在特定场景出现
/// - 缺失字段时 String 默认为 "" / bool 默认为 false
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct OrderInsertResponse {
    #[serde(rename = "offsetFlag")]
    pub offset_flag: String,
    #[serde(rename = "orderType")]
    pub order_type: String,
    #[serde(rename = "reserveMode")]
    pub reserve_mode: String,
    pub fee: String,
    #[serde(rename = "frozenFee")]
    pub frozen_fee: String,
    #[serde(rename = "userID")]
    pub user_id: String,
    #[serde(rename = "masterAccountID")]
    pub master_account_id: String,
    #[serde(rename = "exchangeID")]
    pub exchange_id: String,
    #[serde(rename = "accountID")]
    pub account_id: String,
    #[serde(rename = "orderSysID")]
    pub order_sys_id: String,
    #[serde(rename = "volumeRemain")]
    pub volume_remain: String,
    pub price: String,
    #[serde(rename = "businessValue")]
    pub business_value: String,
    #[serde(rename = "frozenMargin")]
    pub frozen_margin: String,
    #[serde(rename = "instrumentID")]
    pub instrument_id: String,
    #[serde(rename = "posiDirection")]
    pub posi_direction: String,
    #[serde(rename = "volumeMode")]
    pub volume_mode: String,
    pub volume: String,
    #[serde(rename = "insertTime")]
    pub insert_time: String,
    #[serde(rename = "copyMemberID")]
    pub copy_member_id: String,
    pub position: String,
    #[serde(rename = "tradePrice")]
    pub trade_price: String,
    pub leverage: String,
    #[serde(rename = "businessResult")]
    pub business_result: String,
    #[serde(rename = "originalTips")]
    pub original_tips: String,
    #[serde(rename = "availableUse")]
    pub available_use: String,
    #[serde(rename = "orderStatus")]
    pub order_status: String,
    #[serde(rename = "openPrice")]
    pub open_price: String,
    #[serde(rename = "frozenMoney")]
    pub frozen_money: String,
    pub remark: String,
    #[serde(rename = "reserveUse")]
    pub reserve_use: String,
    #[serde(rename = "sessionNo")]
    pub session_no: String,
    #[serde(rename = "isCrossMargin")]
    pub is_cross_margin: String,
    #[serde(rename = "closeProfit")]
    pub close_profit: String,
    #[serde(rename = "businessNo")]
    pub business_no: String,
    #[serde(rename = "relatedOrderSysID")]
    pub related_order_sys_id: String,
    #[serde(rename = "positionID")]
    pub position_id: String,
    #[serde(rename = "mockResp")]
    pub mock_resp: bool,
    #[serde(rename = "deriveSource")]
    pub derive_source: String,
    #[serde(rename = "copyOrderID")]
    pub copy_order_id: String,
    pub currency: String,
    pub turnover: String,
    #[serde(rename = "frontNo")]
    pub front_no: String,
    pub direction: String,
    #[serde(rename = "orderPriceType")]
    pub order_price_type: String,
    #[serde(rename = "volumeCancled")]
    pub volume_cancled: String,
    #[serde(rename = "updateTime")]
    pub update_time: String,
    #[serde(rename = "localID")]
    pub local_id: String,
    #[serde(rename = "volumeTraded")]
    pub volume_traded: String,
    /// Lbank 部分接口返回部分不返回，部分接口返回 Number，部分返回 String
    #[serde(rename = "minVolume")]
    pub min_volume: Option<serde_json::Value>,
    #[serde(rename = "tips")]
    pub tips: Option<serde_json::Value>,
    pub appid: String,
    #[serde(rename = "tradeUnitID")]
    pub trade_unit_id: String,
    #[serde(rename = "businessType")]
    pub business_type: String,
    #[serde(rename = "memberID")]
    pub member_id: String,
    #[serde(rename = "timeCondition")]
    pub time_condition: String,
    #[serde(rename = "copyProfit")]
    pub copy_profit: String,
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
#[derive(Debug, Deserialize, Default, Clone)]
pub struct PositionResponse {
    #[serde(rename = "PositionID")]
    pub position_id: Option<String>,          // 持仓ID
    #[serde(rename = "TradeUnitID")]
    pub trade_unit_id: Option<String>,        // 交易单元ID (平仓必需)
    #[serde(rename = "InstrumentID")]
    pub instrument_id: Option<String>,
    #[serde(rename = "ProductGroup")]
    pub product_group: Option<String>,
    #[serde(rename = "positionType")]
    pub position_type: Option<i32>,
    #[serde(rename = "ExchangeID")]
    pub exchange_id: Option<String>,
    #[serde(rename = "BusinessNo")]
    pub business_no: Option<String>,
    #[serde(rename = "PosiDirection")]
    pub posi_direction: Option<String>,        // "0"=多, "1"=空
    #[serde(rename = "Position")]
    pub position: Option<String>,             // 持仓数量
    #[serde(rename = "OpenPrice")]
    pub open_price: Option<String>,           // 开仓均价
    #[serde(rename = "PositionCost")]
    pub position_cost: Option<String>,        // 持仓成本
    #[serde(rename = "UseMargin")]
    pub use_margin: Option<String>,           // 已用保证金
    #[serde(rename = "Leverage")]
    pub leverage: Option<String>,             // 杠杆
    #[serde(rename = "estimateLiquidationPrice")]
    pub estimate_liquidation_price: Option<String>,  // 预估强平价
    #[serde(rename = "FORCECLOSEPRICE")]
    pub force_close_price: Option<String>,
    #[serde(rename = "CloseProfit")]
    pub close_profit: Option<String>,
    #[serde(rename = "Currency")]
    pub currency: Option<String>,
    #[serde(rename = "IsCrossMargin")]
    pub is_cross_margin: Option<String>,
    #[serde(rename = "AvailableUse")]
    pub available_use: Option<String>,
    #[serde(rename = "FrozenMargin")]
    pub frozen_margin: Option<String>,
    #[serde(rename = "PositionFee")]
    pub position_fee: Option<String>,
    #[serde(rename = "TradeFee")]
    pub trade_fee: Option<String>,
    #[serde(rename = "BeginTime")]
    pub begin_time: Option<String>,
    #[serde(rename = "UpdateTime")]
    pub update_time: Option<String>,
    #[serde(rename = "InsertTime")]
    pub insert_time: Option<String>,
    #[serde(rename = "Remark")]
    pub remark: Option<String>,
    #[serde(rename = "UserID")]
    pub user_id: Option<String>,
    #[serde(rename = "AccountID")]
    pub account_id: Option<String>,
    #[serde(rename = "MemberID")]
    pub member_id: Option<String>,
    #[serde(rename = "ClearCurrency")]
    pub clear_currency: Option<String>,
    #[serde(rename = "PriceCurrency")]
    pub price_currency: Option<String>,
    #[serde(rename = "SettlementGroup")]
    pub settlement_group: Option<String>,
    #[serde(rename = "AdlLevel")]
    pub adl_level: Option<i32>,
    #[serde(rename = "HighestPosition")]
    pub highest_position: Option<String>,
    #[serde(rename = "TotalCloseProfit")]
    pub total_close_profit: Option<String>,
    #[serde(rename = "TotalPositionCost")]
    pub total_position_cost: Option<String>,
    #[serde(rename = "ClosePosition")]
    pub close_position: Option<String>,
    #[serde(rename = "PrePosition")]
    pub pre_position: Option<String>,
    #[serde(rename = "LongFrozen")]
    pub long_frozen: Option<String>,
    #[serde(rename = "ShortFrozen")]
    pub short_frozen: Option<String>,
    #[serde(rename = "PreLongFrozen")]
    pub pre_long_frozen: Option<String>,
    #[serde(rename = "PreShortFrozen")]
    pub pre_short_frozen: Option<String>,
    #[serde(rename = "LongFrozenMargin")]
    pub long_frozen_margin: Option<String>,
    #[serde(rename = "ShortFrozenMargin")]
    pub short_frozen_margin: Option<String>,
    #[serde(rename = "ReserveMode")]
    pub reserve_mode: Option<String>,
    #[serde(rename = "ReserveUse")]
    pub reserve_use: Option<String>,
}

/// Order Query Response - 文档5.3
/// ⚠️ Lbank 实际响应用 PascalCase (首字母大写) 字段名 (如 `OrderSysID`, `InstrumentID`)
/// 与 PositionResponse 字段名风格一致
#[derive(Debug, Deserialize, Clone)]
pub struct OrderResponse {
    #[serde(rename = "OrderSysID")]
    pub order_sys_id: String,
    #[serde(rename = "InstrumentID")]
    pub instrument_id: String,
    pub direction: Option<String>,
    #[serde(rename = "OffsetFlag")]
    pub offset_flag: Option<String>,
    #[serde(rename = "OrderType")]
    pub order_type: Option<String>,
    #[serde(rename = "OrderStatus")]
    pub order_status: Option<String>,
    pub volume: Option<String>,
    pub price: Option<String>,
    #[serde(rename = "VolumeTraded")]
    pub traded_volume: Option<String>,
    #[serde(rename = "OpenPrice")]
    pub open_price: Option<String>,
    pub fee: Option<String>,
    #[serde(rename = "BusinessNo")]
    pub business_no: Option<String>,
    #[serde(rename = "TradePrice")]
    pub trade_price: Option<String>,
    #[serde(rename = "PositionID")]
    pub position_id: Option<String>,
    #[serde(rename = "TradeUnitID")]
    pub trade_unit_id: Option<String>,
}

/// History Order Response (历史委托) - 文档5.3
/// ⚠️ Lbank 实际响应用 PascalCase (首字母大写) 字段名
#[derive(Debug, Deserialize)]
pub struct HistoryOrderResponse {
    #[serde(rename = "OrderSysID")]
    pub order_sys_id: String,
    #[serde(rename = "InstrumentID")]
    pub instrument_id: String,
    pub direction: Option<String>,
    #[serde(rename = "OffsetFlag")]
    pub offset_flag: Option<String>,
    #[serde(rename = "OrderType")]
    pub order_type: Option<String>,
    #[serde(rename = "OrderStatus")]
    pub order_status: Option<String>,
    pub volume: Option<String>,
    pub price: Option<String>,
    #[serde(rename = "VolumeTraded")]
    pub traded_volume: Option<String>,
    #[serde(rename = "TradePrice")]
    pub trade_price: Option<String>,
    #[serde(rename = "OpenPrice")]
    pub open_price: Option<String>,
    #[serde(rename = "CloseProfit")]
    pub close_profit: Option<String>,
    pub fee: Option<String>,
    #[serde(rename = "InsertTime")]
    pub insert_time: Option<i64>,
    #[serde(rename = "VolumeCancled")]
    pub volume_cancled: Option<String>,
    #[serde(rename = "BusinessNo")]
    pub business_no: Option<String>,
    #[serde(rename = "PositionID")]
    pub position_id: Option<String>,
    #[serde(rename = "TradeUnitID")]
    pub trade_unit_id: Option<String>,
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
    #[serde(rename = "assetBalance", default)]
    pub asset_balance: Option<AssetBalance>,
    #[serde(rename = "markedPrice", default)]
    pub marked_price: String,
    #[serde(rename = "longLeverage", default)]
    pub long_leverage: Option<i32>,
    #[serde(rename = "shortLeverage", default)]
    pub short_leverage: Option<i32>,
    #[serde(rename = "longMaxLeverage", default)]
    pub long_max_leverage: Option<i32>,
    #[serde(rename = "shortMaxLeverage", default)]
    pub short_max_leverage: Option<i32>,
    #[serde(rename = "fundingRate", default)]
    pub funding_rate: Option<String>,
    #[serde(rename = "isMarketAcount", default)]
    pub is_market_account: Option<i32>,
    #[serde(rename = "isOnlyClose", default)]
    pub is_only_close: Option<i32>,
    #[serde(default)]
    pub state: Option<i32>,
    #[serde(rename = "isCrossMargin", default)]
    pub is_cross_margin: Option<i32>,
    #[serde(rename = "lastPrice", default)]
    pub last_price: Option<String>,
    #[serde(rename = "wsToken", default)]
    pub ws_token: Option<String>,
    #[serde(rename = "pairType", default)]
    pub pair_type: Option<i32>,
}

/// Asset Balance - 文档3.2
#[derive(Debug, Deserialize)]
pub struct AssetBalance {
    #[serde(default)]
    pub assets: String,
    pub available: String,
    pub balance: String,
    #[serde(rename = "realAvailable", default)]
    pub real_available: String,
    #[serde(rename = "frozenMargin", default)]
    pub frozen_margin: String,
    #[serde(rename = "totalCloseProfit", default)]
    pub total_close_profit: String,
    #[serde(rename = "crossMargin", default)]
    pub cross_margin: Option<String>,
    #[serde(rename = "marginAble", default)]
    pub margin_able: Option<String>,
    #[serde(rename = "frozenFee", default)]
    pub frozen_fee: Option<String>,
    #[serde(rename = "reserveAvailable", default)]
    pub reserve_available: Option<String>,
    #[serde(rename = "reserveMode", default)]
    pub reserve_mode: Option<String>,
    #[serde(rename = "reserveBusiness", default)]
    pub reserve_business: Option<String>,
    #[serde(rename = "reserveRatio", default)]
    pub reserve_ratio: Option<String>,
    #[serde(rename = "reserve", default)]
    pub reserve: Option<String>,
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

/// 单条深度的服务端响应条目 (扁平数组里的元素)
#[derive(Debug, Clone, Deserialize)]
pub struct MarketOrderResponse {
    #[serde(rename = "data")]
    pub data: MarketOrderItem,
    #[serde(default)]
    #[serde(rename = "table")]
    pub table: Option<String>,
}

/// 单个深度条目
///
/// ⚠️ **direction 字段语义未确定**：实测数据中全部 Direction="1" 升序排列 (63444.8→65047.3)。
/// 看起来不像传统买/卖价分布，但需要 `depth` 参数变化或不同接口确认。
/// 暂时**不再假设 0=Bid/1=Ask**，直接透传 direction 字符串，使用方自行判断。
#[derive(Debug, Clone, Deserialize)]
pub struct MarketOrderItem {
    #[serde(rename = "Orders")]
    pub orders: String,
    #[serde(rename = "Price")]
    pub price: String,
    #[serde(rename = "Volume")]
    pub volume: String,
    #[serde(rename = "instrumentID")]
    pub instrument_id: String,
    #[serde(rename = "Direction")]
    pub direction: String,
    #[serde(rename = "ExchangeID")]
    pub exchange_id: String,
    /// 部分响应可能带有成交数
    #[serde(rename = "TradedNum", default)]
    pub traded_num: Option<String>,
    /// 部分响应可能带有手续费率
    #[serde(rename = "FeeRate", default)]
    pub fee_rate: Option<String>,
}

impl MarketOrderItem {
    /// 转为统一价格层，方向语义由调用方传入（通过 `is_bid`）
    pub fn to_level(&self, _is_bid: bool) -> NormalizedPriceLevel {
        NormalizedPriceLevel {
            price: self.price.parse().unwrap_or_default(),
            volume: self.volume.parse().unwrap_or_default(),
            orders: self.orders.parse().unwrap_or(1),
        }
    }
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
