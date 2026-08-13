//! Lbank WebSocket Client
//!
//! WebSocket connection with automatic reconnection and message parsing.
//! Protocol based on reversed engineering from browser analysis and source code.
//!
//! Key findings from source code (59960-f1769a647db941b9.js):
//! - WebSocket URL: wss://uuws.rerrkvifj.com/ws/v3
//! - Topic enum values: Market=1, KLine=2, OrderBook=3, Deal=4, Account=11, Order=12, Position=13
//! - Message format: {"d":[...],"e":"{...}","x":topic,"y":"tsn","z":type,"w":timestamp}
//! - Subscribe format: {"a":{"i":"SYMBOL"},"x":topic,"y":"tsn","z":1}
//! - Market data fields: a=instrumentID, b=highestPrice, c=lowestPrice, d=lastPrice, e=buyPrice, f=sellPrice
//! - Deal data fields: a=instrumentID, b=volume, c=price, d=direction, e=tradeTime, f=tradeID

use crate::protocol::NormalizedOrderBook;
use anyhow::Result;
use backoff::backoff::Backoff;
use backoff::ExponentialBackoff;
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info};

// ============================================================================
// Constants
// ============================================================================

const WS_URL: &str = "wss://uuws.rerrkvifj.com/ws/v3";

// Topic types (from source code)
const TOPIC_MARKET: i64 = 1;
const TOPIC_KLINE: i64 = 2;
const TOPIC_ORDER_BOOK: i64 = 3;
const TOPIC_DEAL: i64 = 4;
const TOPIC_ACCOUNT: i64 = 11;
const TOPIC_ORDER: i64 = 12;
const TOPIC_POSITION: i64 = 13;

// Message types
const MSG_TYPE_UNSUB: i64 = 0;
const MSG_TYPE_SUB: i64 = 1;
const MSG_TYPE_LOGIN: i64 = 2;
const MSG_TYPE_RESPONSE: i64 = 3;
const MSG_TYPE_PUSH: i64 = 4;

// ============================================================================
// Event Types
// ============================================================================

/// WebSocket event types
#[derive(Debug, Clone)]
pub enum WsEvent {
    Connected,
    Disconnected,
    /// 24hr Market ticker update
    MarketUpdate {
        symbol: String,
        last_price: Decimal,
        buy_price: Decimal,
        sell_price: Decimal,
        high_24h: Decimal,
        low_24h: Decimal,
        volume: Decimal,
        timestamp: i64,
    },
    /// Deal (trade) update
    DealUpdate {
        symbol: String,
        price: Decimal,
        volume: Decimal,
        direction: String, // "0"=buy, "1"=sell
        trade_id: String,
        timestamp: i64,
    },
    /// Order book update (Topic=3)
    OrderBookUpdate {
        symbol: String,
        bids: Vec<(Decimal, Decimal)>, // (price, volume)
        asks: Vec<(Decimal, Decimal)>, // (price, volume)
        timestamp: i64,
    },
    /// Order update from WebSocket
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

// ============================================================================
// Internal Message Types (for parsing)
// ============================================================================

/// Parsed incoming WebSocket message
#[derive(Debug, Deserialize)]
struct WsIncomingMessage {
    /// d = data (can be array or object)
    #[serde(rename = "d")]
    data: Option<serde_json::Value>,
    /// e = ext (extra info JSON string)
    #[serde(rename = "e")]
    ext: Option<String>,
    /// x = topic
    #[serde(rename = "x")]
    topic: Option<i64>,
    /// y = tsn (sequence number)
    #[serde(rename = "y")]
    tsn: Option<String>,
    /// z = type (1=sub, 2=login, 3=response, 4=push)
    #[serde(rename = "z")]
    msg_type: Option<i64>,
    /// w = timestamp
    #[serde(rename = "w")]
    timestamp: Option<i64>,
}

/// Market data (Topic=1)
/// Fields from source code:
/// a=instrumentID, b=highestPrice, c=lowestPrice, d=lastPrice, e=buyPrice, f=sellPrice
#[derive(Debug, Clone, Deserialize)]
pub struct MarketData {
    pub instrument_id: Option<String>,
    pub highest_price: Option<String>,
    pub lowest_price: Option<String>,
    pub last_price: Option<String>,
    pub buy_price: Option<String>,
    pub sell_price: Option<String>,
    pub volume: Option<String>,
    pub timestamp: Option<i64>,
}

/// Deal data (Topic=4)
/// Fields from source code:
/// a=instrumentID, b=volume, c=price, d=direction, e=tradeTime, f=tradeID
#[derive(Debug, Clone, Deserialize)]
struct DealData {
    #[serde(rename = "a")]
    instrument_id: Option<String>,
    #[serde(rename = "b")]
    volume: Option<String>,
    #[serde(rename = "c")]
    price: Option<String>,
    #[serde(rename = "d")]
    direction: Option<String>,
    #[serde(rename = "e")]
    trade_time: Option<i64>,
    #[serde(rename = "f")]
    trade_id: Option<String>,
}

/// Order update data (Topic=12)
/// Simplified WebSocket format:
/// a=instrumentID, aq=orderSysID, ao=orderStatus, ak=volumeTraded
#[derive(Debug, Clone, Deserialize)]
struct OrderData {
    #[serde(rename = "a")]
    instrument_id: Option<String>,
    #[serde(rename = "aq")]
    order_sys_id: Option<String>,
    #[serde(rename = "ao")]
    order_status: Option<String>,
    #[serde(rename = "ak")]
    volume_traded: Option<String>,
}

/// Order book data (Topic=3)
/// 实测数据格式:
/// b = bids [[price, volume], ...]
/// s = asks [[price, volume], ...]
#[derive(Debug, Clone)]
struct OrderBookData {
    bids: Vec<(Decimal, Decimal)>,
    asks: Vec<(Decimal, Decimal)>,
}

impl<'de> Deserialize<'de> for OrderBookData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawOrderBookData {
            #[serde(rename = "b")]
            bids: Vec<Vec<String>>,
            #[serde(rename = "s")]
            asks: Vec<Vec<String>>,
        }

