//! # Symbol Selector Module
//!
//! Multi-exchange symbol filtering and rate limit aware selection.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  Rate Limit Budget: e.g., 100 req/s                     │
//! ├─────────────────────────────────────────────────────────┤
//! │                                                         │
//! │  ┌─────────────────────┐     ┌─────────────────────┐  │
//! │  │   Active Pool (N)  │     │  Rotating Pool (M)  │  │
//! │  │   正在交易/监控     │     │  轮动扫描列表       │  │
//! │  │                     │     │                     │  │
//! │  │  BTCUSDT ★  活跃   │     │  DOGEUSDT          │  │
//! │  │  ETHUSDT ★  活跃   │     │  XRPUSDT           │  │
//! │  │  SOLUSDT ★  活跃   │     │  ADAUSDT           │  │
//! │  │  ...                │     │  ...               │  │
//! │  │                     │     │  (每批轮换)        │  │
//! │  └─────────────────────┘     └─────────────────────┘  │
//! │         ↑                            ↑                  │
//! │    高优先级保证                  低优先级轮换            │
//! │    实时性要求                  扫描全覆盖             │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Features
//!
//! - **Symbol Intersection**: Find symbols available on both exchanges
//! - **Watchlist**: Filter by user-defined watchlist
//! - **Rate Limit Aware**: Split budget between active trading and rotating scan
//! - **Async Refresh**: Background refresh of rotating pool

pub mod config;
pub mod pool;
pub mod selector;
pub mod types;

pub use config::{SelectorConfig, Strategy};
pub use pool::{ActivePool, RotatingPool};
pub use selector::SymbolSelector;
pub use types::*;
