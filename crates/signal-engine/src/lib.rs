//! Signal Engine
//!
//! Lead-lag arbitrage signal calculation with filter chain.

pub mod context;
pub mod filters;
pub mod signal;
pub mod state;

pub use context::{FilterResult, SignalContext};
pub use filters::{EntryFilter, FilterChain};
pub use signal::{ArbitrageSignal, SignalDirection, SpreadSnapshot};
pub use state::SignalState;
