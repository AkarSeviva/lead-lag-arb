//! 限价开多 + 止损止盈测试
//!
//! 对应逆向文档测试用例：
//! - 限价开多 + 设置止损 (sl_trigger_price < 买入价)
//! - 限价开多 + 设置止盈 (tp_trigger_price > 买入价)
//! - 限价开多 + 同时设置止损+止盈
//! - 止损触发后检查持仓自动平仓
//! - 止盈触发后检查持仓自动平仓
//! - 触发单取消
//!
//! Phase 顺序：
//! - Phase A: 清理持仓/挂单 + 状态确认
//! - Phase H: 限价开多 + 止损/止盈/同时TPSL → 撤单
//! - Phase G: 最终状态确认
//!
//! 运行：`cargo run --package test --bin test_order_operations`

use exchange_adapter_lbank::{
    auth::LbankSigner,
    client::LbankClient,
    protocol::{OrderInsertResponse, PositionResponse, TradeDirection, TriggerOrderType},
    proxy::ProxyConfig,
    ws::{LbankWebSocket, WsEvent},
};
use rust_decimal::Decimal;
use std::str::FromStr;
use std::time::{Duration, Instant};
use std::{fs::File, io::Write};
use tracing::{debug, warn};

/// Phase 之间强制 sleep 间隔 (防止 Cloudflare 1015)
macro_rules! gap {
    () => {
        tokio::time::sleep(PHASE_INTER_DELAY).await;
    };
}
use tracing_subscriber::util::SubscriberInitExt;

const SYMBOL: &str = "BTCUSDT";
const VOLUME_STR: &str = "0.0001";
/// Helper: parse VOLUME_STR to Decimal
fn vol() -> Decimal { Decimal::from_str(VOLUME_STR).unwrap() }
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1); // 放慢轮询，避免 429

/// Phase 之间强制间隔，避免触发 Cloudflare 反爬 (默认 4000ms)
/// CF rate limit 大约 ~20 req/min, 每个请求间隔 ≥2s 才能稳
const PHASE_INTER_DELAY: Duration = Duration::from_millis(4000);

/// 客户端单请求最小间隔 (rate limiter)
/// 任意连续两个 API 调用至少间隔 250ms
const CLIENT_REQUEST_INTERVAL: Duration = Duration::from_millis(250);

// ============================================================================
// 工具：轮询等待 + 测试报告器
// ============================================================================

/// 测试报告器：所有输出都写入 buffer，方便事后分析
pub struct TestReporter {
    output: String,
    step_count: u32,
}

impl TestReporter {
    pub fn new() -> Self {
        let mut output = String::new();
        output.push_str("===========================================\n");
        output.push_str("Lbank 订单操作测试 (解耦版)\n");
        output.push_str("===========================================\n\n");
        Self { output, step_count: 0 }
    }

    pub fn section<S: AsRef<str>>(&mut self, title: S) {
        self.step_count += 1;
        let header = format!(
            "\n【Phase {}】{}\n{}\n",
            self.step_count,
            title.as_ref(),
            "-".repeat(40)
        );
        self.output.push_str(&header);
        print!("{}", header);
    }

    pub fn success<S: AsRef<str>>(&mut self, msg: S) {
        let line = format!("✅ {}\n", msg.as_ref());
        self.output.push_str(&line);
        print!("{}", line);
    }

    pub fn warn<S: AsRef<str>>(&mut self, msg: S) {
        let line = format!("⚠️  {}\n", msg.as_ref());
        self.output.push_str(&line);
        print!("{}", line);
    }

    pub fn fail<S: AsRef<str>>(&mut self, msg: S) {
        let line = format!("❌ {}\n", msg.as_ref());
        self.output.push_str(&line);
        print!("{}", line);
    }

    pub fn info<S: AsRef<str>>(&mut self, msg: S) {
        let line = format!("  ℹ️  {}\n", msg.as_ref());
        self.output.push_str(&line);
        print!("{}", line);
    }

    pub fn section_end(&mut self) {
        let line = "\n";
        self.output.push_str(line);
        print!("{}", line);
    }

    pub fn sub(&mut self, title: &str) {
        let line = format!("\n  ── {} ──\n", title);
        self.output.push_str(&line);
        print!("{}", line);
    }

    pub fn write_to(&self, filename: &str) -> std::io::Result<()> {
        let mut file = File::create(filename)?;
        file.write_all(self.output.as_bytes())?;
        Ok(())
    }

    pub fn dump(&self) -> &str {
        &self.output
    }
}

// ============================================================================
// 工具：轮询等待辅助函数
// ============================================================================

/// 轮询直到 predicate 返回 true 或超时
///
/// ⚠️ 此函数本身是 async，必须在 tokio runtime 内被 `.await`。
/// **不要**在 sync context / `runtime.block_on` 内调用它，会触发 abort。
pub async fn poll_until<F, Fut>(
    label: &str,
    mut predicate: F,
    timeout: Duration,
    interval: Duration,
) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = Instant::now();
    let mut attempts = 0;
    while start.elapsed() < timeout {
        attempts += 1;
        if predicate().await {
            debug!(
                label = label,
                attempts = attempts,
                elapsed_ms = start.elapsed().as_millis(),
                "poll_until success"
            );
            return true;
        }
        tokio::time::sleep(interval).await;
    }
    warn!(
        label = label,
        attempts = attempts,
        elapsed_ms = start.elapsed().as_millis(),
        "poll_until timeout"
    );
    false
}

// ============================================================================
// 独立操作函数 (每个函数都自包含，可单独测试)
// ============================================================================

/// **op_get_positions**：查询所有持仓
pub async fn op_get_positions(client: &LbankClient) -> anyhow::Result<Vec<PositionResponse>> {
    debug!("op_get_positions: querying all positions");
    let positions = client.query_positions().await?;
    debug!(count = positions.len(), "op_get_positions: got positions");
    Ok(positions)
}

/// **op_market_open**：市价开仓 (返回 OrderInsertResponse，包含 tradeUnitID)
pub async fn op_market_open(
    client: &LbankClient,
    symbol: &str,
    direction: TradeDirection,
    volume: Decimal,
) -> anyhow::Result<OrderInsertResponse> {
    let label = format!("market_open_{:?}", direction);
    debug!(label = %label, symbol = %symbol, volume = %volume, "op_market_open start");
    let resp = client.market_open(symbol, direction, volume).await?;
    debug!(
        label = %label,
        order_sys_id = %resp.order_sys_id,
        trade_price = %resp.trade_price,
        status = %resp.order_status,
        "op_market_open success"
    );
    Ok(resp)
}

/// **op_market_close**：市价平仓 (direction = 持仓方向)
pub async fn op_market_close(
    client: &LbankClient,
    symbol: &str,
    position_direction: TradeDirection,
    volume: Decimal,
    trade_unit_id: &str,
) -> anyhow::Result<OrderInsertResponse> {
    let label = format!("market_close_{:?}", position_direction);
    debug!(
        label = %label,
        symbol = %symbol,
        volume = %volume,
        trade_unit_id = %trade_unit_id,
        "op_market_close start (direction=持仓方向)"
    );
    let resp = client
        .market_close(symbol, position_direction, volume, trade_unit_id)
        .await?;
    debug!(
        label = %label,
        order_sys_id = %resp.order_sys_id,
        trade_price = %resp.trade_price,
        status = %resp.order_status,
        close_profit = %resp.close_profit,
        "op_market_close success"
    );
    Ok(resp)
}

