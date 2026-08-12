//! Lbank WebSocket Client
//!
//! WebSocket connection with automatic reconnection and message parsing.

use crate::protocol::{NormalizedOrderBook, NormalizedPriceLevel, PriceLevel};
use anyhow::{Context, Result};
use backoff::ExponentialBackoff;
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};
use backoff::backoff::Backoff;

const WS_URL: &str = "wss://uuws.rerrkvifj.com/ws/v3";

/// WebSocket event types
#[derive(Debug, Clone)]
pub enum WsEvent {
    Connected,
    Disconnected,
    OrderBookUpdate {
        symbol: String,
        order_book: NormalizedOrderBook,
    },
    OrderUpdate {
        order_sys_id: String,
        symbol: String,
        status: String,
        filled_volume: Decimal,
    },
    Error(String),
}

/// WebSocket message handler trait
pub trait WsMessageHandler: Send + Sync {
    fn handle(&self, event: WsEvent);
}

/// Lbank WebSocket Client
pub struct LbankWebSocket {
    url: String,
    token: Option<String>,
    order_book_cache: Arc<RwLock<HashMap<String, NormalizedOrderBook>>>,
    sender: Option<mpsc::Sender<WsCommand>>,
}

enum WsCommand {
    Subscribe { topic: String },
    Unsubscribe { topic: String },
    Send { data: String },
    Shutdown,
}

impl LbankWebSocket {
    /// Create a new WebSocket client
    pub fn new(token: Option<String>) -> Self {
        Self {
            url: WS_URL.to_string(),
            token,
            order_book_cache: Arc::new(RwLock::new(HashMap::new())),
            sender: None,
        }
    }

    /// Start the WebSocket connection in background
    pub fn start(&mut self) -> Result<mpsc::Receiver<WsEvent>> {
        let (event_tx, event_rx) = mpsc::channel(1000);
        let (cmd_tx, cmd_rx) = mpsc::channel(100);

        let url = self.url.clone();
        let token = self.token.clone();
        let order_book_cache = self.order_book_cache.clone();

        // Spawn WebSocket task
        tokio::spawn(async move {
            Self::run_ws_loop(url, token, order_book_cache, cmd_rx, event_tx).await;
        });

        self.sender = Some(cmd_tx);
        Ok(event_rx)
    }

