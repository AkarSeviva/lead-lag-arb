//! Common types used across the config module

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Trading direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Long,
    Short,
}

impl Direction {
    pub fn from_lbank_code(code: &str) -> Option<Self> {
        match code {
            "0" => Some(Self::Long),
            "1" => Some(Self::Short),
            _ => None,
        }
    }

    pub fn to_lbank_code(&self) -> &'static str {
        match self {
            Self::Long => "0",
            Self::Short => "1",
        }
    }
}

/// Order side
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderSide {
    Buy,
    Sell,
}

/// Order type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderType {
    Limit,
    Market,
    Trigger, // Plan order / take-profit stop-loss
}

/// Order status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Pending,
    Submitted,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
}

/// Position information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub direction: Direction,
    pub volume: Decimal,
    pub entry_price: Decimal,
    pub unrealized_pnl: Decimal,
}

/// Order request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderRequest {
    pub symbol: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub volume: Decimal,
    pub price: Option<Decimal>,
    pub client_order_id: Option<String>,
}

/// Order response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderResponse {
    pub order_id: String,
    pub symbol: String,
    pub status: OrderStatus,
    pub filled_volume: Decimal,
    pub avg_price: Option<Decimal>,
    pub message: Option<String>,
}
