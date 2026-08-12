//! Exit Strategy
//!
//! Implements different exit methods based on strategy.

use config::{Direction, strategy::StrategyConfig};
use rust_decimal::Decimal;
use crate::state_machine::{ExitDecision, ExitMethod};

/// Exit strategy manager
pub struct ExitStrategy {
    config: StrategyConfig,
    use_method_1: bool, // True = trigger GTC on TP condition, False = pre-set GTC
}

impl ExitStrategy {
    pub fn new(config: StrategyConfig, use_method_1: bool) -> Self {
        Self {
            config,
            use_method_1,
        }
    }

    /// Determine exit method based on conditions
    pub fn decide_exit(
        &self,
        current_spread_bps: Decimal,
        entry_spread_bps: Decimal,
        holding_time_secs: u64,
        gtc_pending: bool,
    ) -> Option<ExitDecision> {
        let tp_ratio = &self.config.tp_ratio;
        let sl_ratio = &self.config.sl_ratio;

        // TP condition: spread converged
        let tp_threshold = entry_spread_bps * (Decimal::ONE - *tp_ratio);
        if current_spread_bps <= tp_threshold {
            return Some(self.trigger_tp(gtc_pending));
        }

        // SL condition: spread widened significantly
        let sl_threshold = entry_spread_bps * (Decimal::ONE + *sl_ratio);
        if current_spread_bps >= sl_threshold {
            return Some(ExitDecision {
                method: ExitMethod::ForcedMarket,
                reason: "Stop-loss".to_string(),
                realized_pnl: Decimal::ZERO,
            });
        }

        // Max holding time
        if holding_time_secs >= self.config.max_holding_secs {
            return Some(ExitDecision {
                method: ExitMethod::ForcedMarket,
                reason: "Max holding time".to_string(),
                realized_pnl: Decimal::ZERO,
            });
        }

        None
    }

    /// Trigger take-profit exit
    fn trigger_tp(&self, gtc_pending: bool) -> ExitDecision {
        if self.use_method_1 {
            // Method 1: Trigger GTC on current price
            ExitDecision {
                method: ExitMethod::GtcLimit,
                reason: "TP condition met (Method 1)".to_string(),
                realized_pnl: Decimal::ZERO,
            }
        } else {
            // Method 2: Should have pre-set GTC already
            if gtc_pending {
                ExitDecision {
                    method: ExitMethod::TakerIoc,
                    reason: "GTC not filled, aggressive exit (Method 2)".to_string(),
                    realized_pnl: Decimal::ZERO,
                }
            } else {
                ExitDecision {
                    method: ExitMethod::GtcLimit,
                    reason: "TP condition met (Method 2)".to_string(),
                    realized_pnl: Decimal::ZERO,
                }
            }
        }
    }

    /// Decide what to do when GTC times out
    pub fn gtc_timeout_decision(&self) -> ExitMethod {
        // Fallback to taker IOC
        ExitMethod::TakerIoc
    }

    /// Calculate expected TP price
    pub fn calc_tp_price(&self, entry_price: Decimal, direction: Direction) -> Decimal {
        let tp_pct = self.config.tp_ratio / Decimal::from(10000); // Convert bps to decimal
        match direction {
            Direction::Long => entry_price * (Decimal::ONE + tp_pct),
            Direction::Short => entry_price * (Decimal::ONE - tp_pct),
        }
    }
}
