//! Exchange Gateway Connection Manager
//!
//! Manages single exchange WebSocket connection with auto-reconnect.

use anyhow::Result;
use backoff::backoff::Backoff;
use backoff::ExponentialBackoff;
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Authenticating,
    Connected,
    Reconnecting,
}

/// Gateway configuration
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub name: String,
    pub ws_url: String,
    pub auth_message: Option<String>,
    pub subscriptions: Vec<String>,
    pub ping_interval: Duration,
    pub reconnect_delay: Duration,
    pub max_reconnect_attempts: usize,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            name: "unknown".to_string(),
            ws_url: "wss://example.com/ws".to_string(),
            auth_message: None,
            subscriptions: Vec::new(),
            ping_interval: Duration::from_secs(20),
            reconnect_delay: Duration::from_secs(1),
            max_reconnect_attempts: 100,
        }
    }
}

/// Raw market message with metadata
#[derive(Debug, Clone)]
pub struct RawMessage {
    pub exchange: String,
    pub topic: String,
    pub data: Vec<u8>,
    pub local_timestamp: i64,
    pub remote_timestamp: Option<i64>,
    pub seq_id: Option<u64>,
}

impl RawMessage {
    pub fn parse_json<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_slice(&self.data).ok()
    }
}

/// Connection event
#[derive(Debug)]
pub enum ConnectionEvent {
    Connected,
    Disconnected,
    Subscribed { topic: String },
    Message(RawMessage),
    Error(String),
}

/// Exchange gateway managing a single WebSocket connection
pub struct ExchangeGateway {
    config: GatewayConfig,
    state: Arc<RwLock<ConnectionState>>,
    last_message: Arc<RwLock<Instant>>,
    reconnect_count: Arc<RwLock<usize>>,
    subscriptions: Arc<RwLock<HashMap<String, bool>>>,
    sender: Option<mpsc::Sender<GatewayCommand>>,
}

enum GatewayCommand {
    Subscribe { topic: String },
    Unsubscribe { topic: String },
    Send { data: String },
    Shutdown,
}

impl ExchangeGateway {
    /// Create a new gateway
    pub fn new(config: GatewayConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(ConnectionState::Disconnected)),
            last_message: Arc::new(RwLock::new(Instant::now())),
            reconnect_count: Arc::new(RwLock::new(0)),
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            sender: None,
        }
    }

    /// Start the gateway (spawns background task)
    pub fn start(&mut self) -> mpsc::Receiver<ConnectionEvent> {
        let (event_tx, event_rx) = mpsc::channel(1000);
        let (cmd_tx, cmd_rx) = mpsc::channel(100);

        let config = self.config.clone();

        // Spawn connection task
        tokio::spawn(async move {
            run_gateway(config, cmd_rx, event_tx).await;
        });

        self.sender = Some(cmd_tx);
        event_rx
    }

    /// Get current connection state
    pub fn state(&self) -> ConnectionState {
        *self.state.read()
    }

    /// Get time since last message
    pub fn last_message_age(&self) -> Duration {
        Instant::now() - *self.last_message.read()
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        *self.state.read() == ConnectionState::Connected
    }

    /// Subscribe to a topic
    pub async fn subscribe(&self, topic: String) -> Result<()> {
        if let Some(ref sender) = self.sender {
            sender.send(GatewayCommand::Subscribe { topic }).await?;
        }
        Ok(())
    }

    /// Unsubscribe from a topic
    pub async fn unsubscribe(&self, topic: String) -> Result<()> {
        if let Some(ref sender) = self.sender {
            sender.send(GatewayCommand::Unsubscribe { topic }).await?;
        }
        Ok(())
    }

    /// Shutdown the gateway
    pub async fn shutdown(&mut self) {
        if let Some(ref sender) = self.sender {
            let _ = sender.send(GatewayCommand::Shutdown).await;
        }
        *self.state.write() = ConnectionState::Disconnected;
    }
}

