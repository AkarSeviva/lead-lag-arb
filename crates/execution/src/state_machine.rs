//! Execution State Machine
//!
//! Order lifecycle management with TP/SL/Timeout exit handling.

use config::{Direction, strategy::StrategyConfig};
use parking_lot::RwLock;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Order state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderState {
    /// Order pending submission
    PendingSubmit,
    /// Order submitted to exchange
    Submitted,
    /// Order partially filled
    PartiallyFilled,
    /// Order fully filled
    Filled,
    /// Exit order pending
    ExitPending(ExitMethod),
    /// Exit order filled
    ExitFilled,
    /// Cancelled
    Cancelled,
    /// Rejected
    Rejected,
    /// Timeout forced close
    TimeoutForced,
}

/// Exit method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitMethod {
    /// GTC limit order (passive)
    GtcLimit,
    /// Taker IOC order (aggressive)
    TakerIoc,
    /// Forced market order (emergency)
    ForcedMarket,
}

impl std::fmt::Display for ExitMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExitMethod::GtcLimit => write!(f, "GTC Limit"),
            ExitMethod::TakerIoc => write!(f, "Taker IOC"),
            ExitMethod::ForcedMarket => write!(f, "Forced Market"),
        }
    }
}

/// Tracked position
#[derive(Debug, Clone)]
pub struct TrackedPosition {
    /// Position ID
    pub id: String,
    /// Symbol
    pub symbol: String,
    /// Direction
    pub direction: Direction,
    /// Entry order ID
    pub entry_order_id: String,
    /// Entry price
    pub entry_price: Decimal,
    /// Entry time
    pub entry_time: Instant,
    /// Entry spread (in bps)
    pub entry_spread_bps: Decimal,
    /// Filled volume
    pub filled_volume: Decimal,
    /// Exit order ID
    pub exit_order_id: Option<String>,
    /// Exit method
    pub exit_method: Option<ExitMethod>,
    /// Current state
    pub state: OrderState,
    /// Unrealized PnL
    pub unrealized_pnl: Decimal,
    /// Realized PnL
    pub realized_pnl: Decimal,
}

impl TrackedPosition {
    pub fn new(
        id: String,
        symbol: String,
        direction: Direction,
        order_id: String,
        entry_price: Decimal,
        spread_bps: Decimal,
        volume: Decimal,
    ) -> Self {
        Self {
            id,
            symbol,
            direction,
            entry_order_id: order_id,
            entry_price,
            entry_time: Instant::now(),
            entry_spread_bps: spread_bps,
            filled_volume: volume,
            exit_order_id: None,
            exit_method: None,
            state: OrderState::Submitted,
            unrealized_pnl: Decimal::ZERO,
            realized_pnl: Decimal::ZERO,
        }
    }

    /// Check if position should exit based on TP/SL
    pub fn check_exit_conditions(
        &self,
        current_spread: Decimal,
        tp_ratio: Decimal,
        sl_ratio: Decimal,
    ) -> Option<ExitDecision> {
        let entry_spread = self.entry_spread_bps;
        let current_deviation = (current_spread - entry_spread).abs();

        // TP condition: spread converged
        let tp_threshold = entry_spread * (Decimal::ONE - tp_ratio);
        if current_spread <= tp_threshold {
            return Some(ExitDecision {
                method: ExitMethod::GtcLimit,
                reason: "Take-profit".to_string(),
                realized_pnl: self.unrealized_pnl,
            });
        }

        // SL condition: spread widened
        let sl_threshold = entry_spread * (Decimal::ONE + sl_ratio);
        if current_spread >= sl_threshold {
            return Some(ExitDecision {
                method: ExitMethod::ForcedMarket,
                reason: "Stop-loss".to_string(),
                realized_pnl: self.unrealized_pnl,
            });
        }

        None
    }

    /// Holding time in seconds
    pub fn holding_time_secs(&self) -> u64 {
        self.entry_time.elapsed().as_secs()
    }

    /// Check if holding time exceeded
    pub fn holding_time_exceeded(&self, max_secs: u64) -> bool {
        self.holding_time_secs() >= max_secs
    }
}

/// Exit decision
#[derive(Debug, Clone)]
pub struct ExitDecision {
    pub method: ExitMethod,
    pub reason: String,
    pub realized_pnl: Decimal,
}

/// Execution event
#[derive(Debug, Clone)]
pub enum ExecutionEvent {
    /// Position opened
    PositionOpened { position: TrackedPosition },
    /// Position updated
    PositionUpdated { position: TrackedPosition },
    /// Exit triggered
    ExitTriggered { position_id: String, decision: ExitDecision },
    /// Position closed
    PositionClosed { position: TrackedPosition, was_sl: bool },
    /// Error
    Error { position_id: String, error: String },
}

/// Execution engine
pub struct ExecutionEngine {
    positions: Arc<RwLock<HashMap<String, TrackedPosition>>>,
    config: Arc<StrategyConfig>,
    event_tx: Option<mpsc::Sender<ExecutionEvent>>,
}

impl ExecutionEngine {
    pub fn new(config: StrategyConfig) -> Self {
        Self {
            positions: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(config),
            event_tx: None,
        }
    }

    /// Start engine with event channel
    pub fn start(&mut self) -> mpsc::Receiver<ExecutionEvent> {
        let (tx, rx) = mpsc::channel(100);
        self.event_tx = Some(tx);
        rx
    }

