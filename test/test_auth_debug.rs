//! 认证调试测试 - 打印所有发送的 headers 和签名

use exchange_adapter_lbank::{auth::LbankSigner, client::LbankClient, proxy::ProxyConfig};
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .finish()
        .init();

    info!("===========================================");
    info!("认证调试测试");
    info!("===========================================");

    // 创建签名器
    let signer = LbankSigner::new(
        "23bec4f8489109e112812c2c2c7c31b3".to_string(),
        "LBA8G85737".to_string(),
        "0688c69dd06a41f38c482e0f46719ed8".to_string(),
        Some("hZlegXdOAxOsNqUVl7oL8p8lwE3dIeqQ".to_string()),
    );

    // 获取 headers
    let path = "/cfd/agg/v1/sendQryAll";
    let headers = signer.get_headers("POST", path);

    info!("生成的签名信息:");
    info!("  Method: POST");
    info!("  Path: {}", path);
    info!("  Timestamp: {}", headers.timestamp);
    info!("  UID: {}", headers.uid);
    info!("  Token: {}", headers.token);
    info!("  Device ID: {}", headers.device_id);
    info!("  Signature: {}", headers.signature);
    info!("  Version: {}", headers.version_code);

    // 构建签名字符串
    let sign_string = signer.build_sign_string("POST", path, headers.timestamp.parse().unwrap());
    info!("  签名字符串 (前300字符): {}", &sign_string[..300.min(sign_string.len())]);

    // 创建客户端
    let proxy_config = ProxyConfig::default();
    let client = Arc::new(LbankClient::with_base_url_and_proxy(
        signer,
        "https://uuapi.rerrkvifj.com",
        proxy_config,
    )?);

    info!("\n发送请求...");

    // 发送请求
    match client.get_account_balance("BTCUSDT").await {
        Ok(info) => {
            info!("✅ 请求成功!");
            if let Some(asset_balance) = &info.asset_balance {
                info!("  USDT available: {}", asset_balance.available);
            }
        }
        Err(e) => {
            error!("❌ 请求失败: {}", e);
        }
    }

    Ok(())
}
