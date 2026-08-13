//! 认证调试测试 - 保存结果到文件

use exchange_adapter_lbank::{auth::LbankSigner, client::LbankClient, proxy::ProxyConfig};
use serde::Serialize;
use std::fs::File;
use std::io::Write;
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

fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .finish()
        .init();

    let mut output = String::new();
    output.push_str("===========================================\n");
    output.push_str("认证调试测试 - 使用正确参数\n");
    output.push_str("===========================================\n\n");

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

    output.push_str(&format!("生成的签名信息:\n"));
    output.push_str(&format!("  Method: POST\n"));
    output.push_str(&format!("  Path: {}\n", path));
    output.push_str(&format!("  Timestamp: {}\n", headers.timestamp));
    output.push_str(&format!("  UID: {}\n", headers.uid));
    output.push_str(&format!("  Token: {}\n", headers.token));
    output.push_str(&format!("  Device ID: {}\n", headers.device_id));
    output.push_str(&format!("  Signature: {}\n", headers.signature));
    output.push_str(&format!("  Version: {}\n", headers.version_code));
    output.push_str("\n");

    // 创建客户端
    let proxy_config = ProxyConfig::default();
    let client = Arc::new(LbankClient::with_base_url_and_proxy(
        signer,
        "https://uuapi.rerrkvifj.com",
        proxy_config,
    )?);

    output.push_str("发送请求...\n\n");

    // 使用正确的参数格式
    let request = MarketOrderRequest {
        product_group: "SwapU",
        exchange_id: "Exchange",
        instrument_id: "BTCUSDT",
        depth: 25,
    };
    
    // 打印请求体
    let body_json = serde_json::to_string(&request).unwrap();
    output.push_str(&format!("  Request Body: {}\n\n", body_json));

    // 发送请求 - 使用 serde_json::Value 接收原始 JSON
    let rt = tokio::runtime::Runtime::new()?;
    match rt.block_on(client.post::<_, serde_json::Value>(path, &request)) {
        Ok(resp) => {
            output.push_str("✅ 请求成功!\n\n");
            output.push_str("Response:\n");
            output.push_str(&serde_json::to_string_pretty(&resp).unwrap_or_default());
            output.push_str("\n");
        }
        Err(e) => {
            output.push_str(&format!("❌ 请求失败: {}\n", e));
        }
    }

    // 写入文件
    let filename = "test_auth_debug_result.json";
    let mut file = File::create(filename)?;
    file.write_all(output.as_bytes())?;
    
    println!("结果已保存到: {}", filename);

    Ok(())
}
