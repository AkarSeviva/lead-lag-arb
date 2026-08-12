//! Position Tracker

use rust_decimal::Decimal;
use parking_lot::RwLock;
use std::collections::HashMap;
use config::Direction;

/// Tracked position
#[derive(Debug, Clone)]
pub struct Position {
    pub symbol: String,
    pub direction: Direction,
    pub size: Decimal,         // Volume
    pub entry_price: Decimal,   // Average entry price
    pub notional: Decimal,     // Position size in quote currency
    pub unrealized_pnl: Decimal,
    pub filled: Decimal,       // Filled volume
    pub entry_time: chrono::DateTime<chrono::Utc>,
}

impl Position {
    pub fn new(symbol: String, direction: Direction, size: Decimal, entry_price: Decimal) -> Self {
        Self {
            symbol,
            direction,
            size,
            entry_price,
            notional: size * entry_price,
            unrealized_pnl: Decimal::ZERO,
            filled: Decimal::ZERO,
            entry_time: chrono::Utc::now(),
        }
    }

    pub fn update_pnl(&mut self, current_price: Decimal) {
        let pnl_per_unit = match self.direction {
            Direction::Long => current_price - self.entry_price,
            Direction::Short => self.entry_price - current_price,
        };
        self.unrealized_pnl = pnl_per_unit * self.filled;
    }

    pub fn is_profitable(&self) -> bool {
        self.unrealized_pnl > Decimal::ZERO
    }

    pub fn is_breakeven(&self) -> bool {
        self.unrealized_pnl == Decimal::ZERO
    }
}

/// Position tracker
pub struct PositionTracker {
    positions: RwLock<HashMap<String, Position>>,
}

impl PositionTracker {
    pub fn new() -> Self {
        Self {
            positions: RwLock::new(HashMap::new()),
        }
    }

    pub fn add(&self, position: Position) {
        let mut positions = self.positions.write();
        positions.insert(position.symbol.clone(), position);
    }

    pub fn get(&self, symbol: &str) -> Option<Position> {
        let positions = self.positions.read();
        positions.get(symbol).cloned()
    }

    pub fn remove(&self, symbol: &str) {
        let mut positions = self.positions.write();
        positions.remove(symbol);
    }

    pub fn update(&self, symbol: &str, pnl: Decimal, filled: Decimal) {
        let mut positions = self.positions.write();
        if let Some(pos) = positions.get_mut(symbol) {
            pos.unrealized_pnl = pnl;
            pos.filled = filled;
        }
    }

    pub fn get_all(&self) -> Vec<Position> {
        let positions = self.positions.read();
        positions.values().cloned().collect()
    }

    pub fn active_count(&self) -> usize {
        let positions = self.positions.read();
        positions.len()
    }

    pub fn total_exposure(&self) -> Decimal {
        let positions = self.positions.read();
        positions.values().map(|p| p.notional).sum()
    }

    pub fn total_pnl(&self) -> Decimal {
        let positions = self.positions.read();
        positions.values().map(|p| p.unrealized_pnl).sum()
    }

    pub fn clear(&self) {
        let mut positions = self.positions.write();
        positions.clear();
    }

    pub fn has_position(&self, symbol: &str) -> bool {
        let positions = self.positions.read();
        positions.contains_key(symbol)
    }
}

impl Default for PositionTracker {
    fn default() -> Self {
        Self::new()
    }
}