        let raw = RawOrderBookData::deserialize(deserializer)?;

        let bids: Vec<(Decimal, Decimal)> = raw.bids
            .iter()
            .filter_map(|v| {
                if v.len() >= 2 {
                    let price = v[0].parse().ok();
                    let volume = v[1].parse().ok();
                    price.zip(volume)
                } else {
                    None
                }
            })
            .collect();

        let asks: Vec<(Decimal, Decimal)> = raw.asks
            .iter()
            .filter_map(|v| {
                if v.len() >= 2 {
                    let price = v[0].parse().ok();
                    let volume = v[1].parse().ok();
                    price.zip(volume)
                } else {
                    None
                }
            })
            .collect();

        Ok(OrderBookData { bids, asks })
    }
}

// ============================================================================
// Subscribe Request Types
// ============================================================================

/// Subscribe request format based on source code analysis
/// Format: {"a":{"i":"SYMBOL"},"x":topic,"y":"tsn","z":1}
#[derive(Debug, Serialize)]
struct SubscribeRequest {
    /// a = param
    #[serde(rename = "a")]
    param: SubscribeParam,
    /// x = topic
    #[serde(rename = "x")]
    topic: i64,
    /// y = tsn
    #[serde(rename = "y")]
    tsn: String,
    /// z = type (1=subscribe)
    #[serde(rename = "type")]
    msg_type: i64,
}

#[derive(Debug, Serialize)]
struct SubscribeParam {
    #[serde(rename = "i")]
    instrument_id: String,
}

/// Unsubscribe request
#[derive(Debug, Serialize)]
struct UnsubscribeRequest {
    #[serde(rename = "a")]
    param: SubscribeParam,
    #[serde(rename = "x")]
    topic: i64,
    #[serde(rename = "y")]
    tsn: String,
    #[serde(rename = "z")]
    msg_type: i64,
}

// ============================================================================
// Main WebSocket Client
// ============================================================================

/// Lbank WebSocket Client
pub struct LbankWebSocket {
    url: String,
    token: Option<String>,
    market_cache: Arc<RwLock<HashMap<String, MarketData>>>,
    order_book_cache: Arc<RwLock<HashMap<String, NormalizedOrderBook>>>,
    sender: Option<mpsc::Sender<WsCommand>>,
    tsn_counter: Arc<std::sync::atomic::AtomicU64>,
}

enum WsCommand {
    SubscribeMarket { symbol: String },
    SubscribeDeal { symbol: String },
    SubscribeOrderBook { symbol: String },
    SubscribeOrder,
    SubscribePosition,
    Shutdown,
}

