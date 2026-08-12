//! Lbank Order Management
//!
//! Handles order lifecycle, state machine, and order tracking.

use crate::client::LbankClient;
use crate::protocol::{OffsetFlag, OrderInsertResponse, TradeDirection};
use anyhow::{Context, Result};
use parking_lot::RwLock;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Order state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderState {
    Pending,
    Submitted,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
    Unknown,
}

impl From<&str> for OrderState {
    fn from(s: &str) -> Self {
        match s {
            "0" | "submitting" => Self::Pending,
            "1" | "submitted" | "new" => Self::Submitted,
            "2" | "partial_fill" => Self::PartiallyFilled,
            "3" | "filled" | "complete" => Self::Filled,
            "4" | "cancelled" | "canceled" => Self::Cancelled,
            "5" | "rejected" | "error" => Self::Rejected,
            _ => Self::Unknown,
        }
    }
}

/// Active order tracking
#[derive(Debug, Clone)]
pub struct TrackedOrder {
    pub client_order_id: String,
    pub exchange_order_id: Option<String>,
    pub symbol: String,
    pub side: TradeDirection,
    pub offset: OffsetFlag,
    pub volume: Decimal,
    pub filled_volume: Decimal,
    pub avg_price: Option<Decimal>,
    pub state: OrderState,
    pub created_at: Instant,
    pub last_update: Instant,
}

impl TrackedOrder {
    pub fn remaining_volume(&self) -> Decimal {
        self.volume - self.filled_volume
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, OrderState::Pending | OrderState::Submitted | OrderState::PartiallyFilled)
    }
}

/// Order update event
#[derive(Debug, Clone)]
pub struct OrderEvent {
    pub client_order_id: String,
    pub exchange_order_id: Option<String>,
    pub symbol: String,
    pub state: OrderState,
    pub filled_volume: Decimal,
    pub avg_price: Option<Decimal>,
    pub message: Option<String>,
}

/// Lbank Order Manager
pub struct LbankOrderManager {
    client: LbankClient,
    orders: Arc<RwLock<HashMap<String, TrackedOrder>>>,
    event_tx: Option<mpsc::Sender<OrderEvent>>,
}

impl LbankOrderManager {
    pub fn new(client: LbankClient) -> Self {
        Self {
            client,
            orders: Arc::new(RwLock::new(HashMap::new())),
            event_tx: None,
        }
    }

    /// Start order manager with event channel
    pub fn start(&mut self) -> mpsc::Receiver<OrderEvent> {
        let (tx, rx) = mpsc::channel(100);
        self.event_tx = Some(tx);
        rx
    }

    /// Submit a market order
    pub async fn submit_market_order(
        &self,
        symbol: &str,
        side: TradeDirection,
        offset: OffsetFlag,
        volume: Decimal,
    ) -> Result<String> {
        let client_order_id = Uuid::new_v4().to_string();

        // Track the order
        {
            let mut orders = self.orders.write();
            orders.insert(client_order_id.clone(), TrackedOrder {
                client_order_id: client_order_id.clone(),
                exchange_order_id: None,
                symbol: symbol.to_string(),
                side,
                offset,
                volume,
                filled_volume: Decimal::ZERO,
                avg_price: None,
                state: OrderState::Pending,
                created_at: Instant::now(),
                last_update: Instant::now(),
            });
        }

        // Place the order
        let response = self.client.place_market_order(symbol, side, offset, volume).await?;

        // Update tracking
        self.update_order(&client_order_id, &response);

        Ok(client_order_id)
    }

    /// Submit a limit order
    pub async fn submit_limit_order(
        &self,
        symbol: &str,
        side: TradeDirection,
        offset: OffsetFlag,
        volume: Decimal,
        price: Decimal,
    ) -> Result<String> {
        let client_order_id = Uuid::new_v4().to_string();

        // Track the order
        {
            let mut orders = self.orders.write();
            orders.insert(client_order_id.clone(), TrackedOrder {
                client_order_id: client_order_id.clone(),
                exchange_order_id: None,
                symbol: symbol.to_string(),
                side,
                offset,
                volume,
                filled_volume: Decimal::ZERO,
                avg_price: None,
                state: OrderState::Pending,
                created_at: Instant::now(),
                last_update: Instant::now(),
            });
        }

        // Place the order
        let response = self.client.place_limit_order(symbol, side, offset, volume, price).await?;

        // Update tracking
        self.update_order(&client_order_id, &response);

        Ok(client_order_id)
    }

