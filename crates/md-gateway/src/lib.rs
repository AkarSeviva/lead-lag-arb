//! Market Data Gateway
//!
//! Manages WebSocket connections to exchanges with reconnection and message routing.

pub mod connection;
pub mod message;
pub mod router;

pub use connection::{ConnectionState, ExchangeGateway};
pub use message::RawMarketMessage;
pub use router::MessageRouter;
