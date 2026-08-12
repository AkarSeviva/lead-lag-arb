//! Clock Synchronization
//!
//! NTP-like clock synchronization with exchange time servers.

use crate::types::{ClockOffset, LatencyStats, SyncState};
use anyhow::Result;
use parking_lot::RwLock;
use rust_decimal::Decimal;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Clock synchronizer for an exchange
pub struct ClockSynchronizer {
    /// Exchange identifier
    exchange: String,
    /// Current offset (exchange_time - local_time)
    offset: Arc<RwLock<Option<ClockOffset>>>,
    /// Recent RTT measurements for statistics
    rtt_history: Arc<RwLock<VecDeque<Duration>>>,
    /// Sync state
    state: Arc<RwLock<SyncState>>,
    /// HTTP client for time queries
    client: reqwest::Client,
}

impl ClockSynchronizer {
    /// Create a new synchronizer
    pub fn new(exchange: String) -> Self {
        Self {
            exchange,
            offset: Arc::new(RwLock::new(None)),
            rtt_history: Arc::new(RwLock::new(VecDeque::with_capacity(100))),
            state: Arc::new(RwLock::new(SyncState::Disconnected)),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Get synchronized time
    pub fn now(&self) -> i64 {
        let offset = self.offset.read();
        let local = chrono::Utc::now().timestamp_millis();
        if let Some(o) = offset.as_ref() {
            local + o.offset_ms
        } else {
            local
        }
    }

    /// Get local time (unadjusted)
    pub fn local_now(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    /// Get current offset
    pub fn get_offset(&self) -> Option<ClockOffset> {
        self.offset.read().clone()
    }

    /// Get sync state
    pub fn get_state(&self) -> SyncState {
        self.state.read().clone()
    }

    /// Get latency statistics
    pub fn get_stats(&self) -> LatencyStats {
        let rtt_history = self.rtt_history.read();
        let rtts: Vec<Duration> = rtt_history.iter().cloned().collect();

        if rtts.is_empty() {
            return LatencyStats::default();
        }

        let mut sorted = rtts.clone();
        sorted.sort();

        let count = sorted.len();
        let sum: Duration = sorted.iter().sum();
        let mean = sum / count as u32;

        let p50_idx = count * 50 / 100;
        let p95_idx = count * 95 / 100;
        let p99_idx = count * 99 / 100;

        LatencyStats {
            p50_ms: sorted[p50_idx].as_millis() as i64,
            p95_ms: sorted[p95_idx].as_millis() as i64,
            p99_ms: sorted[p99_idx].as_millis() as i64,
            mean_ms: mean.as_millis() as i64,
            sample_count: count,
            one_way_latency_ms: sorted[p50_idx].as_millis() as i64 / 2,
        }
    }

    /// Perform initial synchronization with multiple samples
    pub async fn sync(&self, time_url: &str, samples: usize) -> Result<ClockOffset> {
        info!(exchange = %self.exchange, samples, "Starting clock sync");
        *self.state.write() = SyncState::Syncing;

        let mut offsets = Vec::with_capacity(samples);

        for i in 0..samples {
            match self.measure_offset(time_url).await {
                Ok((offset, rtt)) => {
                    offsets.push(offset.clone());
                    if !offsets.is_empty() {
                        let mut history = self.rtt_history.write();
                        history.push_back(rtt);
                        if history.len() > 100 {
                            history.pop_front();
                        }
                    }
                    debug!(exchange = %self.exchange, sample = i + 1, offset_ms = offset.offset_ms, rtt_ms = rtt.as_millis(), "Sync sample");
                }
                Err(e) => {
                    warn!(exchange = %self.exchange, error = %e, "Sync sample failed");
                }
            }

            // Small delay between samples
            if i < samples - 1 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        if offsets.is_empty() {
            *self.state.write() = SyncState::Error;
            anyhow::bail!("Clock sync failed: no successful samples");
        }

        // Use median offset (most robust to outliers)
        offsets.sort_by_key(|o| o.offset_ms);
        let median = offsets[offsets.len() / 2].clone();

        // Calculate offset stability
        let mean_offset: i64 = offsets.iter().map(|o| o.offset_ms).sum::<i64>() / offsets.len() as i64;
        let max_deviation = offsets.iter()
            .map(|o| (o.offset_ms - mean_offset).abs())
            .max()
            .unwrap_or(0);

        let offset = ClockOffset {
            offset_ms: median.offset_ms,
            rtt_ms: median.rtt_ms,
            local_timestamp: chrono::Utc::now().timestamp_millis(),
            stability_ms: max_deviation,
        };

        *self.offset.write() = Some(offset.clone());
        *self.state.write() = SyncState::Synced;

        info!(
            exchange = %self.exchange,
            offset_ms = offset.offset_ms,
            stability_ms = offset.stability_ms,
            "Clock sync completed"
        );

        Ok(offset)
    }

    /// Measure offset with a single sample
    async fn measure_offset(&self, time_url: &str) -> Result<(ClockOffset, Duration)> {
        let t0 = Instant::now();
        let t1 = chrono::Utc::now().timestamp_millis();

        let response = self.client.get(time_url).send().await?;
        let t4 = Instant::now();

        let rtt = t4 - t0;
        let t2 = chrono::Utc::now().timestamp_millis();

        let body = response.text().await?;
        let server_time: i64 = match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(json) => {
                let v = json.get("timestamp");
                match v.and_then(|vv| vv.as_i64()) {
                    Some(ts) => ts,
                    None => {
                        match v.and_then(|vv| vv.as_str()).and_then(|s| s.parse().ok()) {
                            Some(ts) => ts,
                            None => body.trim().parse::<i64>().unwrap_or(0),
                        }
                    }
                }
            }
            Err(_) => {
                // Try parsing as simple timestamp
                let t: i64 = body.trim().parse().unwrap_or(0);
                // If it looks like seconds, convert to ms
                if t < 1_000_000_000_000 { t * 1000 } else { t }
            }
        };

        let t3 = chrono::Utc::now().timestamp_millis();

        // NTP-like offset calculation
        // offset = ((t1 - t0) + (t2 - t3)) / 2
        let offset_ms = ((t2 - t1) + (t3 - t1)) / 2;

        Ok((ClockOffset {
            offset_ms,
            rtt_ms: rtt.as_millis() as i64,
            local_timestamp: t1,
            stability_ms: 0,
        }, rtt))
    }

    /// Update offset continuously (called periodically)
    pub async fn update_offset(&self, time_url: &str) -> Result<()> {
        let (offset, rtt) = self.measure_offset(time_url).await?;

        // Update RTT history
        {
            let mut history = self.rtt_history.write();
            history.push_back(rtt);
            if history.len() > 100 {
                history.pop_front();
            }
        }

        // Update offset (exponential moving average)
        {
            let mut current = self.offset.write();
            if let Some(ref mut existing) = *current {
                // EMA with alpha = 0.3
                existing.offset_ms = (existing.offset_ms * 7 + offset.offset_ms) / 8;
                existing.rtt_ms = (existing.rtt_ms * 7 + offset.rtt_ms) / 8;
                existing.local_timestamp = chrono::Utc::now().timestamp_millis();
            } else {
                *current = Some(offset);
            }
        }

        Ok(())
    }

    /// Convert local timestamp to synchronized time
    pub fn to_synced_time(&self, local_ts: i64) -> i64 {
        let offset = self.offset.read();
        if let Some(o) = offset.as_ref() {
            local_ts + o.offset_ms
        } else {
            local_ts
        }
    }

    /// Convert synchronized time to local time
    pub fn to_local_time(&self, synced_ts: i64) -> i64 {
        let offset = self.offset.read();
        if let Some(o) = offset.as_ref() {
            synced_ts - o.offset_ms
        } else {
            synced_ts
        }
    }
}

impl Clone for ClockSynchronizer {
    fn clone(&self) -> Self {
        Self {
            exchange: self.exchange.clone(),
            offset: self.offset.clone(),
            rtt_history: self.rtt_history.clone(),
            state: self.state.clone(),
            client: self.client.clone(),
        }
    }
}
