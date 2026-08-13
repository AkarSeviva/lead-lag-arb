//! Funnel Runner - Handles async refresh of the symbol funnel

use crate::funnel::{FunnelConfig, FunnelStats, SymbolFunnel, SymbolQuality, SpreadDirection};
use rust_decimal::Decimal;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::time::{interval, Duration};

/// Funnel refresh events
#[derive(Debug, Clone)]
pub enum FunnelEvent {
    /// Intersection pool updated
    IntersectionUpdated(Vec<String>),
    /// Quality pool updated
    QualityUpdated(Vec<SymbolQuality>),
    /// Target selected
    TargetSelected { symbol: String, spread_bps: Decimal },
    /// Target cleared (no opportunities)
    TargetCleared,
    /// Funnel stats snapshot
    Stats(FunnelStats),
}

/// Funnel Runner - Manages async refresh of funnel pools
pub struct FunnelRunner {
    funnel: Arc<SymbolFunnel>,
    event_tx: broadcast::Sender<FunnelEvent>,
}

impl FunnelRunner {
    pub fn new(config: FunnelConfig) -> Self {
        let funnel = Arc::new(SymbolFunnel::new(config));
        let (event_tx, _) = broadcast::channel(100);
        
        Self { funnel, event_tx }
    }

    pub fn funnel(&self) -> Arc<SymbolFunnel> {
        Arc::clone(&self.funnel)
    }

    pub fn event_receiver(&self) -> broadcast::Receiver<FunnelEvent> {
        self.event_tx.subscribe()
    }

    /// 更新交集池 (需要 Binance 和 Lbank 的币种列表)
    pub async fn update_intersection(
        &self,
        binance_symbols: Vec<String>,
        lbank_symbols: Vec<String>,
    ) {
        let binance_set: HashSet<String> = binance_symbols.into_iter().collect();
        let lbank_set: HashSet<String> = lbank_symbols.into_iter().collect();
        
        self.funnel.update_intersection(binance_set, lbank_set).await;
        
        let symbols = self.funnel.get_intersection().await;
        let _ = self.event_tx.send(FunnelEvent::IntersectionUpdated(symbols));
    }

    /// 批量更新质量数据 (用于 Level 2 刷新)
    /// 这个方法接收外部计算好的质量数据
    pub async fn update_quality(&self, qualities: Vec<SymbolQuality>) {
        let snapshot = qualities.clone();
        self.funnel.update_quality(qualities).await;
        let _ = self.event_tx.send(FunnelEvent::QualityUpdated(snapshot));
    }

    /// 选择目标币种 (Level 3)
    pub async fn select_target(&self) -> Option<String> {
        let target = self.funnel.select_target().await;

        if let Some(ref symbol) = target {
            if let Some(quality) = self.funnel.get_quality(symbol).await {
                let _ = self.event_tx.send(FunnelEvent::TargetSelected {
                    symbol: symbol.clone(),
                    spread_bps: quality.spread_bps,
                });
            }
        } else {
            let _ = self.event_tx.send(FunnelEvent::TargetCleared);
        }

        target
    }

    /// 获取当前状态
    pub async fn get_stats(&self) -> FunnelStats {
        self.funnel.get_stats().await
    }

    /// 启动交集刷新任务 (Level 1)
    pub async fn start_intersection_refresh(
        &self,
        binance_provider: impl Fn() -> Vec<String> + Send + 'static,
        lbank_provider: impl Fn() -> Vec<String> + Send + 'static,
        mut shutdown: tokio::sync::oneshot::Receiver<()>,
    ) {
        let interval_secs = self.funnel.config().intersection_refresh_secs;
        let mut ticker = interval(Duration::from_secs(interval_secs));
        
        tracing::info!("Starting intersection refresh task (interval: {}s)", interval_secs);
        
        loop {
            tokio::select! {
                _ = &mut shutdown => {
                    tracing::info!("Intersection refresh task shutting down");
                    break;
                }
                _ = ticker.tick() => {
                    let binance = (binance_provider)();
                    let lbank = (lbank_provider)();
                    self.update_intersection(binance, lbank).await;
                }
            }
        }
    }

