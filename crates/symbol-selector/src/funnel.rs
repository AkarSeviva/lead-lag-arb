//! Symbol Funnel - Multi-stage filtering for arbitrage opportunities
//!
//! ## Architecture
//!
//! ```text
//! Level 1: 交集池 (Intersection Pool)
//!   Binance (~500) ∩ Lbank (~500) = ~300 common symbols
//!
//! Level 2: 质量池 (Quality Pool) - 过滤条件:
//!   - 5-level depth on Binance > 1000 USDT
//!   - 1min volatility < 2% (single candle amplitude)
//!   = ~50-100 qualified symbols
//!
//! Level 3: 目标池 (Target Pool) - 排序选择:
//!   - Select symbol with MAX spread
//!   - Avoid hitting rate limits
//!   = 1 symbol for trading
//! ```
//!
//! ## Rate Limit Strategy
//!
//! - Batch refresh Level 1: every 5 minutes (low frequency)
//! - Batch refresh Level 2: every 30 seconds (medium frequency)
//! - Real-time Level 3: continuous monitoring

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, HashMap};
use std::cmp::Ordering;
use tokio::sync::RwLock;

/// 漏斗配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunnelConfig {
    /// Level 1: 交集刷新间隔 (秒)
    pub intersection_refresh_secs: u64,
    /// Level 2: 质量池刷新间隔 (秒)
    pub quality_refresh_secs: u64,
    /// Level 3: 目标选择刷新间隔 (秒)
    pub target_refresh_secs: u64,
    /// 最小价差阈值 (相对于中间价的 bps)
    pub min_spread_bps: Decimal,
    /// Binance 5-level depth 最小深度 (USDT)
    pub min_depth_usdt: Decimal,
    /// 1分钟K线最大波动率 (0.02 = 2%)
    pub max_volatility: Decimal,
    /// 最大同时监控的币种数量
    pub max_monitored: usize,
    /// Rate limit: 每秒最大请求数
    pub rate_limit_per_sec: u32,
}

impl Default for FunnelConfig {
    fn default() -> Self {
        Self {
            intersection_refresh_secs: 300,  // 5分钟刷新交集
            quality_refresh_secs: 30,        // 30秒刷新质量池
            target_refresh_secs: 1,          // 1秒更新目标
            min_spread_bps: Decimal::new(10, 2),  // 0.10% 最小价差
            min_depth_usdt: Decimal::new(1000, 0), // 1000 USDT 深度
            max_volatility: Decimal::new(2, 2),     // 2% 波动率
            max_monitored: 3,                       // 最多同时监控3个
            rate_limit_per_sec: 50,                 // 每秒50个请求
        }
    }
}

/// 单个币种的质量数据
#[derive(Debug, Clone)]
pub struct SymbolQuality {
    pub symbol: String,
    /// Binance 5-level depth (USDT)
    pub binance_depth_5l: Decimal,
    /// 1min 波动率 (0-1, 如 0.015 = 1.5%)
    pub volatility_1m: Decimal,
    /// 当前价差 (bps)
    pub spread_bps: Decimal,
    /// 价差方向: Long/Short/None
    pub spread_direction: SpreadDirection,
    /// 最后更新时间戳
    pub last_updated: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpreadDirection {
    Long,  // leader.bid > follower.ask
    Short, // follower.bid > leader.ask
    None,
}

/// Level 1 Pool: 交集池
pub struct IntersectionPool {
    symbols: RwLock<HashSet<String>>,
    last_updated: RwLock<i64>,
}

impl IntersectionPool {
    pub fn new() -> Self {
        Self {
            symbols: RwLock::new(HashSet::new()),
            last_updated: RwLock::new(0),
        }
    }

    /// 计算交集
    pub async fn update(&self, binance_symbols: HashSet<String>, lbank_symbols: HashSet<String>) {
        let intersection: HashSet<String> = binance_symbols
            .intersection(&lbank_symbols)
            .cloned()
            .collect();
        
        let mut symbols = self.symbols.write().await;
        *symbols = intersection;
        *self.last_updated.write().await = chrono::Utc::now().timestamp();
    }

    pub async fn get_all(&self) -> Vec<String> {
        self.symbols.read().await.iter().cloned().collect()
    }

    pub async fn len(&self) -> usize {
        self.symbols.read().await.len()
    }

