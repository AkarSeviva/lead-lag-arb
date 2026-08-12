//! Pool management for active and rotating symbols

use crate::types::{RateLimitBudget, SelectionResult};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

/// Active monitoring pool - symbols that are currently being traded or closely watched
pub struct ActivePool {
    symbols: RwLock<HashSet<String>>,
    priority: RwLock<HashMap<String, u8>>, // Higher = more priority
}

impl ActivePool {
    pub fn new() -> Self {
        Self {
            symbols: RwLock::new(HashSet::new()),
            priority: RwLock::new(HashMap::new()),
        }
    }

    /// Add a symbol to active monitoring
    pub fn add(&self, symbol: &str, priority: u8) {
        self.symbols.write().insert(symbol.to_string());
        self.priority.write().insert(symbol.to_string(), priority);
    }

    /// Remove a symbol from active monitoring
    pub fn remove(&self, symbol: &str) {
        self.symbols.write().remove(symbol);
        self.priority.write().remove(symbol);
    }

    /// Get all active symbols sorted by priority (highest first)
    pub fn get_all(&self) -> Vec<String> {
        let symbols = self.symbols.read();
        let mut result: Vec<_> = symbols.iter().cloned().collect();
        let priority = self.priority.read();
        result.sort_by(|a, b| {
            priority
                .get(b)
                .unwrap_or(&0)
                .cmp(priority.get(a).unwrap_or(&0))
        });
        result
    }

    /// Check if a symbol is active
    pub fn contains(&self, symbol: &str) -> bool {
        self.symbols.read().contains(symbol)
    }

    /// Get count of active symbols
    pub fn len(&self) -> usize {
        self.symbols.read().len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.symbols.read().is_empty()
    }

    /// Update priority for a symbol
    pub fn update_priority(&self, symbol: &str, priority: u8) {
        if self.symbols.read().contains(symbol) {
            self.priority.write().insert(symbol.to_string(), priority);
        }
    }
}

impl Default for ActivePool {
    fn default() -> Self {
        Self::new()
    }
}

/// Rotating pool - scans through all symbols in batches to find opportunities
pub struct RotatingPool {
    /// All symbols available for rotation
    all_symbols: RwLock<VecDeque<String>>,
    /// Symbols currently in active scan batch
    current_batch: RwLock<Vec<String>>,
    /// Current position in rotation
    position: RwLock<usize>,
    /// Batch size for each rotation
    batch_size: usize,
    /// Rotation interval in seconds
    rotation_interval_secs: u64,
    /// Last rotation timestamp
    last_rotation: RwLock<Option<i64>>,
    /// Count of rotations completed
    rotation_count: RwLock<u64>,
}

impl RotatingPool {
    pub fn new(batch_size: usize, rotation_interval_secs: u64) -> Self {
        Self {
            all_symbols: RwLock::new(VecDeque::new()),
            current_batch: RwLock::new(Vec::new()),
            position: RwLock::new(0),
            batch_size,
            rotation_interval_secs,
            last_rotation: RwLock::new(None),
            rotation_count: RwLock::new(0),
        }
    }

    /// Set the complete list of symbols to rotate through
    pub fn set_symbols(&self, symbols: impl IntoIterator<Item = String>) {
        let mut all = self.all_symbols.write();
        let mut sorted: Vec<_> = symbols.into_iter().collect();
        sorted.sort();
        *all = VecDeque::from(sorted);
    }

    /// Add a single symbol to rotation
    pub fn add_symbol(&self, symbol: String) {
        let mut all = self.all_symbols.write();
        if !all.contains(&symbol) {
            all.push_back(symbol);
            // Sort by converting to Vec
            let mut sorted: Vec<_> = all.drain(..).collect();
            sorted.sort();
            *all = VecDeque::from(sorted);
        }
    }

    /// Remove a symbol from rotation
    pub fn remove_symbol(&self, symbol: &str) {
        let mut all = self.all_symbols.write();
        all.retain(|s| s != symbol);
    }

    /// Check if rotation should happen based on time
    pub fn should_rotate(&self, now: i64) -> bool {
        let last = *self.last_rotation.read();
        match last {
            None => true,
            Some(last_time) => {
                let elapsed = (now - last_time) as u64;
                elapsed >= self.rotation_interval_secs
            }
        }
    }

    /// Perform rotation - move to next batch
    pub fn rotate(&self) -> Vec<String> {
        let all = self.all_symbols.read();
        let mut position = self.position.write();
        let batch_size = self.batch_size;

        if all.is_empty() {
            return Vec::new();
        }

        let total = all.len();
        let start = *position;
        let end = (start + batch_size).min(total);

        let batch: Vec<String> = all
            .iter()
            .skip(start)
            .take(end - start)
            .cloned()
            .collect();

        // Update position for next rotation
        *position = if end >= total { 0 } else { end };

        // Update rotation state
        *self.last_rotation.write() = Some(chrono::Utc::now().timestamp());
        *self.rotation_count.write() += 1;

        // Update current batch
        *self.current_batch.write() = batch.clone();

        batch
    }

    /// Get current batch without rotating
    pub fn get_current_batch(&self) -> Vec<String> {
        self.current_batch.read().clone()
    }

    /// Get all symbols (for inspection)
    pub fn get_all_symbols(&self) -> Vec<String> {
        self.all_symbols.read().iter().cloned().collect()
    }

    /// Get count of all symbols
    pub fn total_symbols(&self) -> usize {
        self.all_symbols.read().len()
    }

    /// Get batch size
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// Get rotation count
    pub fn rotation_count(&self) -> u64 {
        *self.rotation_count.read()
    }

    /// Get last rotation timestamp
    pub fn last_rotation(&self) -> Option<i64> {
        *self.last_rotation.read()
    }
}