    /// Open a new position
    pub fn open_position(
        &self,
        symbol: String,
        direction: Direction,
        order_id: String,
        entry_price: Decimal,
        spread_bps: Decimal,
        volume: Decimal,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();

        let position = TrackedPosition::new(
            id.clone(),
            symbol,
            direction,
            order_id,
            entry_price,
            spread_bps,
            volume,
        );

        let mut positions = self.positions.write();
        positions.insert(id.clone(), position);

        if let Some(ref tx) = self.event_tx {
            let _ = tx.try_send(ExecutionEvent::PositionOpened {
                position: positions.get(&id).unwrap().clone(),
            });
        }

        info!(position_id = %id, "Position opened in execution engine");
        id
    }

    /// Update position state
    pub fn update_position(&self, position_id: &str, state: OrderState, filled: Option<Decimal>) {
        let mut positions = self.positions.write();
        if let Some(pos) = positions.get_mut(position_id) {
            pos.state = state;
            if let Some(f) = filled {
                pos.filled_volume = f;
            }

            if let Some(ref tx) = self.event_tx {
                let _ = tx.try_send(ExecutionEvent::PositionUpdated {
                    position: pos.clone(),
                });
            }
        }
    }

    /// Update PnL
    pub fn update_pnl(&self, position_id: &str, pnl: Decimal) {
        let mut positions = self.positions.write();
        if let Some(pos) = positions.get_mut(position_id) {
            pos.unrealized_pnl = pnl;
        }
    }

    /// Record exit
    pub fn record_exit(&self, position_id: &str, method: ExitMethod, pnl: Decimal) {
        let mut positions = self.positions.write();
        if let Some(pos) = positions.get_mut(position_id) {
            pos.state = OrderState::ExitPending(method);
            pos.exit_method = Some(method);
            pos.realized_pnl = pnl;

            let was_sl = method == ExitMethod::ForcedMarket && pnl < Decimal::ZERO;

            if let Some(ref tx) = self.event_tx {
                let _ = tx.try_send(ExecutionEvent::ExitTriggered {
                    position_id: position_id.to_string(),
                    decision: ExitDecision {
                        method,
                        reason: if was_sl { "Stop-loss" } else { "Take-profit" }.to_string(),
                        realized_pnl: pnl,
                    },
                });
            }
        }
    }

    /// Close position
    pub fn close_position(&self, position_id: &str, was_sl: bool) {
        let mut positions = self.positions.write();
        if let Some(pos) = positions.get_mut(position_id) {
            pos.state = OrderState::ExitFilled;

            if let Some(ref tx) = self.event_tx {
                let _ = tx.try_send(ExecutionEvent::PositionClosed {
                    position: pos.clone(),
                    was_sl,
                });
            }

            info!(
                position_id = %position_id,
                pnl = %pos.realized_pnl,
                was_sl,
                "Position closed"
            );
        }
    }

    /// Get position
    pub fn get_position(&self, position_id: &str) -> Option<TrackedPosition> {
        self.positions.read().get(position_id).cloned()
    }

    /// Get all active positions
    pub fn get_active_positions(&self) -> Vec<TrackedPosition> {
        self.positions.read()
            .values()
            .filter(|p| matches!(p.state, 
                OrderState::Submitted | 
                OrderState::PartiallyFilled | 
                OrderState::ExitPending(_) | 
                OrderState::PendingSubmit
            ))
            .cloned()
            .collect()
    }

    /// Check all positions for exit conditions
    pub fn check_exit_conditions(&self, current_spread: Decimal) -> Vec<(String, ExitDecision)> {
        let mut exits = Vec::new();
        let config = self.config.as_ref();

        let positions = self.positions.read();
        for (id, pos) in positions.iter() {
            if !matches!(pos.state, OrderState::Submitted | OrderState::PartiallyFilled) {
                continue;
            }

            // Check max holding time
            if pos.holding_time_exceeded(config.max_holding_secs) {
                exits.push((
                    id.clone(),
                    ExitDecision {
                        method: ExitMethod::ForcedMarket,
                        reason: "Max holding time exceeded".to_string(),
                        realized_pnl: pos.unrealized_pnl,
                    },
                ));
                continue;
            }

            // Check TP/SL
            if let Some(decision) = pos.check_exit_conditions(
                current_spread,
                config.tp_ratio,
                config.sl_ratio,
            ) {
                exits.push((id.clone(), decision));
            }
        }

        exits
    }

    /// Check GTC order timeout
    pub fn check_gtc_timeout(&self, gtc_timeout_secs: u64) -> Vec<String> {
        let mut timed_out = Vec::new();
        let positions = self.positions.read();

        for (id, pos) in positions.iter() {
            if pos.state == OrderState::ExitPending(ExitMethod::GtcLimit) {
                let elapsed = pos.entry_time.elapsed().as_secs();
                if elapsed >= gtc_timeout_secs {
                    timed_out.push(id.clone());
                }
            }
        }

        timed_out
    }
}

impl Clone for ExecutionEngine {
    fn clone(&self) -> Self {
        Self {
            positions: self.positions.clone(),
            config: self.config.clone(),
            event_tx: self.event_tx.clone(),
        }
    }
}