    pub async fn needs_refresh(&self, config: &FunnelConfig, now: i64) -> bool {
        let last = *self.last_updated.read().await;
        (now - last) as u64 >= config.intersection_refresh_secs
    }
}

impl Default for IntersectionPool {
    fn default() -> Self {
        Self::new()
    }
}

/// Level 2 Pool: 质量池 (深度 + 波动率筛选)
pub struct QualityPool {
    /// 符号 -> 质量数据
    pub(super) data: RwLock<HashMap<String, SymbolQuality>>,
    last_updated: RwLock<i64>,
}

impl QualityPool {
    pub fn new() -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
            last_updated: RwLock::new(0),
        }
    }

    /// 更新质量数据
    pub async fn update(&self, qualities: Vec<SymbolQuality>) {
        let mut data = self.data.write().await;
        data.clear();
        for q in qualities {
            data.insert(q.symbol.clone(), q);
        }
        *self.last_updated.write().await = chrono::Utc::now().timestamp();
    }

    /// 获取符合质量要求的币种列表
    pub async fn get_qualified(&self, config: &FunnelConfig) -> Vec<SymbolQuality> {
        let data = self.data.read().await;
        data.values()
            .filter(|q| {
                q.binance_depth_5l >= config.min_depth_usdt
                    && q.volatility_1m <= config.max_volatility
                    && q.spread_bps >= config.min_spread_bps
            })
            .cloned()
            .collect()
    }

    pub async fn get(&self, symbol: &str) -> Option<SymbolQuality> {
        self.data.read().await.get(symbol).cloned()
    }

    pub async fn needs_refresh(&self, config: &FunnelConfig, now: i64) -> bool {
        let last = *self.last_updated.read().await;
        (now - last) as u64 >= config.quality_refresh_secs
    }

    pub async fn len(&self) -> usize {
        self.data.read().await.len()
    }
}

impl Default for QualityPool {
    fn default() -> Self {
        Self::new()
    }
}

/// Level 3 Pool: 目标池 (选价差最大的)
pub struct TargetPool {
    /// 当前选中的目标币种
    current: RwLock<Option<String>>,
    /// 备选目标列表
    alternatives: RwLock<Vec<String>>,
    last_updated: RwLock<i64>,
}

impl TargetPool {
    pub fn new() -> Self {
        Self {
            current: RwLock::new(None),
            alternatives: RwLock::new(Vec::new()),
            last_updated: RwLock::new(0),
        }
    }

    /// 选择价差最大的币种
    pub async fn select(&self, qualified: Vec<SymbolQuality>) -> Option<String> {
        if qualified.is_empty() {
            *self.current.write().await = None;
            *self.alternatives.write().await = Vec::new();
            return None;
        }

        // 按价差降序排序
        let mut sorted = qualified;
        sorted.sort_by(|a, b| b.spread_bps.partial_cmp(&a.spread_bps).unwrap_or(Ordering::Equal));

        let best = sorted.first().map(|q| q.symbol.clone());
        let alternatives: Vec<String> = sorted.iter().skip(1).take(5).map(|q| q.symbol.clone()).collect();

        *self.current.write().await = best.clone();
        *self.alternatives.write().await = alternatives;
        *self.last_updated.write().await = chrono::Utc::now().timestamp();

        best
    }

    pub async fn get_current(&self) -> Option<String> {
        self.current.read().await.clone()
    }

    pub async fn get_alternatives(&self) -> Vec<String> {
        self.alternatives.read().await.clone()
    }

    pub async fn needs_refresh(&self, config: &FunnelConfig, now: i64) -> bool {
        let last = *self.last_updated.read().await;
        (now - last) as u64 >= config.target_refresh_secs
    }
}

impl Default for TargetPool {
    fn default() -> Self {
        Self::new()
    }
}

/// Rate Limiter - 平滑的速率限制
pub struct RateLimiter {
    /// 时间窗口内的请求计数
    window_counts: RwLock<Vec<(i64, u32)>>,
    /// 窗口大小 (秒)
    window_secs: u64,
    /// 最大请求数
    max_requests: u32,
}

impl RateLimiter {
    pub fn new(max_per_sec: u32) -> Self {
        Self {
            window_counts: RwLock::new(Vec::new()),
            window_secs: 1,
            max_requests: max_per_sec,
        }
    }

    /// 检查是否可以发送请求
    pub async fn can_request(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        let mut counts = self.window_counts.write().await;
        
        // 清理过期的记录
        let cutoff = now - self.window_secs as i64;
        counts.retain(|(ts, _)| *ts > cutoff);
        
        // 计算当前窗口内的总请求数
        let total: u32 = counts.iter().map(|(_, c)| c).sum();
        total < self.max_requests
    }

