//! Main symbol selector implementation

use crate::config::{SelectorConfig, Strategy};
use crate::pool::{ActivePool, PoolManager, RotatingPool};
use crate::types::{SelectionResult, SymbolAvailability};
use parking_lot::RwLock;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Symbol selector that filters and selects symbols based on configuration
pub struct SymbolSelector {
    config: SelectorConfig,
    pool_manager: Arc<PoolManager>,
    /// Symbols available on Binance
    binance_availability: RwLock<Option<SymbolAvailability>>,
    /// Symbols available on Lbank
    lbank_availability: RwLock<Option<SymbolAvailability>>,
    /// Watchlist override (for runtime updates)
    watchlist: RwLock<HashSet<String>>,
    /// Event channel for selection changes
    selection_tx: broadcast::Sender<SelectionEvent>,
}

impl SymbolSelector {
    /// Create a new selector with configuration
    pub fn new(config: SelectorConfig) -> Self {
        let rate_limit = &config.rate_limit;
        let pool_manager = Arc::new(PoolManager::from_config(
            rate_limit.total,
            rate_limit.active_reserved,
            rate_limit.rotating_batch_size,
            rate_limit.rotation_interval_secs,
        ));

        let (selection_tx, _) = broadcast::channel(100);

        Self {
            config,
            pool_manager,
            binance_availability: RwLock::new(None),
            lbank_availability: RwLock::new(None),
            watchlist: RwLock::new(HashSet::new()),
            selection_tx,
        }
    }

    /// Get a clone of the selection event receiver
    pub fn selection_receiver(&self) -> broadcast::Receiver<SelectionEvent> {
        self.selection_rx()
    }

    fn selection_rx(&self) -> broadcast::Receiver<SelectionEvent> {
        self.selection_tx.subscribe()
    }

    /// Update symbol availability for Binance
    pub fn update_binance_symbols(&self, symbols: Vec<String>) {
        let mut availability = self.binance_availability.write();
        let mut avail = SymbolAvailability::new(crate::types::Exchange::Binance);
        avail.symbols.extend(symbols);
        avail.set_last_updated(chrono::Utc::now().timestamp());
        *availability = Some(avail);
    }

    /// Update symbol availability for Lbank
    pub fn update_lbank_symbols(&self, symbols: Vec<String>) {
        let mut availability = self.lbank_availability.write();
        let mut avail = SymbolAvailability::new(crate::types::Exchange::Lbank);
        avail.symbols.extend(symbols);
        avail.set_last_updated(chrono::Utc::now().timestamp());
        *availability = Some(avail);
    }

    /// Get active pool for direct manipulation
    pub fn active_pool(&self) -> Arc<ActivePool> {
        self.pool_manager.active_pool()
    }

    /// Get rotating pool for inspection
    pub fn rotating_pool(&self) -> Arc<RotatingPool> {
        self.pool_manager.rotating_pool()
    }

    /// Add symbol to active monitoring (high priority)
    pub fn add_active(&self, symbol: &str, priority: u8) {
        self.pool_manager.add_active(symbol, priority);
        let _ = self.selection_tx.send(SelectionEvent::ActiveAdded(symbol.to_string()));
    }

    /// Remove symbol from active monitoring
    pub fn remove_active(&self, symbol: &str) {
        self.pool_manager.remove_active(symbol);
        let _ = self.selection_tx.send(SelectionEvent::ActiveRemoved(symbol.to_string()));
    }

    /// Update watchlist at runtime
    pub fn update_watchlist(&self, symbols: Vec<String>) {
        let mut watchlist = self.watchlist.write();
        *watchlist = symbols.into_iter().collect();
        let _ = self.selection_tx.send(SelectionEvent::WatchlistUpdated);
    }