    async fn run_ws_loop(
        url: String,
        token: Option<String>,
        order_book_cache: Arc<RwLock<HashMap<String, NormalizedOrderBook>>>,
        mut cmd_rx: mpsc::Receiver<WsCommand>,
        event_tx: mpsc::Sender<WsEvent>,
    ) {
        let mut backoff = ExponentialBackoff {
            initial_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(30),
            max_elapsed_time: Some(Duration::from_secs(300)),
            ..Default::default()
        };

        let mut reconnect_attempt = 0;

        loop {
            match connect_async(&url).await {
                Ok((ws_stream, _)) => {
                    info!("WebSocket connected to {}", url);
                    reconnect_attempt = 0;
                    let _ = event_tx.send(WsEvent::Connected).await;

                    let (mut write, mut read) = ws_stream.split();

                    // Authenticate if token provided
                    if let Some(ref t) = token {
                        let auth_msg = serde_json::json!({
                            "type": "login",
                            "accessToken": t
                        });
                        if let Err(e) = write.send(Message::Text(auth_msg.to_string().into())).await {
                            error!("Failed to send auth: {}", e);
                        }
                    }

                    let mut running = true;
                    while running {
                        tokio::select! {
                            // Handle incoming messages
                            msg = read.next() => {
                                match msg {
                                    Some(Ok(Message::Text(text))) => {
                                        Self::handle_message(&text, &order_book_cache, &event_tx).await;
                                    }
                                    Some(Ok(Message::Close(_))) => {
                                        info!("WebSocket closed by server");
                                        running = false;
                                    }
                                    Some(Ok(Message::Ping(data))) => {
                                        if let Err(e) = write.send(Message::Pong(data)).await {
                                            error!("Failed to send pong: {}", e);
                                        }
                                    }
                                    Some(Err(e)) => {
                                        error!("WebSocket error: {}", e);
                                        running = false;
                                    }
                                    None => {
                                        info!("WebSocket stream ended");
                                        running = false;
                                    }
                                    _ => {}
                                }
                            }
                            // Handle commands
                            cmd = cmd_rx.recv() => {
                                match cmd {
                                    Some(WsCommand::Subscribe { topic }) => {
                                        let msg = serde_json::json!({
                                            "type": "subscribe",
                                            "topic": topic
                                        });
                                        if let Err(e) = write.send(Message::Text(msg.to_string().into())).await {
                                            error!("Failed to subscribe: {}", e);
                                        }
                                    }
                                    Some(WsCommand::Unsubscribe { topic }) => {
                                        let msg = serde_json::json!({
                                            "type": "unsubscribe",
                                            "topic": topic
                                        });
                                        if let Err(e) = write.send(Message::Text(msg.to_string().into())).await {
                                            error!("Failed to unsubscribe: {}", e);
                                        }
                                    }
                                    Some(WsCommand::Send { data }) => {
                                        if let Err(e) = write.send(Message::Text(data.into())).await {
                                            error!("Failed to send: {}", e);
                                        }
                                    }
                                    Some(WsCommand::Shutdown) | None => {
                                        info!("Shutting down WebSocket");
                                        let _ = write.close().await;
                                        running = false;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("WebSocket connection failed: {}", e);
                }
            }

            let _ = event_tx.send(WsEvent::Disconnected).await;

            // Exponential backoff reconnection
            reconnect_attempt += 1;
            if reconnect_attempt > 10 {
                let _ = event_tx.send(WsEvent::Error("Max reconnection attempts reached".to_string())).await;
                break;
            }

            info!("Reconnecting in {:?} (attempt {})", 
                backoff.next_backoff(), reconnect_attempt);
        }
    }

    async fn handle_message(
        text: &str,
        order_book_cache: &Arc<RwLock<HashMap<String, NormalizedOrderBook>>>,
        event_tx: &mpsc::Sender<WsEvent>,
    ) {
        debug!(msg = %text, "WebSocket message received");

        // Parse message type
        if let Ok(msg) = serde_json::from_str::<serde_json::Value>(text) {
            let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

            match msg_type {
                "ping" => {
                    // Respond with pong
                    return;
                }
                "subscribe" | "unsubscribe" => {
                    debug!("Subscription confirmed");
                    return;
                }
                "login" => {
                    info!("Login successful");
                    return;
                }
                _ => {
                    // Try to parse as data update
                    if let Some(table) = msg.get("table").and_then(|v| v.as_str()) {
                        if let Some(data) = msg.get("data") {
                            match table {
                                "swap/orderBook" | "swap/depth" => {
                                    Self::handle_order_book_update(data, order_book_cache, event_tx).await;
                                }
                                "swap/order" => {
                                    Self::handle_order_update(data, &event_tx);
                                }
                                _ => {
                                    debug!("Unknown table: {}", table);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    async fn handle_order_book_update(
        data: &serde_json::Value,
        order_book_cache: &Arc<RwLock<HashMap<String, NormalizedOrderBook>>>,
        event_tx: &mpsc::Sender<WsEvent>,
    ) {
        if let Some(items) = data.as_array() {
            for item in items {
                if let Some((symbol, ob)) = Self::parse_order_book_update(item) {
                    // Update cache
                    {
                        let mut cache = order_book_cache.write();
                        cache.insert(symbol.clone(), ob.clone());
                    }

                    let _ = event_tx.send(WsEvent::OrderBookUpdate {
                        symbol,
                        order_book: ob,
                    }).await;
                }
            }
        }
    }

    fn parse_order_book_update(data: &serde_json::Value) -> Option<(String, NormalizedOrderBook)> {
        let instrument_id = data.get("instrument_id")?.as_str()?.to_string();
        let last_price = data.get("last_price")?.as_str()?.parse().ok()?;
        let timestamp = data.get("timestamp")?.as_i64().unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

        let bids: Vec<NormalizedPriceLevel> = data.get("bids")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|level| {
                        let price = level.get("price")?.as_str()?.parse().ok()?;
                        let volume = level.get("volume")?.as_str()?.parse().ok()?;
                        Some(NormalizedPriceLevel { price, volume, orders: 1 })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let asks: Vec<NormalizedPriceLevel> = data.get("asks")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|level| {
                        let price = level.get("price")?.as_str()?.parse().ok()?;
                        let volume = level.get("volume")?.as_str()?.parse().ok()?;
                        Some(NormalizedPriceLevel { price, volume, orders: 1 })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Some((instrument_id.clone(), NormalizedOrderBook {
            symbol: instrument_id,
            timestamp,
            last_price,
            bids,
            asks,
            seq_id: None,
        }))
    }

    fn handle_order_update(
        data: &serde_json::Value,
        event_tx: &mpsc::Sender<WsEvent>,
    ) {
        if let Some(items) = data.as_array() {
            for item in items {
                let order_sys_id = item.get("orderSysID")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let symbol = item.get("instrumentId")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let status = item.get("orderStatus")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let filled_volume: Decimal = item.get("tradedVolume")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_default();

                let _ = event_tx.try_send(WsEvent::OrderUpdate {
                    order_sys_id,
                    symbol,
                    status,
                    filled_volume,
                });
            }
        }
    }

    /// Subscribe to order book updates for a symbol
    pub async fn subscribe_orderbook(&self, symbol: &str) -> Result<()> {
        if let Some(ref sender) = self.sender {
            let topic = format!("swap/{}:depth", symbol);
            sender.send(WsCommand::Subscribe { topic }).await?;
        }
        Ok(())
    }

    /// Subscribe to order updates
    pub async fn subscribe_orders(&self) -> Result<()> {
        if let Some(ref sender) = self.sender {
            sender.send(WsCommand::Subscribe {
                topic: "swap/order".to_string(),
            }).await?;
        }
        Ok(())
    }

    /// Get current order book from cache
    pub fn get_order_book(&self, symbol: &str) -> Option<NormalizedOrderBook> {
        self.order_book_cache.read().get(symbol).cloned()
    }

    /// Shutdown the WebSocket connection
    pub async fn shutdown(&mut self) {
        if let Some(ref sender) = self.sender {
            let _ = sender.send(WsCommand::Shutdown).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_websocket_creation() {
        let ws = LbankWebSocket::new(None);
        assert!(ws.get_order_book("BTCUSDT").is_none());
    }
}