    /// 记录一个请求
    pub async fn record(&self) {
        let now = chrono::Utc::now().timestamp();
        let mut counts = self.window_counts.write().await;
        
        // 清理过期的记录
        let cutoff = now - self.window_secs as i64;
        counts.retain(|(ts, _)| *ts > cutoff);
        
        // 添加当前请求
        if let Some(last) = counts.last_mut() {
            if last.0 == now {
                last.1 += 1;
            } else {
                counts.push((now, 1));
            }
        } else {
            counts.push((now, 1));
        }
    }

    /// 批量请求许可
    pub async fn acquire_batch(&self, count: u32) -> u32 {
        let now = chrono::Utc::now().timestamp();
        let mut counts = self.window_counts.write().await;
        
        // 清理过期的记录
        let cutoff = now - self.window_secs as i64;
        counts.retain(|(ts, _)| *ts > cutoff);
        
        // 计算当前窗口内的总请求数
        let current: u32 = counts.iter().map(|(_, c)| c).sum();
        let available = self.max_requests.saturating_sub(current);
        let acquired = available.min(count);
        
        if acquired > 0 {
            if let Some(last) = counts.last_mut() {
                if last.0 == now {
                    last.1 += acquired;
                } else {
                    counts.push((now, acquired));
                }
            } else {
                counts.push((now, acquired));
            }
        }
        
        acquired
    }
}

/// 漏斗状态
#[derive(Debug, Clone)]
pub struct FunnelStats {
    pub intersection_count: usize,
    pub quality_count: usize,
    pub qualified_count: usize,
    pub current_target: Option<String>,
    pub alternatives: Vec<String>,
}

impl Default for FunnelStats {
    fn default() -> Self {
        Self {
            intersection_count: 0,
            quality_count: 0,
            qualified_count: 0,
            current_target: None,
            alternatives: Vec::new(),
        }
    }
}

/// Symbol Funnel - 核心漏斗结构
pub struct SymbolFunnel {
    config: FunnelConfig,
    intersection_pool: IntersectionPool,
    quality_pool: QualityPool,
    target_pool: TargetPool,
    rate_limiter: RateLimiter,
}

impl SymbolFunnel {
    pub fn new(config: FunnelConfig) -> Self {
        let rl = config.rate_limit_per_sec;
        Self {
            config,
            intersection_pool: IntersectionPool::new(),
            quality_pool: QualityPool::new(),
            target_pool: TargetPool::new(),
            rate_limiter: RateLimiter::new(rl),
        }
    }

    /// 更新交集池 (Level 1)
    pub async fn update_intersection(&self, binance: HashSet<String>, lbank: HashSet<String>) {
        self.intersection_pool.update(binance, lbank).await;
    }

    /// 获取交集池中的币种
    pub async fn get_intersection(&self) -> Vec<String> {
        self.intersection_pool.get_all().await
    }

    /// 获取当前目标币种
    pub async fn get_target(&self) -> Option<String> {
        self.target_pool.get_current().await
    }

    /// 获取备选币种
    pub async fn get_alternatives(&self) -> Vec<String> {
        self.target_pool.get_alternatives().await
    }

    /// 批量获取质量数据 (用于批量更新)
    pub async fn get_symbols_for_quality_check(&self) -> Vec<String> {
        self.intersection_pool.get_all().await
    }

    /// 更新质量池 (Level 2)
    pub async fn update_quality(&self, qualities: Vec<SymbolQuality>) {
        self.quality_pool.update(qualities).await;
    }

    /// 获取单个币种的质量数据
    pub async fn get_quality(&self, symbol: &str) -> Option<SymbolQuality> {
        self.quality_pool.get(symbol).await
    }

    /// 选择目标 (Level 3)
    pub async fn select_target(&self) -> Option<String> {
        let qualified = self.quality_pool.get_qualified(&self.config).await;
        self.target_pool.select(qualified).await
    }

    /// 获取漏斗状态
    pub async fn get_stats(&self) -> FunnelStats {
        let intersection_count = self.intersection_pool.len().await;
        let quality_count = self.quality_pool.len().await;
        let qualified = self.quality_pool.get_qualified(&self.config).await;
        let target = self.target_pool.get_current().await;
        let alternatives = self.target_pool.get_alternatives().await;

        FunnelStats {
            intersection_count,
            quality_count,
            qualified_count: qualified.len(),
            current_target: target,
            alternatives,
        }
    }