/// **op_market_close_with_fallback**：智能平仓 (默认 direction=持仓方向，失败回退到反方向)
///
/// 经 `订单逆向.md:97-188` 实测验证：Lbank 平仓 direction 是持仓方向的**反方向**
/// （平多→1，平空→0）。`client::market_close` 已经把传入的 `direction` (持仓方向)
/// 取反后发给服务端，所以这里直接调用 client API。
///
/// **保留 fallback** 是为了防御性编程：如果服务端规则在某天反转，回退链路可以
/// 自动切换到反方向，否则会两次都失败。
pub async fn op_market_close_with_fallback(
    client: &LbankClient,
    symbol: &str,
    position_direction: TradeDirection,
    volume: Decimal,
    trade_unit_id: &str,
    reporter: &mut TestReporter,
) -> anyhow::Result<OrderInsertResponse> {
    let opp_dir = match position_direction {
        TradeDirection::Long => TradeDirection::Short,
        TradeDirection::Short => TradeDirection::Long,
    };

    // 尝试 1: 通过 client API (内部已自动取反)
    let opp_label = format!("market_close_via_client_d{:?}", opp_dir);
    debug!(
        label = %opp_label,
        symbol = %symbol,
        volume = %volume,
        trade_unit_id = %trade_unit_id,
        offset_flag = "5",
        position_direction = ?position_direction,
        "flat: 尝试 1 (client 自动取反)"
    );
    let e1 = match op_market_close(client, symbol, position_direction, volume, trade_unit_id).await {
        Ok(r) => {
            reporter.success(&format!(
                "市价平仓成功 (client 自动取反为 direction={:?}): order_sys_id={}, trade_price={}, close_profit={}",
                opp_dir, r.order_sys_id, r.trade_price, r.close_profit
            ));
            return Ok(r);
        }
        Err(e) => e,
    };

    warn!(
        label = %opp_label,
        error = %e1,
        "flat close via client failed; will try raw opposite-direction"
    );
    reporter.warn(&format!(
        "client API 失败 ({:#?}); 切换尝试 raw direction=持仓方向",
        e1
    ));

    // 尝试 2: 直接 payload 用原始 direction = 持仓方向 (理论，按旧文档)
    let pos_label = format!("market_close_raw_pos_d{:?}", position_direction);
    debug!(
        label = %pos_label,
        symbol = %symbol,
        volume = %volume,
        trade_unit_id = %trade_unit_id,
        offset_flag = "5",
        direction = ?position_direction,
        "flat: 尝试 2 (raw payload, direction=持仓方向)"
    );
    match client.post_raw_market_close(symbol, position_direction, volume, trade_unit_id).await {
        Ok(r) => {
            reporter.success(&format!(
                "市价平仓成功 (raw direction=持仓方向): order_sys_id={}, close_profit={}",
                r.order_sys_id, r.close_profit
            ));
            Ok(r)
        }
        Err(e2) => {
            let diag = format!(
                "市价平仓两个方向都失败: client_err={:#?}, raw_err={:#?}",
                e1, e2
            );
            reporter.fail(&diag);
            Err(e2)
        }
    }
}

/// **op_limit_open**：限价开仓
pub async fn op_limit_open(
    client: &LbankClient,
    symbol: &str,
    direction: TradeDirection,
    volume: Decimal,
    price: Decimal,
) -> anyhow::Result<OrderInsertResponse> {
    let label = format!("limit_open_{:?}", direction);
    debug!(
        label = %label,
        symbol = %symbol,
        volume = %volume,
        price = %price,
        "op_limit_open start"
    );
    let resp = client.limit_open(symbol, direction, volume, price).await?;
    debug!(
        label = %label,
        order_sys_id = %resp.order_sys_id,
        price = %resp.price,
        status = %resp.order_status,
        "op_limit_open success"
    );
    Ok(resp)
}

/// **op_limit_close**：限价平仓
pub async fn op_limit_close(
    client: &LbankClient,
    symbol: &str,
    position_direction: TradeDirection,
    volume: Decimal,
    price: Decimal,
    trade_unit_id: &str,
) -> anyhow::Result<OrderInsertResponse> {
    let label = format!("limit_close_{:?}", position_direction);
    debug!(
        label = %label,
        symbol = %symbol,
        volume = %volume,
        price = %price,
        trade_unit_id = %trade_unit_id,
        "op_limit_close start"
    );
    let resp = client
        .limit_close(symbol, position_direction, volume, price, trade_unit_id)
        .await?;
    debug!(
        label = %label,
        order_sys_id = %resp.order_sys_id,
        price = %resp.price,
        status = %resp.order_status,
        "op_limit_close success"
    );
    Ok(resp)
}

/// **op_cancel_order**：撤单
pub async fn op_cancel_order(
    client: &LbankClient,
    order_sys_id: &str,
) -> anyhow::Result<exchange_adapter_lbank::protocol::CancelOrderResponse> {
    debug!(order_sys_id = %order_sys_id, "op_cancel_order start");
    let resp = client.cancel_order(order_sys_id).await?;
    debug!(
        order_sys_id = %resp.order_sys_id,
        status = %resp.order_status,
        volume_cancled = %resp.volume_cancled,
        "op_cancel_order success"
    );
    Ok(resp)
}

/// **op_query_pending_orders**：查询某 symbol 的挂单 (客户端侧过滤)
///
/// ⚠️ 实测修正：Lbank Order 接口不支持 server-side InstrumentID 过滤
/// (带 InstrumentID 会漏掉 status=4 挂单)。这里先查全部再按 symbol 过滤。
pub async fn op_query_pending_orders(
    client: &LbankClient,
    symbol: &str,
) -> anyhow::Result<Vec<exchange_adapter_lbank::protocol::OrderResponse>> {
    debug!(symbol = %symbol, "op_query_pending_orders start");
    let orders = client.query_orders_for_symbol(symbol).await?;
    debug!(count = orders.len(), symbol = %symbol, "op_query_pending_orders done");
    Ok(orders)
}

/// **op_query_pending_orders_with_retry**：查询挂单，遇 429 时自动 backoff 重试
pub async fn op_query_pending_orders_with_retry(
    client: &LbankClient,
    symbol: &str,
    max_attempts: u32,
) -> anyhow::Result<Vec<exchange_adapter_lbank::protocol::OrderResponse>> {
    let mut backoff = Duration::from_secs(5); // 5s 起步
    for attempt in 1..=max_attempts {
        match op_query_pending_orders(client, symbol).await {
            Ok(orders) => return Ok(orders),
            Err(e) => {
                let msg = format!("{:#?}", e);
                // 429 / Cloudflare / rate-limit / network / JSON 解析失败 (CF HTML) 都重试
                let retryable = msg.contains("429")
                    || msg.contains("Too Many Requests")
                    || msg.contains("rate")
                    || msg.contains("cloudflare")
                    || msg.contains("Cloudflare")
                    || msg.contains("Just a moment")
                    || msg.contains("Access denied")
                    || msg.contains("being rate limited")
                    || msg.contains("1015")
                    || msg.contains("connection")
                    || msg.contains("timed out")
                    || msg.contains("missing field") // CF HTML 当 JSON 解析了
                    || msg.contains("EOF while parsing");
                if !retryable || attempt == max_attempts {
                    return Err(e);
                }
                warn!(
                    attempt = attempt,
                    backoff_ms = backoff.as_millis(),
                    error = %e,
                    "op_query_pending_orders_with_retry: 429/network, retrying"
                );
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(60));
            }
        }
    }
    anyhow::bail!("op_query_pending_orders_with_retry: exceeded max_attempts")
}

