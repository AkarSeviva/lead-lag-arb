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
    pub async fn post<B, R>(&self, path: &str, body: &B) -> Result<R>
    where
        B: serde::Serialize,
        R: serde::de::DeserializeOwned,
    {
        let body_str = serde_json::to_string(body)?;
        let headers = self.signer.get_headers("POST", path);
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
        // reqwest 会自动解压 brotli/gzip/deflate 响应
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
        let headers = self.signer.get_headers("GET", path);
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
        // reqwest 会自动解压 brotli/gzip/deflate 响应
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

        // API 返回扁平数组: [{"table":"MarketOrder","data":{...}}, ...]
        #[derive(Deserialize)]
        struct MarketOrderResponse {
            #[serde(rename = "data")]
            data: MarketOrderItem,
        }

        let resp: Vec<MarketOrderResponse> = self.post("/cfd/market/v1.0/SendQryMarketOrder", &Request {
            product_group: "SwapU",
            exchange_id: "Exchange",
            instrument_id: symbol,
            depth,
        }).await?;

        Ok(resp.into_iter().map(|r| r.data).collect())
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
            #[serde(rename = "productGroup")]
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

    /// 市价开仓 - 文档3.4
    pub async fn market_open(
        &self,
        symbol: &str,
        direction: TradeDirection,
        volume: Decimal,
    ) -> Result<OrderInsertResponse> {
        let req = OrderInsertRequest::new_market_open(
            symbol,
            direction,
            volume.to_string().parse().unwrap_or(0.0),
        );
        self.post("/cfd/cff/v1/SendOrderInsert", &req).await
    }

    /// 市价平仓 - 文档3.5
    pub async fn market_close(
        &self,
        symbol: &str,
        direction: TradeDirection,
        volume: Decimal,
        trade_unit_id: &str,
    ) -> Result<OrderInsertResponse> {
        let req = OrderInsertRequest::new_market_close(
            symbol,
            direction,
            volume.to_string().parse().unwrap_or(0.0),
            trade_unit_id,
        );
        self.post("/cfd/cff/v1/SendOrderInsert", &req).await
    }

    /// 限价开仓 - 文档4.1
    pub async fn limit_open(
        &self,
        symbol: &str,
        direction: TradeDirection,
        volume: Decimal,
        price: Decimal,
    ) -> Result<OrderInsertResponse> {
        let req = OrderInsertRequest::new_limit_open(
            symbol,
            direction,
            volume.to_string().parse().unwrap_or(0.0),
            price.to_string().parse().unwrap_or(0.0),
        );
        self.post("/cfd/cff/v1/SendOrderInsert", &req).await
    }

    /// 限价平仓
    pub async fn limit_close(
        &self,
        symbol: &str,
        direction: TradeDirection,
        volume: Decimal,
        price: Decimal,
        trade_unit_id: &str,
    ) -> Result<OrderInsertResponse> {
        let req = OrderInsertRequest::new_limit_close(
            symbol,
            direction,
            volume.to_string().parse().unwrap_or(0.0),
            price.to_string().parse().unwrap_or(0.0),
            trade_unit_id,
        );
        self.post("/cfd/cff/v1/SendOrderInsert", &req).await
    }

    /// 设置杠杆 - 文档3.3
    pub async fn set_leverage(
        &self,
        symbol: &str,
        leverage: i32,
    ) -> Result<()> {
        #[derive(Deserialize)]
        struct Response { code: i32 }

        let req = SetLeverageRequest {
            instrument_id: symbol.to_string(),
            long_leverage: leverage,
            short_leverage: leverage,
        };

        let resp: Response = self.post("/cfd/position/v1/setMultiLeverage", &req).await?;
        if resp.code == 200 {
            Ok(())
        } else {
            anyhow::bail!("Set leverage failed: code={}", resp.code)
        }
    }

    /// 获取币种的最大可用杠杆 - 文档3.2
    /// 返回 (long_max_leverage, short_max_leverage)
    pub async fn get_max_leverage(&self, symbol: &str) -> Result<(i32, i32)> {
        #[derive(Serialize)]
        struct Request<'a> {
            #[serde(rename = "productGroup")]
            product_group: &'a str,
            #[serde(rename = "instrumentID")]
            instrument_id: &'a str,
            asset: &'a str,
        }

        // 调用 sendQryAll API
        let resp: LbankResponse<AggregateInfoResponse> = self.post("/cfd/agg/v1/sendQryAll", &Request {
            product_group: "SwapU",
            instrument_id: symbol,
            asset: "USDT",
        }).await?;

        // 从响应中提取最大杠杆
        if let Some(data) = resp.data {
            let long_max = data.long_max_leverage.unwrap_or(125);
            let short_max = data.short_max_leverage.unwrap_or(125);
            Ok((long_max, short_max))
        } else {
            // 默认值
            Ok((125, 125))
        }
    }

    /// 初始化杠杆设置 - 获取最大杠杆并设置
    /// 返回实际设置的杠杆值
    pub async fn init_leverage(&self, symbol: &str, requested_leverage: i32) -> Result<i32> {
        // 先获取该币种的最大可用杠杆
        let (long_max, short_max) = self.get_max_leverage(symbol).await?;
        let max_allowed = long_max.min(short_max);

        // 取较小值作为实际设置的杠杆
        let actual_leverage = requested_leverage.min(max_allowed);

        tracing::info!(
            "Initializing leverage for {}: requested={}, max_allowed={}, actual={}",
            symbol, requested_leverage, max_allowed, actual_leverage
        );

        // 设置杠杆
        self.set_leverage(symbol, actual_leverage).await?;

        Ok(actual_leverage)
    }

    /// 止盈止损下单 - 文档4.1
    pub async fn place_stop_order(
        &self,
        symbol: &str,
        direction: TradeDirection,
        volume: Decimal,
        price: Decimal,
        sl_trigger_price: &str,
        tp_trigger_price: &str,
        trigger_order_type: TriggerOrderType,
    ) -> Result<OrderInsertResponse> {
        let req = CloseOrderInsertRequest::new(
            symbol,
            direction,
            volume.to_string().parse().unwrap_or(0.0),
            price.to_string().parse().unwrap_or(0.0),
            sl_trigger_price,
            tp_trigger_price,
            trigger_order_type,
        );

        self.post("/cfd/action/v1.0/SendCloseOrderInsert", &req).await
    }

    /// 撤单 - 文档4.4
    pub async fn cancel_order(&self, order_sys_id: &str) -> Result<CancelOrderResponse> {
        #[derive(Serialize)]
        struct Request {
            #[serde(rename = "OrderSysID")]
            order_sys_id: String,
            #[serde(rename = "ActionFlag")]
            action_flag: String,
        }

        self.post("/cfd/action/v1.0/SendOrderAction", &Request {
            order_sys_id: order_sys_id.to_string(),
            action_flag: "1".to_string(),
        }).await
    }

    /// 查询当前持仓 - 文档5.1
    pub async fn query_positions(&self) -> Result<Vec<PositionResponse>> {
        #[derive(Deserialize)]
        struct PositionListResponse {
            data: Vec<PositionResponse>,
        }
        
        let resp: LbankResponse<PositionListResponse> = self.get(
            "/cfd/query/v1.0/Position",
            Some(&[
                ("ProductGroup", "SwapU"),
                ("Valid", "1"),
                ("pageIndex", "1"),
                ("pageSize", "1000"),
            ]),
        ).await?;
        
        Ok(resp.into_result()?.data)
    }

    /// 查询触发单 - 文档4.3
    pub async fn query_trigger_orders(&self) -> Result<Vec<TriggerOrderResponse>> {
        self.get(
            "/cfd/query/v1.0/TriggerOrder",
            Some(&[
                ("ProductGroup", "SwapU"),
                ("ExchangeID", "Exchange"),
                ("pageIndex", "1"),
                ("pageSize", "1000"),
                ("TriggerOrderType", "12"),  // 查询所有
            ]),
        ).await
    }

    /// 查询历史委托 - 文档5.3
    pub async fn query_history_orders(
        &self,
        start_time: i64,
        end_time: i64,
    ) -> Result<Vec<HistoryOrderResponse>> {
        self.get(
            "/cfd/order/v1/historyAllOrderPage",
            Some(&[
                ("pageNo", "1"),
                ("pageSize", "100"),
                ("fakeOnePage", "1"),
                ("startTime", &start_time.to_string()),
                ("endTime", &end_time.to_string()),
                ("orderBusType", "4"),
            ]),
        ).await
    }

    /// 查询历史成交 - 文档5.2
    pub async fn query_history_trades(
        &self,
        start_time: i64,
        end_time: i64,
    ) -> Result<Vec<TradeResponse>> {
        self.get(
            "/cfd/query/v1.0/Trade",
            Some(&[
                ("ProductGroup", "SwapU"),
                ("orderType", "price"),
                ("pageNo", "1"),
                ("pageSize", "100"),
                ("fakeOnePage", "1"),
                ("startTime", &start_time.to_string()),
                ("endTime", &end_time.to_string()),
            ]),
        ).await
    }

    /// 查询聚合信息(余额) - 文档3.2
    pub async fn query_aggregate_info(&self, symbol: &str) -> Result<AggregateInfoResponse> {
        #[derive(Serialize)]
        struct Request<'a> {
            #[serde(rename = "productGroup")]
            product_group: &'a str,
            #[serde(rename = "instrumentID")]
            instrument_id: &'a str,
            asset: &'a str,
        }

        self.post("/cfd/agg/v1/sendQryAll", &Request {
            product_group: "SwapU",
            instrument_id: symbol,
            asset: "USDT",
        }).await
    }

    /// 查询当前订单
    pub async fn query_orders(&self, symbol: Option<&str>) -> Result<Vec<OrderResponse>> {
        #[derive(Deserialize)]
        struct OrderListResponse {
            data: Vec<OrderResponse>,
        }
        
        let mut params = vec![
            ("ProductGroup", "SwapU"),
            ("ExchangeID", "Exchange"),
            ("pageIndex", "1"),
            ("pageSize", "1000"),
        ];

        if let Some(s) = symbol {
            params.push(("InstrumentID", s));
        }

        let resp: LbankResponse<OrderListResponse> = self.get("/cfd/query/v1.0/Order", Some(&params)).await?;
        Ok(resp.into_result()?.data)
    }

    // ========================================================================
    // Account APIs
    // ========================================================================

    /// 查询账户余额 - 文档3.2 (sendQryAll)
    /// 传入 symbol="BTCUSDT" 获取 USDT 余额
    pub async fn get_account_balance(&self, symbol: &str) -> Result<AggregateInfoResponse> {
        #[derive(Serialize)]
        struct Request<'a> {
            #[serde(rename = "productGroup")]
            product_group: &'a str,
            #[serde(rename = "instrumentID")]
            instrument_id: &'a str,
            asset: &'a str,
        }

        self.post("/cfd/agg/v1/sendQryAll", &Request {
            product_group: "SwapU",
            instrument_id: symbol,
            asset: "USDT",
        }).await
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