    /// 检查是否可以发送请求
    pub async fn can_request(&self) -> bool {
        self.rate_limiter.can_request().await
    }

    /// 记录请求
    pub async fn record_request(&self) {
        self.rate_limiter.record().await
    }

    /// 批量请求许可
    pub async fn acquire_batch(&self, count: u32) -> u32 {
        self.rate_limiter.acquire_batch(count).await
    }

    /// 获取配置
    pub fn config(&self) -> &FunnelConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_intersection_pool() {
        let pool = IntersectionPool::new();
        
        let binance: HashSet<String> = ["BTC", "ETH", "DOGE", "XRP"].iter().map(|s| s.to_string()).collect();
        let lbank: HashSet<String> = ["BTC", "ETH", "SOL", "ADA"].iter().map(|s| s.to_string()).collect();
        
        pool.update(binance, lbank).await;
        
        let symbols = pool.get_all().await;
        assert_eq!(symbols.len(), 2);
        assert!(symbols.contains(&"BTC".to_string()));
        assert!(symbols.contains(&"ETH".to_string()));
    }

    #[tokio::test]
    async fn test_quality_pool_filter() {
        let pool = QualityPool::new();
        let config = FunnelConfig::default();
        
        let qualities = vec![
            SymbolQuality {
                symbol: "BTC".to_string(),
                binance_depth_5l: Decimal::new(5000, 0),  // > 1000
                volatility_1m: Decimal::new(1, 3),        // 0.1% < 2%
                spread_bps: Decimal::new(15, 2),           // 0.15% > 0.10%
                spread_direction: SpreadDirection::Long,
                last_updated: 0,
            },
            SymbolQuality {
                symbol: "ETH".to_string(),
                binance_depth_5l: Decimal::new(500, 0),    // < 1000
                volatility_1m: Decimal::new(1, 3),         // 0.1%
                spread_bps: Decimal::new(15, 2),           // 0.15%
                spread_direction: SpreadDirection::Long,
                last_updated: 0,
            },
            SymbolQuality {
                symbol: "DOGE".to_string(),
                binance_depth_5l: Decimal::new(5000, 0),   // > 1000
                volatility_1m: Decimal::new(5, 2),         // 5% > 2%
                spread_bps: Decimal::new(15, 2),           // 0.15%
                spread_direction: SpreadDirection::Long,
                last_updated: 0,
            },
        ];
        
        pool.update(qualities).await;
        
        let qualified = pool.get_qualified(&config).await;
        assert_eq!(qualified.len(), 1);
        assert_eq!(qualified[0].symbol, "BTC");
    }

    #[tokio::test]
    async fn test_target_selection() {
        let pool = TargetPool::new();
        
        let qualities = vec![
            SymbolQuality {
                symbol: "BTC".to_string(),
                binance_depth_5l: Decimal::new(5000, 0),
                volatility_1m: Decimal::new(1, 3),
                spread_bps: Decimal::new(10, 2),  // 0.10%
                spread_direction: SpreadDirection::Long,
                last_updated: 0,
            },
            SymbolQuality {
                symbol: "ETH".to_string(),
                binance_depth_5l: Decimal::new(5000, 0),
                volatility_1m: Decimal::new(1, 3),
                spread_bps: Decimal::new(20, 2),  // 0.20% - MAX
                spread_direction: SpreadDirection::Long,
                last_updated: 0,
            },
            SymbolQuality {
                symbol: "DOGE".to_string(),
                binance_depth_5l: Decimal::new(5000, 0),
                volatility_1m: Decimal::new(1, 3),
                spread_bps: Decimal::new(15, 2),  // 0.15%
                spread_direction: SpreadDirection::Long,
                last_updated: 0,
            },
        ];
        
        let target = pool.select(qualities).await;
        assert_eq!(target, Some("ETH".to_string()));

        let alternatives = pool.get_alternatives().await;
        assert_eq!(alternatives.len(), 2); // 3 qualified - 1 current = 2
    }

    #[tokio::test]
    async fn test_rate_limiter() {
        let limiter = RateLimiter::new(10);  // 10 req/sec
        
        // First 10 should succeed
        for _ in 0..10 {
            assert!(limiter.can_request().await);
            limiter.record().await;
        }
        
        // 11th should fail
        assert!(!limiter.can_request().await);
    }
}