    /// Perform selection based on current state
    pub fn select(&self) -> SelectionResult {
        let binance = self.binance_availability.read();
        let lbank = self.lbank_availability.read();

        // Get common symbols
        let common_symbols = match (&*binance, &*lbank) {
            (Some(b), Some(l)) => b.intersection(l),
            (Some(b), None) | (None, Some(b)) => b.symbols.iter().cloned().collect(),
            (None, None) => Vec::new(),
        };

        let total_common = common_symbols.len();

        // Apply watchlist filter if configured
        let watchlist = self.watchlist.read();
        let watch_refs: Vec<&str> = if watchlist.is_empty() {
            self.config.watchlist.iter().map(|s| s.as_str()).collect()
        } else {
            watchlist.iter().map(|s| s.as_str()).collect()
        };

        let filtered: Vec<String> = if watch_refs.is_empty() {
            common_symbols
        } else {
            common_symbols
                .into_iter()
                .filter(|s| watch_refs.contains(&s.as_str()))
                .collect()
        };

        let filtered_count = total_common - filtered.len();

        // Set rotating pool symbols (excluding active)
        self.pool_manager.set_rotating_symbols(filtered.clone());

        // Get selection from pool manager
        let mut result = self.pool_manager.get_selection();
        result.filtered_count = filtered_count;

        result
    }

    /// Try to rotate the rotating pool if interval has elapsed
    pub fn try_rotate(&self, now: i64) -> Option<Vec<String>> {
        if let Some(new_batch) = self.pool_manager.maybe_rotate(now) {
            let _ = self.selection_tx.send(SelectionEvent::Rotated(new_batch.clone()));
            Some(new_batch)
        } else {
            None
        }
    }

    /// Get current selection without triggering rotation
    pub fn get_current_selection(&self) -> SelectionResult {
        self.pool_manager.get_selection()
    }

    /// Get budget info
    pub fn budget_info(&self) -> (u32, u32, u32) {
        let budget = self.pool_manager.budget();
        (budget.total, budget.active_reserved, budget.rotating_budget)
    }
}

/// Events emitted by the selector
#[derive(Debug, Clone)]
pub enum SelectionEvent {
    /// A symbol was added to active monitoring
    ActiveAdded(String),
    /// A symbol was removed from active monitoring
    ActiveRemoved(String),
    /// The rotating pool was rotated
    Rotated(Vec<String>),
    /// Watchlist was updated
    WatchlistUpdated,
}

/// Builder for SymbolSelector
pub struct SymbolSelectorBuilder {
    config: SelectorConfig,
}

impl SymbolSelectorBuilder {
    pub fn new() -> Self {
        Self {
            config: SelectorConfig::default(),
        }
    }

    pub fn strategy(mut self, strategy: Strategy) -> Self {
        self.config.strategy = strategy;
        self
    }

    pub fn rate_limit(mut self, total: u32, active_reserved: u32) -> Self {
        self.config.rate_limit.total = total;
        self.config.rate_limit.active_reserved = active_reserved;
        self
    }

    pub fn batch_size(mut self, size: usize) -> Self {
        self.config.rate_limit.rotating_batch_size = size;
        self
    }

    pub fn rotation_interval(mut self, secs: u64) -> Self {
        self.config.rate_limit.rotation_interval_secs = secs;
        self
    }

    pub fn watchlist(mut self, symbols: Vec<String>) -> Self {
        self.config.watchlist = symbols;
        self
    }

    pub fn require_both_exchanges(mut self, required: bool) -> Self {
        self.config.require_both_exchanges = required;
        self
    }

    pub fn build(self) -> SymbolSelector {
        SymbolSelector::new(self.config)
    }
}