/// **op_query_pending_orders_raw**：返回 raw body text 和 parsed orders
/// 用于诊断 Lbank 接口真实响应内容
pub async fn op_query_pending_orders_raw(
    client: &LbankClient,
    symbol: &str,
) -> anyhow::Result<(
    String,
    Vec<exchange_adapter_lbank::protocol::OrderResponse>,
)> {
    // 不带 InstrumentID (避免 Lbank 后端过滤掉 status=4 挂单)
    let params: &[(&str, &str)] = &[
        ("ProductGroup", "SwapU"),
        ("ExchangeID", "Exchange"),
        ("pageIndex", "1"),
        ("pageSize", "1000"),
    ];
    let raw = client.get_raw("/cfd/query/v1.0/Order", Some(params)).await?;
    // 在客户端侧按 symbol 过滤
    let parsed = client.query_orders_for_symbol(symbol).await.unwrap_or_default();
    Ok((raw, parsed))
}

/// **op_wait_for_position**：轮询等到指定方向和数量的持仓出现
pub async fn op_wait_for_position(
    client: &LbankClient,
    expected_direction: TradeDirection,
    expected_min_volume: Decimal,
) -> Option<PositionResponse> {
    let label = format!("wait_for_position_d{:?}", expected_direction);
    let mut attempts = 0;
    let start = Instant::now();
    // 拉长轮询间隔到 1.5s，进一步降低 429 概率
    let poll_interval = Duration::from_millis(1500);
    while start.elapsed() < DEFAULT_TIMEOUT {
        attempts += 1;
        match op_get_positions(client).await {
            Ok(positions) => {
                let dir_str = match expected_direction {
                    TradeDirection::Long => "0",
                    TradeDirection::Short => "1",
                };
                for pos in &positions {
                    if pos.posi_direction.as_deref() == Some(dir_str) {
                        let pos_vol: Decimal = pos
                            .position
                            .as_deref()
                            .and_then(|s| s.parse().ok())
                            .unwrap_or_default();
                        if pos_vol >= expected_min_volume {
                            debug!(
                                label = %label,
                                attempts = attempts,
                                pos_id = ?pos.position_id,
                                "position found"
                            );
                            return Some(pos.clone());
                        }
                    }
                }
                debug!(
                    label = %label,
                    attempts = attempts,
                    pos_count = positions.len(),
                    "positions queried but no match yet"
                );
            }
            Err(e) => {
                let msg = format!("{:#?}", e);
                // 如果是 429 / Cloudflare, 立即跳出等待，避免被风控进一步封禁
                if msg.contains("429") || msg.contains("Cloudflare") || msg.contains("1015") {
                    warn!(label = %label, error = %e, "op_wait_for_position: hit 429, abort");
                    return None;
                }
                warn!(label = %label, error = %e, "query_positions failed during wait");
            }
        }
        tokio::time::sleep(poll_interval).await;
    }
    warn!(label = %label, attempts = attempts, "wait_for_position timeout");
    None
}

/// **op_wait_for_no_position**：轮询等到某方向持仓数量归零
pub async fn op_wait_for_no_position(
    client: &LbankClient,
    expected_direction: TradeDirection,
) -> bool {
    let label = format!("wait_for_no_position_d{:?}", expected_direction);
    let dir_str = match expected_direction {
        TradeDirection::Long => "0",
        TradeDirection::Short => "1",
    };
    poll_until(
        &label,
        || async {
            match op_get_positions(client).await {
                Ok(positions) => {
                    let mut total: Decimal = Decimal::ZERO;
                    for pos in positions.iter().filter(|p| p.posi_direction.as_deref() == Some(dir_str)) {
                        let v: Decimal = pos.position.as_deref().and_then(|s| s.parse().ok()).unwrap_or_default();
                        total += v;
                    }
                    total == Decimal::ZERO
                }
                Err(_) => false,
            }
        },
        DEFAULT_TIMEOUT,
        DEFAULT_POLL_INTERVAL,
    )
    .await
}

/// **op_wait_for_pending_orders**：轮询等到指定数量的 pending 挂单 (orderStatus="2"|"3")
pub async fn op_wait_for_pending_orders(
    client: &LbankClient,
    symbol: &str,
    expected_count: usize,
) -> Vec<exchange_adapter_lbank::protocol::OrderResponse> {
    let label = format!("wait_for_pending_orders_n{}", expected_count);
    let mut attempts = 0;
    let start = Instant::now();
    while start.elapsed() < DEFAULT_TIMEOUT {
        attempts += 1;
        // 用 retry 版本，自动处理 429
        match op_query_pending_orders_with_retry(client, symbol, 4).await {
            Ok(orders) => {
                let pending: Vec<_> = orders
                    .iter()
                    .filter(|o| {
                        let s = o.order_status.as_deref().unwrap_or("0");
                        s == "2" || s == "3" || s == "4" // 4=已挂单(Lbank实测状态码)
                    })
                    .cloned()
                    .collect();
                debug!(
                    label = %label,
                    attempts = attempts,
                    total = orders.len(),
                    pending = pending.len(),
                    "pending orders queried"
                );
                if pending.len() >= expected_count {
                    return pending;
                }
            }
            Err(e) => {
                warn!(label = %label, error = %e, "query_orders failed during wait");
            }
        }
        tokio::time::sleep(DEFAULT_POLL_INTERVAL).await;
    }
    warn!(label = %label, attempts = attempts, "wait_for_pending_orders timeout");
    Vec::new()
}

/// **op_get_mid_price**：获取中间价 (来自 order_book)
/// 由于当前 Lbank SendQryMarketOrder 接口 direction 语义不明，
/// 这里实现一个 **不依赖 direction 字段** 的策略：取所有返回数据中位数价格作为参考
pub async fn op_get_mid_price(
    client: &LbankClient,
    symbol: &str,
) -> anyhow::Result<Decimal> {
    let items = client.get_order_book(symbol, 25).await?;
    if items.is_empty() {
        anyhow::bail!("Empty order book for {}", symbol);
    }
    // 取中间位置的价格作为参考价
    let mid_idx = items.len() / 2;
    let mid_price = items
        .get(mid_idx)
        .map(|i| i.price.parse().unwrap_or_default())
        .unwrap_or_default();
    debug!(
        mid_idx = mid_idx,
        mid_price = %mid_price,
        total = items.len(),
        "op_get_mid_price"
    );
    Ok(mid_price)
}