    /// Update order from API response
    fn update_order(&self, client_order_id: &str, response: &OrderInsertResponse) {
        let mut orders = self.orders.write();
        if let Some(order) = orders.get_mut(client_order_id) {
            order.exchange_order_id = Some(response.order_sys_id.clone());
            order.state = OrderState::Submitted;
            order.last_update = Instant::now();

            if response.volume_remain == "0" {
                order.state = OrderState::Filled;
                order.filled_volume = order.volume;
            }

            debug!(
                client_id = %client_order_id,
                exchange_id = %response.order_sys_id,
                state = ?order.state,
                "Order updated"
            );
        }
    }

    /// Handle incoming order update (from WebSocket or polling)
    pub fn handle_order_update(
        &self,
        exchange_order_id: &str,
        status: &str,
        filled_volume: Decimal,
        avg_price: Option<Decimal>,
    ) {
        let state = OrderState::from(status);

        let mut orders = self.orders.write();
        for (_, order) in orders.iter_mut() {
            if order.exchange_order_id.as_deref() == Some(exchange_order_id) {
                order.state = state;
                order.filled_volume = filled_volume;
                order.avg_price = avg_price;
                order.last_update = Instant::now();

                // Emit event
                if let Some(ref tx) = self.event_tx {
                    let event = OrderEvent {
                        client_order_id: order.client_order_id.clone(),
                        exchange_order_id: order.exchange_order_id.clone(),
                        symbol: order.symbol.clone(),
                        state,
                        filled_volume,
                        avg_price,
                        message: None,
                    };
                    let _ = tx.try_send(event);
                }

                debug!(
                    exchange_id = %exchange_order_id,
                    state = ?state,
                    filled = %filled_volume,
                    "Order update processed"
                );
                break;
            }
        }
    }

    /// Get order by client order ID
    pub fn get_order(&self, client_order_id: &str) -> Option<TrackedOrder> {
        self.orders.read().get(client_order_id).cloned()
    }

    /// Get all active orders for a symbol
    pub fn get_active_orders(&self, symbol: &str) -> Vec<TrackedOrder> {
        self.orders.read()
            .values()
            .filter(|o| o.symbol == symbol && o.is_active())
            .cloned()
            .collect()
    }

    /// Check if order is filled
    pub fn is_filled(&self, client_order_id: &str) -> bool {
        self.orders.read()
            .get(client_order_id)
            .map(|o| o.state == OrderState::Filled)
            .unwrap_or(false)
    }

    /// Get filled volume
    pub fn get_filled_volume(&self, client_order_id: &str) -> Decimal {
        self.orders.read()
            .get(client_order_id)
            .map(|o| o.filled_volume)
            .unwrap_or_default()
    }

    /// Cancel an order
    pub async fn cancel_order(&self, client_order_id: &str) -> Result<()> {
        let exchange_id = {
            let orders = self.orders.read();
            orders.get(client_order_id)
                .and_then(|o| o.exchange_order_id.clone())
        };

        if let Some(exchange_id) = exchange_id {
            info!(exchange_id = %exchange_id, "Cancelling order");
            // TODO: Call cancel API
            // self.client.cancel_order(&exchange_id).await?;

            // Update state
            let mut orders = self.orders.write();
            if let Some(order) = orders.get_mut(client_order_id) {
                order.state = OrderState::Cancelled;
                order.last_update = Instant::now();
            }
        }

        Ok(())
    }

    /// Clean up old orders (older than 1 hour)
    pub fn cleanup_old_orders(&self) {
        let cutoff = Instant::now() - Duration::from_secs(3600);
        let mut orders = self.orders.write();
        orders.retain(|_, o| o.last_update > cutoff || o.is_active());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_order_state_conversion() {
        assert_eq!(OrderState::from("0"), OrderState::Pending);
        assert_eq!(OrderState::from("1"), OrderState::Submitted);
        assert_eq!(OrderState::from("3"), OrderState::Filled);
        assert_eq!(OrderState::from("5"), OrderState::Rejected);
    }
}
