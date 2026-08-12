//! Binance-specific types and normalizations

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Binance symbol pair (e.g., "BTCUSDT")
pub type Symbol = String;

/// Price level from Binance order book
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinancePriceLevel {
    pub price: Decimal,
    pub quantity: Decimal,
}

/// Binance order book depth update
#[derive(Debug, Clone)]
pub struct BinanceDepthUpdate {
    pub symbol: Symbol,
    pub bids: Vec<BinancePriceLevel>,
    pub asks: Vec<BinancePriceLevel>,
    pub update_id: u64,
    pub event_time: i64,
}

/// Binance trade event
#[derive(Debug, Clone)]
pub struct BinanceTrade {
    pub symbol: Symbol,
    pub price: Decimal,
    pub quantity: Decimal,
    pub trade_time: i64,
    pub is_buyer_maker: bool,
    pub trade_id: u64,
}

/// Normalized order book for signal calculation
#[derive(Debug, Clone)]
pub struct NormalizedOrderBook {
    pub symbol: Symbol,
    pub best_bid: Decimal,
    pub best_ask: Decimal,
    pub bid_depth: Decimal,
    pub ask_depth: Decimal,
    pub spread: Decimal,
    pub spread_bps: Decimal,
    pub timestamp: i64,
}

impl NormalizedOrderBook {
    /// Create from Binance depth update
    pub fn from_depth_update(update: BinanceDepthUpdate) -> Self {
        let best_bid = update.bids.first().map(|p| p.price).unwrap_or_default();
        let best_ask = update.asks.first().map(|p| p.price).unwrap_or_default();
        
        let bid_depth: Decimal = update.bids.iter().map(|p| p.quantity).sum();
        let ask_depth: Decimal = update.asks.iter().map(|p| p.quantity).sum();
        
        let spread = best_ask - best_bid;
        
        // Calculate spread in basis points
        let mid_price = (best_bid + best_ask) / Decimal::from(2);
        let spread_bps = if mid_price > Decimal::ZERO {
            (spread / mid_price) * Decimal::from(10000)
        } else {
            Decimal::ZERO
        };

        Self {
            symbol: update.symbol,
            best_bid,
            best_ask,
            bid_depth,
            ask_depth,
            spread,
            spread_bps,
            timestamp: update.event_time,
        }
    }

    /// Calculate mid price
    pub fn mid_price(&self) -> Decimal {
        (self.best_bid + self.best_ask) / Decimal::from(2)
    }
}

/// Market ticker data
#[derive(Debug, Clone)]
pub struct MarketTicker {
    pub symbol: Symbol,
    pub last_price: Decimal,
    pub volume_24h: Decimal,
    pub quote_volume_24h: Decimal,
    pub timestamp: i64,
}

/// Config for Binance connection
#[derive(Debug, Clone)]
pub struct BinanceConfig {
    /// Enable testnet
    pub testnet: bool,
    /// WebSocket stream endpoint
    pub ws_endpoint: String,
    /// Subscription timeout in seconds
    pub subscription_timeout_secs: u64,
    /// Enable compression
    pub compress: bool,
}

impl Default for BinanceConfig {
    fn default() -> Self {
        Self {
            testnet: false,
            ws_endpoint: "wss://stream.binance.com:9443/ws".to_string(),
            subscription_timeout_secs: 60,
            compress: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalized_order_book_from_depth() {
        let update = BinanceDepthUpdate {
            symbol: "BTCUSDT".to_string(),
            bids: vec![
                BinancePriceLevel { price: Decimal::new(50000, 0), quantity: Decimal::new(1, 0) },
                BinancePriceLevel { price: Decimal::new(49999, 0), quantity: Decimal::new(2, 0) },
            ],
            asks: vec![
                BinancePriceLevel { price: Decimal::new(50001, 0), quantity: Decimal::new(1, 0) },
                BinancePriceLevel { price: Decimal::new(50002, 0), quantity: Decimal::new(3, 0) },
            ],
            update_id: 1,
            event_time: 1234567890,
        };

        let book = NormalizedOrderBook::from_depth_update(update);
        
        assert_eq!(book.symbol, "BTCUSDT");
        assert_eq!(book.best_bid, Decimal::new(50000, 0));
        assert_eq!(book.best_ask, Decimal::new(50001, 0));
        assert_eq!(book.spread, Decimal::new(1, 0));
        assert_eq!(book.bid_depth, Decimal::new(3, 0));
        assert_eq!(book.ask_depth, Decimal::new(4, 0));
    }

    #[test]
    fn test_mid_price() {
        let book = NormalizedOrderBook {
            symbol: "ETHUSDT".to_string(),
            best_bid: Decimal::new(3000, 0),
            best_ask: Decimal::new(3002, 0),
            bid_depth: Decimal::new(10, 0),
            ask_depth: Decimal::new(10, 0),
            spread: Decimal::new(2, 0),
            spread_bps: Decimal::new(67, 1), // ~6.67 bps
            timestamp: 1234567890,
        };

        assert_eq!(book.mid_price(), Decimal::new(3001, 0));
    }

    #[test]
    fn test_spread_bps_calculation() {
        let update = BinanceDepthUpdate {
            symbol: "BTCUSDT".to_string(),
            bids: vec![BinancePriceLevel { price: Decimal::new(50000, 0), quantity: Decimal::new(1, 0) }],
            asks: vec![BinancePriceLevel { price: Decimal::new(50010, 0), quantity: Decimal::new(1, 0) }],
            update_id: 1,
            event_time: 1234567890,
        };

        let book = NormalizedOrderBook::from_depth_update(update);
        
        // spread = 10, mid = 50005, bps = 10/50005 * 10000 ≈ 2 bps
        assert!(book.spread_bps < Decimal::new(3, 0));
        assert!(book.spread_bps > Decimal::new(1, 0));
    }
}