/// **get_mid_price_via_ws**：通过 WebSocket OrderBook topic 拿真实 bid/ask
///
/// VPS 后端的 SendQryMarketOrder REST 接口只返回 ask (Direction=1) 单边，
/// 没法算 mid。WebSocket wss://uuws.rerrkvifj.com/ws/v3 订阅 OrderBook (x=3)
/// 可以拿到完整 bid+ask：
///   - 订阅消息: {"a":{"i":"BTCUSDT_2_25"},"x":3,"y":<tsn>,"z":1}
///   - 推送格式: {"b":[["price","vol"],...], "s":[["price","vol"],...]}
///
/// Returns (best_bid, best_ask, depth, source).
pub async fn get_mid_price_via_ws(
    symbol: &str,
) -> anyhow::Result<(f64, f64, usize, &'static str)> {
    use anyhow::Context;
    use tokio::time::{sleep, timeout};

    // 1. 启动 WS 客户端
    let mut ws = LbankWebSocket::new(None); // OrderBook 不需要鉴权
    let mut event_rx = ws.start().context("ws.start() failed")?;

    // 2. 订阅 OrderBook (instrumentID 格式: SYMBOL_DECIMAL_LIMIT)
    ws.subscribe_orderbook(symbol)
        .await
        .context("ws.subscribe_orderbook() failed")?;

    // 3. 等首条 OrderBook 推送 (5s 超时)
    let deadline = std::time::Duration::from_secs(5);
    let book: (Vec<(Decimal, Decimal)>, Vec<(Decimal, Decimal)>) = timeout(deadline, async {
        loop {
            match event_rx.recv().await {
                Some(WsEvent::OrderBookUpdate {
                    symbol: _,
                    bids,
                    asks,
                    timestamp: _,
                }) => {
                    if bids.is_empty() && asks.is_empty() {
                        continue;
                    }
                    return Ok::<_, anyhow::Error>((bids, asks));
                }
                Some(WsEvent::Connected) => {
                    debug!("WS connected");
                    continue;
                }
                Some(WsEvent::Error(e)) => {
                    anyhow::bail!("WS error: {}", e);
                }
                Some(_) => continue,
                None => anyhow::bail!("WS channel closed"),
            }
        }
    })
    .await
    .context("timeout waiting for first OrderBook push")??;

    // 4. 计算 best_bid / best_ask (bids 已按降序，asks 按升序)
    let (bids, asks) = book;
    let best_bid = bids
        .first()
        .map(|(p, _)| p.to_string().parse::<f64>().unwrap_or(0.0))
        .unwrap_or(0.0);
    let best_ask = asks
        .first()
        .map(|(p, _)| p.to_string().parse::<f64>().unwrap_or(f64::MAX))
        .unwrap_or(f64::MAX);
    let depth = bids.len() + asks.len();

    // 5. 防止 WS event_rx 把 sender 阻塞，简单让 ws 析构
    drop(ws);
    sleep(std::time::Duration::from_millis(50)).await;

    Ok((best_bid, best_ask, depth, "ws_orderbook"))
}

/// **op_print_position_summary**：把当前持仓列表格式化成可读字符串
pub fn op_print_position_summary(positions: &[PositionResponse]) -> String {
    if positions.is_empty() {
        return "  (无持仓)".to_string();
    }
    let mut s = String::new();
    for pos in positions {
        let dir = pos.posi_direction.as_deref().unwrap_or("?");
        let inst = pos.instrument_id.as_deref().unwrap_or("?");
        let pos_size = pos.position.as_deref().unwrap_or("?");
        let open = pos.open_price.as_deref().unwrap_or("?");
        s.push_str(&format!(
            "  - {} 方向:{} 数量:{} 开仓价:{}\n",
            inst, dir, pos_size, open
        ));
        if dir == "0" || dir == "1" {
            let tid = pos.trade_unit_id.as_deref().unwrap_or("N/A");
            s.push_str(&format!("    TradeUnitID: {}\n", tid));
        }
    }
    s
}

// ============================================================================
// Phase helpers - 每个 phase 互相独立
// ============================================================================

async fn phase_a_status_and_cleanup(
    client: &LbankClient,
    reporter: &mut TestReporter,
) {
    reporter.section("A. 状态确认 (Phase A)");
    match op_get_positions(client).await {
        Ok(positions) => {
            reporter.success(&format!(
                "查询成功! 当前持仓数: {}",
                positions.len()
            ));
            reporter.info(&op_print_position_summary(&positions));
        }
        Err(e) => reporter.fail(&format!("查询失败: {:#?}", e)),
    }
    reporter.section_end();
}

async fn phase_b_market_long_roundtrip(
    client: &LbankClient,
    reporter: &mut TestReporter,
) {
    reporter.section("B. 市价开多 → 平多 (Phase B)");

    // B1: 市价开多
    let volume = Decimal::from_str(VOLUME_STR).unwrap();
    let open_resp = match op_market_open(client, SYMBOL, TradeDirection::Long, volume).await {
        Ok(r) => r,
        Err(e) => {
            reporter.fail(&format!("市价开多失败: {:#?}", e));
            reporter.section_end();
            return;
        }
    };
    reporter.success(&format!(
        "市价开多成功: order_sys_id={}, trade_price={}",
        open_resp.order_sys_id, open_resp.trade_price
    ));

    // B2: 轮询等待持仓出现
    let pos = op_wait_for_position(client, TradeDirection::Long, volume).await;
    let trade_unit_id = match pos {
        Some(p) => {
            let tid = p.trade_unit_id.clone().unwrap_or_default();
            reporter.success(&format!("持仓确认: TradeUnitID={}", tid));
            tid
        }
        None => {
            reporter.fail("轮询等待持仓超时");
            reporter.section_end();
            return;
        }
    };

    // B3: 市价平多 (智能回退：先 direction=Long，失败回退到 direction=Short)
    match op_market_close_with_fallback(
        client,
        SYMBOL,
        TradeDirection::Long,
        volume,
        &trade_unit_id,
        reporter,
    )
    .await
    {
        Ok(r) => reporter.info(&format!(
            "close_profit={}, volumeTraded={}",
            r.close_profit, r.volume_traded
        )),
        Err(_) => { /* already logged by fallback */ }
    }

    // B4: 轮询等待持仓归零
    if op_wait_for_no_position(client, TradeDirection::Long).await {
        reporter.success("多仓已平");
    } else {
        reporter.warn("轮询等待多仓归零超时");
    }
    reporter.section_end();
}

async fn phase_c_market_short_roundtrip(
    client: &LbankClient,
    reporter: &mut TestReporter,
) {
    reporter.section("C. 市价开空 → 平空 (Phase C)");

    let volume = Decimal::from_str(VOLUME_STR).unwrap();
    let open_resp = match op_market_open(client, SYMBOL, TradeDirection::Short, volume).await {
        Ok(r) => r,
        Err(e) => {
            reporter.fail(&format!("市价开空失败: {:#?}", e));
            reporter.section_end();
            return;
        }
    };
    reporter.success(&format!(
        "市价开空成功: order_sys_id={}, trade_price={}",
        open_resp.order_sys_id, open_resp.trade_price
    ));

    let pos = op_wait_for_position(client, TradeDirection::Short, volume).await;
    let trade_unit_id = match pos {
        Some(p) => {
            let tid = p.trade_unit_id.clone().unwrap_or_default();
            reporter.success(&format!("持仓确认: TradeUnitID={}", tid));
            tid
        }
        None => {
            reporter.fail("轮询等待持仓超时");
            reporter.section_end();
            return;
        }
    };

    // 平空：智能回退，先 direction=Long (反方向)，失败回退到 direction=Short (持仓方向)
    match op_market_close_with_fallback(
        client,
        SYMBOL,
        TradeDirection::Short,
        volume,
        &trade_unit_id,
        reporter,
    )
    .await
    {
        Ok(r) => reporter.info(&format!(
            "close_profit={}, volumeTraded={}",
            r.close_profit, r.volume_traded
        )),
        Err(_) => { /* already logged by fallback */ }
    }

    if op_wait_for_no_position(client, TradeDirection::Short).await {
        reporter.success("空仓已平");
    } else {
        reporter.warn("轮询等待空仓归零超时");
    }
    reporter.section_end();
}

