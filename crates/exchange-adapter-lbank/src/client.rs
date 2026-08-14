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

        debug!(url = %url, body = %body_str, "POST request");
        // 额外用 INFO 输出，作为 payload summary，方便 grep / 不依赖 tracing 配置
        tracing::info!(target: "lbank_api_payload", path = %path, body = %body_str, "▶ Lbank POST payload");

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

        let parsed: LbankResponse<R> = match serde_json::from_str(&body_text) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(target: "lbank_parse_error", "JSON parse failed: {} at line {} col {}", e, e.line(), e.column());
                // 打印完整的类型名，帮助定位
                tracing::error!(target: "lbank_parse_error", "Trying to parse into type: {}", std::any::type_name::<R>());
                // 尝试只用 data 部分解析，看能不能过
                let just_data = serde_json::from_str::<serde_json::Value>(&body_text)
                    .ok()
                    .and_then(|v| v.get("data").cloned())
                    .map(|d| serde_json::to_string_pretty(&d).unwrap_or_default())
                    .unwrap_or_default();
                tracing::error!(target: "lbank_parse_error", "data field raw: {}", just_data);
                return Err(anyhow::anyhow!("JSON parse failed: {}", e)).context("Failed to parse response");
            }
        };

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

        let parsed: LbankResponse<R> = match serde_json::from_str(&body_text) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(target: "lbank_parse_error", "JSON parse failed: {} at line {} col {}", e, e.line(), e.column());
                // 打印完整的类型名，帮助定位
                tracing::error!(target: "lbank_parse_error", "Trying to parse into type: {}", std::any::type_name::<R>());
                // 尝试只用 data 部分解析，看能不能过
                let just_data = serde_json::from_str::<serde_json::Value>(&body_text)
                    .ok()
                    .and_then(|v| v.get("data").cloned())
                    .map(|d| serde_json::to_string_pretty(&d).unwrap_or_default())
                    .unwrap_or_default();
                tracing::error!(target: "lbank_parse_error", "data field raw: {}", just_data);
                return Err(anyhow::anyhow!("JSON parse failed: {}", e)).context("Failed to parse response");
            }
        };

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
    ///
    /// 该接口返回的是一个**已知问题接口**：
    /// - 实际响应里 Direction 全部为 "1" 且按价格升序排列
    /// - 注释 "0"=Ask / "1"=Bid **与实测不符**
    ///
    /// 为了不阻塞上层逻辑，本方法返回原始 `MarketOrderItem` 列表，
    /// **调用方必须自己根据实际数据决定哪条是 best bid / best ask**。
    /// 建议：先按 Price 排序，价格最低 = best ask，价格最高 = best bid。
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

        let req = Request {
            product_group: "SwapU",
            exchange_id: "Exchange",
            instrument_id: symbol,
            depth,
        };
        let body_str = serde_json::to_string(&req)?;
        let path = "/cfd/market/v1.0/SendQryMarketOrder";

        // 复用 post 但解析逻辑分开处理 - 因为该接口返回值顶层是数组不是 LbankResponse
        let url = format!("{}{}", self.base_url, path);
        debug!(url = %url, body = %body_str, "POST request (market order)");

        let headers = self.signer.get_headers("POST", path);
        let response = self
            .client
            .post(&url)
            .headers(headers.into_reqwest_headers())
            .json(&req)
            .send()
            .await
            .context("Request failed")?;

        let status = response.status();
        let body_text = response.text().await.context("Failed to read response")?;
        debug!(status = %status, "market order body_len={}", body_text.len());

        if !status.is_success() {
            anyhow::bail!("API request failed: {} - {}", status, body_text);
        }

        // 解析顶层是 `{"code":200,"data":[{...},{...}]}` 还是直接数组
        let parsed: LbankResponse<Vec<MarketOrderResponse>> =
            serde_json::from_str(&body_text).context("Failed to parse market order response")?;

        if !parsed.is_success() {
            anyhow::bail!("Market order API error: code={}, msg={:?}", parsed.code, parsed.msg);
        }

        let items = parsed.data.unwrap_or_default();

        // 详尽 debug log，记录每条数据的 direction 和价格
        for (idx, item) in items.iter().take(5).enumerate() {
            debug!(
                idx = idx,
                direction = %item.data.direction,
                price = %item.data.price,
                volume = %item.data.volume,
                orders = %item.data.orders,
                "market order item (first 5)"
            );
        }
        debug!(
            total_items = items.len(),
            unique_directions = ?items.iter().map(|i| i.data.direction.clone()).collect::<std::collections::HashSet<_>>(),
            first_price = ?items.iter().next().map(|i| i.data.price.clone()),
            last_price = ?items.iter().last().map(|i| i.data.price.clone()),
            "market order summary"
        );

        Ok(items.into_iter().map(|r| r.data).collect())
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
            volume.normalize().to_string(),
        );
        self.post("/cfd/cff/v1/SendOrderInsert", &req).await
    }

    /// 市价平仓 - 文档3.5
    ///
    /// ⚠️ **实测修正**（来自 `订单逆向.md:97-188` 与 test_order_result.txt）：
    /// Lbank 平仓 direction 与持仓方向 **相反**（不是相同！）
    /// - 平多仓 (posiDirection=0) → Direction="1"  ← 反方向
    /// - 平空仓 (posiDirection=1) → Direction="0"  ← 反方向
    /// - OffsetFlag="5" (平)
    ///
    /// 注：client API 入参 `direction` 的语义仍是 "持仓方向"（让调用方按持仓理解），
    /// 内部 `new_market_close` 已经会把 direction 取反后传给服务端。
    pub async fn market_close(
        &self,
        symbol: &str,
        direction: TradeDirection,
        volume: Decimal,
        trade_unit_id: &str,
    ) -> Result<OrderInsertResponse> {
        // 平仓 direction = 持仓方向的反向 (经实测修正)
        let opposite = match direction {
            TradeDirection::Long => TradeDirection::Short,
            TradeDirection::Short => TradeDirection::Long,
        };
        let req = OrderInsertRequest::new_market_close(
            symbol,
            opposite,
            volume.normalize().to_string(),
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
            volume.normalize().to_string(),
            price.normalize().to_string(),
        );
        self.post("/cfd/cff/v1/SendOrderInsert", &req).await
    }

    /// 限价平仓
    ///
    /// ⚠️ **实测修正**（来自 `订单逆向.md:627-826`）：Lbank 限价平仓 direction 与持仓方向 **相反**。
    /// 调用方传 `TradeDirection::Long` 表示平**多**仓 (内部 Direction="1")，
    /// 传 `TradeDirection::Short` 表示平**空**仓 (内部 Direction="0")。
    /// - 平多仓 (posiDirection=0) → Direction="1"
    /// - 平空仓 (posiDirection=1) → Direction="0"
    /// - OffsetFlag="1" (限价平仓，与市价平 OffsetFlag=5 不同!)
    pub async fn limit_close(
        &self,
        symbol: &str,
        direction: TradeDirection,
        volume: Decimal,
        price: Decimal,
        trade_unit_id: &str,
    ) -> Result<OrderInsertResponse> {
        // 平仓 direction = 持仓方向的反向 (经实测修正)
        let opposite = match direction {
            TradeDirection::Long => TradeDirection::Short,
            TradeDirection::Short => TradeDirection::Long,
        };
        let req = OrderInsertRequest::new_limit_close(
            symbol,
            opposite,
            volume.normalize().to_string(),
            price.normalize().to_string(),
            trade_unit_id,
        );
        self.post("/cfd/cff/v1/SendOrderInsert", &req).await
    }

    /// **市价平仓 (Raw)** —— direction 直接按传入值发送，**不取反**。
    ///
    /// 用于 `op_market_close_with_fallback` 的回退尝试。
    /// 一般不要直接调用，除非明确知道为什么要这么做。
    pub async fn post_raw_market_close(
        &self,
        symbol: &str,
        direction: TradeDirection,
        volume: Decimal,
        trade_unit_id: &str,
    ) -> Result<OrderInsertResponse> {
        let req = OrderInsertRequest::new_market_close(
            symbol,
            direction, // raw，client 不取反
            volume.normalize().to_string(),
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

    /// 查询杠杆 - 文档对应 SendQryLeverage
    /// POST /cfd/action/v1.0/SendQryLeverage
    /// payload: {InstrumentID: "BTCUSDT", ExchangeID: "Exchange"}
    /// 响应: {"code":200,"data":[{"data":{...}}],"message":"成功"}
    /// 返回 (long_leverage, short_leverage, long_max_leverage, short_max_leverage)
    pub async fn get_leverage_info(&self, symbol: &str) -> Result<LeverageInfo> {
        #[derive(Serialize)]
        struct Request {
            #[serde(rename = "InstrumentID")]
            instrument_id: String,
            #[serde(rename = "ExchangeID")]
            exchange_id: String,
        }

        #[derive(Deserialize)]
        struct InnerData {
            #[serde(rename = "data")]
            data: serde_json::Value, // 改为 Value 以隔离问题
        }

        // 响应是 data: [{data: {...}}] - 数组包装，内层 data 才是 LeverageInfo
        let resp: LbankResponse<Vec<InnerData>> = self.post("/cfd/action/v1.0/SendQryLeverage", &Request {
            instrument_id: symbol.to_string(),
            exchange_id: "Exchange".to_string(),
        }).await?;

        // 直接拿原始 Value 然后手动解析 LeverageInfo
        let raw_value = resp
            .data
            .and_then(|arr| arr.into_iter().next())
            .map(|d| d.data)
            .ok_or_else(|| anyhow::anyhow!("No leverage data returned"))?;

        // 逐字段打印原始类型，跟 LeverageInfo 字段对比
        eprintln!("\n[LEVERAGE RAW FIELDS]");
        if let Some(obj) = raw_value.as_object() {
            for (k, v) in obj.iter() {
                let t = match v {
                    serde_json::Value::String(_) => "String",
                    serde_json::Value::Number(n) => if n.is_i64() { "i64" } else if n.is_f64() { "f64" } else { "num" },
                    serde_json::Value::Bool(_) => "bool",
                    serde_json::Value::Array(_) => "Array",
                    serde_json::Value::Object(_) => "Object",
                    serde_json::Value::Null => "Null",
                };
                eprintln!("  {}: type={} value={}", k, t, v);
            }
        }
        eprintln!("[/LEVERAGE RAW FIELDS]\n");

        let info: LeverageInfo = serde_json::from_value(raw_value)
            .map_err(|e| {
                eprintln!("[LEVERAGE PARSE FAILED] {} (line {} col {})", e, e.line(), e.column());
                anyhow::anyhow!("LeverageInfo parse failed: {}", e)
            })?;

        Ok(info)
    }

    /// 获取当前杠杆设置 (简洁版)
    pub async fn get_leverage(&self, symbol: &str) -> Result<(i64, i64)> {
        let info = self.get_leverage_info(symbol).await?;
        Ok((info.long_leverage, info.short_leverage))
    }

    /// 获取最大杠杆 (简洁版)
    pub async fn get_max_leverage(&self, symbol: &str) -> Result<(i64, i64)> {
        let info = self.get_leverage_info(symbol).await?;
        Ok((info.long_max_leverage, info.short_max_leverage))
    }

    /// 初始化杠杆设置 - 获取最大杠杆并设置
    /// 返回实际设置的杠杆值
    pub async fn init_leverage(&self, symbol: &str, requested_leverage: i32) -> Result<i32> {
        let (long_max, short_max) = self.get_max_leverage(symbol).await?;
        let max_allowed = long_max.min(short_max);
        let actual_leverage = requested_leverage.min(max_allowed as i32);

        tracing::info!(
            "Initializing leverage for {}: requested={}, max_allowed={}, actual={}",
            symbol, requested_leverage, max_allowed, actual_leverage
        );

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
            trigger_order_type.as_str(),
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
        let body_text = self.get_raw("/cfd/query/v1.0/Position", Some(&[
            ("ProductGroup", "SwapU"),
            ("Valid", "1"),
            ("pageIndex", "1"),
            ("pageSize", "1000"),
        ])).await?;
        
        // 解析 JSON，处理两种可能的格式:
        // 1. {"code":200,"data":{"data":[...]}} - 嵌套格式
        // 2. {"code":200,"data":[...]} - 直接数组格式
        let json: serde_json::Value = serde_json::from_str(&body_text)?;
        
        let code = json.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
        if code != 200 {
            let msg = json.get("message").and_then(|v| v.as_str()).unwrap_or("Unknown error");
            anyhow::bail!("API error: {} - {}", code, msg);
        }
        
        // 提取 data 字段
        let data_array = if let Some(arr) = json.get("data").and_then(|d| d.get("data")).and_then(|d| d.as_array()) {
            arr.clone()
        } else if let Some(arr) = json.get("data").and_then(|d| d.as_array()) {
            arr.clone()
        } else {
            // data 为 null 或空对象，返回空数组
            return Ok(Vec::new());
        };
        
        // 反序列化持仓列表
        let positions: Vec<PositionResponse> = serde_json::from_value(serde_json::Value::Array(data_array))?;
        Ok(positions)
    }
    
    /// 获取原始响应文本 (pub 供调试/自定义解析使用)
    pub async fn get_raw(&self, path: &str, query: Option<&[(&str, &str)]>) -> Result<String> {


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

        let response = self.client.get(&url)
            .headers(headers.into_reqwest_headers())
            .send().await
            .context("Request failed")?;

        let status = response.status();
        let body_text = response.text().await.context("Failed to read response")?;

        debug!(status = %status, body = %body_text);

        if !status.is_success() {
            error!(status = %status, body = %body_text, "Request failed");
            anyhow::bail!("API request failed: {} - {}", status, body_text);
        }

        Ok(body_text)
    }

    /// 查询触发单 - 文档4.3 (默认查所有类型)
    pub async fn query_trigger_orders(&self) -> Result<Vec<TriggerOrderResponse>> {
        self.get(
            "/cfd/query/v1.0/TriggerOrder",
            Some(&[
                ("ProductGroup", "SwapU"),
                ("ExchangeID", "Exchange"),
                ("pageIndex", "1"),
                ("pageSize", "1000"),
                ("TriggerOrderType", "12"), // 查询所有
            ]),
        ).await
    }

    /// 查询触发单 (支持指定 TriggerOrderType)
    /// type="1"=持仓止盈止损, "2"=订单止盈止损, "3"=计划委托
    /// "12"=所有类型
    pub async fn query_trigger_orders_typed(&self, trigger_order_type: &str) -> Result<Vec<TriggerOrderResponse>> {
        self.get(
            "/cfd/query/v1.0/TriggerOrder",
            Some(&[
                ("ProductGroup", "SwapU"),
                ("ExchangeID", "Exchange"),
                ("pageIndex", "1"),
                ("pageSize", "1000"),
                ("TriggerOrderType", trigger_order_type),
            ]),
        ).await
    }

    /// 取消触发单 - 文档: SendTriggerOrderAction
    /// ActionFlag="1" = 撤销
    pub async fn cancel_trigger_order(&self, order_sys_id: &str) -> Result<TriggerOrderActionResponse> {
        let req = TriggerOrderActionRequest {
            order_sys_id: order_sys_id.to_string(),
            action_flag: "1".to_string(),
        };
        self.post("/cfd/action/v1.0/SendTriggerOrderAction", &req).await
    }

    /// 给已有仓位添加止盈止损 - 文档: SendTriggerOrderInsert
    /// TriggerOrderType="1" (持仓止盈止损)
    /// direction 应为持仓方向 (0=多, 1=空), offset_flag="8"
    pub async fn add_position_tpsl(
        &self,
        symbol: &str,
        trade_unit_id: &str,
        posi_direction: TradeDirection,
        volume: f64,
        sl_trigger_price: f64,
        tp_trigger_price: f64,
    ) -> Result<TriggerOrderActionResponse> {
        let req = AddPositionTPSLRequest::new(
            symbol,
            trade_unit_id,
            posi_direction,
            volume,
            sl_trigger_price,
            tp_trigger_price,
        );
        self.post("/cfd/cff/v1/SendTriggerOrderInsert", &req).await
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

    /// 查询当前**所有挂单** (不限 symbol)
    ///
    /// ⚠️ **实测修正**（来自 `限价单挂单.md:121-274`）：**不要**在 URL 里加
    /// `InstrumentID=<symbol>` 过滤条件！Lbank 后端在带 InstrumentID 时**只返回
    /// 已成交订单 (status=1, status=5 等)**，而**不会返回 status=4 的限价挂单**。
    /// 只有不带 InstrumentID 时才能查到 status=4 的限价挂单。
    ///
    /// 如果需要过滤某交易对，请在客户端侧 `Vec<OrderResponse>` 上 filter：
    /// ```ignore
    /// let orders = client.query_orders().await?;
    /// let btc: Vec<_> = orders.iter().filter(|o| o.instrument_id == "BTCUSDT").collect();
    /// ```
    ///
    /// 返回的订单状态：
    /// - `orderStatus="1"` 已成交 (limit order 全部或部分成交)
    /// - `orderStatus="4"` 已挂单 (active limit order) ← 我们关心的
    /// - `orderStatus="5"` 部分成交
    /// - `orderStatus="6"` 已撤单
    /// - `orderStatus="0"` 全部撤销
    pub async fn query_orders(&self) -> Result<Vec<OrderResponse>> {
        let params: &[(&str, &str)] = &[
            ("ProductGroup", "SwapU"),
            ("ExchangeID", "Exchange"),
            ("pageIndex", "1"),
            ("pageSize", "1000"),
        ];

        let body_text = self.get_raw("/cfd/query/v1.0/Order", Some(params)).await?;

        // 解析 JSON，处理两种可能的格式
        let json: serde_json::Value = serde_json::from_str(&body_text)?;

        let code = json.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
        if code != 200 {
            let msg = json.get("message").and_then(|v| v.as_str()).unwrap_or("Unknown error");
            anyhow::bail!("API error: {} - {}", code, msg);
        }

        // 提取 data 字段
        let data_array = if let Some(arr) = json.get("data").and_then(|d| d.get("data")).and_then(|d| d.as_array()) {
            arr.clone()
        } else if let Some(arr) = json.get("data").and_then(|d| d.as_array()) {
            arr.clone()
        } else {
            return Ok(Vec::new());
        };

        let orders: Vec<OrderResponse> = serde_json::from_value(serde_json::Value::Array(data_array))?;
        Ok(orders)
    }

    /// 查询某 symbol 的挂单 (客户端侧 filter)
    ///
    /// 内部调用 `query_orders()` 然后按 `instrument_id` 过滤。
    /// Lbank 的 Order 接口不支持 server-side InstrumentID 过滤 (会漏掉挂单)。
    pub async fn query_orders_for_symbol(&self, symbol: &str) -> Result<Vec<OrderResponse>> {
        let all = self.query_orders().await?;
        Ok(all
            .into_iter()
            .filter(|o| o.instrument_id == symbol)
            .collect())
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
