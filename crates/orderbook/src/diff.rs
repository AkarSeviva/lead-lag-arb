//! Order Book Diff (Incremental Update)

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Individual price level update
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceLevelUpdate {
    pub price: Decimal,
    pub volume: Decimal,
    pub orders: u32,
}

/// Order book diff (incremental update)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookDiff {
    pub symbol: String,
    pub timestamp: i64,
    pub bids: Vec<PriceLevelUpdate>,
    pub asks: Vec<PriceLevelUpdate>,
    pub seq_id: Option<u64>,
}

impl OrderBookDiff {
    pub fn new(symbol: String) -> Self {
        Self {
            symbol,
            timestamp: chrono::Utc::now().timestamp_millis(),
            bids: Vec::new(),
            asks: Vec::new(),
            seq_id: None,
        }
    }

    pub fn add_bid(&mut self, price: Decimal, volume: Decimal, orders: u32) {
        self.bids.push(PriceLevelUpdate { price, volume, orders });
    }

    pub fn add_ask(&mut self, price: Decimal, volume: Decimal, orders: u32) {
        self.asks.push(PriceLevelUpdate { price, volume, orders });
    }

    pub fn is_empty(&self) -> bool {
        self.bids.is_empty() && self.asks.is_empty()
    }
}

/// Trait for sources that can provide diffs
pub trait DiffSource {
    fn next_diff(&mut self) -> Option<OrderBookDiff>;
}