impl Default for SymbolSelectorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Start background rotation task
pub async fn start_rotation_task(selector: Arc<SymbolSelector>, mut shutdown: tokio::sync::oneshot::Receiver<()>) {
    use tokio::time::{interval, Duration};

    let batch_size = selector.rotating_pool().batch_size() as u64;
    let mut ticker = interval(Duration::from_secs(batch_size));

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("Rotation task shutting down");
                break;
            }
            _ = ticker.tick() => {
                let now = chrono::Utc::now().timestamp();
                selector.try_rotate(now);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_selector_basic() {
        let selector = SymbolSelectorBuilder::new()
            .rate_limit(100, 10)
            .batch_size(5)
            .rotation_interval(1)
            .build();

        selector.update_binance_symbols(vec![
            "BTCUSDT".into(),
            "ETHUSDT".into(),
            "DOGEUSDT".into(),
            "XRPUSDT".into(),
            "ADAUSDT".into(),
        ]);

        selector.update_lbank_symbols(vec![
            "BTCUSDT".into(),
            "ETHUSDT".into(),
            "DOGEUSDT".into(),
            "SOLUSDT".into(),
        ]);

        let result = selector.select();

        // BTCUSDT, ETHUSDT, DOGEUSDT should be in intersection
        assert!(result.all_available.contains(&"BTCUSDT".to_string()));
        assert!(result.all_available.contains(&"ETHUSDT".to_string()));
        assert!(result.all_available.contains(&"DOGEUSDT".to_string()));
        assert!(!result.all_available.contains(&"SOLUSDT".to_string())); // Not on Binance
    }

    #[tokio::test]
    async fn test_watchlist_filter() {
        let selector = SymbolSelectorBuilder::new()
            .rate_limit(100, 10)
            .watchlist(vec!["BTCUSDT".into(), "ETHUSDT".into()])
            .build();

        selector.update_binance_symbols(vec![
            "BTCUSDT".into(),
            "ETHUSDT".into(),
            "DOGEUSDT".into(),
            "XRPUSDT".into(),
        ]);

        selector.update_lbank_symbols(vec![
            "BTCUSDT".into(),
            "ETHUSDT".into(),
            "DOGEUSDT".into(),
            "XRPUSDT".into(),
        ]);

        let result = selector.select();

        // Only watchlist symbols should be in rotating
        assert!(result.all_available.len() <= 2);
    }

    #[tokio::test]
    async fn test_active_symbols() {
        let selector = SymbolSelectorBuilder::new()
            .rate_limit(100, 10)
            .batch_size(5)
            .build();

        selector.update_binance_symbols(vec![
            "BTCUSDT".into(),
            "ETHUSDT".into(),
            "DOGEUSDT".into(),
        ]);

        selector.update_lbank_symbols(vec![
            "BTCUSDT".into(),
            "ETHUSDT".into(),
            "DOGEUSDT".into(),
        ]);

        // Add to active pool
        selector.add_active("BTCUSDT", 10);

        let result = selector.select();

        assert!(result.active_symbols.contains(&"BTCUSDT".to_string()));
        assert_eq!(result.active_symbols.len(), 1);

        // BTCUSDT should not be in rotating
        let rotating_all = selector.rotating_pool().get_all_symbols();
        assert!(!rotating_all.contains(&"BTCUSDT".to_string()));
    }

    #[tokio::test]
    async fn test_rotation() {
        let selector = SymbolSelectorBuilder::new()
            .rate_limit(100, 10)
            .batch_size(2)
            .rotation_interval(0) // Immediate rotation
            .build();

        selector.update_binance_symbols(vec![
            "A".into(),
            "B".into(),
            "C".into(),
            "D".into(),
        ]);

        selector.update_lbank_symbols(vec![
            "A".into(),
            "B".into(),
            "C".into(),
            "D".into(),
        ]);

        selector.select();

        // First rotation
        let now = chrono::Utc::now().timestamp();
        let batch1 = selector.try_rotate(now).unwrap_or_default();
        assert_eq!(batch1.len(), 2);

        // Second rotation
        let batch2 = selector.try_rotate(now).unwrap_or_default();
        assert_eq!(batch2.len(), 2);

        // Batches should be different
        assert_ne!(batch1, batch2);
    }

    #[tokio::test]
    async fn test_event_emission() {
        let selector = SymbolSelectorBuilder::new()
            .rate_limit(100, 10)
            .build();

        let mut rx = selector.selection_receiver();

        selector.add_active("BTCUSDT", 10);

        // Should receive ActiveAdded event
        let event = rx.try_recv().unwrap();
        match event {
            SelectionEvent::ActiveAdded(s) => assert_eq!(s, "BTCUSDT"),
            _ => panic!("Expected ActiveAdded event"),
        }
    }
}
