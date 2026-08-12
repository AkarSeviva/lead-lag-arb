//! Order Book Snapshot

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Price level for snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceLevel {
    pub price: Decimal,
    pub volume: Decimal,
    pub orders: u32,
}

impl PriceLevel {
    pub fn new(price: Decimal, volume: Decimal, orders: u32) -> Self {
        Self { price, volume, orders }
    }
}

/// Order book snapshot (immutable)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookSnapshot {
    pub symbol: String,
    pub timestamp: i64,
    pub bids: Vec<PriceLevel>,
    pub asks: Vec<PriceLevel>,
    pub seq_id: Option<u64>,
}

impl OrderBookSnapshot {
    /// Best bid
    pub fn best_bid(&self) -> Option<Decimal> {
        self.bids.first().map(|l| l.price)
    }

    /// Best ask
    pub fn best_ask(&self) -> Option<Decimal> {
        self.asks.first().map(|l| l.price)
    }

    /// Spread
    pub fn spread(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) if ask > bid => Some(ask - bid),
            _ => None,
        }
    }

    /// Mid price
    pub fn mid_price(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some((bid + ask) / Decimal::from(2)),
            _ => None,
        }
    }

    /// Total bid volume at top N levels
    pub fn bid_volume(&self, levels: usize) -> Decimal {
        self.bids.iter()
            .take(levels)
            .map(|l| l.volume)
            .sum()
    }

    /// Total ask volume at top N levels
    pub fn ask_volume(&self, levels: usize) -> Decimal {
        self.asks.iter()
            .take(levels)
            .map(|l| l.volume)
            .sum()
    }

    /// Order book imbalance
    pub fn imbalance(&self, levels: usize) -> Option<Decimal> {
        let bid_vol = self.bid_volume(levels);
        let ask_vol = self.ask_volume(levels);
        let total = bid_vol + ask_vol;

        if total.is_zero() {
            None
        } else {
            Some((bid_vol - ask_vol) / total)
        }
    }

    /// Is valid (has both bids and asks)
    pub fn is_valid(&self) -> bool {
        self.bids.first().zip(self.asks.first()).is_some()
    }
}
