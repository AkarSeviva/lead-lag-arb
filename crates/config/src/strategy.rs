//! Strategy configuration types

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Main strategy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    /// Entry threshold (e.g., 0.001 = 0.1%)
    pub entry_threshold: Decimal,

    /// Take-profit ratio relative to entry spread
    pub tp_ratio: Decimal,

    /// Stop-loss ratio relative to entry spread
    pub sl_ratio: Decimal,

    /// Max holding time in seconds
    pub max_holding_secs: u64,

    /// GTC order timeout in seconds before fallback to market
    pub gtc_timeout_secs: u64,

    /// Entry filters
    pub filters: FilterConfig,

    /// Risk parameters
    pub risk: RiskConfig,

    /// Capital management
    pub capital: CapitalConfig,

    /// Network configuration (proxy, timeouts, etc.)
    pub network: NetworkConfig,
}

/// Filter configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterConfig {
    /// Min spread duration in milliseconds
    pub min_spread_duration_ms: u64,

    /// Max spread duration in milliseconds
    pub max_spread_duration_ms: u64,

    /// Min depth on leader side (in quote currency)
    pub min_leader_depth_usd: Decimal,

    /// Max 1-minute volatility to allow entry (e.g., 0.02 = 2%)
    pub max_volatility: Decimal,

    /// Cooldown after stop-loss in seconds
    pub cooldown_after_sl_secs: u64,
}

/// Risk configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    /// Max concurrent positions
    pub max_concurrent_positions: usize,

    /// Max position size per trade (in quote currency)
    pub max_position_usd: Decimal,

    /// Max total exposure (in quote currency)
    pub max_total_exposure_usd: Decimal,

    /// Circuit breaker: pause after N consecutive losses
    pub circuit_breaker_losses: usize,

    /// Circuit breaker: pause duration in seconds
    pub circuit_breaker_pause_secs: u64,

    /// Cooldown after stop-loss in seconds
    pub cooldown_after_sl_secs: u64,
}

/// Capital configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapitalConfig {
    /// Initial capital in USDT
    pub initial_capital: Decimal,

    /// Position size as fraction of capital (0.0 - 1.0)
    pub position_fraction: Decimal,

    /// Enable leverage
    pub leverage: u32,

    /// Max notional per trade
    pub max_notional: Decimal,
}

/// Network configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Enable HTTP proxy
    pub proxy_enabled: bool,

    /// Proxy server address (e.g., "127.0.0.1:7890")
    pub proxy_addr: String,

    /// Connection timeout in seconds
    pub connect_timeout_secs: u64,

    /// Request timeout in seconds
    pub request_timeout_secs: u64,

    /// Enable keep-alive
    pub keep_alive: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            proxy_enabled: false,
            proxy_addr: "127.0.0.1:7890".to_string(),
            connect_timeout_secs: 10,
            request_timeout_secs: 30,
            keep_alive: true,
        }
    }
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            // Target 0.1% spread capture
            entry_threshold: Decimal::new(10, 4), // 0.001
            tp_ratio: Decimal::new(1, 0),          // 1.0 = full convergence
            sl_ratio: Decimal::new(1, 0),          // 1.0 = double spread
            max_holding_secs: 30,
            gtc_timeout_secs: 5,
            filters: FilterConfig::default(),
            risk: RiskConfig::default(),
            capital: CapitalConfig::default(),
            network: NetworkConfig::default(),
        }
    }
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            min_spread_duration_ms: 100,
            max_spread_duration_ms: 300,
            min_leader_depth_usd: Decimal::new(100_000, 0), // $100k
            max_volatility: Decimal::new(2, 2),             // 2%
            cooldown_after_sl_secs: 5,
        }
    }
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_concurrent_positions: 3,
            max_position_usd: Decimal::new(1000, 0),        // $1000
            max_total_exposure_usd: Decimal::new(5000, 0),  // $5000
            circuit_breaker_losses: 5,
            circuit_breaker_pause_secs: 300,
            cooldown_after_sl_secs: 5,
        }
    }
}

impl Default for CapitalConfig {
    fn default() -> Self {
        Self {
            initial_capital: Decimal::new(10_000, 0),    // $10,000
            position_fraction: Decimal::new(1, 1),       // 10%
            leverage: 1,
            max_notional: Decimal::new(10_000, 0),       // $10,000
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = StrategyConfig::default();
        assert_eq!(config.entry_threshold, Decimal::new(10, 4));
        assert_eq!(config.filters.min_spread_duration_ms, 100);
        assert_eq!(config.risk.max_concurrent_positions, 3);
    }
}
