//! Lbank REST API Client
//!
//! HTTP client with connection pooling and authentication support.

use crate::auth::LbankSigner;
use crate::protocol::*;
use crate::proxy::{ProxyClient, ProxyConfig};
use anyhow::{Context, Result};
use reqwest::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, instrument};

/// REST API base URL
const API_BASE: &str = "https://uuapi.rerrkvifj.com";

/// Lbank REST API Client
#[derive(Clone)]
pub struct LbankClient {
    client: Client,
    signer: Arc<LbankSigner>,
    base_url: String,
    proxy: Option<ProxyClient>,
}

impl LbankClient {
    /// Create a new client with connection pooling
    pub fn new(signer: LbankSigner) -> Result<Self> {
        Self::with_proxy(signer, ProxyConfig::default())
    }

    /// Create with proxy configuration
    pub fn with_proxy(signer: LbankSigner, proxy_config: ProxyConfig) -> Result<Self> {
        let proxy = ProxyClient::new(proxy_config.clone());
        let client = proxy.build_reqwest_client()?;

        Ok(Self {
            client,
            signer: Arc::new(signer),
            base_url: API_BASE.to_string(),
            proxy: Some(proxy),
        })
    }

    /// Create with custom base URL (for testing)
    pub fn with_base_url(signer: LbankSigner, base_url: &str) -> Result<Self> {
        Self::with_base_url_and_proxy(signer, base_url, ProxyConfig::default())
    }

    /// Create with custom base URL and proxy
    pub fn with_base_url_and_proxy(
        signer: LbankSigner,
        base_url: &str,
        proxy_config: ProxyConfig,
    ) -> Result<Self> {
        let proxy = ProxyClient::new(proxy_config.clone());
        let client = proxy.build_reqwest_client()?;

        Ok(Self {
            client,
            signer: Arc::new(signer),
            base_url: base_url.to_string(),
            proxy: Some(proxy),
        })
    }

    /// Check if proxy is enabled
    pub fn is_proxy_enabled(&self) -> bool {
        self.proxy.as_ref().map(|p| p.is_enabled()).unwrap_or(false)
    }

    /// Get proxy configuration
    pub fn proxy_config(&self) -> Option<&ProxyConfig> {
        self.proxy.as_ref().map(|p| p.config())
    }

    /// Make an authenticated POST request
    #[instrument(skip(self, body), fields(path = %path))]
    #[allow(dead_code)]
    pub async fn post<B, R>(&self, path: &str, body: &B) -> Result<R>
    where
        B: serde::Serialize,
        R: serde::de::DeserializeOwned,
    {
        let body_str = serde_json::to_string(body)?;
        let headers = self.signer.get_headers(&body_str, path);
        let url = format!("{}{}", self.base_url, path);

        debug!(url = %url, "POST request");

        let response = self
            .client
            .post(&url)
            .headers(headers.into_reqwest_headers())
            .json(body)
            .send()
            .await
            .context("Request failed")?;

        let status = response.status();
        let body_text = response.text().await.context("Failed to read response")?;

        debug!(status = %status, body = %body_text);

        if !status.is_success() {
            error!(status = %status, body = %body_text, "Request failed");
            anyhow::bail!("API request failed: {} - {}", status, body_text);
        }

        let parsed: LbankResponse<R> =
            serde_json::from_str(&body_text).context("Failed to parse response")?;

        parsed.into_result().context("API returned error")
    }

    /// Make an authenticated GET request
    #[instrument(skip(self), fields(path = %path))]
    #[allow(dead_code)]
    pub async fn get<R>(&self, path: &str, query: Option<&[(&str, &str)]>) -> Result<R>
    where
        R: serde::de::DeserializeOwned,
    {
        let headers = self.signer.get_headers("", path);
        let mut url = format!("{}{}", self.base_url, path);

        if let Some(query) = query {
            let query_str: String = query
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join("&");
            url = format!("{}?{}", url, query_str);
        }

        debug!(url = %url, "GET request");

        let mut req_builder = self.client.get(&url).headers(headers.into_reqwest_headers());

        if let Some(query) = query {
            req_builder = req_builder.query(query);
        }

        let response = req_builder.send().await.context("Request failed")?;

        let status = response.status();
        let body_text = response.text().await.context("Failed to read response")?;

        debug!(status = %status, body = %body_text);

        if !status.is_success() {
            error!(status = %status, body = %body_text, "Request failed");
            anyhow::bail!("API request failed: {} - {}", status, body_text);
        }

        let parsed: LbankResponse<R> =
            serde_json::from_str(&body_text).context("Failed to parse response")?;

        parsed.into_result().context("API returned error")
    }

