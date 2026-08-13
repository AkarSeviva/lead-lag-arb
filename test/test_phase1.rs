//! Phase 1: 连接与认证测试
//!
//! 测试目标：
//! 1. REST API 连接
//! 2. WebSocket 连接
//! 3. 签名验证
//!
//! 运行方式: cargo run --bin test_phase1

use anyhow::Result;
use exchange_adapter_lbank::{auth::LbankSigner, client::LbankClient};
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
    info!("Phase 1: 连接与认证测试");
    info!("===========================================");

    // 认证凭证
    let signer = LbankSigner::new(
        "23bec4f8489109e112812c2c2c7c31b3".to_string(),
        "LBA8G85737".to_string(),
        "0688c69dd06a41f38c482e0f46719ed8".to_string(),
        Some("hZlegXdOAxOsNqUVl7oL8p8lwE3dIeqQ".to_string()),
    );
    let client = Arc::new(LbankClient::new(signer)?);

    info!("✅ LbankClient 初始化成功");

    // ===========================================
    // 测试 1: 查询账户余额 (验证认证 + 获取余额)
    // ===========================================
    info!("\n[Test 1] 查询账户余额 (sendQryAll)...");
    match client.get_account_balance("BTCUSDT").await {
        Ok(balance) => {
            info!("✅ 认证成功! 账户余额:");
            if let Some(asset_balance) = &balance.asset_balance {
                info!("  - USDT available: {}", asset_balance.available);
                info!("  - USDT balance: {}", asset_balance.balance);
                info!("  - 冻结保证金: {}", asset_balance.frozen_margin);
                info!("  - 已结盈亏: {}", asset_balance.total_close_profit);
            } else {
                info!("  (无 assetBalance 字段)");
            }
            if let Some(leverage) = balance.long_leverage {
                info!("  - 当前杠杆: {}x", leverage);
            }
        }
        Err(e) => {
            error!("❌ 查询失败: {}", e);
            error!("请检查认证凭证是否正确");
            return Err(e);
        }
    }

    // ===========================================
    // 测试 3: WebSocket 连接测试
    // ===========================================
    info!("\n[Test 3] WebSocket 连接测试...");
    info!("WebSocket 测试需要单独运行，见 test_ws_connect.rs");
    info!("跳过 WebSocket 测试...");

    info!("\n===========================================");
    info!("Phase 1 测试完成!");
    info!("===========================================");

    Ok(())
}