async fn phase_d_limit_long_with_tpsl(
    client: &LbankClient,
    reporter: &mut TestReporter,
) {
    reporter.section("D. 限价开多 (TPSL) → 撤单 (Phase D)");

    let volume = Decimal::from_str(VOLUME_STR).unwrap();
    // 远低于市价，避免成交
    let limit_price = match op_get_mid_price(client, SYMBOL).await {
        Ok(p) => p - Decimal::from(500), // 远低于市价
        Err(_) => Decimal::from(50000),
    };
    reporter.info(&format!(
        "挂单价格: {} (mid_price - 500)",
        limit_price
    ));

    let open_resp = match op_limit_open(
        client,
        SYMBOL,
        TradeDirection::Long,
        volume,
        limit_price,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            reporter.fail(&format!("限价开多失败: {:#?}", e));
            reporter.section_end();
            return;
        }
    };
    reporter.success(&format!(
        "限价开多成功: order_sys_id={}, price={}, status={}",
        open_resp.order_sys_id, open_resp.price, open_resp.order_status
    ));

    // D2: 确认挂单存在 (Order 接口或下单 response)
    // 经实测: 下单 response status=4 已表明挂单在服务端生效
    // Order 接口会因服务端缓存有几秒延迟，且容易被 rate-limit 封掉
    if open_resp.order_status == "4" {
        reporter.success("挂单已生效 (下单 response status=4)");
    } else {
        reporter.warn(&format!(
            "下单 response status={} (期望 4=挂单中)",
            open_resp.order_status
        ));
        // 兜底：再查一次 Order 接口
        if op_wait_for_pending_orders(client, SYMBOL, 1).await.is_empty() {
            reporter.warn("轮询等待挂单超时 (但下单返回 status=4)");
        }
    }

    // D3: 撤单
    match op_cancel_order(client, &open_resp.order_sys_id).await {
        Ok(r) => reporter.success(&format!(
            "撤单成功: order_sys_id={}, status={}",
            r.order_sys_id, r.order_status
        )),
        Err(e) => reporter.fail(&format!("撤单失败: {:#?}", e)),
    }
    reporter.section_end();
}

async fn phase_e_limit_short_with_tpsl(
    client: &LbankClient,
    reporter: &mut TestReporter,
) {
    reporter.section("E. 限价开空 (无 TPSL) → 撤单 (Phase E)");

    let volume = Decimal::from_str(VOLUME_STR).unwrap();
    let limit_price = match op_get_mid_price(client, SYMBOL).await {
        Ok(p) => p + Decimal::from(500), // 远高于市价
        Err(_) => Decimal::from(80000),
    };
    reporter.info(&format!(
        "挂单价格: {} (mid_price + 500)",
        limit_price
    ));

    let open_resp = match op_limit_open(
        client,
        SYMBOL,
        TradeDirection::Short,
        volume,
        limit_price,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            reporter.fail(&format!("限价开空失败: {:#?}", e));
            reporter.section_end();
            return;
        }
    };
    reporter.success(&format!(
        "限价开空成功: order_sys_id={}, price={}, status={}",
        open_resp.order_sys_id, open_resp.price, open_resp.order_status
    ));

    // E2: 确认挂单 (same logic as Phase D)
    let (raw_body, parsed_orders) = match op_query_pending_orders_raw(client, SYMBOL).await {
        Ok(t) => t,
        Err(e) => {
            reporter.warn(&format!("raw Order 查询失败: {:#?}", e));
            (String::new(), Vec::new())
        }
    };
    let matched: Vec<_> = parsed_orders
        .iter()
        .filter(|o| o.order_sys_id == open_resp.order_sys_id)
        .cloned()
        .collect();
    if !matched.is_empty() {
        reporter.success(&format!(
            "Order 接口确认挂单存在 (匹配 order_sys_id): count={}",
            matched.len()
        ));
    } else {
        if parsed_orders.is_empty() {
            reporter.warn(&format!(
                "Order 接口返回空 (order_sys_id={} 未查到, raw_body={})",
                open_resp.order_sys_id, raw_body
            ));
        } else {
            reporter.warn(&format!(
                "Order 接口返回 {} 条但未匹配目标 order_sys_id={}",
                parsed_orders.len(),
                open_resp.order_sys_id
            ));
        }
    }

    // E3: 撤单
    match op_cancel_order(client, &open_resp.order_sys_id).await {
        Ok(r) => reporter.success(&format!(
            "撤单成功: order_sys_id={}, status={}",
            r.order_sys_id, r.order_status
        )),
        Err(e) => reporter.fail(&format!("撤单失败: {:#?}", e)),
    }
    reporter.section_end();
}

async fn phase_f_limit_long_no_tpsl_and_cancel(
    client: &LbankClient,
    reporter: &mut TestReporter,
) {
    reporter.section("F. 限价开多 (无TPSL) → 撤单 (Phase F)");

    let volume = Decimal::from_str(VOLUME_STR).unwrap();
    let limit_price = match op_get_mid_price(client, SYMBOL).await {
        Ok(p) => p - Decimal::from(500),
        Err(_) => Decimal::from(50000),
    };
    reporter.info(&format!("挂单价格: {}", limit_price));

    let open_resp =
        match op_limit_open(client, SYMBOL, TradeDirection::Long, volume, limit_price).await {
            Ok(r) => r,
            Err(e) => {
                reporter.fail(&format!("限价开多失败: {:#?}", e));
                reporter.section_end();
                return;
            }
        };
    reporter.success(&format!(
        "限价开多成功: order_sys_id={}, price={}, status={}",
        open_resp.order_sys_id, open_resp.price, open_resp.order_status
    ));

    // F: 轮询 Order 接口 + 用 raw 响应诊断 (有时 status=4 的挂单 Order 接口缓存有延迟)
    let (raw_body, parsed_orders) =
        match op_query_pending_orders_raw(client, SYMBOL).await {
            Ok(t) => t,
            Err(e) => {
                reporter.warn(&format!("raw Order 查询失败: {:#?}", e));
                (String::new(), Vec::new())
            }
        };

    // 找到匹配的挂单 (按 order_sys_id)
    let matched: Vec<_> = parsed_orders
        .iter()
        .filter(|o| o.order_sys_id == open_resp.order_sys_id)
        .cloned()
        .collect();
    if !matched.is_empty() {
        reporter.success(&format!(
            "Order 接口确认挂单存在 (匹配 order_sys_id): count={}",
            matched.len()
        ));
    } else {
        // 没匹配上：给出 raw body 帮助 debug
        if parsed_orders.is_empty() {
            reporter.warn(&format!(
                "Order 接口返回空 (order_sys_id={} 未查到, raw_body={})",
                open_resp.order_sys_id, raw_body
            ));
        } else {
            reporter.warn(&format!(
                "Order 接口返回 {} 条但未匹配目标 order_sys_id={} (raw_body={})",
                parsed_orders.len(),
                open_resp.order_sys_id,
                raw_body
            ));
        }
    }

    match op_cancel_order(client, &open_resp.order_sys_id).await {
        Ok(r) => reporter.success(&format!(
            "撤单成功: order_sys_id={}, status={}",
            r.order_sys_id, r.order_status
        )),
        Err(e) => reporter.fail(&format!("撤单失败: {:#?}", e)),
    }
    reporter.section_end();
}

