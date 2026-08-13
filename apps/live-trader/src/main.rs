//! Lead-Lag Arbitrage Live Trader
//!
//! Main entry point for the trading system.
//!
//! ## 订单执行架构 (基于策略指导 Section 4)
//!
//! ### 开仓 (Entry) - 必须 Taker
//! - 调用: `OrderExecutor::open_position`
//! - Lbank 字段: `OrderPriceType=4` (Market), `OffsetFlag=0` (Open)
//! - 原因: 信号窗口仅 100-300ms，挂限价单可能错过机会
//!
//! ### 平仓 (Exit) - Method 1: GTC + Timeout Fallback
//! 1. TP 信号触发 → 在最新价挂 GTC Limit
//!    - Lbank: `OrderPriceType=0` (Limit), `OffsetFlag=5` (CloseAll)
//!    - 优点: 利润最大化
//! 2. GTC 超时 (5s) → 撤单 → 市价单
//!    - Lbank: `OrderPriceType=4` (Market), `OffsetFlag=5` (CloseAll)
//!    - 保底成交
//!
//! ### 止损 (Stop-Loss) - 必须 Market
//! - 调用: `OrderExecutor::stop_loss`
//! - Lbank: `OrderPriceType=4` (Market), `OffsetFlag=5` (CloseAll)
//! - 不挂限价单，不惜代价出场
//!
//! ## 漏斗架构
//!
//! ```text
//! Level 1: 交集池 (Binance ∩ Lbank) -> ~300 symbols
//! Level 2: 质量池 (depth>1000, vol<2%, spread>0.10%) -> ~50-100 symbols
//! Level 3: 目标池 (MAX spread) -> 1 symbol
//! ```

use anyhow::Result;
use clap::Parser;
use config::strategy::StrategyConfig;
use config::Direction;
use exchange_adapter_lbank::{auth::LbankSigner, client::LbankClient};
use execution::{
    CloseOrderParams, OpenOrderParams, OrderExecutor, OrderResult,
};
use rust_decimal::Decimal;
use signal_engine::{
    context::SignalContext,
    filters::{CooldownFilter, DepthConfirmFilter, FilterChain, SpreadThresholdFilter},
};
use std::collections::HashSet;
use std::sync::Arc;
use symbol_selector::{
    funnel::{SpreadDirection, SymbolQuality},
    FunnelConfig, FunnelRunner,
};
use symbol_selector::runner::QualityCalculator;
use tokio::sync::RwLock;
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Config file path
    #[arg(short, long, default_value = "config/strategy.toml")]
    config: String,

    /// Dry run mode
    #[arg(short, long)]
    dry_run: bool,

    /// Log level
    #[arg(short, long, default_value = "info")]
    log_level: String,
}

/// Shared state for active trading symbol
#[derive(Clone)]
struct TradingState {
    current_symbol: Arc<RwLock<Option<String>>>,
}

impl TradingState {
    fn new() -> Self {
        Self {
            current_symbol: Arc::new(RwLock::new(None)),
        }
    }

    async fn set(&self, symbol: Option<String>) {
        let mut current = self.current_symbol.write().await;
        if current.as_ref() != symbol.as_ref() {
            if let Some(ref s) = symbol {
                info!("Switching trading target: {:?} -> {}", current, s);
            } else {
                info!("Clearing trading target (was {:?})", current);
            }
            *current = symbol;
        }
    }

