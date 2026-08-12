//! Entry Filters
//!
//! Chain of responsibility pattern for signal filtering.

use crate::context::{FilterResult, SignalContext};
use rust_decimal::Decimal;

/// Entry filter trait
pub trait EntryFilter: Send + Sync {
    /// Filter name for logging
    fn name(&self) -> &'static str;

    /// Check if signal passes this filter
    fn check(&self, ctx: &SignalContext) -> FilterResult;
}

/// Filter chain
pub struct FilterChain {
    filters: Vec<Box<dyn EntryFilter>>,
}

impl FilterChain {
    pub fn new() -> Self {
        Self { filters: Vec::new() }
    }

    /// Add a filter to the chain
    pub fn add<F: EntryFilter + 'static>(&mut self, filter: F) {
        self.filters.push(Box::new(filter));
    }

    /// Run all filters
    pub fn run(&self, ctx: &SignalContext) -> (bool, Vec<String>) {
        let mut passed = Vec::new();
        let mut failed_reason: Option<&str> = None;

        for filter in &self.filters {
            let result = filter.check(ctx);
            if result.passed {
                passed.push(filter.name().to_string());
            } else {
                failed_reason = result.reason;
                break;
            }
        }

        if failed_reason.is_some() {
            (false, vec![])
        } else {
            (true, passed)
        }
    }

    /// Get filter count
    pub fn len(&self) -> usize {
        self.filters.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }
}

impl Default for FilterChain {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Concrete Filters
// ============================================================================

/// Spread duration filter (100-300ms persistence)
pub struct SpreadDurationFilter {
    pub min_duration_ms: u64,
    pub max_duration_ms: u64,
}

impl SpreadDurationFilter {
    pub fn new(min_ms: u64, max_ms: u64) -> Self {
        Self {
            min_duration_ms: min_ms,
            max_duration_ms: max_ms,
        }
    }
}

impl EntryFilter for SpreadDurationFilter {
    fn name(&self) -> &'static str {
        "SpreadDurationFilter"
    }

    fn check(&self, ctx: &SignalContext) -> FilterResult {
        if !ctx.state.spread_persisted(self.min_duration_ms) {
            return FilterResult::fail("Spread not persisted long enough");
        }

        if let Some(ref snapshot) = ctx.spread_snapshot {
            let bps = snapshot.spread_bps(ctx.calc_direction()).abs();
            if bps > Decimal::new(self.max_duration_ms as i64, 2) {
                return FilterResult::fail("Spread persisted too long (likely stale)");
            }
        }

        FilterResult::pass()
    }
}

/// Depth confirmation filter
pub struct DepthConfirmFilter {
    pub min_leader_depth_usd: Decimal,
}

impl DepthConfirmFilter {
    pub fn new(min_depth_usd: Decimal) -> Self {
        Self { min_leader_depth_usd: min_depth_usd }
    }
}

impl EntryFilter for DepthConfirmFilter {
    fn name(&self) -> &'static str {
        "DepthConfirmFilter"
    }

    fn check(&self, ctx: &SignalContext) -> FilterResult {
        if let Some(ref snapshot) = ctx.spread_snapshot {
            let direction = ctx.calc_direction();
            let required_depth = match direction {
                crate::signal::SignalDirection::Long => snapshot.leader_bid_depth,
                crate::signal::SignalDirection::Short => snapshot.leader_ask_depth,
                crate::signal::SignalDirection::None => return FilterResult::fail("No signal direction"),
            };

            if required_depth < self.min_leader_depth_usd {
                return FilterResult::fail("Insufficient leader depth");
            }
        }

        FilterResult::pass()
    }
}

/// Volatility filter
pub struct VolatilityFilter {
    pub max_volatility_bps: Decimal,
}

impl VolatilityFilter {
    pub fn new(max_volatility_pct: Decimal) -> Self {
        // Convert percentage to basis points
        let max_volatility_bps = max_volatility_pct * Decimal::from(100);
        Self { max_volatility_bps }
    }
}

impl EntryFilter for VolatilityFilter {
    fn name(&self) -> &'static str {
        "VolatilityFilter"
    }

    fn check(&self, ctx: &SignalContext) -> FilterResult {
        // TODO: Implement actual volatility calculation from order book
        // For now, pass
        FilterResult::pass()
    }
}

/// Cooldown filter (after stop-loss)
pub struct CooldownFilter {
    pub cooldown_secs: u64,
}

impl CooldownFilter {
    pub fn new(cooldown_secs: u64) -> Self {
        Self { cooldown_secs }
    }
}

impl EntryFilter for CooldownFilter {
    fn name(&self) -> &'static str {
        "CooldownFilter"
    }

    fn check(&self, ctx: &SignalContext) -> FilterResult {
        use std::time::Duration;

        if ctx.state.state == crate::context::State::Cooldown {
            let elapsed = ctx.state.state_since.elapsed();
            if elapsed < Duration::from_secs(self.cooldown_secs) {
                return FilterResult::fail("In cooldown period");
            }
        }

        FilterResult::pass()
    }
}

/// Spread threshold filter
pub struct SpreadThresholdFilter {
    pub min_threshold_bps: Decimal,
}

impl SpreadThresholdFilter {
    pub fn new(threshold_bps: Decimal) -> Self {
        Self { min_threshold_bps: threshold_bps }
    }
}

impl EntryFilter for SpreadThresholdFilter {
    fn name(&self) -> &'static str {
        "SpreadThresholdFilter"
    }

    fn check(&self, ctx: &SignalContext) -> FilterResult {
        if let Some(ref snapshot) = ctx.spread_snapshot {
            let direction = ctx.calc_direction();
            let bps = snapshot.spread_bps(direction).abs();

            if bps < self.min_threshold_bps {
                return FilterResult::fail("Spread below threshold");
            }
        } else {
            return FilterResult::fail("No spread snapshot");
        }

        FilterResult::pass()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_chain() {
        let mut chain = FilterChain::new();
        chain.add(SpreadThresholdFilter::new(Decimal::new(15, 1))); // 1.5 bps
        chain.add(DepthConfirmFilter::new(Decimal::new(100_000, 0))); // $100k

        assert_eq!(chain.len(), 2);
        assert!(!chain.is_empty());
    }

    #[test]
    fn test_cooldown_filter() {
        let filter = CooldownFilter::new(5);
        assert_eq!(filter.name(), "CooldownFilter");
    }
}
