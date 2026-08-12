//! Metrics Module
//!
//! Prometheus metrics for monitoring.

use prometheus::{Counter, Gauge, Histogram, Encoder, TextEncoder};
use parking_lot::RwLock;
use std::collections::HashMap;

/// Signal funnel metrics
pub struct SignalMetrics {
    pub signals_scanned: Counter,
    pub signals_qualified: Counter,
    pub signals_executed: Counter,
}

impl SignalMetrics {
    pub fn new() -> Self {
        Self {
            signals_scanned: Counter::new("signals_scanned_total", "Total signals scanned").unwrap(),
            signals_qualified: Counter::new("signals_qualified_total", "Qualified signals").unwrap(),
            signals_executed: Counter::new("signals_executed_total", "Executed signals").unwrap(),
        }
    }
}

impl Default for SignalMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Execution metrics
pub struct ExecutionMetrics {
    pub positions_opened: Counter,
    pub positions_closed: Counter,
    pub wins: Counter,
    pub losses: Counter,
    pub total_pnl: Gauge,
    pub win_rate: Gauge,
}

impl ExecutionMetrics {
    pub fn new() -> Self {
        Self {
            positions_opened: Counter::new("positions_opened_total", "Positions opened").unwrap(),
            positions_closed: Counter::new("positions_closed_total", "Positions closed").unwrap(),
            wins: Counter::new("wins_total", "Winning trades").unwrap(),
            losses: Counter::new("losses_total", "Losing trades").unwrap(),
            total_pnl: Gauge::new("total_pnl", "Total PnL").unwrap(),
            win_rate: Gauge::new("win_rate", "Win rate").unwrap(),
        }
    }
}

impl Default for ExecutionMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Latency metrics
pub struct LatencyMetrics {
    pub feed_latency_p50: Gauge,
    pub feed_latency_p99: Gauge,
    pub order_latency_p50: Gauge,
    pub order_latency_p99: Gauge,
}

impl LatencyMetrics {
    pub fn new() -> Self {
        Self {
            feed_latency_p50: Gauge::new("feed_latency_p50_ms", "Feed latency P50").unwrap(),
            feed_latency_p99: Gauge::new("feed_latency_p99_ms", "Feed latency P99").unwrap(),
            order_latency_p50: Gauge::new("order_latency_p50_ms", "Order latency P50").unwrap(),
            order_latency_p99: Gauge::new("order_latency_p99_ms", "Order latency P99").unwrap(),
        }
    }
}

impl Default for LatencyMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Metrics registry
pub struct MetricsRegistry {
    signals: SignalMetrics,
    execution: ExecutionMetrics,
    latency: LatencyMetrics,
    custom_gauges: RwLock<HashMap<String, f64>>,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self {
            signals: SignalMetrics::new(),
            execution: ExecutionMetrics::new(),
            latency: LatencyMetrics::new(),
            custom_gauges: RwLock::new(HashMap::new()),
        }
    }

    /// Export metrics as Prometheus format
    pub fn export(&self) -> String {
        let encoder = TextEncoder::new();
        let metric_families = prometheus::gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer).unwrap();
        String::from_utf8(buffer).unwrap()
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}