    async fn get(&self) -> Option<String> {
        self.current_symbol.read().await.clone()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    let log_level = match args.log_level.as_str() {
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(log_level)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .finish();

    tracing::subscriber::set_global_default(subscriber)?;

    info!("=== Lead-Lag Arbitrage Trader ===");
    info!("Dry run mode: {}", args.dry_run);

    // Load configuration
    let config = load_config(&args.config)?;
    info!(
        "Configuration loaded: entry_threshold={}%",
        config.entry_threshold
    );

    // Initialize trading state
    let trading_state = TradingState::new();

    // Initialize Lbank client
    info!("Initializing Lbank client...");
    
    // ============================================================
    // 认证凭证 (从浏览器 LocalStorage/Headers 获取)
    // ============================================================
    // ex-uid: LBA8G85737
    // ex-token: 0688c69dd06a41f38c482e0f46719ed8
    // ex-device-id: hZlegXdOAxOsNqUVl7oL8p8lwE3dIeqQ
    // apiSecret: 23bec4f8489109e112812c2c2c7c31b3 (Base64 解码)
    let signer = LbankSigner::new(
        "23bec4f8489109e112812c2c2c7c31b3".to_string(),
        "LBA8G85737".to_string(),
        "0688c69dd06a41f38c482e0f46719ed8".to_string(),
        Some("hZlegXdOAxOsNqUVl7oL8p8lwE3dIeqQ".to_string()),  // 使用浏览器同款 device_id
    );
    let client = Arc::new(LbankClient::new(signer)?);

    // Initialize OrderExecutor with anti-detection jitter (50ms)
    let order_executor = OrderExecutor::new(client.clone()).with_jitter(50);
    info!(
        "Order executor initialized (entry=Market, exit=Method1: GTC->{}s->Market, stop-loss=Market)",
        config.gtc_timeout_secs
    );

    // ===== FUNNEL SETUP =====
    let funnel_config = FunnelConfig::default();
    let funnel_runner = Arc::new(FunnelRunner::new(funnel_config.clone()));
    let calculator = Arc::new(QualityCalculator::new(funnel_config.clone()));

    // Step 1: Compute initial intersection (Level 1)
    info!("Step 1: Fetching Binance and Lbank symbol lists...");
    let (binance_symbols, lbank_symbols) = fetch_all_symbols(&client).await?;
    info!(
        "Binance: {} symbols, Lbank: {} symbols",
        binance_symbols.len(),
        lbank_symbols.len()
    );

    let binance_set: HashSet<String> = binance_symbols.into_iter().collect();
    let lbank_set: HashSet<String> = lbank_symbols.into_iter().collect();
    let intersection: HashSet<String> = binance_set.intersection(&lbank_set).cloned().collect();
    info!("Intersection (Level 1 pool): {} symbols", intersection.len());

    funnel_runner
        .update_intersection(
            binance_set.into_iter().collect(),
            lbank_set.into_iter().collect(),
        )
        .await;

    // Step 2: Quality filtering (Level 2)
    // For each symbol in intersection, fetch depth and compute volatility
    info!("Step 2: Computing quality scores for Level 2 pool...");
    let intersection_vec: Vec<String> = intersection.into_iter().collect();
    let qualities = collect_qualities(
        &client,
        &intersection_vec,
        &calculator,
        funnel_config.rate_limit_per_sec,
    )
    .await;

    info!(
        "Collected quality data for {} symbols (Level 1 -> Level 2)",
        qualities.len()
    );

    funnel_runner.update_quality(qualities.clone()).await;

    // Step 3: Select target (Level 3)
    info!("Step 3: Selecting target symbol (Level 3 pool)...");
    let target = funnel_runner.select_target().await;

    match &target {
        Some(symbol) => {
            info!("Selected target: {}", symbol);

            // Set leverage for the selected symbol
            let requested_leverage = config.capital.leverage;
            match client.init_leverage(symbol, requested_leverage as i32).await {
                Ok(actual) => {
                    info!("Leverage initialized for {}: {}x", symbol, actual);
                }
                Err(e) => {
                    error!("Failed to initialize leverage for {}: {}", symbol, e);
                }
            }

            trading_state.set(Some(symbol.clone())).await;
        }
        None => {
            error!("No suitable symbol found in intersection pool");
            return Err(anyhow::anyhow!("No target symbol available"));
        }
    }

    // Print funnel stats
    let stats = funnel_runner.get_stats().await;
    info!("=== Funnel Stats ===");
    info!("  Level 1 (intersection): {} symbols", stats.intersection_count);
    info!("  Level 2 (quality): {} symbols", stats.quality_count);
    info!("  Level 2 (qualified): {} symbols", stats.qualified_count);
    info!("  Level 3 (target): {:?}", stats.current_target);
    info!("  Alternatives: {:?}", stats.alternatives);

    // Initialize signal engine, filter chain, etc.
    let current_symbol = trading_state.get().await.unwrap_or_default();
    info!("Initializing signal engine for {}", current_symbol);
    let signal_ctx = SignalContext::new(
        current_symbol.clone(),
        config.entry_threshold * Decimal::from(100),
    );

    let mut filter_chain = FilterChain::new();
    filter_chain.add(SpreadThresholdFilter::new(config.entry_threshold));
    filter_chain.add(DepthConfirmFilter::new(config.filters.min_leader_depth_usd));
    filter_chain.add(CooldownFilter::new(config.filters.cooldown_after_sl_secs));
    info!("Filter chain initialized with {} filters", filter_chain.len());

    // Main trading loop
    info!("Starting main trading loop on {}", current_symbol);

    let (shutdown_tx, _shutdown_rx) = tokio::sync::oneshot::channel();
    let main_handle = tokio::spawn(async move {
        let mut last_target_refresh = 0i64;
        let target_refresh_secs = 1; // Refresh target every 1 second

        loop {
            let now = chrono::Utc::now().timestamp();

            // Periodic target refresh (Level 3)
            if now - last_target_refresh >= target_refresh_secs {
                last_target_refresh = now;
                let new_target = funnel_runner.select_target().await;
                if new_target.as_ref() != trading_state.get().await.as_ref() {
                    if let Some(ref new_sym) = new_target {
                        // Set leverage for new symbol
                        let requested_leverage = config.capital.leverage;
                        if let Err(e) = client.init_leverage(new_sym, requested_leverage as i32).await {
                            error!("Failed to set leverage for {}: {}", new_sym, e);
                        }
                    }
                    trading_state.set(new_target).await;
                }
            }

            // Process signals (placeholder)
            if let Some(symbol) = trading_state.get().await {
                info!("Monitoring {} for spread opportunities", symbol);

                // ==========================================================
                // 订单执行流程示例 (Method 1 Entry/Exit)
                // ==========================================================
                // 当信号触发时 (伪代码示意):
                //   1. open_position() - 市价开仓 (Taker, OrderPriceType=4)
                //   2. wait_for_spread_convergence()
                //   3. close_position_method1() - GTC + 5s 超时回退
                //
                // 开仓代码示例:
                // ```rust
                // let open_params = OpenOrderParams {
                //     symbol: symbol.clone(),
                //     direction: Direction::Long,  // 信号方向
                //     volume: dec!(0.01),           // 计算后的仓位
                //     expected_price: current_price,
                // };
                // match order_executor.open_position(open_params).await {
                //     Ok(OrderResult::OpenSuccess { order_id, .. }) => {
                //         info!("Opened: {}", order_id);
                //         // ... 等待 TP/SL 信号 ...
                //         let close_params = CloseOrderParams {
                //             symbol: symbol.clone(),
                //             direction: Direction::Long,
                //             volume: dec!(0.01),
                //             trade_unit_id: trade_unit_id,
                //             gtc_price: target_price,
                //             gtc_timeout_secs: 5,
                //             entry_price: entry_price,
                //         };
                //         order_executor.close_position_method1(close_params).await?;
                //     }
                //     _ => warn!("Open failed"),
                // }
                // ```
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    });

    // Wait for shutdown
    tokio::signal::ctrl_c().await?;
    info!("Shutdown signal received");
    let _ = shutdown_tx.send(());

    let _ = main_handle.await;
    info!("Shutting down...");
    Ok(())
}

/// Fetch all symbol lists from Binance and Lbank
async fn fetch_all_symbols(
    client: &LbankClient,
) -> Result<(Vec<String>, Vec<String>)> {
    // For now, return a hardcoded list since we don't have a Binance client yet
    // In production, this would call Binance's /api/v3/exchangeInfo
    let binance_symbols = vec![
        "BTCUSDT".to_string(),
        "ETHUSDT".to_string(),
        "SOLUSDT".to_string(),
        "DOGEUSDT".to_string(),
        "XRPUSDT".to_string(),
        "ADAUSDT".to_string(),
        "AVAXUSDT".to_string(),
        "MATICUSDT".to_string(),
        "LINKUSDT".to_string(),
        "DOTUSDT".to_string(),
        "LTCUSDT".to_string(),
        "BCHUSDT".to_string(),
    ];

    // Lbank symbols (placeholder - would call /cfd/agg/v1/instrument)
    let lbank_symbols = vec![
        "BTCUSDT".to_string(),
        "ETHUSDT".to_string(),
        "SOLUSDT".to_string(),
        "DOGEUSDT".to_string(),
        "XRPUSDT".to_string(),
        "ADAUSDT".to_string(),
        "AVAXUSDT".to_string(),
        "LINKUSDT".to_string(),
        "DOTUSDT".to_string(),
    ];

    Ok((binance_symbols, lbank_symbols))
}

/// Collect quality data for symbols in the intersection pool
async fn collect_qualities(
    _client: &LbankClient,
    symbols: &[String],
    calculator: &QualityCalculator,
    rate_limit_per_sec: u32,
) -> Vec<SymbolQuality> {
    let mut qualities = Vec::new();
    let interval_ms = 1000 / rate_limit_per_sec.max(1) as u64;

    for symbol in symbols {
        // Rate limit: pause between requests
        tokio::time::sleep(tokio::time::Duration::from_millis(interval_ms)).await;

        // For now, simulate quality data
        // In production, we'd fetch:
        // 1. Binance 5-level depth via WebSocket or REST
        // 2. 1-min candlestick to compute volatility
        // 3. Both exchanges' order books to compute spread
        let quality = calculator.calculate(
            symbol,
            Decimal::new(5000, 0),  // depth 5000 USDT
            Decimal::new(1, 3),      // vol 0.1%
            Decimal::new(15, 2),     // spread 0.15%
            SpreadDirection::Long,
        );

        if calculator.passes_filter(&quality) {
            qualities.push(quality);
        }
    }

    qualities
}

fn load_config(path: &str) -> Result<StrategyConfig> {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let config: StrategyConfig = toml::from_str(&content)?;
            Ok(config)
        }
        Err(_) => {
            info!("Config file not found, using defaults");
            Ok(StrategyConfig::default())
        }
    }
}
