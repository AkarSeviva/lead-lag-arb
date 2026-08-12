//! Signal Context
//!
//! Shared context for signal calculation and filtering.

use crate::signal::{ArbitrageSignal, SignalDirection, SpreadSnapshot};
use orderbook::book::OrderBook;
use parking_lot::RwLock;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use chrono::Utc;

/// Signal calculation context
pub struct SignalContext {
    /// Symbol
    pub symbol: String,
    /// Leader (Binance) order book
    pub leader_book: Arc<RwLock<OrderBook>>,
    /// Follower (Lbank) order book
    pub follower_book: Arc<RwLock<OrderBook>>,
    /// Entry threshold in basis points
    pub entry_threshold_bps: Decimal,
    /// Current spread snapshot
    pub spread_snapshot: Option<SpreadSnapshot>,
    /// Signal state
    pub state: SignalState,
    /// Filter results
    pub filter_results: HashMap<String, FilterResult>,
}

impl SignalContext {
    pub fn new(symbol: String, entry_threshold_bps: Decimal) -> Self {
        Self {
            symbol,
            leader_book: Arc::new(RwLock::new(OrderBook::new("LEADER".to_string()))),
            follower_book: Arc::new(RwLock::new(OrderBook::new("FOLLOWER".to_string()))),
            entry_threshold_bps,
            spread_snapshot: None,
            state: SignalState::default(),
            filter_results: HashMap::new(),
        }
    }

    /// Update leader order book
    pub fn update_leader_book(&mut self, book: OrderBook) {
        let mut lock = self.leader_book.write();
        *lock = book;
    }

    /// Update follower order book
    pub fn update_follower_book(&mut self, book: OrderBook) {
        let mut lock = self.follower_book.write();
        *lock = book;
    }

    /// Calculate spread snapshot
    pub fn calc_spread(&mut self) -> Option<SpreadSnapshot> {
        let leader = self.leader_book.read();
        let follower = self.follower_book.read();

        let (leader_bid, leader_ask) = (leader.best_bid()?, leader.best_ask()?);
        let (follower_bid, follower_ask) = (follower.best_bid()?, follower.best_ask()?);

        let snapshot = SpreadSnapshot::from_orderbooks(
            &self.symbol,
            chrono::Utc::now().timestamp_millis(),
            leader_bid,
            leader_ask,
            leader.bid_volume_at_levels(1),
            leader.ask_volume_at_levels(1),
            follower_bid,
            follower_ask,
        );

        self.spread_snapshot = Some(snapshot.clone());
        Some(snapshot)
    }

    /// Determine signal direction based on spread
    pub fn calc_direction(&self) -> SignalDirection {
        if let Some(ref snapshot) = self.spread_snapshot {
            let bps = snapshot.spread_bps(SignalDirection::Long).abs();
            if bps >= self.entry_threshold_bps && snapshot.delta_long > Decimal::ZERO {
                return SignalDirection::Long;
            }

            let bps_short = snapshot.spread_bps(SignalDirection::Short).abs();
            if bps_short >= self.entry_threshold_bps && snapshot.delta_short > Decimal::ZERO {
                return SignalDirection::Short;
            }
        }
        SignalDirection::None
    }
}

/// Signal state machine
#[derive(Debug, Clone)]
pub struct SignalState {
    /// Current state
    pub state: State,
    /// Time in current state
    pub state_since: Instant,
    /// Recent spread history for duration tracking
    pub spread_history: Vec<SpreadHistoryEntry>,
    /// Consecutive signal count
    pub signal_count: usize,
    /// Last signal time
    pub last_signal_time: Option<Instant>,
}

impl Default for SignalState {
    fn default() -> Self {
        Self {
            state: State::Idle,
            state_since: Instant::now(),
            spread_history: Vec::new(),
            signal_count: 0,
            last_signal_time: None,
        }
    }
}

impl SignalState {
    /// Update state
    pub fn set_state(&mut self, new_state: State) {
        if self.state != new_state {
            self.state = new_state;
            self.state_since = Instant::now();
        }
    }

    /// Record spread for duration tracking
    pub fn record_spread(&mut self, spread_bps: Decimal, duration_ms: u64) {
        self.spread_history.push(SpreadHistoryEntry {
            timestamp: Instant::now(),
            spread_bps,
            duration_ms,
        });

        // Keep only last 100 entries
        if self.spread_history.len() > 100 {
            self.spread_history.remove(0);
        }
    }

    /// Check if spread has persisted for minimum duration
    pub fn spread_persisted(&self, min_duration_ms: u64) -> bool {
        if self.spread_history.len() < 2 {
            return false;
        }

        let now = Instant::now();
        let oldest = self.spread_history.first().unwrap();

        let elapsed = now.duration_since(oldest.timestamp);
        elapsed >= Duration::from_millis(min_duration_ms)
    }

    /// Increment signal count
    pub fn increment_signal(&mut self) {
        self.signal_count += 1;
        self.last_signal_time = Some(Instant::now());
    }

    /// Reset state
    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.state_since = Instant::now();
        self.spread_history.clear();
    }
}

/// State machine states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Idle, waiting for signal
    Idle,
    /// Spread detected, monitoring
    Monitoring,
    /// Signal qualified, ready to trade
    Qualified,
    /// In position
    InPosition,
    /// Cooldown after stop-loss
    Cooldown,
}

/// Spread history entry
#[derive(Debug, Clone)]
pub struct SpreadHistoryEntry {
    pub timestamp: Instant,
    pub spread_bps: Decimal,
    pub duration_ms: u64,
}

/// Filter result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterResult {
    pub passed: bool,
    pub reason: Option<&'static str>,
}

impl FilterResult {
    pub fn pass() -> Self {
        Self { passed: true, reason: None }
    }

    pub fn fail(reason: &'static str) -> Self {
        Self { passed: false, reason: Some(reason) }
    }
}
