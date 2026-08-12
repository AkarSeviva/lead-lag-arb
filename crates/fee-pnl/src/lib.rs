//! Fee and PnL Calculation Module
//!
//! Implements EV model from the strategy research.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Fee configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeConfig {
    /// Taker fee rate (e.g., 0.0006 = 0.06%)
    pub taker_fee: Decimal,
    /// Maker fee rate
    pub maker_fee: Decimal,
    /// Rebate rate (0.0 - 1.0)
    pub rebate_rate: Decimal,
    /// Account type
    pub account_type: AccountType,
}

impl Default for FeeConfig {
    fn default() -> Self {
        Self {
            taker_fee: Decimal::new(6, 4),      // 0.06%
            maker_fee: Decimal::new(2, 4),      // 0.02%
            rebate_rate: Decimal::new(8, 1),     // 80% rebate
            account_type: AccountType::Standard,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum AccountType {
    Standard,
    Vip,
    MarketMaker,
    RebateAgent,
}

/// PnL record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PnlRecord {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub symbol: String,
    pub direction: String,
    pub entry_price: Decimal,
    pub exit_price: Decimal,
    pub volume: Decimal,
    pub entry_fee: Decimal,
    pub exit_fee: Decimal,
    pub gross_pnl: Decimal,
    pub net_pnl: Decimal,
    pub win: bool,
    pub exit_reason: String,
}

/// Fee calculator
pub struct FeeCalculator {
    config: FeeConfig,
}

impl FeeCalculator {
    pub fn new(config: FeeConfig) -> Self {
        Self { config }
    }

    /// Calculate round-trip fee
    pub fn round_trip_fee(&self, notional: Decimal) -> Decimal {
        let taker_total = self.config.taker_fee * Decimal::from(2);
        let rebate = taker_total * self.config.rebate_rate;
        notional * (taker_total - rebate)
    }

    /// Calculate net fee percentage
    pub fn net_fee_pct(&self) -> Decimal {
        let taker_total = self.config.taker_fee * Decimal::from(2);
        let rebate = taker_total * self.config.rebate_rate;
        taker_total - rebate
    }
}

/// EV calculator based on strategy research
pub struct EvCalculator {
    fee_calculator: FeeCalculator,
}

impl EvCalculator {
    pub fn new(fee_config: FeeConfig) -> Self {
        Self {
            fee_calculator: FeeCalculator::new(fee_config),
        }
    }

    /// Calculate expected value per trade
    /// E[Profit] = p × (R - f) + (1 - p) × (-L - f)
    pub fn expected_profit(&self, p: Decimal, r: Decimal, l: Decimal) -> Decimal {
        let f = self.fee_calculator.net_fee_pct();
        p * (r - f) + (Decimal::ONE - p) * (-l - f)
    }

    /// Calculate breakeven win rate
    /// p = (L + f) / (R + L)
    pub fn breakeven_win_rate(&self, r: Decimal, l: Decimal) -> Decimal {
        let f = self.fee_calculator.net_fee_pct();
        (l + f) / (r + l)
    }

    /// Check if strategy is profitable given parameters
    pub fn is_profitable(&self, p: Decimal, r: Decimal, l: Decimal) -> bool {
        self.expected_profit(p, r, l) > Decimal::ZERO
    }
}

/// PnL tracker
pub struct PnlTracker {
    records: Vec<PnlRecord>,
    total_pnl: Decimal,
    win_count: usize,
    loss_count: usize,
}

impl PnlTracker {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            total_pnl: Decimal::ZERO,
            win_count: 0,
            loss_count: 0,
        }
    }

    pub fn record_trade(&mut self, record: PnlRecord) {
        self.records.push(record.clone());
        self.total_pnl += record.net_pnl;
        if record.win {
            self.win_count += 1;
        } else {
            self.loss_count += 1;
        }
    }

    pub fn win_rate(&self) -> Decimal {
        let total = self.win_count + self.loss_count;
        if total == 0 {
            Decimal::ZERO
        } else {
            Decimal::from(self.win_count) / Decimal::from(total)
        }
    }

    pub fn average_win(&self) -> Decimal {
        if self.win_count == 0 {
            Decimal::ZERO
        } else {
            // Would need to track separately
            Decimal::ZERO
        }
    }
}

impl Default for PnlTracker {
    fn default() -> Self {
        Self::new()
    }
}
