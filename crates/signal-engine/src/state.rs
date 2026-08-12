//! Signal State Machine

use crate::context::State;
use crate::signal::{ArbitrageSignal, SignalDirection};
use parking_lot::RwLock;
use rust_decimal::Decimal;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;
use chrono::Utc;

/// Ring buffer for spread history
pub struct SpreadRingBuffer {
    entries: VecDeque<SpreadEntry>,
    capacity: usize,
}

impl SpreadRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, entry: SpreadEntry) {
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn entries(&self) -> &VecDeque<SpreadEntry> {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Spread history entry
#[derive(Debug, Clone)]
pub struct SpreadEntry {
    pub timestamp: Instant,
    pub spread_bps: Decimal,
    pub direction: SignalDirection,
}

/// Signal state manager
pub struct SignalState {
    /// Current state
    pub state: State,
    /// Time in current state
    pub state_since: Instant,
    /// Spread history
    pub spread_history: SpreadRingBuffer,
    /// Current signal
    pub current_signal: Option<ArbitrageSignal>,
    /// Signal count (for statistics)
    pub signals_scanned: u64,
    pub signals_qualified: u64,
    pub signals_executed: u64,
    /// Cooldown end time
    pub cooldown_until: Option<Instant>,
}

impl SignalState {
    pub fn new(spread_history_size: usize) -> Self {
        Self {
            state: State::Idle,
            state_since: Instant::now(),
            spread_history: SpreadRingBuffer::new(spread_history_size),
            current_signal: None,
            signals_scanned: 0,
            signals_qualified: 0,
            signals_executed: 0,
            cooldown_until: None,
        }
    }

    /// Transition to new state
    pub fn transition(&mut self, new_state: State) {
        if self.state != new_state {
            tracing::debug!(from = ?self.state, to = ?new_state, "State transition");
            self.state = new_state;
            self.state_since = Instant::now();
        }
    }

    /// Check if in cooldown
    pub fn is_in_cooldown(&self) -> bool {
        if let Some(until) = self.cooldown_until {
            Instant::now() < until
        } else {
            false
        }
    }

    /// Set cooldown
    pub fn set_cooldown(&mut self, duration_secs: u64) {
        self.cooldown_until = Some(Instant::now() + std::time::Duration::from_secs(duration_secs));
        self.transition(State::Cooldown);
    }

    /// Clear cooldown
    pub fn clear_cooldown(&mut self) {
        self.cooldown_until = None;
    }

    /// Increment scanned count
    pub fn inc_scanned(&mut self) {
        self.signals_scanned += 1;
    }

    /// Increment qualified count
    pub fn inc_qualified(&mut self) {
        self.signals_qualified += 1;
    }

    /// Increment executed count
    pub fn inc_executed(&mut self) {
        self.signals_executed += 1;
    }

    /// Reset for new cycle
    pub fn reset(&mut self) {
        self.transition(State::Idle);
        self.current_signal = None;
    }
}

impl Default for SignalState {
    fn default() -> Self {
        Self::new(100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_transition() {
        let mut state = SignalState::default();
        assert_eq!(state.state, State::Idle);

        state.transition(State::Monitoring);
        assert_eq!(state.state, State::Monitoring);

        state.transition(State::InPosition);
        assert_eq!(state.state, State::InPosition);
    }

    #[test]
    fn test_cooldown() {
        let mut state = SignalState::default();
        assert!(!state.is_in_cooldown());

        state.set_cooldown(5);
        assert!(state.is_in_cooldown());

        state.clear_cooldown();
        assert!(!state.is_in_cooldown());
    }

    #[test]
    fn test_ring_buffer() {
        let mut buf = SpreadRingBuffer::new(3);
        
        buf.push(SpreadEntry {
            timestamp: Instant::now(),
            spread_bps: Decimal::new(10, 0),
            direction: SignalDirection::Long,
        });
        
        assert_eq!(buf.len(), 1);
        
        buf.push(SpreadEntry {
            timestamp: Instant::now(),
            spread_bps: Decimal::new(20, 0),
            direction: SignalDirection::Long,
        });
        buf.push(SpreadEntry {
            timestamp: Instant::now(),
            spread_bps: Decimal::new(30, 0),
            direction: SignalDirection::Long,
        });
        
        assert_eq!(buf.len(), 3);
        
        // Should evict oldest
        buf.push(SpreadEntry {
            timestamp: Instant::now(),
            spread_bps: Decimal::new(40, 0),
            direction: SignalDirection::Long,
        });
        
        assert_eq!(buf.len(), 3);
    }
}