/// Main gateway run loop
async fn run_gateway(
    config: GatewayConfig,
    mut cmd_rx: mpsc::Receiver<GatewayCommand>,
    event_tx: mpsc::Sender<ConnectionEvent>,
) {
    let exchange_name = config.name.clone();
    let mut backoff = ExponentialBackoff {
        initial_interval: config.reconnect_delay,
        max_interval: Duration::from_secs(30),
        max_elapsed_time: None, // Keep trying forever
        ..Default::default()
    };

    let mut reconnect_attempt = 0;

    loop {
        *get_state_for_exchange(&exchange_name).write() = ConnectionState::Connecting;
        let _ = event_tx.send(ConnectionEvent::Disconnected).await;

        info!(exchange = %exchange_name, "Connecting to WebSocket");

        match connect_async(&config.ws_url).await {
            Ok((ws_stream, _)) => {
                info!(exchange = %exchange_name, "WebSocket connected");
                *get_state_for_exchange(&exchange_name).write() = ConnectionState::Connected;
                reconnect_attempt = 0;
                let _ = event_tx.send(ConnectionEvent::Connected).await;

                let (mut write, mut read) = ws_stream.split();

                // Authenticate if configured
                if let Some(ref auth_msg) = config.auth_message {
                    *get_state_for_exchange(&exchange_name).write() = ConnectionState::Authenticating;
                    if let Err(e) = write.send(Message::Text(auth_msg.clone().into())).await {
                        error!(exchange = %exchange_name, error = %e, "Auth failed");
                        continue;
                    }
                    *get_state_for_exchange(&exchange_name).write() = ConnectionState::Connected;
                }

                // Subscribe to configured topics
                for topic in &config.subscriptions {
                    let sub_msg = serde_json::json!({
                        "type": "subscribe",
                        "topic": topic
                    }).to_string();
                    if let Err(e) = write.send(Message::Text(sub_msg.into())).await {
                        error!(exchange = %exchange_name, topic = %topic, error = %e, "Subscribe failed");
                    } else {
                        let _ = event_tx.send(ConnectionEvent::Subscribed { topic: topic.clone() }).await;
                    }
                }

                // Ping interval
                let mut ping_interval = tokio::time::interval(config.ping_interval);

                let mut running = true;
                while running {
                    tokio::select! {
                        // Ping
                        _ = ping_interval.tick() => {
                            if let Err(e) = write.send(Message::Ping(vec![].into())).await {
                                error!(exchange = %exchange_name, error = %e, "Ping failed");
                                running = false;
                            }
                        }

                        // Incoming message
                        msg = read.next() => {
                            match msg {
                                Some(Ok(Message::Text(text))) => {
                                    let now = Instant::now();
                                    // Process message
                                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                        let topic = json.get("table")
                                            .or(json.get("topic"))
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("unknown")
                                            .to_string();

                                        let remote_ts = json.get("timestamp").and_then(|v| v.as_i64());

                                        let raw = RawMessage {
                                            exchange: exchange_name.clone(),
                                            topic,
                                            data: text.into_bytes(),
                                            local_timestamp: chrono::Utc::now().timestamp_millis(),
                                            remote_timestamp: remote_ts,
                                            seq_id: None,
                                        };
                                        let _ = event_tx.send(ConnectionEvent::Message(raw)).await;
                                    }
                                }
                                Some(Ok(Message::Pong(_))) => {
                                    debug!(exchange = %exchange_name, "Pong received");
                                }
                                Some(Ok(Message::Close(reason))) => {
                                    info!(exchange = %exchange_name, ?reason, "Connection closed");
                                    running = false;
                                }
                                Some(Err(e)) => {
                                    error!(exchange = %exchange_name, error = %e, "WebSocket error");
                                    running = false;
                                }
                                None => {
                                    info!(exchange = %exchange_name, "Connection ended");
                                    running = false;
                                }
                                _ => {}
                            }
                        }

                        // Commands
                        cmd = cmd_rx.recv() => {
                            match cmd {
                                Some(GatewayCommand::Subscribe { topic }) => {
                                    let sub_msg = serde_json::json!({
                                        "type": "subscribe",
                                        "topic": topic.clone()
                                    }).to_string();
                                    if let Err(e) = write.send(Message::Text(sub_msg.into())).await {
                                        error!(exchange = %exchange_name, topic = %topic, error = %e, "Subscribe failed");
                                    } else {
                                        let _ = event_tx.send(ConnectionEvent::Subscribed { topic }).await;
                                    }
                                }
                                Some(GatewayCommand::Unsubscribe { topic }) => {
                                    let unsub_msg = serde_json::json!({
                                        "type": "unsubscribe",
                                        "topic": topic.clone()
                                    }).to_string();
                                    if let Err(e) = write.send(Message::Text(unsub_msg.into())).await {
                                        error!(exchange = %exchange_name, topic = %topic, error = %e, "Unsubscribe failed");
                                    }
                                }
                                Some(GatewayCommand::Send { data }) => {
                                    if let Err(e) = write.send(Message::Text(data.into())).await {
                                        error!(exchange = %exchange_name, error = %e, "Send failed");
                                    }
                                }
                                Some(GatewayCommand::Shutdown) | None => {
                                    info!(exchange = %exchange_name, "Shutdown requested");
                                    let _ = write.close().await;
                                    running = false;
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                error!(exchange = %exchange_name, error = %e, "Connection failed");
            }
        }

        *get_state_for_exchange(&exchange_name).write() = ConnectionState::Disconnected;
        let _ = event_tx.send(ConnectionEvent::Disconnected).await;

        // Reconnect with backoff
        reconnect_attempt += 1;
        if reconnect_attempt > config.max_reconnect_attempts {
            let msg = format!("Max reconnect attempts ({}) reached", config.max_reconnect_attempts);
            error!(exchange = %exchange_name, msg = %msg);
            let _ = event_tx.send(ConnectionEvent::Error(msg)).await;
            break;
        }

        let delay = backoff.next_backoff().unwrap_or(Duration::from_secs(1));
        info!(
            exchange = %exchange_name,
            attempt = reconnect_attempt,
            "Reconnecting in {:?}",
            delay
        );

        tokio::time::sleep(delay).await;
    }
}

// Global state storage (simplified for this example)
// In production, this would be managed properly
fn get_state_for_exchange(_name: &str) -> Arc<RwLock<ConnectionState>> {
    Arc::new(RwLock::new(ConnectionState::Disconnected))
}
