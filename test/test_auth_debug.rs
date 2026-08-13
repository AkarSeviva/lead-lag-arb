//! 认证调试测试 - 保存结果到文件

use exchange_adapter_lbank::{auth::LbankSigner, client::LbankClient, proxy::ProxyConfig};
use serde::Serialize;
use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::util::SubscriberInitExt;

fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .finish()
        .init();

    let mut output = String::new();
    output.push_str("===========================================\n");
    output.push_str("Lbank API 测试\n");
    output.push_str("===========================================\n\n");

    // 创建签名器
    let signer = LbankSigner::new(
        "23bec4f8489109e112812c2c2c7c31b3".to_string(),
        "LBA8G85737".to_string(),
        "0688c69dd06a41f38c482e0f46719ed8".to_string(),
        Some("hZlegXdOAxOsNqUVl7oL8p8lwE3dIeqQ".to_string()),
    );

    // 创建客户端
    let proxy_config = ProxyConfig::default();
    let client = Arc::new(LbankClient::with_base_url_and_proxy(
        signer,
        "https://uuapi.rerrkvifj.com",
        proxy_config,
    )?);

    let rt = tokio::runtime::Runtime::new()?;

    // 测试 1: 市场订单簿
    output.push_str("【测试 1】市场订单簿 (get_order_book)\n");
    output.push_str("----------------------------------------\n");
    match rt.block_on(client.get_order_book("BTCUSDT", 25)) {
        Ok(items) => {
            output.push_str(&format!("✅ 成功! 获取到 {} 条订单\n\n", items.len()));
            output.push_str("前10条:\n");
            for (i, item) in items.iter().take(10).enumerate() {
                output.push_str(&format!(
                    "  {}. 价格={}, 数量={}, 方向={}\n",
                    i + 1, item.price, item.volume, item.direction
                ));
            }
        }
        Err(e) => {
            output.push_str(&format!("❌ 失败: {}\n", e));
        }
    }
    output.push_str("\n");

    // 测试 2: 账户余额
    output.push_str("【测试 2】账户余额 (get_account_balance)\n");
    output.push_str("----------------------------------------\n");
    match rt.block_on(client.get_account_balance("BTCUSDT")) {
        Ok(info) => {
            output.push_str("✅ 成功!\n\n");
            if let Some(balance) = &info.asset_balance {
                output.push_str(&format!("  资产余额: {}\n", balance.balance));
                output.push_str(&format!("  可用余额: {}\n", balance.available));
                output.push_str(&format!("  冻结保证金: {}\n", balance.frozen_margin));
            }
            output.push_str(&format!("  标记价格: {}\n", info.marked_price));
        }
        Err(e) => {
            output.push_str(&format!("❌ 失败: {}\n", e));
        }
    }

    // 写入文件
    let filename = "test_auth_debug_result.json";
    let mut file = File::create(filename)?;
    file.write_all(output.as_bytes())?;
    
    println!("结果已保存到: {}", filename);

    Ok(())
}
