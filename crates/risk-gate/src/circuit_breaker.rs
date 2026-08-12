//! Circuit Breaker

use std::time::Instant;
use parking_lot::RwLock;

/// Circuit breaker state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Normal,
    Warning,
    Open,
    HalfOpen,
}

/// Circuit breaker
pub struct CircuitBreaker {
    state: RwLock<CircuitState>,
    consecutive_losses: RwLock<usize>,
    threshold: usize,
    pause_duration_secs: u64,
    opened_at: RwLock<Option<Instant>>,
}

impl CircuitBreaker {
    pub fn new(threshold: usize, pause_secs: u64) -> Self {
        Self {
            state: RwLock::new(CircuitState::Normal),
            consecutive_losses: RwLock::new(0),
            threshold,
            pause_duration_secs: pause_secs,
            opened_at: RwLock::new(None),
        }
    }

    pub fn check(&self) -> crate::gate::RiskCheckResult {
        let state = *self.state.read();
        
        match state {
            CircuitState::Normal | CircuitState::Warning => {
                crate::gate::RiskCheckResult::allow("circuit_breaker")
            }
            CircuitState::Open => {
                let opened_at = self.opened_at.read();
                if let Some(at) = *opened_at {
                    let elapsed = Instant::now().duration_since(at);
                    let pause = std::time::Duration::from_secs(self.pause_duration_secs);
                    
                    if elapsed >= pause {
                        // Transition to half-open
                        *self.state.write() = CircuitState::HalfOpen;
                        return crate::gate::RiskCheckResult::allow("circuit_breaker");
                    }
                }
                
                crate::gate::RiskCheckResult::deny(
                    "circuit_breaker",
                    format!("Circuit breaker open, {:.1}s remaining", 
                        self.pause_duration_secs as f64),
                )
            }
            CircuitState::HalfOpen => {
                // Allow limited trading to test
                crate::gate::RiskCheckResult::allow("circuit_breaker")
            }
        }
    }

    pub fn record_loss(&self) {
        let mut losses = self.consecutive_losses.write();
        *losses += 1;
        
        if *losses >= self.threshold {
            *self.state.write() = CircuitState::Open;
            *self.opened_at.write() = Some(Instant::now());
        } else if *losses >= self.threshold / 2 {
            *self.state.write() = CircuitState::Warning;
        }
    }

    pub fn record_win(&self) {
        let mut losses = self.consecutive_losses.write();
        *losses = 0;
        *self.state.write() = CircuitState::Normal;
    }

    pub fn state(&self) -> CircuitState {
        *self.state.read()
    }

    pub fn reset(&self) {
        *self.state.write() = CircuitState::Normal;
        *self.consecutive_losses.write() = 0;
        *self.opened_at.write() = None;
    }

    pub fn consecutive_losses(&self) -> usize {
        *self.consecutive_losses.read()
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(5, 300) // 5 losses, 5 min pause
    }
}