    // ========================================================================
    // Market Data APIs
    // ========================================================================

    /// Get 24hr ticker for all instruments
    pub async fn get_tickers_24hr(&self) -> Result<Vec<Ticker24hr>> {
        #[derive(Serialize)]
        struct Request {
            product: Vec<String>,
        }

        self.post("/cfd/instrment/v1/ticker/24hr/intact", &Request {
            product: vec!["FUTURES".to_string()],
        }).await
    }

    /// Get order book depth
    pub async fn get_order_book(
        &self,
        symbol: &str,
        depth: usize,
    ) -> Result<Vec<MarketOrderItem>> {
        #[derive(Serialize)]
        struct Request<'a> {
            #[serde(rename = "ProductGroup")]
            product_group: &'a str,
            #[serde(rename = "ExchangeID")]
            exchange_id: &'a str,
            #[serde(rename = "InstrumentID")]
            instrument_id: &'a str,
            depth: usize,
        }

        #[derive(Deserialize)]
        struct Response {
            table: String,
            data: Vec<MarketOrderItem>,
        }

        let resp: Response = self.post("/cfd/market/v1.0/SendQryMarketOrder", &Request {
            product_group: "SwapU",
            exchange_id: "Exchange",
            instrument_id: symbol,
            depth,
        }).await?;

