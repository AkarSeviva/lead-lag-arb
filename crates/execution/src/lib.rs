//! Execution Engine
//!
//! Order lifecycle management with state machine.

pub mod state_machine;
pub mod exit;
pub mod monitor;

pub use state_machine::{ExecutionEngine, OrderState, ExitMethod};
pub use exit::ExitStrategy;
pub use monitor::PositionMonitor;
