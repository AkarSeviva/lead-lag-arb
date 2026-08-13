//! # Config Module
//!
//! Configuration management with hot-reload support for the lead-lag arbitrage system.

pub mod env;
pub mod strategy;
pub mod types;

pub use env::{get, init, optional, require};
pub use strategy::{StrategyConfig, NetworkConfig, FeeConfig, CapitalConfig};
pub use types::*;
