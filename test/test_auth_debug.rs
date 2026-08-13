//! 认证调试测试 - 使用正确的 body 参数

use exchange_adapter_lbank::{auth::LbankSigner, client::LbankClient, proxy::ProxyConfig};
use serde::Serialize;
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Serialize)]
struct MarketOrderRequest<'a> {
    #[serde(rename = "ProductGroup")]
    product_group: &'a str,
    #[serde(rename = "ExchangeID")]
    exchange_id: &'a str,
    #[serde(rename = "InstrumentID")]
    instrument_id: &'a str,
    depth: i32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .finish()
        .init();

    info!("===========================================");
    info!("认证调试测试 - 使用正确参数");
    info!("===========================================");

    // 创建签名器
    let signer = LbankSigner::new(
        "23bec4f8489109e112812c2c2c7c31b3".to_string(),
        "LBA8G85737".to_string(),
        "0688c69dd06a41f38c482e0f46719ed8".to_string(),
        Some("hZlegXdOAxOsNqUVl7oL8p8lwE3dIeqQ".to_string()),
    );

    // 使用正确的路径
    let path = "/cfd/market/v1.0/SendQryMarketOrder";
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

    // 创建客户端
    let proxy_config = ProxyConfig::default();
    let client = Arc::new(LbankClient::with_base_url_and_proxy(
        signer,
        "https://uuapi.rerrkvifj.com",
        proxy_config,
    )?);

    info!("\n发送请求...");

    // 使用正确的参数格式
    let request = MarketOrderRequest {
        product_group: "SwapU",
        exchange_id: "Exchange",
        instrument_id: "BTCUSDT",
        depth: 25,
    };
    
    // 打印请求体
    let body_json = serde_json::to_string(&request).unwrap();
    info!("  Request Body: {}", body_json);

    // 发送请求
    match client.post::<_, serde_json::Value>(path, &request).await {
        Ok(resp) => {
            info!("✅ 请求成功!");
            info!("  Response: {}", serde_json::to_string_pretty(&resp).unwrap_or_default());
        }
        Err(e) => {
            error!("❌ 请求失败: {}", e);
        }
    }

    Ok(())
}
