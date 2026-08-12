//! Binance Exchange Adapter
//!
//! Provides market data from Binance for lead signal detection.
//! Binance serves as the "leader" exchange in the lead-lag arbitrage strategy.
//!
//! # Architecture
//! - Uses direct WebSocket connections to Binance streams
//! - Binance provides the "lead" signal (price discovery)
//! - Lbank is the "follower" where we execute trades
//!
//! # WebSocket Streams
//! - Depth stream: `<symbol>@depth@100ms` for order book
//! - Trade stream: `<symbol>@trade` for trade ticks

pub mod market;
pub mod types;

pub use market::{BinanceMarketData, BinanceMarketDataBuilder, ConnectionStatus, MarketEvent};
pub use types::*;
