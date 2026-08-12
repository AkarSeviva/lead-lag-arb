//! Binance Market Data Adapter
//!
//! Provides market data streams from Binance using direct WebSocket connections.
//! Implements the MarketDataProvider trait for integration with the signal engine.

use crate::types::{
    BinanceConfig, BinanceDepthUpdate, BinancePriceLevel, BinanceTrade, MarketTicker,
    NormalizedOrderBook,
};
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info, warn};

/// Market data event for signal engine
#[derive(Debug, Clone)]
pub enum MarketEvent {
    /// Order book depth update
    DepthUpdate(NormalizedOrderBook),
    /// Individual trade
    Trade(BinanceTrade),
    /// Ticker update
    Ticker(MarketTicker),
    /// Connection status change
    Status(ConnectionStatus),
}

/// Connection status
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    Connecting,
    Connected,
    Disconnected,
    Reconnecting,
    Error(String),
}

/// Binance market data provider
pub struct BinanceMarketData {
    config: BinanceConfig,
    status_tx: broadcast::Sender<ConnectionStatus>,
}

impl BinanceMarketData {
    /// Create a new Binance market data provider
    pub fn new(config: BinanceConfig) -> Self {
        let (status_tx, _) = broadcast::channel(100);
        Self { config, status_tx }
    }

    /// Get connection status receiver
    pub fn status_receiver(&self) -> broadcast::Receiver<ConnectionStatus> {
        self.status_tx.subscribe()
    }

    /// Start receiving depth updates for a symbol
    /// Returns a channel receiver for market events
    pub async fn subscribe_depth(&self, symbol: &str) -> Result<mpsc::Receiver<MarketEvent>> {
        let (tx, rx) = mpsc::channel(1000);
        let symbol = symbol.to_uppercase();

        // Convert symbol to Binance format (lowercase for streams)
        let stream_name = format!("{}@depth@100ms", symbol.to_lowercase());
        let ws_url = format!("{}/{}", self.config.ws_endpoint, stream_name);

        info!(symbol = %symbol, url = %ws_url, "Subscribing to Binance depth stream");

        // Extract fields needed for the async task
        let status_tx = self.status_tx.clone();
        let symbol_clone = symbol.clone();

        tokio::spawn(async move {
            status_tx.send(ConnectionStatus::Connecting).ok();

            match Self::connect_depth_stream_inner(&ws_url, tx, &symbol_clone).await {
                Ok(()) => {
                    status_tx.send(ConnectionStatus::Disconnected).ok();
                }
                Err(e) => {
                    error!(error = %e, "Depth stream error");
                    status_tx.send(ConnectionStatus::Error(e.to_string())).ok();
                }
            }
        });

        Ok(rx)
    }

