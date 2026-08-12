//! Arbitrage Signal Types

use rust_decimal::Decimal;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Signal direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignalDirection {
    Long,  // Buy B (follower's ask) when A.bid > B.ask
    Short, // Sell B (follower's bid) when B.bid > A.ask
    None,
}

impl SignalDirection {
    pub fn is_long(&self) -> bool {
        matches!(self, Self::Long)
    }

    pub fn is_short(&self) -> bool {
        matches!(self, Self::Short)
    }

    pub fn is_active(&self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Spread snapshot for signal calculation
#[derive(Debug, Clone)]
pub struct SpreadSnapshot {
    /// Symbol
    pub symbol: String,
    /// Timestamp
    pub timestamp: i64,
    /// Leader (A) best bid
    pub leader_bid: Decimal,
    /// Leader (A) best ask
    pub leader_ask: Decimal,
    /// Follower (B) best bid
    pub follower_bid: Decimal,
    /// Follower (B) best ask
    pub follower_ask: Decimal,
    /// ΔP_long = P^A_bid - P^B_ask (positive = long signal)
    pub delta_long: Decimal,
    /// ΔP_short = P^B_bid - P^A_ask (positive = short signal)
    pub delta_short: Decimal,
    /// Leader depth at bid (quote currency)
    pub leader_bid_depth: Decimal,
    /// Leader depth at ask (quote currency)
    pub leader_ask_depth: Decimal,
}

impl SpreadSnapshot {
    /// Create from two order books
    pub fn from_orderbooks(
        symbol: &str,
        timestamp: i64,
        leader_bid: Decimal,
        leader_ask: Decimal,
        leader_bid_vol: Decimal,
        leader_ask_vol: Decimal,
        follower_bid: Decimal,
        follower_ask: Decimal,
    ) -> Self {
        let delta_long = leader_bid - follower_ask;
        let delta_short = follower_bid - leader_ask;

        Self {
            symbol: symbol.to_string(),
            timestamp,
            leader_bid,
            leader_ask,
            leader_bid_depth: leader_bid_vol * leader_bid,
            leader_ask_depth: leader_ask_vol * leader_ask,
            follower_bid,
            follower_ask,
            delta_long,
            delta_short,
        }
    }

    /// Spread at entry (for TP/SL calculation)
    pub fn entry_spread(&self, direction: SignalDirection) -> Decimal {
        match direction {
            SignalDirection::Long => self.delta_long,
            SignalDirection::Short => self.delta_short,
            SignalDirection::None => Decimal::ZERO,
        }
    }

    /// Current spread as basis points
    pub fn spread_bps(&self, direction: SignalDirection) -> Decimal {
        let spread = match direction {
            SignalDirection::Long => self.delta_long,
            SignalDirection::Short => self.delta_short,
            SignalDirection::None => return Decimal::ZERO,
        };

        let mid = (self.leader_bid + self.follower_ask) / Decimal::from(2);
        if mid.is_zero() {
            Decimal::ZERO
        } else {
            spread / mid * Decimal::from(10000)
        }
    }

    /// Check if signal is valid
    pub fn is_valid(&self) -> bool {
        self.leader_bid > Decimal::ZERO
            && self.leader_ask > Decimal::ZERO
            && self.follower_bid > Decimal::ZERO
            && self.follower_ask > Decimal::ZERO
            && self.leader_bid < self.leader_ask  // bid < ask
            && self.follower_bid < self.follower_ask  // bid < ask
    }
}

/// Arbitrage signal
#[derive(Debug, Clone)]
pub struct ArbitrageSignal {
    /// Signal ID
    pub id: String,
    /// Symbol
    pub symbol: String,
    /// Direction
    pub direction: SignalDirection,
    /// Entry spread (in quote currency)
    pub entry_spread: Decimal,
    /// Entry spread as basis points
    pub entry_spread_bps: Decimal,
    /// Leader best bid/ask at entry
    pub leader_bid: Decimal,
    pub leader_ask: Decimal,
    /// Follower best bid/ask at entry
    pub follower_bid: Decimal,
    pub follower_ask: Decimal,
    /// When signal was generated
    pub timestamp: i64,
    /// Local timestamp
    pub local_ts: i64,
    /// Estimated execution price (follower side)
    pub expected_entry_price: Decimal,
    /// Filters that passed
    pub filters_passed: Vec<String>,
    /// Signal quality score (0-1)
    pub quality_score: Decimal,
}

impl ArbitrageSignal {
    /// Create a new signal
    pub fn new(
        symbol: String,
        direction: SignalDirection,
        snapshot: &SpreadSnapshot,
        filters_passed: Vec<String>,
        quality_score: Decimal,
    ) -> Self {
        let expected_entry_price = match direction {
            SignalDirection::Long => snapshot.follower_ask,
            SignalDirection::Short => snapshot.follower_bid,
            SignalDirection::None => Decimal::ZERO,
        };

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            symbol,
            direction,
            entry_spread: snapshot.entry_spread(direction),
            entry_spread_bps: snapshot.spread_bps(direction),
            leader_bid: snapshot.leader_bid,
            leader_ask: snapshot.leader_ask,
            follower_bid: snapshot.follower_bid,
            follower_ask: snapshot.follower_ask,
            timestamp: snapshot.timestamp,
            local_ts: chrono::Utc::now().timestamp_millis(),
            expected_entry_price,
            filters_passed,
            quality_score,
        }
    }

    /// Check if signal is still valid
    pub fn is_valid(&self, current_spread: Decimal, max_deviation_bps: Decimal) -> bool {
        let current_bps = self.entry_spread_bps;
        let deviation = ((current_spread - self.entry_spread) / self.entry_spread * Decimal::from(10000)).abs();
        deviation <= max_deviation_bps
    }

    /// Calculate TP/SL levels based on entry spread
    pub fn calc_tp_sl(&self, tp_ratio: Decimal, sl_ratio: Decimal) -> (Decimal, Decimal) {
        let entry = self.entry_spread;
        let tp = entry * tp_ratio;
        let sl = entry * sl_ratio;
        (tp, sl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spread_snapshot() {
        let snapshot = SpreadSnapshot::from_orderbooks(
            "BTCUSDT",
            1000,
            Decimal::new(63990, 0),   // leader_bid (买入价, 较低)
            Decimal::new(64010, 0),   // leader_ask (卖出价, 较高)
            Decimal::new(1, 0),       // leader_bid_vol
            Decimal::new(1, 0),        // leader_ask_vol
            Decimal::new(64000, 0),   // follower_bid
            Decimal::new(64005, 0),    // follower_ask
        );

        assert!(snapshot.is_valid());
        // delta_long = leader_bid(63990) - follower_ask(64005) = -15
        assert_eq!(snapshot.delta_long, Decimal::new(-15, 0));
        // delta_short = follower_bid(64000) - leader_ask(64010) = -10
        assert_eq!(snapshot.delta_short, Decimal::new(-10, 0));
    }

    #[test]
    fn test_signal_direction() {
        assert!(SignalDirection::Long.is_long());
        assert!(!SignalDirection::Long.is_short());
        assert!(SignalDirection::Short.is_short());
        assert!(!SignalDirection::Short.is_long());
    }
}
