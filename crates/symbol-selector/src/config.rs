//! Configuration for symbol selector

use serde::{Deserialize, Serialize};

/// Selection strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    /// Only monitor symbols in watchlist
    Watchlist,
    /// Monitor all symbols (intersection only)
    All,
    /// Rate-limit aware rotating selection
    RateLimitAware,
    /// Combine watchlist with rate-limit rotation
    WatchlistWithRotation,
}

impl Default for Strategy {
    fn default() -> Self {
        Self::RateLimitAware
    }
}

/// Rate limit configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Total requests per second allowed for symbol data
    pub total: u32,
    /// Reserved for active monitoring
    pub active_reserved: u32,
    /// Batch size for rotating pool
    pub rotating_batch_size: usize,
    /// Rotation interval in seconds
    pub rotation_interval_secs: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            total: 100,
            active_reserved: 10,
            rotating_batch_size: 20,
            rotation_interval_secs: 5,
        }
    }
}

impl RateLimitConfig {
    pub fn new(total: u32, active_reserved: u32, rotating_batch_size: usize, rotation_interval_secs: u64) -> Self {
        Self {
            total,
            active_reserved,
            rotating_batch_size,
            rotation_interval_secs,
        }
    }

    /// Create from percentage allocation
    pub fn with_percentage(total: u32, active_percent: u32, rotating_batch_size: usize, rotation_interval_secs: u64) -> Self {
        Self {
            total,
            active_reserved: (total * active_percent) / 100,
            rotating_batch_size,
            rotation_interval_secs,
        }
    }
}

/// Main selector configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectorConfig {
    /// Selection strategy to use
    pub strategy: Strategy,
    /// Rate limit configuration
    pub rate_limit: RateLimitConfig,
    /// Symbols to watch (for watchlist strategies)
    pub watchlist: Vec<String>,
    /// Whether to require symbols on both exchanges
    #[serde(default = "default_true")]
    pub require_both_exchanges: bool,
    /// Exchange to use as primary (for single-exchange modes)
    #[serde(default)]
    pub primary_exchange: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for SelectorConfig {
    fn default() -> Self {
        Self {
            strategy: Strategy::default(),
            rate_limit: RateLimitConfig::default(),
            watchlist: Vec::new(),
            require_both_exchanges: true,
            primary_exchange: None,
        }
    }
}

impl SelectorConfig {
    pub fn watchlist(watchlist: Vec<String>) -> Self {
        Self {
            strategy: Strategy::Watchlist,
            watchlist,
            ..Default::default()
        }
    }

    pub fn rate_limit_aware(rate_limit: RateLimitConfig) -> Self {
        Self {
            strategy: Strategy::RateLimitAware,
            rate_limit,
            ..Default::default()
        }
    }

    pub fn watchlist_with_rotation(watchlist: Vec<String>, rate_limit: RateLimitConfig) -> Self {
        Self {
            strategy: Strategy::WatchlistWithRotation,
            watchlist,
            rate_limit,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = SelectorConfig::default();
        assert_eq!(config.strategy, Strategy::RateLimitAware);
        assert_eq!(config.rate_limit.total, 100);
        assert_eq!(config.rate_limit.active_reserved, 10);
        assert!(config.require_both_exchanges);
    }

    #[test]
    fn test_watchlist_config() {
        let config = SelectorConfig::watchlist(vec!["BTCUSDT".into(), "ETHUSDT".into()]);
        assert_eq!(config.strategy, Strategy::Watchlist);
        assert_eq!(config.watchlist.len(), 2);
    }

    #[test]
    fn test_rate_limit_config() {
        let config = RateLimitConfig::with_percentage(200, 20, 30, 10);
        assert_eq!(config.total, 200);
        assert_eq!(config.active_reserved, 40);
        assert_eq!(config.rotating_batch_size, 30);
    }
}