    /// Connect to depth stream and process messages (static version)
    async fn connect_depth_stream_inner(
        url: &str,
        tx: mpsc::Sender<MarketEvent>,
        symbol: &str,
    ) -> Result<()> {
        let ws_stream = tokio_tungstenite::connect_async(url)
            .await
            .context("Failed to connect to Binance WebSocket")?
            .0;

        let (mut write, mut read) = ws_stream.split();

        // Send ping to keep connection alive
        let ping_msg = serde_json::json!({"method": "PING"});
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                ping_msg.to_string(),
            ))
            .await
            .context("Failed to send ping")?;

        while let Some(msg) = read.next().await {
            match msg {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    if let Some(depth_update) = Self::parse_depth_update(&text, symbol) {
                        let normalized = NormalizedOrderBook::from_depth_update(depth_update);
                        if tx.send(MarketEvent::DepthUpdate(normalized)).await.is_err() {
                            break;
                        }
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Ping(data)) => {
                    write
                        .send(tokio_tungstenite::tungstenite::Message::Pong(data))
                        .await
                        .ok();
                }
                Err(e) => {
                    error!(error = %e, "WebSocket error");
                    break;
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Parse Binance depth update message
    fn parse_depth_update(msg: &str, symbol: &str) -> Option<BinanceDepthUpdate> {
        let json: serde_json::Value = serde_json::from_str(msg).ok()?;

        // Skip subscription confirmation
        if json.get("result").is_some() && json.get("id").is_some() {
            return None;
        }

        let event_type = json.get("e")?.as_str()?;
        if event_type != "depthUpdate" {
            return None;
        }

        let bids = json
            .get("b")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|level| {
                        let price = level.get(0)?.as_str()?.parse().ok()?;
                        let qty = level.get(1)?.as_str()?.parse().ok()?;
                        Some(BinancePriceLevel { price, quantity: qty })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let asks = json
            .get("a")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|level| {
                        let price = level.get(0)?.as_str()?.parse().ok()?;
                        let qty = level.get(1)?.as_str()?.parse().ok()?;
                        Some(BinancePriceLevel { price, quantity: qty })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let update_id = json.get("u").or(json.get("lastUpdateId"))?.as_u64()?;
        let event_time = json.get("E")?.as_i64()?;

        Some(BinanceDepthUpdate {
            symbol: symbol.to_string(),
            bids,
            asks,
            update_id,
            event_time,
        })
    }

    /// Start receiving trades for a symbol
    pub async fn subscribe_trades(&self, symbol: &str) -> Result<mpsc::Receiver<MarketEvent>> {
        let (tx, rx) = mpsc::channel(1000);
        let symbol = symbol.to_uppercase();
        let config = self.config.clone();

        let stream_name = format!("{}@trade", symbol.to_lowercase());
        let ws_url = format!("{}/{}", config.ws_endpoint, stream_name);

        info!(symbol = %symbol, url = %ws_url, "Subscribing to Binance trade stream");

        tokio::spawn(async move {
            if let Err(e) = Self::connect_trade_stream(&ws_url, tx).await {
                error!(error = %e, "Trade stream error");
            }
        });

        Ok(rx)
    }

    /// Connect to trade stream
    async fn connect_trade_stream(
        url: &str,
        tx: mpsc::Sender<MarketEvent>,
    ) -> Result<()> {
        let ws_stream = tokio_tungstenite::connect_async(url)
            .await
            .context("Failed to connect to Binance WebSocket")?
            .0;

        let (_write, mut read) = ws_stream.split();

        while let Some(msg) = read.next().await {
            if let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = msg {
                if let Some(trade) = Self::parse_trade(&text) {
                    if tx.send(MarketEvent::Trade(trade)).await.is_err() {
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    /// Parse Binance trade message
    fn parse_trade(msg: &str) -> Option<BinanceTrade> {
        let json: serde_json::Value = serde_json::from_str(msg).ok()?;

        let symbol = json.get("s")?.as_str()?.to_string();
        let price: Decimal = json.get("p")?.as_str()?.parse().ok()?;
        let quantity: Decimal = json.get("q")?.as_str()?.parse().ok()?;
        let trade_time = json.get("T")?.as_i64()?;
        let trade_id = json.get("t")?.as_u64()?;
        let is_buyer_maker = json.get("m")?.as_bool()?;

        Some(BinanceTrade {
            symbol,
            price,
            quantity,
            trade_time,
            is_buyer_maker,
            trade_id,
        })
    }

    /// Create combined stream for multiple symbols
    pub async fn subscribe_combined(
        &self,
        symbols: &[&str],
    ) -> Result<HashMap<String, mpsc::Receiver<MarketEvent>>> {
        let mut receivers = HashMap::new();

        for symbol in symbols {
            let rx = self.subscribe_depth(symbol).await?;
            receivers.insert(symbol.to_uppercase(), rx);
        }

        Ok(receivers)
    }
}

/// Builder for Binance market data
pub struct BinanceMarketDataBuilder {
    config: BinanceConfig,
}

impl BinanceMarketDataBuilder {
    pub fn new() -> Self {
        Self {
            config: BinanceConfig::default(),
        }
    }

    /// Use testnet environment
    pub fn testnet(mut self) -> Self {
        self.config.testnet = true;
        self.config.ws_endpoint = "wss://testnet.binance.vision/ws".to_string();
        self
    }

    /// Use production environment (default)
    pub fn production(mut self) -> Self {
        self.config.testnet = false;
        self.config.ws_endpoint = "wss://stream.binance.com:9443/ws".to_string();
        self
    }

    /// Build the market data provider
    pub fn build(self) -> BinanceMarketData {
        BinanceMarketData::new(self.config)
    }
}

impl Default for BinanceMarketDataBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_depth_update() {
        let msg = r#"{
            "e": "depthUpdate",
            "E": 1234567890,
            "s": "BTCUSDT",
            "U": 1,
            "u": 2,
            "b": [["50000.00", "1.0"]],
            "a": [["50001.00", "2.0"]]
        }"#;

        let update = BinanceMarketData::parse_depth_update(msg, "BTCUSDT");
        assert!(update.is_some());
        let update = update.unwrap();
        assert_eq!(update.symbol, "BTCUSDT");
        assert_eq!(update.bids.len(), 1);
        assert_eq!(update.asks.len(), 1);
    }

    #[test]
    fn test_parse_trade() {
        let msg = r#"{
            "e": "trade",
            "E": 1234567890,
            "s": "ETHUSDT",
            "t": 12345,
            "p": "3000.00",
            "q": "1.5",
            "T": 1234567890,
            "m": true
        }"#;

        let trade = BinanceMarketData::parse_trade(msg);
        assert!(trade.is_some());
        let trade = trade.unwrap();
        assert_eq!(trade.symbol, "ETHUSDT");
        assert_eq!(trade.trade_id, 12345);
    }
}
