//! Core types for symbol selection

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// A trading symbol (e.g., "BTCUSDT")
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Symbol(pub String);

impl Symbol {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for Symbol {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Symbol {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Exchange identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Exchange {
    Binance,
    Lbank,
}

/// Rate limit budget allocation
#[derive(Debug, Clone)]
pub struct RateLimitBudget {
    /// Total requests per second allowed
    pub total: u32,
    /// Reserved for active monitoring (high priority)
    pub active_reserved: u32,
    /// For rotating scan (low priority)
    pub rotating_budget: u32,
}

impl RateLimitBudget {
    pub fn new(total: u32, active_reserved: u32) -> Self {
        assert!(
            active_reserved <= total,
            "Active reserved ({}) cannot exceed total ({})",
            active_reserved,
            total
        );
        Self {
            total,
            active_reserved,
            rotating_budget: total.saturating_sub(active_reserved),
        }
    }

    /// Create with percentage allocation
    pub fn with_percentage(total: u32, active_percent: u32) -> Self {
        let active_reserved = (total * active_percent) / 100;
        Self::new(total, active_reserved)
    }

    pub fn active_budget(&self) -> u32 {
        self.active_reserved
    }

    pub fn rotating_budget(&self) -> u32 {
        self.rotating_budget
    }
}

/// Selection result containing symbols for different purposes
#[derive(Debug, Clone)]
pub struct SelectionResult {
    /// Symbols to actively monitor (high priority)
    pub active_symbols: Vec<String>,
    /// Symbols in current rotating batch
    pub rotating_symbols: Vec<String>,
    /// All available symbols (for reference)
    pub all_available: Vec<String>,
    /// Symbols filtered out (not on watchlist, etc.)
    pub filtered_count: usize,
}

impl SelectionResult {
    pub fn is_empty(&self) -> bool {
        self.active_symbols.is_empty() && self.rotating_symbols.is_empty()
    }

    pub fn total_count(&self) -> usize {
        self.active_symbols.len() + self.rotating_symbols.len()
    }
}

/// Symbol availability info from an exchange
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolAvailability {
    pub exchange: Exchange,
    pub symbols: HashSet<String>,
    pub last_updated: i64,
}

impl SymbolAvailability {
    pub fn new(exchange: Exchange) -> Self {
        Self {
            exchange,
            symbols: HashSet::new(),
            last_updated: 0,
        }
    }

    pub fn with_symbols(mut self, symbols: impl IntoIterator<Item = String>) -> Self {
        self.symbols.extend(symbols);
        self
    }

    pub fn set_last_updated(&mut self, timestamp: i64) {
        self.last_updated = timestamp;
    }

    pub fn contains(&self, symbol: &str) -> bool {
        self.symbols.contains(symbol)
    }

    /// Get intersection with another availability set
    pub fn intersection(&self, other: &SymbolAvailability) -> Vec<String> {
        self.symbols
            .intersection(&other.symbols)
            .cloned()
            .collect()
    }
}

/// Statistics for monitoring
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SelectorStats {
    pub total_symbols: usize,
    pub active_count: usize,
    pub rotating_count: usize,
    pub filtered_count: usize,
    pub last_rotation: Option<i64>,
    pub rotation_count: u64,
}

impl SelectorStats {
    pub fn new(result: &SelectionResult) -> Self {
        Self {
            total_symbols: result.all_available.len(),
            active_count: result.active_symbols.len(),
            rotating_count: result.rotating_symbols.len(),
            filtered_count: result.filtered_count,
            last_rotation: None,
            rotation_count: 0,
        }
    }
}
