//! HTTP CONNECT Proxy Client
//!
//! Provides proxy configuration for reqwest HTTP client.

use anyhow::Context;
use std::time::Duration;
use tracing::info;

/// Proxy configuration
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Enable proxy
    pub enabled: bool,
    /// Proxy server address (e.g., "127.0.0.1:7890")
    pub proxy_addr: String,
    /// Connection timeout in seconds
    pub connect_timeout_secs: u64,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            proxy_addr: "127.0.0.1:7890".to_string(),
            connect_timeout_secs: 10,
        }
    }
}

impl ProxyConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn connect_timeout(&self) -> Duration {
        Duration::from_secs(self.connect_timeout_secs)
    }
}

/// Proxy client for HTTP CONNECT tunneling
#[derive(Clone)]
pub struct ProxyClient {
    config: ProxyConfig,
}

impl ProxyClient {
    pub fn new(config: ProxyConfig) -> Self {
        Self { config }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    pub fn proxy_addr(&self) -> &str {
        &self.config.proxy_addr
    }

    pub fn config(&self) -> &ProxyConfig {
        &self.config
    }

    /// Build a reqwest Client with proxy configuration
    pub fn build_reqwest_client(&self) -> anyhow::Result<reqwest::Client> {
        let timeout = Duration::from_secs(30);
        
        let mut builder = reqwest::Client::builder()
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(30))
            .timeout(timeout);

        if self.config.enabled {
            let proxy_url = format!("http://{}", self.config.proxy_addr);
            let proxy = reqwest::Proxy::http(&proxy_url)
                .context("Failed to create HTTP proxy")?;

            builder = builder.proxy(proxy);
            info!(proxy = %self.config.proxy_addr, "reqwest client configured with proxy");
        }

        builder.build().context("Failed to build reqwest client")
    }
}

/// TCP connector that supports proxy (for WebSocket connections)
pub struct ProxyTcpConnector {
    config: ProxyConfig,
}

impl ProxyTcpConnector {
    pub fn new(config: ProxyConfig) -> Self {
        Self { config }
    }

    /// Check if proxy is enabled
    pub fn is_proxy_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Get proxy address
    pub fn proxy_addr(&self) -> &str {
        &self.config.proxy_addr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proxy_config_default() {
        let config = ProxyConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.proxy_addr, "127.0.0.1:7890");
    }

    #[test]
    fn test_proxy_enabled() {
        let config = ProxyConfig {
            enabled: true,
            proxy_addr: "127.0.0.1:7890".to_string(),
            connect_timeout_secs: 10,
        };
        let client = ProxyClient::new(config);
        assert!(client.is_enabled());
        assert_eq!(client.proxy_addr(), "127.0.0.1:7890");
    }
}
