//! Order Book Reconstruction
//!
//! Maintains a reconstructed L2 order book with bid/ask levels.

use crate::snapshot::{OrderBookSnapshot, PriceLevel as SnapshotPriceLevel};
use indexmap::IndexMap;
use parking_lot::RwLock;
use rust_decimal::Decimal;
use std::sync::Arc;

/// Price level in the order book
#[derive(Debug, Clone)]
pub struct PriceLevel {
    /// Price
    pub price: Decimal,
    /// Total volume at this price
    pub volume: Decimal,
    /// Number of orders at this price
    pub orders: u32,
}

impl PriceLevel {
    pub fn new(price: Decimal, volume: Decimal, orders: u32) -> Self {
        Self { price, volume, orders }
    }
}

impl From<&SnapshotPriceLevel> for PriceLevel {
    fn from(level: &SnapshotPriceLevel) -> Self {
        Self {
            price: level.price,
            volume: level.volume,
            orders: level.orders,
        }
    }
}

/// Order book side (bid or ask)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Bid,
    Ask,
}

/// Sorted order book side
#[derive(Debug, Clone)]
pub struct BookSide {
    /// Price levels sorted by price
    levels: IndexMap<Decimal, PriceLevel>,
    /// Side type
    side: Side,
}

impl BookSide {
    pub fn new(side: Side) -> Self {
        Self {
            levels: IndexMap::new(),
            side,
        }
    }

    /// Get the best price level
    pub fn best(&self) -> Option<&PriceLevel> {
        match self.side {
            Side::Bid => self.levels.iter().next().map(|(_, v)| v), // Highest bid
            Side::Ask => self.levels.iter().next().map(|(_, v)| v), // Lowest ask
        }
    }

    /// Get best price
    pub fn best_price(&self) -> Option<Decimal> {
        self.best().map(|l| l.price)
    }

    /// Update or insert a price level
    pub fn update(&mut self, price: Decimal, volume: Decimal, orders: u32) {
        if volume.is_zero() && orders == 0 {
            self.levels.remove(&price);
        } else {
            self.levels.insert(price, PriceLevel { price, volume, orders });
        }
        self.sort();
    }

    /// Remove a price level
    pub fn remove(&mut self, price: &Decimal) {
        self.levels.remove(price);
    }

    /// Get total volume up to N levels
    pub fn volume_at_levels(&self, n: usize) -> Decimal {
        self.levels.values()
            .take(n)
            .map(|l| l.volume)
            .sum()
    }

    /// Get depth (number of price levels)
    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    /// Get all levels
    pub fn levels(&self) -> &IndexMap<Decimal, PriceLevel> {
        &self.levels
    }

    /// Sort levels appropriately for this side
    fn sort(&mut self) {
        match self.side {
            Side::Bid => {
                // Sort bids descending (best bid first)
                let mut keys: Vec<_> = self.levels.keys().cloned().collect();
                keys.sort_by(|a, b| b.cmp(a));
                let levels_new: IndexMap<Decimal, PriceLevel> = keys.into_iter()
                    .filter_map(|k| self.levels.remove(&k))
                    .map(|p| (p.price, p))
                    .collect();
                self.levels = levels_new;
            }
            Side::Ask => {
                // Sort asks ascending (best ask first) - already default order
                let mut keys: Vec<_> = self.levels.keys().cloned().collect();
                keys.sort();
                let levels_new: IndexMap<Decimal, PriceLevel> = keys.into_iter()
                    .filter_map(|k| self.levels.shift_remove(&k))
                    .map(|p| (p.price, p))
                    .collect();
                self.levels = levels_new;
            }
        }
    }

    /// Clear all levels
    pub fn clear(&mut self) {
        self.levels.clear();
    }
}

/// Complete order book
#[derive(Debug, Clone)]
pub struct OrderBook {
    /// Symbol
    pub symbol: String,
    /// Bid side
    bids: BookSide,
    /// Ask side
    asks: BookSide,
    /// Last update timestamp
    pub timestamp: i64,
    /// Sequence ID (if available)
    pub seq_id: Option<u64>,
    /// Local processing timestamp
    pub local_ts: i64,
}

impl OrderBook {
    pub fn new(symbol: String) -> Self {
        Self {
            symbol,
            bids: BookSide::new(Side::Bid),
            asks: BookSide::new(Side::Ask),
            timestamp: 0,
            seq_id: None,
            local_ts: chrono::Utc::now().timestamp_millis(),
        }
    }

    /// Create from snapshot
    pub fn from_snapshot(snapshot: &OrderBookSnapshot) -> Self {
        let mut book = Self::new(snapshot.symbol.clone());

        for level in &snapshot.bids {
            book.bids.update(level.price, level.volume, level.orders);
        }

        for level in &snapshot.asks {
            book.asks.update(level.price, level.volume, level.orders);
        }

        book.timestamp = snapshot.timestamp;
        book.seq_id = snapshot.seq_id;
        book.local_ts = chrono::Utc::now().timestamp_millis();

        book
    }

