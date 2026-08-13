//! Execution Engine
//!
//! Order lifecycle management with state machine.
//!
//! ## Order Types (基于 Lbank 逆向文档)
//!
//! - **开仓**: Market (`OrderPriceType=4`, `OffsetFlag=0`)
//! - **平仓**: GTC Limit + Timeout Fallback to Market
//!   - GTC: `OrderPriceType=0`, `OffsetFlag=5`
//!   - Market: `OrderPriceType=4`, `OffsetFlag=5`
//! - **止损**: Market (强制出场)

pub mod state_machine;
pub mod exit;
pub mod monitor;
pub mod order_executor;
pub mod order_types;

pub use state_machine::{ExecutionEngine, OrderState, ExitMethod, TrackedPosition};
pub use exit::ExitStrategy;
pub use monitor::PositionMonitor;
pub use order_executor::{
    CloseOrderParams, OpenOrderParams, OrderEvent, OrderExecutor, OrderResult,
};
pub use order_types::{OffsetFlag, OrderKind, OrderPriceType, TriggerOrderType};