    /// 启动质量池刷新任务 (Level 2)
    /// 质量数据需要外部计算后传入
    pub async fn start_quality_refresh(
        &self,
        quality_provider: impl Fn(Vec<String>) -> Vec<SymbolQuality> + Send + 'static,
        mut shutdown: tokio::sync::oneshot::Receiver<()>,
    ) {
        let interval_secs = self.funnel.config().quality_refresh_secs;
        let mut ticker = interval(Duration::from_secs(interval_secs));
        
        tracing::info!("Starting quality refresh task (interval: {}s)", interval_secs);
        
        loop {
            tokio::select! {
                _ = &mut shutdown => {
                    tracing::info!("Quality refresh task shutting down");
                    break;
                }
                _ = ticker.tick() => {
                    // 获取需要检查质量的币种
                    let symbols = self.funnel.get_symbols_for_quality_check().await;
                    
                    if symbols.is_empty() {
                        continue;
                    }
                    
                    // 检查 rate limit
                    let batch_size = self.funnel.config().rate_limit_per_sec as usize / 2;
                    let batch_size = batch_size.max(10).min(50);
                    
                    // 分批获取质量数据
                    for chunk in symbols.chunks(batch_size) {
                        if !self.funnel.can_request().await {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                        
                        let qualities = quality_provider(chunk.to_vec());
                        self.update_quality(qualities).await;
                        self.funnel.record_request().await;
                    }
                }
            }
        }
    }

    /// 启动目标选择任务 (Level 3)
    pub async fn start_target_selection(
        &self,
        mut shutdown: tokio::sync::oneshot::Receiver<()>,
    ) {
        let interval_secs = self.funnel.config().target_refresh_secs;
        let mut ticker = interval(Duration::from_secs(interval_secs));
        
        tracing::info!("Starting target selection task (interval: {}s)", interval_secs);
        
        loop {
            tokio::select! {
                _ = &mut shutdown => {
                    tracing::info!("Target selection task shutting down");
                    break;
                }
                _ = ticker.tick() => {
                    let target = self.select_target().await;
                    
                    let stats = self.get_stats().await;
                    let _ = self.event_tx.send(FunnelEvent::Stats(stats));
                    
                    if target.is_some() {
                        tracing::debug!("Current target: {:?}", target);
                    }
                }
            }
        }
    }

    /// 获取当前目标
    pub async fn get_current_target(&self) -> Option<String> {
        self.funnel.get_target().await
    }

    /// 获取备选目标
    pub async fn get_alternatives(&self) -> Vec<String> {
        self.funnel.get_alternatives().await
    }
}

/// Quality Calculator - 计算币种质量数据
pub struct QualityCalculator {
    config: FunnelConfig,
}

impl QualityCalculator {
    pub fn new(config: FunnelConfig) -> Self {
        Self { config }
    }

    /// 计算单个币种的质量数据
    /// 需要提供:
    /// - binance_depth_5l: Binance 5-level 累计深度 (USDT)
    /// - volatility_1m: 1分钟K线波动率 (0-1)
    /// - 当前价差 (bps)
    pub fn calculate(
        &self,
        symbol: &str,
        binance_depth_5l: Decimal,
        volatility_1m: Decimal,
        spread_bps: Decimal,
        spread_direction: SpreadDirection,
    ) -> SymbolQuality {
        SymbolQuality {
            symbol: symbol.to_string(),
            binance_depth_5l,
            volatility_1m,
            spread_bps,
            spread_direction,
            last_updated: chrono::Utc::now().timestamp(),
        }
    }

    /// 检查币种是否通过质量筛选
    pub fn passes_filter(&self, quality: &SymbolQuality) -> bool {
        quality.binance_depth_5l >= self.config.min_depth_usdt
            && quality.volatility_1m <= self.config.max_volatility
            && quality.spread_bps >= self.config.min_spread_bps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_funnel_runner() {
        let config = FunnelConfig::default();
        let runner = FunnelRunner::new(config);
        
        // Update intersection
        runner.update_intersection(
            vec!["BTC".to_string(), "ETH".to_string()],
            vec!["BTC".to_string(), "SOL".to_string()],
        ).await;
        
        let stats = runner.get_stats().await;
        assert_eq!(stats.intersection_count, 1);
    }

    #[test]
    fn test_quality_calculator() {
        let config = FunnelConfig::default();
        let calculator = QualityCalculator::new(config);
        
        let quality = calculator.calculate(
            "BTC",
            Decimal::new(5000, 0),  // depth
            Decimal::new(1, 3),       // volatility 0.1%
            Decimal::new(15, 2),      // spread 0.15%
            SpreadDirection::Long,
        );
        
        assert!(calculator.passes_filter(&quality));
    }
}
