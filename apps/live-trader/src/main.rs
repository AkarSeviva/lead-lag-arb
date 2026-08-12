//! Lead-Lag Arbitrage Live Trader
//!
//! Main entry point for the trading system.

use anyhow::Result;
use clap::Parser;
use config::strategy::StrategyConfig;
use exchange_adapter_lbank::{auth::LbankSigner, client::LbankClient};
use md_gateway::{connection::ExchangeGateway, connection::GatewayConfig};
use signal_engine::{filters::{FilterChain, SpreadThresholdFilter, DepthConfirmFilter, CooldownFilter}, context::SignalContext};
use tracing::{info, error, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Config file path
    #[arg(short, long, default_value = "config/strategy.toml")]
    config: String,

    /// Symbol to trade
    #[arg(short, long, default_value = "BTCUSDT")]
    symbol: String,

    /// Dry run mode
    #[arg(short, long)]
    dry_run: bool,

    /// Log level
    #[arg(short, long, default_value = "info")]
    log_level: String,
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

    info!("Starting Lead-Lag Arbitrage Trader");
    info!("Symbol: {}", args.symbol);
    info!("Dry run: {}", args.dry_run);

    // Load configuration
    let config = load_config(&args.config)?;
    info!("Configuration loaded: entry_threshold={}%", config.entry_threshold);

    // Initialize components
    info!("Initializing components...");

    // 1. Clock sync
    let clock_sync = clock_sync::ClockSynchronizer::new("LBank".to_string());
    info!("Clock synchronizer initialized");

    // 2. Risk gate
    let risk_gate = risk_gate::RiskGate::new(config.risk.clone());
    info!("Risk gate initialized");

    // 3. Signal engine context
    let mut signal_ctx = SignalContext::new(
        args.symbol.clone(),
        config.entry_threshold * Decimal::from(100), // Convert to bps
    );

    // 4. Filter chain
    let mut filter_chain = FilterChain::new();
    filter_chain.add(SpreadThresholdFilter::new(config.entry_threshold));
    filter_chain.add(DepthConfirmFilter::new(config.filters.min_leader_depth_usd));
    filter_chain.add(CooldownFilter::new(config.filters.cooldown_after_sl_secs));
    info!("Filter chain initialized with {} filters", filter_chain.len());

    // 5. Execution engine
    let execution_engine = execution::ExecutionEngine::new(config.clone());
    info!("Execution engine initialized");

    // 6. PnL tracker
    let pnl_tracker = fee_pnl::PnlTracker::new();
    info!("PnL tracker initialized");

    // 7. Metrics
    let metrics_registry = metrics::MetricsRegistry::new();
    info!("Metrics registry initialized");

    // Main trading loop
    info!("Starting main trading loop...");

    let mut running = true;
    while running {
        // Simulate receiving order book updates
        // In real implementation, this would come from WebSocket

        // Check risk gate
        let risk_results = risk_gate.check(&args.symbol);
        if !risk_gate.all_passed(&risk_results) {
            info!("Risk gate blocked trading");
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            continue;
        }

        // Process signals (placeholder)
        info!("Processing signals for {}", args.symbol);

        // Sleep to avoid tight loop
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }

    info!("Shutting down...");
    Ok(())
}

fn load_config(path: &str) -> Result<StrategyConfig> {
    // Try to load from file, fall back to default
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

// Re-export for use in main
use rust_decimal::Decimal;
