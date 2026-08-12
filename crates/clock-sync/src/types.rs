//! Clock Sync Types

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Clock offset measurement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClockOffset {
    /// Offset in milliseconds (exchange_time - local_time)
    pub offset_ms: i64,
    /// Round-trip time in milliseconds
    pub rtt_ms: i64,
    /// Local timestamp when measured
    pub local_timestamp: i64,
    /// Estimated stability (max deviation from mean)
    pub stability_ms: i64,
}

impl ClockOffset {
    /// One-way latency estimate (RTT / 2)
    pub fn one_way_latency_ms(&self) -> i64 {
        self.rtt_ms / 2
    }
}

/// Latency statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LatencyStats {
    /// P50 latency (ms)
    pub p50_ms: i64,
    /// P95 latency (ms)
    pub p95_ms: i64,
    /// P99 latency (ms)
    pub p99_ms: i64,
    /// Mean latency (ms)
    pub mean_ms: i64,
    /// Number of samples
    pub sample_count: usize,
    /// Estimated one-way latency (P50 / 2)
    pub one_way_latency_ms: i64,
}

impl LatencyStats {
    /// Check if latency is acceptable for trading
    pub fn is_acceptable(&self, threshold_ms: i64) -> bool {
        self.p99_ms <= threshold_ms
    }
}

/// Synchronization state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    /// Not connected
    Disconnected,
    /// Currently syncing
    Syncing,
    /// Successfully synced
    Synced,
    /// Degraded (high latency)
    Degraded,
    /// Error state
    Error,
}

impl Default for SyncState {
    fn default() -> Self {
        Self::Disconnected
    }
}

impl From<SyncState> for &str {
    fn from(state: SyncState) -> Self {
        match state {
            SyncState::Disconnected => "disconnected",
            SyncState::Syncing => "syncing",
            SyncState::Synced => "synced",
            SyncState::Degraded => "degraded",
            SyncState::Error => "error",
        }
    }
}

/// Timestamp with sync metadata
#[derive(Debug, Clone)]
pub struct SyncedTimestamp {
    /// Synchronized timestamp
    pub timestamp: i64,
    /// Local receive timestamp
    pub local_ts: i64,
    /// Estimated one-way latency
    pub latency_ms: i64,
    /// Source exchange
    pub exchange: String,
}

impl SyncedTimestamp {
    /// Create from exchange timestamp and local receive time
    pub fn new(exchange_ts: i64, local_ts: i64, exchange: &str) -> Self {
        Self {
            timestamp: exchange_ts,
            local_ts,
            latency_ms: (local_ts - exchange_ts).max(0),
            exchange: exchange.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offset_calculation() {
        let offset = ClockOffset {
            offset_ms: 100,
            rtt_ms: 20,
            local_timestamp: 1000,
            stability_ms: 5,
        };

        assert_eq!(offset.one_way_latency_ms(), 10);
    }

    #[test]
    fn test_latency_stats() {
        let stats = LatencyStats {
            p50_ms: 10,
            p95_ms: 50,
            p99_ms: 100,
            mean_ms: 15,
            sample_count: 1000,
            one_way_latency_ms: 5,
        };

        assert!(stats.is_acceptable(100));
        assert!(!stats.is_acceptable(50));
    }
}
