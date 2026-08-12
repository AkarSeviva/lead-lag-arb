//! Message Router
//!
//! Routes incoming messages to appropriate handlers based on topic.

use crate::message::RawMarketMessage;
use crate::connection::ConnectionEvent;
use anyhow::Result;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, trace};

/// Message handler trait
pub trait MessageHandler: Send + Sync {
    fn handle(&self, msg: RawMarketMessage);
}

/// Router for distributing messages to handlers
pub struct MessageRouter {
    handlers: Arc<RwLock<HashMap<String, Box<dyn MessageHandler>>>>,
    event_rx: Option<mpsc::Receiver<ConnectionEvent>>,
}

impl MessageRouter {
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(RwLock::new(HashMap::new())),
            event_rx: None,
        }
    }

    /// Start the router with event receiver
    pub fn start(&mut self, event_rx: mpsc::Receiver<ConnectionEvent>) {
        self.event_rx = Some(event_rx);
    }

    /// Register a handler for a topic pattern
    pub fn register<H>(&self, topic_pattern: &str, handler: H)
    where
        H: MessageHandler + 'static,
    {
        let mut handlers = self.handlers.write();
        handlers.insert(topic_pattern.to_string(), Box::new(handler));
    }

    /// Route a message to appropriate handlers
    pub fn route(&self, msg: &RawMarketMessage) {
        let handlers = self.handlers.read();

        for (pattern, handler) in handlers.iter() {
            if Self::matches(&msg.topic, pattern) {
                debug!(topic = %msg.topic, pattern = %pattern, "Routing message");
                handler.handle(msg.clone());
            }
        }
    }

    /// Check if topic matches pattern
    fn matches(topic: &str, pattern: &str) -> bool {
        if pattern.ends_with('*') {
            topic.starts_with(&pattern[..pattern.len()-1])
        } else {
            topic == pattern
        }
    }
}

impl Default for MessageRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Broadcast router that sends to all handlers
pub struct BroadcastRouter {
    handlers: Arc<RwLock<Vec<Box<dyn MessageHandler>>>>,
}

impl BroadcastRouter {
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn register<H>(&self, handler: H)
    where
        H: MessageHandler + 'static,
    {
        let mut handlers = self.handlers.write();
        handlers.push(Box::new(handler));
    }

    pub fn broadcast(&self, msg: &RawMarketMessage) {
        let handlers = self.handlers.read();
        for handler in handlers.iter() {
            handler.handle(msg.clone());
        }
    }
}

impl Default for BroadcastRouter {
    fn default() -> Self {
        Self::new()
    }
}
