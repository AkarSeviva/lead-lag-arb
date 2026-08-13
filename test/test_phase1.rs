//! Phase 1: 直连测试 (不走代理)
//!
//! 测试目标：
//! 1. VPS 直连 Lbank 是否可达
//! 2. REST API 响应
//!
//! 运行方式: cargo run --bin test_phase1

use anyhow::Result;
use exchange_adapter_lbank::{auth::LbankSigner, client::LbankClient, proxy::ProxyConfig};
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    let subscriber = FmtSubscriber::builder()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("===========================================");
    info!("Phase 1: 直连测试 (不走代理)");
    info!("===========================================");

    // 认证凭证
    let signer = LbankSigner::new(
        "23bec4f8489109e112812c2c2c7c31b3".to_string(),
        "LBA8G85737".to_string(),
        "0688c69dd06a41f38c482e0f46719ed8".to_string(),
        Some("hZlegXdOAxOsNqUVl7oL8p8lwE3dIeqQ".to_string()),
    );

    // 使用无代理配置
    let proxy_config = ProxyConfig::default(); // enabled=false
    let client = Arc::new(LbankClient::with_base_url_and_proxy(
        signer,
        "https://uuapi.rerrkvifj.com",
        proxy_config,
    )?);

    info!("✅ LbankClient 初始化成功 (无代理)");
    info!("目标服务器: https://uuapi.rerrkvifj.com");

    // ===========================================
    // 测试 1: 直连测试 - sendQryAll 获取余额
    // ===========================================
    info!("\n[Test 1] 直连测试 - sendQryAll...");

    match client.get_account_balance("BTCUSDT").await {
        Ok(info) => {
            info!("✅ 直连成功! 账户信息:");
            if let Some(asset_balance) = &info.asset_balance {
                info!("  - USDT available: {}", asset_balance.available);
                info!("  - USDT balance: {}", asset_balance.balance);
                info!("  - 冻结保证金: {}", asset_balance.frozen_margin);
                info!("  - 已结盈亏: {}", asset_balance.total_close_profit);
            }
            if let Some(leverage) = info.long_leverage {
                info!("  - 当前杠杆: {}x", leverage);
            }
        }
        Err(e) => {
            error!("❌ 直连失败: {}", e);
            error!("VPS 可能无法直连 Lbank，需要代理");
            return Err(e);
        }
    }

    info!("\n===========================================");
    info!("Phase 1 测试完成!");
    info!("===========================================");

    Ok(())
}
