//! Lbank API Authentication
//!
//! Implements HMAC-SHA256 + Token based authentication as reversed from browser analysis.
//!
//! Signature string format (源码确认):
//! [METHOD][PATH][TIMESTAMP][USER_AGENT][VERSION_CODE][CHANNEL][CLIENT_TYPE][DEVICE_ID]

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use ring::hmac::{self, HMAC_SHA256};

/// Lbank API Signer
#[derive(Clone)]
pub struct LbankSigner {
    /// API Secret (obtained from login)
    api_secret: String,
    /// Device ID
    device_id: String,
    /// Client version code
    version_code: String,
    /// Channel (WEB)
    channel: String,
    /// Client type (WEB)
    client_type: String,
    /// User Agent string
    user_agent: String,
    /// User ID
    uid: String,
    /// Login token
    token: String,
}

impl LbankSigner {
    /// Create a new signer with credentials
    pub fn new(
        api_secret: String,
        uid: String,
        token: String,
        device_id: Option<String>,
    ) -> Self {
        Self {
            api_secret,
            uid,
            token,
            device_id: device_id.unwrap_or_else(|| generate_device_id()),
            version_code: "20251120".to_string(),
            channel: "WEB".to_string(),
            client_type: "WEB".to_string(),
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36".to_string(),
        }
    }

    /// Generate signature for a request
    /// sign_string = [METHOD][PATH][TIMESTAMP][USER_AGENT][VERSION_CODE][CHANNEL][CLIENT_TYPE][DEVICE_ID]
    pub fn sign(&self, method: &str, path: &str) -> (String, i64) {
        let timestamp = chrono::Utc::now().timestamp_millis();
        let sign_string = self.build_sign_string(method, path, timestamp);
        let signature = self.compute_hmac_sha256(&sign_string);
        (signature, timestamp)
    }

    /// Build the signature string (public for testing)
    pub fn build_sign_string(&self, method: &str, path: &str, timestamp: i64) -> String {
        // Format: [METHOD][PATH][TIMESTAMP][USER_AGENT][VERSION_CODE][CHANNEL][CLIENT_TYPE][DEVICE_ID]
        format!(
            "{}{}{}{}{}{}{}{}",
            method.to_uppercase(),
            path,
            timestamp,
            self.user_agent,
            self.version_code,
            self.channel,
            self.client_type,
            self.device_id
        )
    }

    /// Compute HMAC-SHA256 and return Base64 encoded signature
    fn compute_hmac_sha256(&self, message: &str) -> String {
        let key = hmac::Key::new(HMAC_SHA256, self.api_secret.as_bytes());
        let signature = hmac::sign(&key, message.as_bytes());

        // Direct Base64 encode of the raw HMAC output
        BASE64.encode(signature.as_ref())
    }

    /// Get required headers for a request
    pub fn get_headers(&self, method: &str, path: &str) -> LbankHeaders {
        let (signature, timestamp) = self.sign(method, path);
        
        LbankHeaders {
            timestamp: timestamp.to_string(),
            token: self.token.clone(),
            uid: self.uid.clone(),
            signature,
            device_id: self.device_id.clone(),
            version_code: self.version_code.clone(),
            channel: self.channel.clone(),
            client_type: self.client_type.clone(),
            user_agent: self.user_agent.clone(),
            language: "zh-TW".to_string(),
            station: "1".to_string(),
            client_source: "WEB".to_string(),
            business_version_code: "202".to_string(),
            version_flage: true,
        }
    }

    /// Get timestamp header
    pub fn timestamp(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}

/// Lbank request headers
#[derive(Debug, Clone)]
pub struct LbankHeaders {
    pub timestamp: String,
    pub token: String,
    pub uid: String,
    pub signature: String,
    pub device_id: String,
    pub version_code: String,
    pub channel: String,
    pub client_type: String,
    pub user_agent: String,
    pub language: String,
    pub station: String,
    pub client_source: String,
    pub business_version_code: String,
    pub version_flage: bool,
}

impl LbankHeaders {
    pub fn into_reqwest_headers(&self) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

        let mut headers = HeaderMap::new();

