//! Risk Gate
//!
//! Central risk management component.

use crate::circuit_breaker::CircuitBreaker;
use crate::position::{Position, PositionTracker};
use anyhow::Result;
use config::strategy::RiskConfig;
use parking_lot::RwLock;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Risk check result
#[derive(Debug, Clone)]
pub struct RiskCheckResult {
    pub allowed: bool,
    pub reason: Option<String>,
    pub check: &'static str,
}

impl RiskCheckResult {
    pub fn allow(check: &'static str) -> Self {
        Self { allowed: true, reason: None, check }
    }

    pub fn deny(check: &'static str, reason: String) -> Self {
        Self { allowed: false, reason: Some(reason), check }
    }
}

/// Risk gate for pre-trade checks
pub struct RiskGate {
    config: Arc<arc_swap::ArcSwap<RiskConfig>>,
    position_tracker: PositionTracker,
    circuit_breaker: CircuitBreaker,
    cooldown_tracker: CooldownTracker,
    api_quota: ApiQuotaTracker,
}

impl RiskGate {
    pub fn new(config: RiskConfig) -> Self {
        Self {
            config: Arc::new(arc_swap::ArcSwap::from_pointee(config)),
            position_tracker: PositionTracker::new(),
            circuit_breaker: CircuitBreaker::default(),
            cooldown_tracker: CooldownTracker::new(),
            api_quota: ApiQuotaTracker::new(),
        }
    }

    /// Update configuration (hot reload)
    pub fn update_config(&self, config: RiskConfig) {
        self.config.store(Arc::new(config));
    }

    /// Check if trading is allowed
    pub fn check(&self, symbol: &str) -> Vec<RiskCheckResult> {
        let mut results = Vec::new();
        let config = self.config.load();

        // 1. Circuit breaker check
        results.push(self.circuit_breaker.check());

        // 2. Cooldown check
        results.push(self.cooldown_tracker.check(symbol));

        // 3. Concurrent position limit
        results.push(self.check_concurrent_positions(&config));

        // 4. Symbol position limit
        results.push(self.check_position_limit(symbol, &config));

        // 5. Total exposure limit
        results.push(self.check_total_exposure(&config));

        // 6. API quota check
        results.push(self.api_quota.check());

        results
    }

    /// Check concurrent positions
    fn check_concurrent_positions(&self, config: &RiskConfig) -> RiskCheckResult {
        let count = self.position_tracker.active_count();
        if count >= config.max_concurrent_positions {
            RiskCheckResult::deny(
                "max_concurrent_positions",
                format!("Active positions {} >= limit {}", count, config.max_concurrent_positions),
            )
        } else {
            RiskCheckResult::allow("max_concurrent_positions")
        }
    }

    /// Check position size limit
    fn check_position_limit(&self, symbol: &str, config: &RiskConfig) -> RiskCheckResult {
        if let Some(pos) = self.position_tracker.get(symbol) {
            if pos.notional >= config.max_position_usd {
                return RiskCheckResult::deny(
                    "max_position_usd",
                    format!("Position {} >= limit {}", pos.notional, config.max_position_usd),
                );
            }
        }
        RiskCheckResult::allow("max_position_usd")
    }

    /// Check total exposure
    fn check_total_exposure(&self, config: &RiskConfig) -> RiskCheckResult {
        let total = self.position_tracker.total_exposure();
        if total >= config.max_total_exposure_usd {
            RiskCheckResult::deny(
                "max_total_exposure_usd",
                format!("Total exposure {} >= limit {}", total, config.max_total_exposure_usd),
            )
        } else {
            RiskCheckResult::allow("max_total_exposure_usd")
        }
    }

    /// Record a new position
    pub fn record_position_open(&self, position: Position) {
        let symbol = position.symbol.clone();
        let size = position.size;
        self.position_tracker.add(position);
        info!(symbol = %symbol, size = %size, "Position opened, risk gate updated");
    }