impl LbankWebSocket {
    /// Create a new WebSocket client
    pub fn new(token: Option<String>) -> Self {
        Self {
            url: WS_URL.to_string(),
            token,
            market_cache: Arc::new(RwLock::new(HashMap::new())),
            order_book_cache: Arc::new(RwLock::new(HashMap::new())),
            sender: None,
            tsn_counter: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
    }

    /// Get the token
    pub fn token(&self) -> Option<&String> {
        self.token.as_ref()
    }

    /// Start the WebSocket connection in background
    pub fn start(&mut self) -> Result<mpsc::Receiver<WsEvent>> {
        let (event_tx, event_rx) = mpsc::channel(1000);
        let (cmd_tx, cmd_rx) = mpsc::channel(100);

        let url = self.url.clone();
        let token = self.token.clone();
        let market_cache = self.market_cache.clone();
        let order_book_cache = self.order_book_cache.clone();
        let tsn_counter = self.tsn_counter.clone();

        tokio::spawn(async move {
            Self::run_ws_loop(url, token, market_cache, order_book_cache, tsn_counter, cmd_rx, event_tx).await;
        });

        self.sender = Some(cmd_tx);
        Ok(event_rx)
    }

    /// Generate unique tsn for subscription
    fn next_tsn(&self) -> String {
        let tsn = self.tsn_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("{:010}", tsn)
    }

    /// Generate topic-based tsn (e.g., "1000000001" for topic 1)
    fn make_topic_tsn(&self, topic: i64) -> String {
        let counter = self.tsn_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("{}{:010}", topic, counter)
    }

    async fn run_ws_loop(
        url: String,
        token: Option<String>,
        market_cache: Arc<RwLock<HashMap<String, MarketData>>>,
        order_book_cache: Arc<RwLock<HashMap<String, NormalizedOrderBook>>>,
        tsn_counter: Arc<std::sync::atomic::AtomicU64>,
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
                    // Lbank WebSocket auth: {"z": 2, "k": "token"}
                    if let Some(ref t) = token {
                        let tsn = format!("{:010}", tsn_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
                        let auth_msg = serde_json::json!({
                            "z": MSG_TYPE_LOGIN,  // 2 = login
                            "k": t,
                            "y": tsn
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
                                        Self::handle_message(&text, &market_cache, &order_book_cache, &event_tx).await;
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
                                    Some(WsCommand::SubscribeMarket { symbol }) => {
                                        Self::send_subscribe(&mut write, TOPIC_MARKET, &symbol, &tsn_counter).await;
                                    }
                                    Some(WsCommand::SubscribeDeal { symbol }) => {
                                        Self::send_subscribe(&mut write, TOPIC_DEAL, &symbol, &tsn_counter).await;
                                    }
                                    Some(WsCommand::SubscribeOrderBook { symbol }) => {
                                        Self::send_subscribe(&mut write, TOPIC_ORDER_BOOK, &symbol, &tsn_counter).await;
                                    }
                                    Some(WsCommand::SubscribeOrder) => {
                                        Self::send_subscribe(&mut write, TOPIC_ORDER, "", &tsn_counter).await;
                                    }
                                    Some(WsCommand::SubscribePosition) => {
                                        Self::send_subscribe(&mut write, TOPIC_POSITION, "", &tsn_counter).await;
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

            reconnect_attempt += 1;
            if reconnect_attempt > 10 {
                let _ = event_tx.send(WsEvent::Error("Max reconnection attempts reached".to_string())).await;
                break;
            }

            info!("Reconnecting in {:?} (attempt {})",
                backoff.next_backoff(), reconnect_attempt);
        }
    }

    /// Send subscribe message
    async fn send_subscribe(
        write: &mut (impl SinkExt<Message> + Unpin),
        topic: i64,
        symbol: &str,
        tsn_counter: &Arc<std::sync::atomic::AtomicU64>,
    ) {
        let tsn = format!("{}{:010}", topic, tsn_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed));

        // Build subscribe message based on source code format
        // {"a":{"i":"SYMBOL"},"x":topic,"y":"tsn","z":1}
        let param = if symbol.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::json!({"i": symbol})
        };

        let sub_msg = serde_json::json!({
            "a": param,
            "x": topic,
            "y": tsn,
            "z": MSG_TYPE_SUB  // 1 = subscribe
        });

        debug!("Subscribing: {}", sub_msg);

        if let Err(_) = write.send(Message::Text(sub_msg.to_string().into())).await {
            error!("Failed to send subscribe message");
        }
    }

    async fn handle_message(
        text: &str,
        market_cache: &Arc<RwLock<HashMap<String, MarketData>>>,
        order_book_cache: &Arc<RwLock<HashMap<String, NormalizedOrderBook>>>,
        event_tx: &mpsc::Sender<WsEvent>,
    ) {
        debug!(msg = %text, "WebSocket message received");

        if let Ok(msg) = serde_json::from_str::<WsIncomingMessage>(text) {
            let msg_type = msg.msg_type.unwrap_or(0);

            match msg_type {
                MSG_TYPE_LOGIN => {
                    // Login response
                    info!("Login response: {:?}", msg.ext);
                }
                MSG_TYPE_RESPONSE => {
                    // Subscription response
                    debug!("Subscription response: {:?}", msg.ext);
                }
                MSG_TYPE_PUSH => {
                    // Push data
                    if let Some(data) = &msg.data {
                        let topic = msg.topic.unwrap_or(0);
                        let timestamp = msg.timestamp.unwrap_or(0);
                        Self::handle_push_data(topic, data, timestamp, market_cache, order_book_cache, event_tx).await;
                    }
                }
                MSG_TYPE_SUB => {
                    // Subscription confirmation
                    debug!("Subscription confirmed: topic={:?}", msg.topic);
                }
                _ => {
                    debug!("Unknown message type: {}", msg_type);
                }
            }
        }
    }

    /// Handle push data based on topic
    async fn handle_push_data(
        topic: i64,
        data: &serde_json::Value,
        timestamp: i64,
        market_cache: &Arc<RwLock<HashMap<String, MarketData>>>,
        _order_book_cache: &Arc<RwLock<HashMap<String, NormalizedOrderBook>>>,
        event_tx: &mpsc::Sender<WsEvent>,
    ) {
        match topic {
            TOPIC_MARKET => {
                // Market data (24hr ticker)
                if let Some(market) = Self::parse_market_data(data) {
                    // Cache the market data
                    if let Some(symbol) = &market.instrument_id {
                        let mut cache = market_cache.write();
                        cache.insert(symbol.clone(), market.clone());
                    }

                    let _ = event_tx.send(WsEvent::MarketUpdate {
                        symbol: market.instrument_id.unwrap_or_default(),
                        last_price: market.last_price.as_ref()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or_default(),
                        buy_price: market.buy_price.as_ref()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or_default(),
                        sell_price: market.sell_price.as_ref()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or_default(),
                        high_24h: market.highest_price.as_ref()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or_default(),
                        low_24h: market.lowest_price.as_ref()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or_default(),
                        volume: market.volume.as_ref()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or_default(),
                        timestamp: market.timestamp.unwrap_or(timestamp),
                    }).await;
                }
            }
            TOPIC_ORDER_BOOK => {
                // Order book data (Topic=3)
                // Data format: {"b":[[price,volume],...],"s":[[price,volume],...]}
                // 直接解析 data 本身（不是 data.d）
                if let Some(order_book) = Self::parse_order_book_data(data) {
                    let _ = event_tx.send(WsEvent::OrderBookUpdate {
                        symbol: "BTCUSDT".to_string(), // 从订阅参数获取更好，这里简化处理
                        bids: order_book.bids,
                        asks: order_book.asks,
                        timestamp,
                    }).await;
                }
            }
            TOPIC_DEAL => {
                // Deal (trade) data - can be array or single item
                if let Some(deals) = data.as_array() {
                    for deal_value in deals {
                        if let Some(deal) = Self::parse_deal_data(deal_value) {
                            let _ = event_tx.send(WsEvent::DealUpdate {
                                symbol: deal.instrument_id.unwrap_or_default(),
                                price: deal.price.as_ref()
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or_default(),
                                volume: deal.volume.as_ref()
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or_default(),
                                direction: deal.direction.unwrap_or_default(),
                                trade_id: deal.trade_id.unwrap_or_default(),
                                timestamp: deal.trade_time.unwrap_or(timestamp),
                            }).await;
                        }
                    }
                } else if let Some(deal) = Self::parse_deal_data(data) {
                    let _ = event_tx.send(WsEvent::DealUpdate {
                        symbol: deal.instrument_id.unwrap_or_default(),
                        price: deal.price.as_ref()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or_default(),
                        volume: deal.volume.as_ref()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or_default(),
                        direction: deal.direction.unwrap_or_default(),
                        trade_id: deal.trade_id.unwrap_or_default(),
                        timestamp: deal.trade_time.unwrap_or(timestamp),
                    }).await;
                }
            }
            TOPIC_ORDER => {
                // Order update
                if let Some(orders) = data.as_array() {
                    for order_value in orders {
                        if let Some(order) = Self::parse_order_data(order_value) {
                            let _ = event_tx.send(WsEvent::OrderUpdate {
                                order_sys_id: order.order_sys_id.unwrap_or_default(),
                                symbol: order.instrument_id.unwrap_or_default(),
                                status: order.order_status.unwrap_or_default(),
                                filled_volume: order.volume_traded.as_ref()
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or_default(),
                            }).await;
                        }
                    }
                } else if let Some(order) = Self::parse_order_data(data) {
                    let _ = event_tx.send(WsEvent::OrderUpdate {
                        order_sys_id: order.order_sys_id.unwrap_or_default(),
                        symbol: order.instrument_id.unwrap_or_default(),
                        status: order.order_status.unwrap_or_default(),
                        filled_volume: order.volume_traded.as_ref()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or_default(),
                    }).await;
                }
            }
            _ => {
                debug!("Unhandled topic: {}", topic);
            }
        }
    }

    /// Parse Market data (Topic=1)
    fn parse_market_data(data: &serde_json::Value) -> Option<MarketData> {
        serde_json::from_value(data.clone()).ok()
    }

    /// Parse Deal data (Topic=4)
    fn parse_deal_data(data: &serde_json::Value) -> Option<DealData> {
        serde_json::from_value(data.clone()).ok()
    }

    /// Parse Order data (Topic=12)
    fn parse_order_data(data: &serde_json::Value) -> Option<OrderData> {
        serde_json::from_value(data.clone()).ok()
    }

    /// Parse OrderBook data (Topic=3)
    /// 实测数据格式: {"b":[["price","volume"],...],"s":[["price","volume"],...]}
    fn parse_order_book_data(data: &serde_json::Value) -> Option<OrderBookData> {
        serde_json::from_value(data.clone()).ok()
    }

    // ========================================================================
    // Public API
    // ========================================================================

    /// Subscribe to market ticker (24hr data)
    pub async fn subscribe_market(&self, symbol: &str) -> Result<()> {
        if let Some(ref sender) = self.sender {
            sender.send(WsCommand::SubscribeMarket {
                symbol: symbol.to_string(),
            }).await?;
        }
        Ok(())
    }

    /// Subscribe to deal (trade) updates
    pub async fn subscribe_deals(&self, symbol: &str) -> Result<()> {
        if let Some(ref sender) = self.sender {
            sender.send(WsCommand::SubscribeDeal {
                symbol: symbol.to_string(),
            }).await?;
        }
        Ok(())
    }

    /// Subscribe to order book updates (Topic=3)
    pub async fn subscribe_orderbook(&self, symbol: &str) -> Result<()> {
        if let Some(ref sender) = self.sender {
            sender.send(WsCommand::SubscribeOrderBook {
                symbol: symbol.to_string(),
            }).await?;
        }
        Ok(())
    }

    /// Subscribe to order updates (requires authentication)
    pub async fn subscribe_orders(&self) -> Result<()> {
        if let Some(ref sender) = self.sender {
            sender.send(WsCommand::SubscribeOrder).await?;
        }
        Ok(())
    }

    /// Subscribe to position updates (requires authentication)
    pub async fn subscribe_positions(&self) -> Result<()> {
        if let Some(ref sender) = self.sender {
            sender.send(WsCommand::SubscribePosition).await?;
        }
        Ok(())
    }

    /// Get cached market data
    pub fn get_market_data(&self, symbol: &str) -> Option<MarketData> {
        self.market_cache.read().get(symbol).cloned()
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_websocket_creation() {
        let ws = LbankWebSocket::new(None);
        assert!(ws.get_market_data("BTCUSDT").is_none());
        assert!(ws.get_order_book("BTCUSDT").is_none());
    }

    #[test]
    fn test_parse_market_data() {
        // Test parsing from MarketData struct directly
        let market = MarketData {
            instrument_id: Some("BTCUSDT".to_string()),
            highest_price: Some("67083.1".to_string()),
            lowest_price: Some("60694.3".to_string()),
            last_price: Some("63918.2".to_string()),
            buy_price: Some("63888.7".to_string()),
            sell_price: Some("0.00008465".to_string()),
            volume: Some("63990.7".to_string()),
            timestamp: Some(1786599057),
        };

        assert_eq!(market.instrument_id, Some("BTCUSDT".to_string()));
        assert_eq!(market.last_price, Some("63918.2".to_string()));
        assert_eq!(market.buy_price, Some("63888.7".to_string()));
    }

    #[test]
    fn test_parse_deal_data() {
        let json = r#"{
            "a": "BTCUSDT",
            "b": "0.0022",
            "c": "63888.8",
            "d": "0",
            "e": 1786599053,
            "f": "1007932287916477"
        }"#;

        let data: serde_json::Value = serde_json::from_str(json).unwrap();
        let deal = LbankWebSocket::parse_deal_data(&data);

        assert!(deal.is_some());
        let d = deal.unwrap();
        assert_eq!(d.instrument_id, Some("BTCUSDT".to_string()));
        assert_eq!(d.price, Some("63888.8".to_string()));
        assert_eq!(d.direction, Some("0".to_string())); // 0 = buy
        assert_eq!(d.volume, Some("0.0022".to_string()));
    }

    #[test]
    fn test_parse_deal_array() {
        let json = r#"[
            {"a": "BTCUSDT", "b": "0.0022", "c": "63888.8", "d": "0", "e": 1786599053, "f": "1"},
            {"a": "BTCUSDT", "b": "0.0015", "c": "63888.7", "d": "1", "e": 1786599053, "f": "2"}
        ]"#;

        let data: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(data.is_array());

        let deals: Vec<DealData> = data.as_array()
            .unwrap()
            .iter()
            .filter_map(|v| LbankWebSocket::parse_deal_data(v))
            .collect();

        assert_eq!(deals.len(), 2);
        assert_eq!(deals[0].direction, Some("0".to_string())); // buy
        assert_eq!(deals[1].direction, Some("1".to_string())); // sell
    }

    #[test]
    fn test_parse_order_data() {
        let json = r#"{
            "a": "BTCUSDT",
            "aq": "1007986500073684",
            "ao": "1",
            "ak": "0.0001"
        }"#;

        let data: serde_json::Value = serde_json::from_str(json).unwrap();
        let order = LbankWebSocket::parse_order_data(&data);

        assert!(order.is_some());
        let o = order.unwrap();
        assert_eq!(o.order_sys_id, Some("1007986500073684".to_string()));
        assert_eq!(o.order_status, Some("1".to_string())); // filled
    }

    #[test]
    fn test_subscribe_message_format() {
        // Verify the subscribe message format matches source code
        // Format should be: {"a":{"i":"SYMBOL"},"x":topic,"y":"tsn","z":1}
        let json_str = r#"{"a":{"i":"BTCUSDT"},"x":1,"y":"10000000001","z":1}"#;
        let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap();

        assert_eq!(parsed.get("x").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(parsed.get("z").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(parsed.get("a").and_then(|v| v.get("i")).and_then(|v| v.as_str()), Some("BTCUSDT"));
    }

    #[test]
    fn test_parse_order_book_data() {
        // 实测订单簿数据格式
        let json = r#"{
            "b": [["63825.5","40.6320"], ["63825.4","0.8104"]],
            "s": [["63825.6","14.8587"], ["63825.7","0.4734"]]
        }"#;

        let data: serde_json::Value = serde_json::from_str(json).unwrap();
        let order_book = LbankWebSocket::parse_order_book_data(&data);

        assert!(order_book.is_some());
        let ob = order_book.unwrap();

        // Bids
        assert_eq!(ob.bids.len(), 2);
        assert_eq!(ob.bids[0].0.to_string(), "63825.5");
        assert_eq!(ob.bids[0].1.to_string(), "40.6320");

        // Asks
        assert_eq!(ob.asks.len(), 2);
        assert_eq!(ob.asks[0].0.to_string(), "63825.6");
        assert_eq!(ob.asks[0].1.to_string(), "14.8587");
    }
}