        // Core auth headers
        headers.insert(HeaderName::from_static("ex-timestamp"), HeaderValue::from_str(&self.timestamp).unwrap());
        headers.insert(HeaderName::from_static("ex-token"), HeaderValue::from_str(&self.token).unwrap());
        headers.insert(HeaderName::from_static("ex-uid"), HeaderValue::from_str(&self.uid).unwrap());
        headers.insert(HeaderName::from_static("ex-signature"), HeaderValue::from_str(&self.signature).unwrap());
        headers.insert(HeaderName::from_static("ex-device-id"), HeaderValue::from_str(&self.device_id).unwrap());
        headers.insert(HeaderName::from_static("ex-client-version-code"), HeaderValue::from_str(&self.version_code).unwrap());
        headers.insert(HeaderName::from_static("ex-client-type"), HeaderValue::from_str(&self.client_type).unwrap());
        headers.insert(HeaderName::from_static("ex-client-channel"), HeaderValue::from_str(&self.channel).unwrap());
        headers.insert(HeaderName::from_static("ex-client-source"), HeaderValue::from_str(&self.client_source).unwrap());

        // Browser/Platform headers
        headers.insert(HeaderName::from_static("ex-browser-name"), HeaderValue::from_static("Chrome"));
        headers.insert(HeaderName::from_static("ex-browser-version"), HeaderValue::from_static("151.0.0.0"));
        headers.insert(HeaderName::from_static("ex-os-name"), HeaderValue::from_static("Windows"));
        headers.insert(HeaderName::from_static("ex-os-version"), HeaderValue::from_static("10.0"));
        headers.insert(HeaderName::from_static("ex-language"), HeaderValue::from_str(&self.language).unwrap());
        headers.insert(HeaderName::from_static("ex-station"), HeaderValue::from_str(&self.station).unwrap());

        // Business headers
        headers.insert(HeaderName::from_static("businessversioncode"), HeaderValue::from_str(&self.business_version_code).unwrap());
        headers.insert(HeaderName::from_static("versionflage"), HeaderValue::from_static("true"));

        // User-Agent
        headers.insert(
            HeaderName::from_static("user-agent"),
            HeaderValue::from_str(&self.user_agent).unwrap()
        );

        // Browser fingerprint headers
        headers.insert(HeaderName::from_static("origin"), HeaderValue::from_static("https://www.lbank.com"));
        headers.insert(HeaderName::from_static("referer"), HeaderValue::from_static("https://www.lbank.com/"));
        headers.insert(HeaderName::from_static("source"), HeaderValue::from_static("4"));
        
        // sec-ch-ua headers (browser fingerprint)
        headers.insert(HeaderName::from_static("sec-ch-ua"), HeaderValue::from_static("\"Not=A?Brand\";v=\"99\", \"Google Chrome\";v=\"151\", \"Chromium\";v=\"151\""));
        headers.insert(HeaderName::from_static("sec-ch-ua-mobile"), HeaderValue::from_static("?0"));
        headers.insert(HeaderName::from_static("sec-ch-ua-platform"), HeaderValue::from_static("\"Windows\""));
        headers.insert(HeaderName::from_static("sec-fetch-dest"), HeaderValue::from_static("empty"));
        headers.insert(HeaderName::from_static("sec-fetch-mode"), HeaderValue::from_static("cors"));
        headers.insert(HeaderName::from_static("sec-fetch-site"), HeaderValue::from_static("cross-site"));

        // Accept headers
        headers.insert(HeaderName::from_static("accept"), HeaderValue::from_static("*/*"));
        headers.insert(HeaderName::from_static("accept-encoding"), HeaderValue::from_static("gzip, deflate, br"));
        headers.insert(HeaderName::from_static("accept-language"), HeaderValue::from_str(&self.language).unwrap());

        headers
    }
}

/// Generate a random device ID
fn generate_device_id() -> String {
    use uuid::Uuid;
    Uuid::new_v4().to_string().replace("-", "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signer_creation() {
        let signer = LbankSigner::new(
            "test_secret".to_string(),
            "TEST_UID".to_string(),
            "test_token".to_string(),
            None,
        );

        let (signature, timestamp) = signer.sign("POST", "/test/path");

        assert!(!signature.is_empty());
        assert!(timestamp > 0);
    }

    #[test]
    fn test_headers_generation() {
        let signer = LbankSigner::new(
            "test_secret".to_string(),
            "TEST_UID".to_string(),
            "test_token".to_string(),
            Some("test_device_id".to_string()),
        );

        let headers = signer.get_headers("POST", "/test/path");

        assert_eq!(headers.uid, "TEST_UID");
        assert_eq!(headers.token, "test_token");
        assert_eq!(headers.device_id, "test_device_id");
        assert!(!headers.signature.is_empty());
    }
}