/// Combined pool manager that coordinates active and rotating pools
pub struct PoolManager {
    active: Arc<ActivePool>,
    rotating: Arc<RotatingPool>,
    budget: RateLimitBudget,
    /// Symbols excluded from rotating (active ones)
    exclude_from_rotating: RwLock<HashSet<String>>,
}

impl PoolManager {
    pub fn new(budget: RateLimitBudget, batch_size: usize, rotation_interval_secs: u64) -> Self {
        Self {
            active: Arc::new(ActivePool::new()),
            rotating: Arc::new(RotatingPool::new(batch_size, rotation_interval_secs)),
            budget,
            exclude_from_rotating: RwLock::new(HashSet::new()),
        }
    }

    pub fn from_config(
        total_rate_limit: u32,
        active_reserved: u32,
        rotating_batch_size: usize,
        rotation_interval_secs: u64,
    ) -> Self {
        let budget = RateLimitBudget::new(total_rate_limit, active_reserved);
        Self::new(budget, rotating_batch_size, rotation_interval_secs)
    }

    /// Get active pool reference
    pub fn active_pool(&self) -> Arc<ActivePool> {
        Arc::clone(&self.active)
    }

    /// Get rotating pool reference
    pub fn rotating_pool(&self) -> Arc<RotatingPool> {
        Arc::clone(&self.rotating)
    }

    /// Get budget info
    pub fn budget(&self) -> &RateLimitBudget {
        &self.budget
    }

    /// Add symbol to active pool
    pub fn add_active(&self, symbol: &str, priority: u8) {
        self.active.add(symbol, priority);
        self.exclude_from_rotating.write().insert(symbol.to_string());
        self.rotating.remove_symbol(symbol);
    }

    /// Remove symbol from active pool
    pub fn remove_active(&self, symbol: &str) {
        self.active.remove(symbol);
        self.exclude_from_rotating.write().remove(symbol);
    }

    /// Set symbols for rotating pool (excluding active ones)
    pub fn set_rotating_symbols(&self, symbols: impl IntoIterator<Item = String>) {
        let exclude = self.exclude_from_rotating.read().clone();
        let filtered: Vec<String> = symbols
            .into_iter()
            .filter(|s| !exclude.contains(s))
            .collect();
        self.rotating.set_symbols(filtered);
    }

    /// Perform rotation if needed and time has elapsed
    pub fn maybe_rotate(&self, now: i64) -> Option<Vec<String>> {
        if self.rotating.should_rotate(now) {
            Some(self.rotating.rotate())
        } else {
            None
        }
    }

    /// Get current selection result
    pub fn get_selection(&self) -> SelectionResult {
        let active_symbols = self.active.get_all();
        let rotating_symbols = self.rotating.get_current_batch();
        let all_available = self.rotating.get_all_symbols();

        SelectionResult {
            active_symbols,
            rotating_symbols,
            all_available,
            filtered_count: 0, // Updated by selector
        }
    }

    /// Calculate how many active slots are available
    pub fn available_active_slots(&self) -> u32 {
        let active_count = self.active.len() as u32;
        self.budget.active_reserved.saturating_sub(active_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_active_pool() {
        let pool = ActivePool::new();

        pool.add("BTCUSDT", 10);
        pool.add("ETHUSDT", 5);
        pool.add("SOLUSDT", 8);

        let symbols = pool.get_all();
        assert_eq!(symbols[0], "BTCUSDT"); // Highest priority
        assert_eq!(symbols[1], "SOLUSDT");
        assert_eq!(symbols[2], "ETHUSDT");

        pool.remove("BTCUSDT");
        assert!(!pool.contains("BTCUSDT"));
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn test_rotating_pool() {
        let pool = RotatingPool::new(3, 1);

        pool.set_symbols(vec![
            "DOGEUSDT".into(),
            "XRPUSDT".into(),
            "ADAUSDT".into(),
            "DOTUSDT".into(),
            "AVAXUSDT".into(),
            "MATICUSDT".into(),
        ]);

        assert_eq!(pool.total_symbols(), 6);
        assert_eq!(pool.batch_size(), 3);

        // First rotation
        let batch1 = pool.rotate();
        assert_eq!(batch1.len(), 3);
        assert!(batch1.contains(&"ADAUSDT".to_string())); // Should be first (sorted)

        // Second rotation
        let batch2 = pool.rotate();
        assert_eq!(batch2.len(), 3);
        assert_eq!(pool.rotation_count(), 2);

        // Third rotation - wraps around
        let batch3 = pool.rotate();
        assert_eq!(batch3.len(), 3);
    }

    #[test]
    fn test_rate_limit_budget() {
        let budget = RateLimitBudget::new(100, 10);
        assert_eq!(budget.active_reserved, 10);
        assert_eq!(budget.rotating_budget, 90);

        let budget2 = RateLimitBudget::with_percentage(100, 20);
        assert_eq!(budget2.active_reserved, 20);
        assert_eq!(budget2.rotating_budget, 80);
    }

    #[test]
    fn test_pool_manager() {
        let manager = PoolManager::from_config(100, 10, 5, 1);

        manager.add_active("BTCUSDT", 10);
        manager.add_active("ETHUSDT", 8);

        manager.set_rotating_symbols(vec![
            "DOGEUSDT".into(),
            "XRPUSDT".into(),
            "ADAUSDT".into(),
        ]);

        let selection = manager.get_selection();
        assert_eq!(selection.active_symbols.len(), 2);
        assert!(selection.active_symbols.contains(&"BTCUSDT".to_string()));

        // Rotating symbols should not include active ones
        let rotating = manager.rotating_pool().get_all_symbols();
        assert!(!rotating.contains(&"BTCUSDT".to_string()));
    }
}