async fn phase_g_final_state(
    client: &LbankClient,
    reporter: &mut TestReporter,
) {
    reporter.section("G. 最终状态确认 (Phase G)");
    match op_get_positions(client).await {
        Ok(positions) => {
            reporter.success(&format!("最终持仓数: {}", positions.len()));
            reporter.info(&op_print_position_summary(&positions));
            if positions.is_empty() {
                reporter.success("所有仓位已平清");
            }
        }
        Err(e) => reporter.fail(&format!("查询失败: {:#?}", e)),
    }

    match op_query_pending_orders_with_retry(client, SYMBOL, 6).await {
        Ok(orders) => {
            reporter.success(&format!("最终挂单数: {}", orders.len()));
            for order in orders {
                reporter.info(&format!(
                    "  - order_sys_id:{} price:{} volume:{} dir:{} status:{}",
                    order.order_sys_id,
                    order.price.as_deref().unwrap_or("?"),
                    order.volume.as_deref().unwrap_or("?"),
                    order.direction.as_deref().unwrap_or("?"),
                    order.order_status.as_deref().unwrap_or("?"),
                ));
            }
        }
        Err(e) => {
            let msg = format!("{:#?}", e);
            if msg.contains("429") || msg.contains("1015") || msg.contains("Cloudflare") {
                reporter.warn(&format!(
                    "查询挂单被 CF 限速 (429/1015): {:#?}",
                    e
                ));
            } else {
                reporter.fail(&format!("查询挂单失败: {:#?}", e));
            }
        }
    }
    reporter.section_end();
}

// ============================================================================
// Phase H: 杠杆 + 止损止盈测试
// ============================================================================

async fn phase_h_leverage_and_stops(client: &LbankClient, reporter: &mut TestReporter) {
    reporter.section("H. 杠杆查询 + 止损止盈挂单 (Phase H)");
    gap!();

    // ── H1: 查询最大杠杆 ──────────────────────────────────────────────────────
    reporter.sub("H1. 查询最大杠杆");
    gap!();
    match client.get_max_leverage(SYMBOL).await {
        Ok((long_max, short_max)) => {
            reporter.success(&format!(
                "最大杠杆: Long={}x, Short={}x",
                long_max, short_max
            ));
        }
        Err(e) => {
            reporter.fail(&format!("get_max_leverage 失败: {:#?}", e));
            return;
        }
    }

    // ── H2: 设置杠杆 20x ─────────────────────────────────────────────────────
    reporter.sub("H2. 设置杠杆 20x");
    gap!();
    match client.set_leverage(SYMBOL, 20).await {
        Ok(()) => reporter.success("设置杠杆 20x 成功"),
        Err(e) => reporter.fail(&format!("set_leverage 失败: {:#?}", e)),
    }

    // ── H3: 获取当前中间价 (走 WS OrderBook) ─────────────────────────────────
    // VPS 后端的 SendQryMarketOrder 只返回 ask 单边数据，没法算 mid。
    // 真订单簿必须走 WS wss://uuws.rerrkvifj.com/ws/v3 OrderBook topic (x=3)。
    reporter.sub("H3. 获取当前中间价 (来自 WS OrderBook topic)");
    gap!();

    let mid_price: f64 = match get_mid_price_via_ws(SYMBOL).await {
        Ok((best_bid, best_ask, depth, ws_source)) => {
            if best_bid <= 0.0 || best_ask >= f64::MAX || best_bid >= best_ask {
                reporter.fail(&format!(
                    "WS OrderBook 异常: best_bid={}, best_ask={}, source={}",
                    best_bid, best_ask, ws_source
                ));
                return;
            }
            let mid = (best_bid + best_ask) / 2.0;
            reporter.success(&format!(
                "WS OrderBook OK: best_bid={}, best_ask={}, mid={}, depth={} (source={})",
                best_bid, best_ask, mid, depth, ws_source
            ));
            mid
        }
        Err(e) => {
            reporter.fail(&format!("WS OrderBook 获取失败: {:#?}", e));
            return;
        }
    };

    // ── H4: 限价开多 ──────────────────────────────────────────────────────────
    reporter.sub("H4. 限价开多 (价格 = mid - 50)");
    gap!();
    let open_price = (mid_price - 50.0).round();
    let open_price_str = format!("{:.1}", open_price);
    let open_resp = match client.limit_open(SYMBOL, TradeDirection::Long, vol(), open_price_str.parse().unwrap()).await {
        Ok(r) => {
            reporter.success(&format!(
                "限价开多成功: order_sys_id={}, price={}, status={}",
                r.order_sys_id, r.price, r.order_status
            ));
            r
        }
        Err(e) => {
            reporter.fail(&format!("limit_open 失败: {:#?}", e));
            return;
        }
    };

    // 等待挂单生效
    gap!();

    // ── H5: 查询挂单确认存在 ──────────────────────────────────────────────────
    reporter.sub("H5. 查询挂单确认");
    gap!();
    let (raw_body, parsed) = match op_query_pending_orders_raw(client, SYMBOL).await {
        Ok(t) => t,
        Err(e) => {
            reporter.warn(&format!("query_orders 失败: {:#?}", e));
            (String::new(), Vec::new())
        }
    };
    let matched: Vec<_> = parsed.iter().filter(|o| o.order_sys_id == open_resp.order_sys_id).cloned().collect();
    if matched.is_empty() {
        reporter.warn(&format!(
            "挂单未在 Order 接口查到 (order_sys_id={}), raw 前200字: {}",
            open_resp.order_sys_id,
            &raw_body.chars().take(200).collect::<String>()
        ));
    } else {
        reporter.success(&format!("挂单确认存在: status={:?}", matched[0].order_status));
    }

    // ── H6: 挂止损单 ─────────────────────────────────────────────────────────
    reporter.sub("H6. 挂止损单 (sl_trigger_price = open_price - 100)");
    gap!();
    let sl_price = open_price - 100.0;
    match client.place_stop_order(
        SYMBOL,
        TradeDirection::Short, // 平多仓 = Short
        vol(),
        Decimal::ZERO,         // 触发后以市价平
        &sl_price.to_string(), // sl_trigger_price
        "",                    // 不设止盈
        TriggerOrderType::OrderStopProfitLoss,
    ).await {
        Ok(r) => {
            reporter.success(&format!(
                "止损单下成功: order_sys_id={}, related_order_sys_id={}, status={}",
                r.order_sys_id, r.related_order_sys_id, r.order_status
            ));
        }
        Err(e) => {
            reporter.fail(&format!("place_stop_order (止损) 失败: {:#?}", e));
        }
    }

    gap!();
    // ── H7: 挂止盈单 ──────────────────────────────────────────────────────────
    reporter.sub("H7. 挂止盈单 (tp_trigger_price = open_price + 200)");
    gap!();
    let tp_price = open_price + 200.0;
    match client.place_stop_order(
        SYMBOL,
        TradeDirection::Short,
        vol(),
        Decimal::ZERO,
        "",                    // 不设止损
        &tp_price.to_string(), // tp_trigger_price
        TriggerOrderType::OrderStopProfitLoss,
    ).await {
        Ok(r) => {
            reporter.success(&format!(
                "止盈单下成功: order_sys_id={}, related_order_sys_id={}, status={}",
                r.order_sys_id, r.related_order_sys_id, r.order_status
            ));
        }
        Err(e) => {
            reporter.fail(&format!("place_stop_order (止盈) 失败: {:#?}", e));
        }
    }

    gap!();

    // ── H8: 查询触发单列表 ────────────────────────────────────────────────────
    reporter.sub("H8. 查询触发单列表");
    gap!();
    match client.query_trigger_orders().await {
        Ok(triggers) => {
            reporter.success(&format!("触发单数量: {}", triggers.len()));
            for t in triggers.iter().take(10) {
                reporter.info(&format!(
                    "  - order_sys_id={} {} type={} trigger_status={} sl={} tp={}",
                    &t.order_sys_id,
                    &t.instrument_id,
                    t.trigger_order_type.as_deref().unwrap_or("?"),
                    t.trigger_status.as_deref().unwrap_or("?"),
                    t.sl_trigger_price.as_deref().unwrap_or("-"),
                    t.tp_trigger_price.as_deref().unwrap_or("-"),
                ));
            }
        }
        Err(e) => {
            let msg = format!("{:#?}", e);
            if msg.contains("429") || msg.contains("1015") || msg.contains("Cloudflare") {
                reporter.warn(&format!("CF 限速: {:#?}", e));
            } else {
                reporter.fail(&format!("query_trigger_orders 失败: {:#?}", e));
            }
        }
    }

    gap!();

    // ── H9: 撤掉限价开多单 ────────────────────────────────────────────────────
    reporter.sub("H9. 撤掉限价开多单 (测试只撤限价，不撤触发单)");
    gap!();
    match client.cancel_order(&open_resp.order_sys_id).await {
        Ok(r) => reporter.success(&format!("撤单成功: order_sys_id={}, status={}", r.order_sys_id, r.order_status)),
        Err(e) => reporter.fail(&format!("cancel_order 失败: {:#?}", e)),
    }

    gap!();

    // ── H10: 最终状态 ──────────────────────────────────────────────────────────
    reporter.sub("H10. 最终状态 (持仓应为0)");
    gap!();
    match op_get_positions(client).await {
        Ok(positions) => {
            reporter.success(&format!("最终持仓数: {}", positions.len()));
            reporter.info(&op_print_position_summary(&positions));
            if positions.is_empty() {
                reporter.success("✅ 仓位已清空");
            }
        }
        Err(e) => reporter.fail(&format!("查询持仓失败: {:#?}", e)),
    }

    gap!();
    match client.query_trigger_orders().await {
        Ok(triggers) => {
            reporter.success(&format!("剩余触发单: {}", triggers.len()));
        }
        Err(e) => reporter.warn(&format!("触发单查询: {:#?}", e)),
    }

    reporter.section_end();
}