    /// Update position
    pub fn update_position(&self, symbol: &str, pnl: Decimal, filled: Decimal) {
        self.position_tracker.update(symbol, pnl, filled);
    }

    /// Record position closed
    pub fn record_position_close(&self, symbol: &str, realized_pnl: Decimal, was_stop_loss: bool) {
        self.position_tracker.remove(symbol);
        
        // Trigger cooldown if stop-loss
        if was_stop_loss {
            let config = self.config.load();
            self.cooldown_tracker.set(symbol, Duration::from_secs(config.cooldown_after_sl_secs));
            
            // Update circuit breaker
            self.circuit_breaker.record_loss();
            
            warn!(symbol = %symbol, pnl = %realized_pnl, "Position closed with stop-loss, cooldown started");
        }
    }

    /// Check all results
    pub fn all_passed(&self, results: &[RiskCheckResult]) -> bool {
        results.iter().all(|r| r.allowed)
    }

    /// Get position tracker
    pub fn positions(&self) -> &PositionTracker {
        &self.position_tracker
    }

    /// Get circuit breaker state
    pub fn circuit_state(&self) -> crate::circuit_breaker::CircuitState {
        self.circuit_breaker.state()
    }

    /// Reset circuit breaker (manual intervention)
    pub fn reset_circuit_breaker(&self) {
        self.circuit_breaker.reset();
        info!("Circuit breaker reset");
    }
}

/// Cooldown tracker per symbol
pub struct CooldownTracker {
    cooldowns: RwLock<HashMap<String, std::time::Instant>>,
}

impl CooldownTracker {
    pub fn new() -> Self {
        Self {
            cooldowns: RwLock::new(HashMap::new()),
        }
    }

    pub fn set(&self, symbol: &str, duration: Duration) {
        let mut cooldowns = self.cooldowns.write();
        cooldowns.insert(symbol.to_string(), std::time::Instant::now() + duration);
    }

    pub fn check(&self, symbol: &str) -> RiskCheckResult {
        let cooldowns = self.cooldowns.read();
        if let Some(end) = cooldowns.get(symbol) {
            if std::time::Instant::now() < *end {
                let remaining = end.duration_since(std::time::Instant::now());
                return RiskCheckResult::deny(
                    "cooldown",
                    format!("Cooldown active, {}ms remaining", remaining.as_millis()),
                );
            }
        }
        RiskCheckResult::allow("cooldown")
    }

    pub fn clear(&self, symbol: &str) {
        let mut cooldowns = self.cooldowns.write();
        cooldowns.remove(symbol);
    }
}

impl Default for CooldownTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// API quota tracker
pub struct ApiQuotaTracker {
    requests_per_second: RwLock<HashMap<String, (usize, std::time::Instant)>>,
    limit: usize,
}

impl ApiQuotaTracker {
    pub fn new() -> Self {
        Self {
            requests_per_second: RwLock::new(HashMap::new()),
            limit: 120, // requests per second limit
        }
    }

    pub fn record_request(&self, endpoint: &str) {
        let mut requests = self.requests_per_second.write();
        let now = std::time::Instant::now();
        
        // Clean old entries
        requests.retain(|_, (_, t)| now.duration_since(*t) < Duration::from_secs(1));
        
        let entry = requests.entry(endpoint.to_string()).or_insert((0, now));
        entry.0 += 1;
        entry.1 = now;
    }

    pub fn check(&self) -> RiskCheckResult {
        let requests = self.requests_per_second.read();
        let total: usize = requests.values().map(|(c, _)| c).sum();
        
        if total >= self.limit {
            RiskCheckResult::deny("api_quota", format!("API rate limit: {}/s", total))
        } else {
            RiskCheckResult::allow("api_quota")
        }
    }
}

impl Default for ApiQuotaTracker {
    fn default() -> Self {
        Self::new()
    }
}
