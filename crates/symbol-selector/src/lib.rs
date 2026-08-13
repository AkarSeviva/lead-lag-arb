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
//! ## Symbol Funnel (漏斗模型)
//!
//! ```text
//! Level 1: 交集池 (Intersection Pool)
//!   Binance (~500) ∩ Lbank (~500) = ~300 common symbols
//!   Refresh: every 5 minutes
//!
//! Level 2: 质量池 (Quality Pool)
//!   - Binance 5-level depth > 1000 USDT
//!   - 1min volatility < 2%
//!   Refresh: every 30 seconds
//!
//! Level 3: 目标池 (Target Pool)
//!   - Select symbol with MAX spread
//!   - Avoid rate limits
//!   Refresh: every 1 second
//! ```
//!
//! ## Features
//!
//! - **Symbol Intersection**: Find symbols available on both exchanges
//! - **Watchlist**: Filter by user-defined watchlist
//! - **Rate Limit Aware**: Split budget between active trading and rotating scan
//! - **Async Refresh**: Background refresh of rotating pool
//! - **Symbol Funnel**: Multi-stage filtering for arbitrage opportunities

pub mod config;
pub mod funnel;
pub mod pool;
pub mod runner;
pub mod selector;
pub mod types;

pub use config::{SelectorConfig, Strategy};
pub use funnel::{FunnelConfig, FunnelStats, RateLimiter, SymbolFunnel};
pub use pool::{ActivePool, RotatingPool};
pub use runner::FunnelRunner;
pub use selector::SymbolSelector;
pub use types::*;