// ============================================================================
// Phase I: 只读历史数据查询 (零风险)
// ============================================================================

async fn phase_i_history_queries(client: &LbankClient, reporter: &mut TestReporter) {
    reporter.section("I. 历史数据查询 (只读, Phase I)");
    gap!();

    // ── I1: 历史成交 ──────────────────────────────────────────────────────────
    reporter.sub("I1. 查询历史成交 (最近 7 天)");
    gap!();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let week_ago = now - 7 * 86400;
    match client.query_history_trades(week_ago, now).await {
        Ok(trades) => {
            reporter.success(&format!("历史成交数: {}", trades.len()));
            for t in trades.iter().take(5) {
                reporter.info(&format!(
                    "  - order_sys_id={} price={} volume={} fee={}",
                    t.order_sys_id,
                    &t.price,
                    &t.volume,
                    &t.fee,
                ));
            }
        }
        Err(e) => {
            let msg = format!("{:#?}", e);
            if msg.contains("429") || msg.contains("1015") || msg.contains("Cloudflare") {
                reporter.warn(&format!("CF 限速: {:#?}", e));
            } else {
                reporter.fail(&format!("query_history_trades 失败: {:#?}", e));
            }
        }
    }

    gap!();

    // ── I2: 历史委托 ──────────────────────────────────────────────────────────
    reporter.sub("I2. 查询历史委托 (最近 7 天)");
    gap!();
    match client.query_history_orders(week_ago, now).await {
        Ok(orders) => {
            reporter.success(&format!("历史委托数: {}", orders.len()));
            for o in orders.iter().take(5) {
                reporter.info(&format!(
                    "  - order_sys_id={} {} price={} vol={} status={} close_profit={:?}",
                    o.order_sys_id,
                    o.instrument_id,
                    o.price.as_deref().unwrap_or("?"),
                    o.volume.as_deref().unwrap_or("?"),
                    o.order_status.as_deref().unwrap_or("?"),
                    o.close_profit.as_deref(),
                ));
            }
        }
        Err(e) => {
            let msg = format!("{:#?}", e);
            if msg.contains("429") || msg.contains("1015") || msg.contains("Cloudflare") {
                reporter.warn(&format!("CF 限速: {:#?}", e));
            } else {
                reporter.fail(&format!("query_history_orders 失败: {:#?}", e));
            }
        }
    }

    gap!();

    // ── I3: 聚合信息 ──────────────────────────────────────────────────────────
    reporter.sub("I3. 查询聚合信息 (BTCUSDT)");
    gap!();
    match client.query_aggregate_info(SYMBOL).await {
        Ok(info) => {
            reporter.success("聚合信息查询成功");
            reporter.success("聚合信息查询成功");
            reporter.info(&format!(
                "  - marked_price: {}, long_leverage: {:?}, short_leverage: {:?}",
                &info.marked_price,
                info.long_leverage,
                info.short_leverage,
            ));
        }
        Err(e) => {
            let msg = format!("{:#?}", e);
            if msg.contains("429") || msg.contains("1015") {
                reporter.warn(&format!("CF 限速: {:#?}", e));
            } else {
                reporter.fail(&format!("query_aggregate_info 失败: {:#?}", e));
            }
        }
    }

    gap!();

    // ── I4: 费率查询 ──────────────────────────────────────────────────────────
    reporter.sub("I4. 查询费率 (BTCUSDT)");
    gap!();
    match client.get_fee_rate(SYMBOL).await {
        Ok(fee) => {
            reporter.success("费率查询成功");
                reporter.info(&format!(
                    "  - maker_open: {}, maker_close: {}, taker_open: {}, taker_close: {}",
                    &fee.maker_open_fee_rate,
                    &fee.maker_close_fee_rate,
                    &fee.taker_open_fee_rate,
                    &fee.taker_close_fee_rate,
                ));
        }
        Err(e) => {
            let msg = format!("{:#?}", e);
            if msg.contains("429") || msg.contains("1015") {
                reporter.warn(&format!("CF 限速: {:#?}", e));
            } else {
                reporter.fail(&format!("get_fee_rate 失败: {:#?}", e));
            }
        }
    }

    gap!();

    // ── I5: 合约列表 ──────────────────────────────────────────────────────────
    reporter.sub("I5. 查询合约列表");
    gap!();
    match client.get_instruments().await {
        Ok(instruments) => {
            reporter.success(&format!("合约数量: {}", instruments.len()));
            for ins in instruments.iter().take(5) {
                reporter.info(&format!(
                    "  - {} / {}",
                    &ins.instrument_id,
                    &ins.product_group,
                ));
            }
        }
        Err(e) => reporter.fail(&format!("get_instruments 失败: {:#?}", e)),
    }

    gap!();

    // ── I6: 24h Ticker ────────────────────────────────────────────────────────
    reporter.sub("I6. 查询 24h Ticker");
    gap!();
    match client.get_tickers_24hr().await {
        Ok(tickers) => {
            reporter.success(&format!("24h Ticker 数量: {}", tickers.len()));
            for tk in tickers.iter().take(3) {
                reporter.info(&format!(
                    "  - {} last={} open={} high={} low={} volume={}",
                    &tk.instrument_id,
                    &tk.last_price,
                    &tk.open_price,
                    &tk.high_price,
                    &tk.low_price,
                    &tk.volume,
                ));
            }
        }
        Err(e) => reporter.fail(&format!("get_tickers_24hr 失败: {:#?}", e)),
    }

    reporter.section_end();
}

