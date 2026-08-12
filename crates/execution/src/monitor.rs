//! Position Monitor
//!
//! Monitors open positions and triggers exit conditions.

use crate::state_machine::{ExecutionEngine, TrackedPosition};
use anyhow::Result;
use parking_lot::RwLock;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{debug, info, warn};

/// Position monitor
pub struct PositionMonitor {
    execution_engine: ExecutionEngine,
    current_spread: Arc<RwLock<Decimal>>,
    gtc_timeout_secs: u64,
    check_interval: Duration,
}

impl PositionMonitor {
    pub fn new(
        execution_engine: ExecutionEngine,
        gtc_timeout_secs: u64,
        check_interval_ms: u64,
    ) -> Self {
        Self {
            execution_engine,
            current_spread: Arc::new(RwLock::new(Decimal::ZERO)),
            gtc_timeout_secs,
            check_interval: Duration::from_millis(check_interval_ms),
        }
    }

    /// Update current spread
    pub fn update_spread(&self, spread_bps: Decimal) {
        let mut current = self.current_spread.write();
        *current = spread_bps;
    }

    /// Start monitoring loop
    pub async fn start(mut self, mut shutdown: tokio::sync::oneshot::Receiver<()>) {
        let mut ticker = interval(self.check_interval);

        loop {
            tokio::select! {
                _ = &mut shutdown => {
                    info!("Position monitor shutting down");
                    break;
                }
                _ = ticker.tick() => {
                    self.check_positions().await;
                }
            }
        }
    }

    /// Check all positions
    async fn check_positions(&self) {
        let spread = *self.current_spread.read();

        // Check for TP/SL conditions
        let exits = self.execution_engine.check_exit_conditions(spread);

        for (position_id, decision) in exits {
            info!(
                position_id = %position_id,
                method = %decision.method,
                reason = %decision.reason,
                "Exit condition triggered"
            );

            self.execution_engine.record_exit(&position_id, decision.method, decision.realized_pnl);
        }

        // Check GTC timeouts
        let timed_out = self.execution_engine.check_gtc_timeout(self.gtc_timeout_secs);

        for position_id in timed_out {
            warn!(
                position_id = %position_id,
                "GTC order timed out, switching to taker"
            );

            self.execution_engine.record_exit(
                &position_id,
                crate::state_machine::ExitMethod::TakerIoc,
                Decimal::ZERO,
            );
        }
    }

    /// Get active position count
    pub fn active_count(&self) -> usize {
        self.execution_engine.get_active_positions().len()
    }
}
