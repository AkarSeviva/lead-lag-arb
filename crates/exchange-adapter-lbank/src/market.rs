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
use parking_lot::RwLock;

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
    /// Trade event from deal updates
    Trade {
        exchange: &'static str,
        symbol: String,
        price: Decimal,
        volume: Decimal,
        side: &'static str,
        timestamp: i64,
    },
    /// 24hr Market ticker update
    Ticker {
        exchange: &'static str,
        symbol: String,
        last_price: Decimal,
        buy_price: Decimal,
        sell_price: Decimal,
        high_24h: Decimal,
        low_24h: Decimal,
        volume: Decimal,
        timestamp: i64,
    },
    /// Order book raw update (direct from WS Topic=3)
    OrderBookRaw {
        exchange: &'static str,
        symbol: String,
        bids: Vec<(Decimal, Decimal)>,
        asks: Vec<(Decimal, Decimal)>,
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
    trade_cache: Arc<RwLock<Vec<TradeCacheEntry>>>,
}

struct TradeCacheEntry {
    symbol: String,
    price: Decimal,
    volume: Decimal,
    side: String,
    timestamp: i64,
}

impl LbankMarketData {
    /// Create a new market data adapter
    pub fn new(client: LbankClient) -> Self {
        Self {
            client,
            ws: LbankWebSocket::new(None),
            trade_cache: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Create with authentication token
    pub fn new_with_auth(client: LbankClient, token: String) -> Self {
        Self {
            client,
            ws: LbankWebSocket::new(Some(token)),
            trade_cache: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Initialize and start WebSocket connection
    pub async fn start(&mut self) -> Result<mpsc::Receiver<MarketEvent>> {
        let mut ws = LbankWebSocket::new(self.ws.token().cloned());
        let ws_rx = ws.start()?;
        self.ws = ws;

        let trade_cache = self.trade_cache.clone();

        // Create channel to convert WS events to market events
        let (tx, rx) = mpsc::channel(1000);

        // Spawn task to convert events
        tokio::spawn(async move {
            let mut ws_rx = ws_rx;
            while let Some(event) = ws_rx.recv().await {
                if let Some(market_event) = ws_event_to_market_event("LBank", event, &trade_cache) {
                    let _ = tx.send(market_event).await;
                }
            }
        });

        Ok(rx)
    }

    /// Subscribe to market data (ticker + trades + orderbook) for a symbol
    pub async fn subscribe_market_data(&self, symbol: &str) -> Result<()> {
        self.ws.subscribe_market(symbol).await?;
        self.ws.subscribe_deals(symbol).await?;
        self.ws.subscribe_orderbook(symbol).await?;
        Ok(())
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

            // Lbank format: direction = "0" (Ask/卖盘/s), "1" (Bid/买盘/b)
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

    /// Get cached market data from WebSocket
    pub fn get_cached_market(&self, symbol: &str) -> Option<MarketDataCache> {
        self.ws.get_market_data(symbol).map(|m| MarketDataCache {
            symbol: m.instrument_id.unwrap_or_default(),
            last_price: m.last_price.as_ref()
                .and_then(|s| s.parse().ok())
                .unwrap_or_default(),
            buy_price: m.buy_price.as_ref()
                .and_then(|s| s.parse().ok())
                .unwrap_or_default(),
            sell_price: m.sell_price.as_ref()
                .and_then(|s| s.parse().ok())
                .unwrap_or_default(),
        })
    }
}

/// Cached market data structure
#[derive(Debug, Clone)]
pub struct MarketDataCache {
    pub symbol: String,
    pub last_price: Decimal,
    pub buy_price: Decimal,
    pub sell_price: Decimal,
}

impl MarketDataAdapter for LbankMarketData {
    fn get_order_book(&self, symbol: &str) -> Option<NormalizedOrderBook> {
        self.ws.get_order_book(symbol)
    }

    fn subscribe_orderbook(&self, _symbol: &str) -> Result<mpsc::Receiver<MarketEvent>> {
        // Note: OrderBook subscription via WebSocket is not fully implemented
        // Use REST API get_order_book_snapshot() for order book data
        let (_tx, rx) = mpsc::channel(100);
        Ok(rx)
    }
}

/// Convert WebSocket event to market event
pub fn ws_event_to_market_event(
    exchange: &'static str,
    event: WsEvent,
    trade_cache: &Arc<RwLock<Vec<TradeCacheEntry>>>,
) -> Option<MarketEvent> {
    match event {
        WsEvent::MarketUpdate {
            symbol,
            last_price,
            buy_price,
            sell_price,
            high_24h,
            low_24h,
            volume,
            timestamp,
        } => {
            Some(MarketEvent::Ticker {
                exchange,
                symbol,
                last_price,
                buy_price,
                sell_price,
                high_24h,
                low_24h,
                volume,
                timestamp,
            })
        }
        WsEvent::DealUpdate {
            symbol,
            price,
            volume,
            direction,
            trade_id: _,
            timestamp,
        } => {
            // Cache the trade for building order book
            {
                let mut cache = trade_cache.write();
                // Keep last 100 trades per symbol
                cache.retain(|t| t.symbol != symbol);
                cache.push(TradeCacheEntry {
                    symbol: symbol.clone(),
                    price,
                    volume,
                    side: direction.clone(),
                    timestamp,
                });
            }

            // Convert direction to side
            let side = if direction == "0" { "buy" } else { "sell" };

            Some(MarketEvent::Trade {
                exchange,
                symbol,
                price,
                volume,
                side,
                timestamp,
            })
        }
        WsEvent::OrderBookUpdate {
            symbol,
            bids,
            asks,
            timestamp,
        } => {
            // Convert raw order book data to NormalizedOrderBook
            let price_levels_to_normalized = |levels: Vec<(Decimal, Decimal)>| -> Vec<NormalizedPriceLevel> {
                levels.into_iter()
                    .map(|(price, volume)| NormalizedPriceLevel {
                        price,
                        volume,
                        orders: 1, // Lbank WS doesn't provide order count
                    })
                    .collect()
            };

            let bids_norm = price_levels_to_normalized(bids);
            let asks_norm = price_levels_to_normalized(asks);

            // Determine last price from best bid/ask
            let best_bid = bids_norm.first().map(|l| l.price);
            let best_ask = asks_norm.first().map(|l| l.price);
            let last_price = best_bid.or(best_ask).unwrap_or_default();

            let symbol_clone = symbol.clone();

            Some(MarketEvent::OrderBook {
                exchange,
                symbol: symbol_clone,
                order_book: NormalizedOrderBook {
                    symbol,
                    timestamp,
                    last_price,
                    bids: bids_norm,
                    asks: asks_norm,
                    seq_id: None,
                },
                local_timestamp: chrono::Utc::now().timestamp_millis(),
            })
        }
        WsEvent::OrderUpdate { .. } => None, // Not a market event
        WsEvent::Connected => None,
        WsEvent::Disconnected => None,
        WsEvent::Error(_) => None,
    }
}
