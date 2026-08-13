//! 打印所有发送的 HTTP headers

use exchange_adapter_lbank::auth::LbankSigner;
use tracing::info;

fn main() {
    // 创建签名器
    let signer = LbankSigner::new(
        "23bec4f8489109e112812c2c2c7c31b3".to_string(),
        "LBA8G85737".to_string(),
        "0688c69dd06a41f38c482e0f46719ed8".to_string(),
        Some("hZlegXdOAxOsNqUVl7oL8p8lwE3dIeqQ".to_string()),
    );

    let path = "/cfd/agg/v1/sendQryAll";
    let headers = signer.get_headers("POST", path);

    info!("===========================================");
    info!("生成的认证信息:");
    info!("===========================================");
    info!("  ex-timestamp: {}", headers.timestamp);
    info!("  ex-uid: {}", headers.uid);
    info!("  ex-token: [REDACTED]");
    info!("  ex-signature: [REDACTED]");
    info!("  ex-device-id: {}", headers.device_id);
    info!("  ex-client-type: {}", headers.client_type);
    info!("  ex-client-channel: {}", headers.channel);
    info!("  ex-client-version-code: {}", headers.version_code);
    info!("  ex-client-source: {}", headers.client_source);
    info!("  ex-language: {}", headers.language);
    info!("  ex-station: {}", headers.station);
    info!("  businessversioncode: {}", headers.business_version_code);
    info!("  versionflage: {}", headers.version_flage);
    info!("  user-agent: {}", headers.user_agent);
    
    // 打印签名字符串
    let timestamp: i64 = headers.timestamp.parse().unwrap();
    let sign_string = signer.build_sign_string("POST", path, timestamp);
    info!("");
    info!("签名字符串:");
    info!("{}", sign_string);
    
    info!("===========================================");
}
