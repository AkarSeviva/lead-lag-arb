//! Lbank Exchange Adapter
//!
//! Implementation for Lbank perpetual futures API (reversed engineered).
//! API Base: https://uuapi.rerrkvifj.com
//! WebSocket: wss://uuws.rerrkvifj.com/ws/v3

pub mod auth;
pub mod client;
pub mod market;
pub mod orders;
pub mod protocol;
pub mod proxy;
pub mod ws;

pub use auth::LbankSigner;
pub use client::LbankClient;
pub use market::{LbankMarketData, MarketEvent};
pub use orders::{LbankOrderManager, OrderEvent};
pub use protocol::*;
pub use proxy::{ProxyClient, ProxyConfig};
pub use ws::LbankWebSocket;
