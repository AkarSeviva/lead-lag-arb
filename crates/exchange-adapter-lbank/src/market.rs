//! Lbank Market Data Adapter
//!
//! Provides unified interface for market data access.

use crate::client::LbankClient;
use crate::protocol::{NormalizedOrderBook, NormalizedPriceLevel};
use crate::ws::{LbankWebSocket, WsEvent};
use anyhow::Result;
use rust_decimal::Decimal;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::debug;

/// Market event types for the signal engine
#[derive(Debug, Clone)]
pub enum MarketEvent {
    /// Order book snapshot update
    OrderBook {
        exchange: &'static str,
        symbol: String,
        order_book: NormalizedOrderBook,
        local_timestamp: i64,
    },
    /// Trade event (if available)
    Trade {
        exchange: &'static str,
        symbol: String,
        price: Decimal,
        volume: Decimal,
        side: &'static str,
        timestamp: i64,
    },
}

/// Market data interface for signal engine
pub trait MarketDataAdapter: Send + Sync {
    /// Get current order book
    fn get_order_book(&self, symbol: &str) -> Option<NormalizedOrderBook>;

    /// Subscribe to order book updates, returns receiver
    fn subscribe_orderbook(&self, symbol: &str) -> Result<mpsc::Receiver<MarketEvent>>;
}

/// Lbank market data implementation
pub struct LbankMarketData {
    client: LbankClient,
    ws: LbankWebSocket,
}

impl LbankMarketData {
    pub fn new(client: LbankClient) -> Self {
        Self {
            client,
            ws: LbankWebSocket::new(None),
        }
    }

    /// Initialize and start WebSocket connection
    pub async fn start(&mut self) -> Result<mpsc::Receiver<MarketEvent>> {
        let mut ws = LbankWebSocket::new(None);
        let ws_rx = ws.start()?;
        self.ws = ws;

        // Create channel to convert WS events to market events
        let (tx, rx) = mpsc::channel(1000);

        // Spawn task to convert events
        tokio::spawn(async move {
            let mut ws_rx = ws_rx;
            while let Some(event) = ws_rx.recv().await {
                if let Some(market_event) = ws_event_to_market_event("LBank", event) {
                    let _ = tx.send(market_event).await;
                }
            }
        });

        Ok(rx)
    }

    /// Get current order book via REST (snapshot)
    pub async fn get_order_book_snapshot(&self, symbol: &str) -> Result<NormalizedOrderBook> {
        let items = self.client.get_order_book(symbol, 25).await?;

        let mut bids = Vec::new();
        let mut asks = Vec::new();

        for item in items {
            let level = NormalizedPriceLevel {
                price: item.price.parse().unwrap_or_default(),
                volume: item.volume.parse().unwrap_or_default(),
                orders: item.orders.parse().unwrap_or(1),
            };

            if item.direction == "1" {
                bids.push(level);
            } else {
                asks.push(level);
            }
        }

        // Sort bids descending (best bid first)
        bids.sort_by(|a, b| b.price.cmp(&a.price));
        // Sort asks ascending (best ask first)
        asks.sort_by(|a, b| a.price.cmp(&b.price));

        let last_price = asks.first()
            .or(bids.first())
            .map(|l| l.price)
            .unwrap_or_default();

        Ok(NormalizedOrderBook {
            symbol: symbol.to_string(),
            timestamp: chrono::Utc::now().timestamp_millis(),
            last_price,
            bids,
            asks,
            seq_id: None,
        })
    }
}

impl MarketDataAdapter for LbankMarketData {
    fn get_order_book(&self, symbol: &str) -> Option<NormalizedOrderBook> {
        self.ws.get_order_book(symbol)
    }

    fn subscribe_orderbook(&self, symbol: &str) -> Result<mpsc::Receiver<MarketEvent>> {
        // For now, create a channel - actual implementation would integrate with ws
        let (_tx, rx) = mpsc::channel(100);
        
        // TODO: Subscribe through WebSocket
        Ok(rx)
    }
}

/// Convert WebSocket event to market event
pub fn ws_event_to_market_event(
    exchange: &'static str,
    event: WsEvent,
) -> Option<MarketEvent> {
    match event {
        WsEvent::OrderBookUpdate { symbol, order_book } => {
            Some(MarketEvent::OrderBook {
                exchange,
                symbol,
                order_book,
                local_timestamp: chrono::Utc::now().timestamp_millis(),
            })
        }
        WsEvent::OrderUpdate { .. } => None, // Not a market event
        WsEvent::Connected => None,
        WsEvent::Disconnected => None,
        WsEvent::Error(_) => None,
    }
}