        Ok(resp.data)
    }

    /// Get instrument info
    pub async fn get_instruments(&self) -> Result<Vec<InstrumentInfo>> {
        #[derive(Serialize)]
        struct Request<'a> {
            #[serde(rename = "ProductGroup")]
            product_group: &'a str,
        }

        self.post("/cfd/agg/v1/instrument", &Request {
            product_group: "SwapU",
        }).await
    }

    /// Get aggregate info (position limits, marked price)
    pub async fn get_aggregate_info(&self, symbol: &str) -> Result<AggregateInfo> {
        #[derive(Serialize)]
        struct Request<'a> {
            product_group: &'a str,
            #[serde(rename = "instrumentID")]
            instrument_id: &'a str,
            asset: &'a str,
        }

        #[derive(Deserialize)]
        struct Response {
            #[serde(rename = "isMarketAcount")]
            is_market_account: i32,
            #[serde(rename = "longMaxVolume")]
            long_max_volume: String,
            #[serde(rename = "shortMaxVolume")]
            short_max_volume: String,
            #[serde(rename = "longMaxLeverage")]
            long_max_leverage: i32,
            #[serde(rename = "shortMaxLeverage")]
            short_max_leverage: i32,
            #[serde(rename = "markedPrice")]
            marked_price: String,
            #[serde(rename = "isOnlyClose")]
            is_only_close: i32,
            state: i32,
        }

        let resp: Response = self.post("/cfd/agg/v1/sendQryAll", &Request {
            product_group: "SwapU",
            instrument_id: symbol,
            asset: "USDT",
        }).await?;

        Ok(AggregateInfo {
            is_market_account: resp.is_market_account,
            long_max_volume: resp.long_max_volume,
            short_max_volume: resp.short_max_volume,
            long_max_leverage: resp.long_max_leverage,
            short_max_leverage: resp.short_max_leverage,
            marked_price: resp.marked_price,
            is_only_close: resp.is_only_close,
            state: resp.state,
        })
    }

    /// Get fee rate
    pub async fn get_fee_rate(&self, symbol: &str) -> Result<FeeRateResponse> {
        #[derive(Deserialize)]
        struct Inner {
            #[serde(rename = "makerOpenFeeRate")]
            maker_open_fee_rate: String,
            #[serde(rename = "makerCloseFeeRate")]
            maker_close_fee_rate: String,
            #[serde(rename = "takerOpenFeeRate")]
            taker_open_fee_rate: String,
            #[serde(rename = "takerCloseFeeRate")]
            taker_close_fee_rate: String,
        }

        let resp: Inner = self.get(
            "/cfd/user/v1/userFee",
            Some(&[("instrumentID", symbol)]),
        ).await?;

        Ok(FeeRateResponse {
            maker_open_fee_rate: resp.maker_open_fee_rate,
            maker_close_fee_rate: resp.maker_close_fee_rate,
            taker_open_fee_rate: resp.taker_open_fee_rate,
            taker_close_fee_rate: resp.taker_close_fee_rate,
        })
    }

    // ========================================================================
    // Trading APIs
    // ========================================================================

    /// Place a market order
    pub async fn place_market_order(
        &self,
        symbol: &str,
        direction: TradeDirection,
        offset: OffsetFlag,
        volume: Decimal,
    ) -> Result<OrderInsertResponse> {
        let req = OrderInsertRequest::new_market_order(
            symbol,
            direction,
            offset,
            volume.to_string().parse().unwrap_or(0.0),
        );

        self.post("/cfd/cff/v1/SendOrderInsert", &req).await
    }

    /// Place a limit order
    pub async fn place_limit_order(
        &self,
        symbol: &str,
        direction: TradeDirection,
        offset: OffsetFlag,
        volume: Decimal,
        price: Decimal,
    ) -> Result<OrderInsertResponse> {
        let req = OrderInsertRequest::new_limit_order(
            symbol,
            direction,
            offset,
            volume.to_string().parse().unwrap_or(0.0),
            price.to_string().parse().unwrap_or(0.0),
        );

        self.post("/cfd/cff/v1/SendOrderInsert", &req).await
    }

    /// Query positions
    pub async fn query_positions(&self) -> Result<Vec<PositionResponse>> {
        #[derive(Deserialize)]
        struct Response {
            #[serde(rename = "instrumentId")]
            instrument_id: String,
            #[serde(rename = "exchangeId")]
            exchange_id: String,
            direction: Option<String>,
            #[serde(rename = "positionVolume")]
            position_volume: Option<String>,
            #[serde(rename = "positionCost")]
            position_cost: Option<String>,
            #[serde(rename = "positionProfit")]
            position_profit: Option<String>,
            #[serde(rename = "useVolume")]
            use_volume: Option<String>,
            #[serde(rename = "frozenVolume")]
            frozen_volume: Option<String>,
            #[serde(rename = "canCloseVolume")]
            can_close_volume: Option<String>,
        }

        let resp: Vec<Response> = self.get(
            "/cfd/query/v1.0/Position",
            Some(&[
                ("ProductGroup", "SwapU"),
                ("Valid", "1"),
                ("pageIndex", "1"),
                ("pageSize", "1000"),
            ]),
        ).await?;

        Ok(resp.into_iter().map(|r| PositionResponse {
            instrument_id: r.instrument_id,
            exchange_id: r.exchange_id,
            direction: r.direction,
            position_volume: r.position_volume,
            position_cost: r.position_cost,
            position_profit: r.position_profit,
            use_volume: r.use_volume,
            frozen_volume: r.frozen_volume,
            can_close_volume: r.can_close_volume,
        }).collect())
    }

    /// Query orders
    pub async fn query_orders(&self, symbol: Option<&str>) -> Result<Vec<OrderResponse>> {
        let mut params = vec![
            ("ProductGroup", "SwapU"),
            ("ExchangeID", "Exchange"),
            ("pageIndex", "1"),
            ("pageSize", "1000"),
        ];

        if let Some(s) = symbol {
            params.push(("InstrumentID", s));
        }

        self.get("/cfd/query/v1.0/Order", Some(&params)).await
    }

    // ========================================================================
    // WebSocket Token
    // ========================================================================

    /// Get WebSocket authentication token
    pub async fn get_ws_token(&self) -> Result<String> {
        #[derive(Serialize)]
        struct Request {}

        #[derive(Deserialize)]
        struct Response {
            #[serde(rename = "code")]
            code: i32,
            #[serde(rename = "data")]
            data: Option<String>,
        }

        let resp: Response = self.post("/cfd/user/v1/generateWsToken", &Request {}).await?;

        if resp.code == 200 {
            resp.data.ok_or_else(|| anyhow::anyhow!("Empty token in response"))
        } else {
            anyhow::bail!("Failed to get WS token: code={}", resp.code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let signer = LbankSigner::new(
            "test_secret".to_string(),
            "TEST_UID".to_string(),
            "test_token".to_string(),
            None,
        );
        
        let client = LbankClient::new(signer);
        assert!(client.is_ok());
    }
}
