//! Raw Market Message Types

use serde::{Deserialize, Serialize};

/// Raw message from WebSocket with parsing support
#[derive(Debug, Clone)]
pub struct RawMarketMessage {
    /// Exchange identifier
    pub exchange: String,
    /// Message topic/table
    pub topic: String,
    /// Raw JSON bytes
    pub data: Vec<u8>,
    /// Local receive timestamp (milliseconds)
    pub local_ts: i64,
    /// Remote timestamp if available (milliseconds)
    pub remote_ts: Option<i64>,
    /// Sequence ID if available
    pub seq_id: Option<u64>,
    /// Message type
    pub msg_type: String,
}

impl RawMarketMessage {
    /// Parse as generic JSON value
    pub fn parse_json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.data).ok()
    }

    /// Parse as specific type
    pub fn parse<T: for<'de> Deserialize<'de>>(&self) -> Option<T> {
        serde_json::from_slice(&self.data).ok()
    }

    /// Check if message is a subscription confirmation
    pub fn is_subscription_ack(&self) -> bool {
        self.msg_type == "subscribe" || self.msg_type == "unsubscribe"
    }

    /// Check if message is a ping
    pub fn is_ping(&self) -> bool {
        self.msg_type == "ping"
    }
}

/// Generic WS message wrapper
#[derive(Debug, Deserialize)]
pub struct WsMessage {
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    #[serde(rename = "table")]
    pub table: Option<String>,
    #[serde(rename = "topic")]
    pub topic: Option<String>,
    #[serde(rename = "data")]
    pub data: Option<serde_json::Value>,
    #[serde(rename = "timestamp")]
    pub timestamp: Option<i64>,
}

impl WsMessage {
    pub fn topic(&self) -> &str {
        self.table.as_deref()
            .or(self.topic.as_deref())
            .unwrap_or("unknown")
    }
}