    /// Update bids
    pub fn update_bids(&mut self, price: Decimal, volume: Decimal, orders: u32) {
        self.bids.update(price, volume, orders);
        self.local_ts = chrono::Utc::now().timestamp_millis();
    }

    /// Update asks
    pub fn update_asks(&mut self, price: Decimal, volume: Decimal, orders: u32) {
        self.asks.update(price, volume, orders);
        self.local_ts = chrono::Utc::now().timestamp_millis();
    }

    /// Apply a diff update
    pub fn apply_diff(&mut self, diff: &crate::diff::OrderBookDiff) {
        for update in &diff.bids {
            self.bids.update(update.price, update.volume, update.orders);
        }
        for update in &diff.asks {
            self.asks.update(update.price, update.volume, update.orders);
        }
        self.local_ts = chrono::Utc::now().timestamp_millis();
    }

    /// Best bid price
    pub fn best_bid(&self) -> Option<Decimal> {
        self.bids.best_price()
    }

    /// Best ask price
    pub fn best_ask(&self) -> Option<Decimal> {
        self.asks.best_price()
    }

    /// Spread (ask - bid)
    pub fn spread(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) if ask > bid => Some(ask - bid),
            _ => None,
        }
    }

    /// Spread as percentage of mid price
    pub fn spread_pct(&self) -> Option<Decimal> {
        let mid = self.mid_price()?;
        let spread = self.spread()?;
        if mid.is_zero() {
            None
        } else {
            Some(spread / mid * Decimal::from(10000) / Decimal::from(100)) // Basis points
        }
    }

    /// Mid price
    pub fn mid_price(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some((bid + ask) / Decimal::from(2)),
            _ => None,
        }
    }

    /// Bid volume at top N levels
    pub fn bid_volume_at_levels(&self, levels: usize) -> Decimal {
        self.bids.volume_at_levels(levels)
    }

    /// Ask volume at top N levels
    pub fn ask_volume_at_levels(&self, levels: usize) -> Decimal {
        self.asks.volume_at_levels(levels)
    }

    /// Imbalance (bid volume - ask volume) / total volume
    pub fn imbalance(&self, levels: usize) -> Option<Decimal> {
        let bid_vol = self.bids.volume_at_levels(levels);
        let ask_vol = self.asks.volume_at_levels(levels);
        let total = bid_vol + ask_vol;

        if total.is_zero() {
            None
        } else {
            Some((bid_vol - ask_vol) / total)
        }
    }

    /// Depth at N levels
    pub fn depth(&self) -> (usize, usize) {
        (self.bids.depth(), self.asks.depth())
    }

    /// Volume weighted average price at N levels
    pub fn vwap(&self, levels: usize) -> Option<Decimal> {
        let mut total_value = Decimal::ZERO;
        let mut total_volume = Decimal::ZERO;

        for level in self.bids.levels().values().take(levels) {
            total_value += level.price * level.volume;
            total_volume += level.volume;
        }
        for level in self.asks.levels().values().take(levels) {
            total_value += level.price * level.volume;
            total_volume += level.volume;
        }

        if total_volume.is_zero() {
            None
        } else {
            Some(total_value / total_volume)
        }
    }

    /// To snapshot
    pub fn snapshot(&self) -> OrderBookSnapshot {
        OrderBookSnapshot {
            symbol: self.symbol.clone(),
            timestamp: self.timestamp,
            bids: self.bids.levels().values().map(|l| SnapshotPriceLevel {
                price: l.price,
                volume: l.volume,
                orders: l.orders,
            }).collect(),
            asks: self.asks.levels().values().map(|l| SnapshotPriceLevel {
                price: l.price,
                volume: l.volume,
                orders: l.orders,
            }).collect(),
            seq_id: self.seq_id,
        }
    }

    /// Clear the order book
    pub fn clear(&mut self) {
        self.bids.clear();
        self.asks.clear();
    }
}

/// Thread-safe wrapper
pub type SharedOrderBook = Arc<RwLock<OrderBook>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spread_calculation() {
        let mut book = OrderBook::new("BTCUSDT".to_string());
        book.update_bids(Decimal::new(64000, 0), Decimal::new(100, 0), 1);
        book.update_asks(Decimal::new(64010, 0), Decimal::new(100, 0), 1);

        assert_eq!(book.spread(), Some(Decimal::new(10, 0)));
        assert_eq!(book.mid_price(), Some(Decimal::new(64005, 0)));
    }

    #[test]
    fn test_imbalance() {
        let mut book = OrderBook::new("BTCUSDT".to_string());
        book.update_bids(Decimal::new(64000, 0), Decimal::new(100, 0), 1);
        book.update_asks(Decimal::new(64010, 0), Decimal::new(50, 0), 1);

        // (100 - 50) / 150 = 0.33
        let imbalance = book.imbalance(1).unwrap();
        assert!(imbalance > Decimal::new(30, 2) && imbalance < Decimal::new(35, 2));
    }
}