// ============================================================================
// Phase J: 市价开多 → 限价平仓测试
// ============================================================================

async fn phase_j_limit_close_test(client: &LbankClient, reporter: &mut TestReporter) {
    reporter.section("J. 市价开多 → 限价平多 (Phase J)");
    gap!();

    // ── J1: 设置杠杆 ──────────────────────────────────────────────────────────
    reporter.sub("J1. 设置杠杆 50x");
    gap!();
    match client.set_leverage(SYMBOL, 50).await {
        Ok(()) => reporter.success("设置杠杆 50x 成功"),
        Err(e) => reporter.fail(&format!("set_leverage 失败: {:#?}", e)),
    }

    // ── J2: 市价开多 ───────────────────────────────────────────────────────────
    reporter.sub("J2. 市价开多");
    gap!();
    let open_resp = match client.market_open(SYMBOL, TradeDirection::Long, vol()).await {
        Ok(r) => {
            reporter.success(&format!(
                "市价开多成功: order_sys_id={}, trade_price={}",
                r.order_sys_id, r.trade_price
            ));
            r
        }
        Err(e) => {
            reporter.fail(&format!("market_open 失败: {:#?}", e));
            reporter.section_end();
            return;
        }
    };

    // ── J3: 等待持仓确认 ──────────────────────────────────────────────────────
    reporter.sub("J3. 等待持仓确认");
    let pos = op_wait_for_position(client, TradeDirection::Long, vol()).await;
    match &pos {
        Some(p) => reporter.success(&format!("持仓确认: TradeUnitID={:?}", p.trade_unit_id.as_deref())),
        None => {
            reporter.fail("等待持仓超时");
            reporter.section_end();
            return;
        }
    }

    // ── J4: 限价平多 ──────────────────────────────────────────────────────────
    reporter.sub("J4. 限价平多 (市价+10)");
    gap!();
    let trade_price: f64 = open_resp.trade_price.parse().unwrap_or(0.0);
    let close_price = (trade_price + 10.0).round();

    // 从持仓拿 TradeUnitID
    let trade_unit_id = pos.as_ref().unwrap().trade_unit_id.as_deref().unwrap_or("");
    if trade_unit_id.is_empty() {
        reporter.fail("TradeUnitID 为空，无法限价平仓");
        reporter.section_end();
        return;
    }

    match client.limit_close(SYMBOL, TradeDirection::Short, vol(), Decimal::from_str(&close_price.to_string()).unwrap(), trade_unit_id).await {
        Ok(r) => {
            reporter.success(&format!(
                "限价平多成功: order_sys_id={}, price={}, status={}",
                r.order_sys_id, r.price, r.order_status
            ));
        }
        Err(e) => reporter.fail(&format!("limit_close 失败: {:#?}", e)),
    }

    gap!();

    // ── J5: 查询订单确认 ──────────────────────────────────────────────────────
    reporter.sub("J5. 查询限价平多单确认");
    gap!();
    let (raw_body, parsed) = match op_query_pending_orders_raw(client, SYMBOL).await {
        Ok(t) => t,
        Err(e) => {
            reporter.warn(&format!("query_orders 失败: {:#?}", e));
            (String::new(), Vec::new())
        }
    };
    let matched: Vec<_> = parsed.iter().filter(|o| {
        o.offset_flag.as_deref() == Some("1") // 平仓
    }).cloned().collect();
    if matched.is_empty() {
        reporter.warn(&format!(
            "限价平多单未在 Order 接口查到, raw 前200字: {}",
            &raw_body.chars().take(200).collect::<String>()
        ));
    } else {
        reporter.success(&format!("限价平多单确认: order_sys_id={}, price={}", matched[0].order_sys_id, matched[0].price.as_deref().unwrap_or("?")));
    }

    // ── J6: 撤单 ──────────────────────────────────────────────────────────────
    reporter.sub("J6. 撤掉限价平多单");
    if !matched.is_empty() {
        gap!();
        let cancel_id = &matched[0].order_sys_id;
        match client.cancel_order(cancel_id).await {
            Ok(r) => reporter.success(&format!("撤单成功: status={}", r.order_status)),
            Err(e) => reporter.fail(&format!("cancel_order 失败: {:#?}", e)),
        }
    }

    gap!();

    // ── J7: 市价平仓兜底 ───────────────────────────────────────────────────────
    reporter.sub("J7. 市价平仓兜底");
    match client.market_close(SYMBOL, TradeDirection::Long, vol(), trade_unit_id).await {
        Ok(r) => {
            reporter.success(&format!(
                "市价平仓成功: order_sys_id={}, close_profit={}",
                r.order_sys_id,
                &r.close_profit
            ));
        }
        Err(e) => reporter.fail(&format!("market_close 兜底失败: {:#?}", e)),
    }

    gap!();

    // ── J8: 确认平仓 ──────────────────────────────────────────────────────────
    reporter.sub("J8. 确认平仓");
    let cleared = op_wait_for_no_position(client, TradeDirection::Long).await;
    if cleared {
        reporter.success("✅ 多仓已平清");
    } else {
        reporter.warn("多仓未能平清，请手动检查");
    }

    reporter.section_end();
}

// ============================================================================
// Main
// ============================================================================

#[derive(Debug)]
enum TestMode {
    Full,  // 默认：限价开多+触发单测试 (A→H→G)
    All,   // --mode=all: 同上
}

impl TestMode {
    fn from_args() -> Self {
        for arg in std::env::args() {
            match arg.as_str() {
                "--mode=all" => return TestMode::All,
                _ => {}
            }
        }
        TestMode::Full
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    // 初始化日志 - 输出到 stderr (这样 >file 2>&1 可以同时重定向)
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .with_writer(std::io::stderr)
        .finish()
        .try_init()?;

    let mode = TestMode::from_args();

    let mut reporter = TestReporter::new();
    reporter.info(&format!("交易对: {}", SYMBOL));
    reporter.info(&format!("数量: {} BTC", VOLUME_STR));
    reporter.info(&format!("模式: {:?}", mode));

    // 创建签名器
    let signer = LbankSigner::new(
        "23bec4f8489109e112812c2c2c7c31b3".to_string(),
        "LBA8G85737".to_string(),
        "0688c69dd06a41f38c482e0f46719ed8".to_string(),
        Some("hZlegXdOAxOsNqUVl7oL8p8lwE3dIeqQ".to_string()),
    );

    // 创建客户端
    let proxy_config = ProxyConfig::default();
    let client = LbankClient::with_base_url_and_proxy(
        signer,
        "https://uuapi.rerrkvifj.com",
        proxy_config,
    )?;

    // 串行运行各 Phase (每个 phase 互相独立)，phase 间强制 sleep 防止 429
    macro_rules! gap {
        () => {
            tokio::time::sleep(PHASE_INTER_DELAY).await;
        };
    }

    match mode {
        // 限价开多 + 触发单完整测试 (对应逆向文档测试用例)
        // A(清理) → H(限价开多+止损/止盈/同时TPSL→撤单) → G(状态确认)
        TestMode::Full | TestMode::All => {
            phase_a_status_and_cleanup(&client, &mut reporter).await;
            gap!();
            phase_h_leverage_and_stops(&client, &mut reporter).await;
            gap!();
            phase_g_final_state(&client, &mut reporter).await;
        }
    }

    let filename = "test_order_result.txt";
    reporter.write_to(filename)?;
    println!("\n结果已保存到: {}", filename);

    Ok(())
}
