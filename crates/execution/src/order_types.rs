//! Order Types - Lbank 订单类型映射
//!
//! 基于逆向分析报告 (Lbank_逆向分析报告.md:398-413)
//! - 4.8 OrderPriceType: 0=LIMIT, 1=ANY, 4=MARKET
//! - 4.7 OffsetFlag: 0=OPEN, 5=CLOSE_ALL, 6=CLOSE_APPOINT_ORDER
//! - 4.9 TriggerOrderType: 1=POSITION_STOP_PROFIT_LOSS, 2=ORDER_STOP_PROFIT_LOSS, 3=PLAN
//! - 4.4 ActionFlag: 1=撤单

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// 订单价格类型 - Lbank OrderPriceType
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderPriceType {
    /// 限价单 (Lbank code "0")
    Limit,
    /// 任价单 (Lbank code "1") - IOC 语义
    Any,
    /// 市价单 (Lbank code "4")
    Market,
}

impl OrderPriceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Limit => "0",
            Self::Any => "1",
            Self::Market => "4",
        }
    }

    pub fn is_passive(&self) -> bool {
        matches!(self, Self::Limit) // Limit 才会挂单（Maker）
    }
}

/// 开平标志 - Lbank OffsetFlag
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OffsetFlag {
    /// 开仓
    Open,
    /// 平仓 (实际用 "5" = CLOSE_ALL)
    Close,
    /// 全平 (Lbank code "5")
    CloseAll,
    /// 止盈止损平仓 (Lbank code "6")
    CloseAppointOrder,
    /// 触发单平仓 (Lbank code "8")
    CloseMax,
}

impl OffsetFlag {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "0",
            Self::Close | Self::CloseAll => "5",
            Self::CloseAppointOrder => "6",
            Self::CloseMax => "8",
        }
    }
}

/// 触发单类型 - Lbank TriggerOrderType
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerOrderType {
    /// 持仓止盈止损
    PositionStopProfitLoss,
    /// 订单止盈止损
    OrderStopProfitLoss,
    /// 计划委托
    Plan,
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

/// 策略层面的订单种类 - 映射到 Lbank 字段
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderKind {
    /// 开仓市价单 (Lbank: OrderPriceType=4, OffsetFlag=0)
    /// 策略: 进场所必 (延迟套利信号窗口 100-300ms)
    OpenMarket,

    /// 开仓限价单 (Lbank: OrderPriceType=0, OffsetFlag=0)
    /// 策略: 不推荐 - 信号窗口太短，限价单可能不成交
    OpenLimit { price: Decimal },

    /// 平仓 GTC 限价单 (Maker)
    /// 策略: Method 1 - TP 信号触发后在最新价挂 GTC
    /// Lbank: OrderPriceType=0, OffsetFlag=5
    CloseGtc { price: Decimal },

    /// 平仓市价单 (Taker, IOC 语义)
    /// 策略: GTC timeout fallback, 或 StopLoss
    /// Lbank: OrderPriceType=4, OffsetFlag=5
    CloseMarket,

    /// 带止盈止损的限价平仓单
    /// 策略: 不主动用，但保留接口
    /// Lbank: 走 /cfd/action/v1.0/SendCloseOrderInsert
    CloseWithSlTp {
        price: Decimal,
        sl_trigger_price: Decimal,
        tp_trigger_price: Decimal,
    },
}

impl OrderKind {
    pub fn is_passive(&self) -> bool {
        matches!(self, Self::OpenLimit { .. } | Self::CloseGtc { .. })
    }

    pub fn is_market(&self) -> bool {
        matches!(self, Self::OpenMarket | Self::CloseMarket)
    }

    pub fn is_close(&self) -> bool {
        matches!(
            self,
            Self::CloseGtc { .. } | Self::CloseMarket | Self::CloseWithSlTp { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_order_price_type_codes() {
        assert_eq!(OrderPriceType::Limit.as_str(), "0");
        assert_eq!(OrderPriceType::Any.as_str(), "1");
        assert_eq!(OrderPriceType::Market.as_str(), "4");
    }

    #[test]
    fn test_offset_flag_codes() {
        assert_eq!(OffsetFlag::Open.as_str(), "0");
        // Lbank 实际 API 用 "5" 表示平仓
        assert_eq!(OffsetFlag::Close.as_str(), "5");
        assert_eq!(OffsetFlag::CloseAll.as_str(), "5");
    }

    #[test]
    fn test_trigger_order_type_codes() {
        assert_eq!(TriggerOrderType::PositionStopProfitLoss.as_str(), "1");
        assert_eq!(TriggerOrderType::OrderStopProfitLoss.as_str(), "2");
        assert_eq!(TriggerOrderType::Plan.as_str(), "3");
    }

    #[test]
    fn test_order_kind_classification() {
        assert!(OrderKind::OpenMarket.is_market());
        assert!(OrderKind::CloseMarket.is_market());
        assert!(OrderKind::CloseGtc {
            price: Decimal::new(100, 0)
        }
        .is_passive());

        assert!(OrderKind::CloseGtc {
            price: Decimal::new(100, 0)
        }
        .is_close());
        assert!(OrderKind::CloseMarket.is_close());
    }
}