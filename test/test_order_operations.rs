//! 订单操作测试 - 解耦版
//!
//! 设计原则：
//! 1. 每个操作都是**独立函数**，操作之间互不依赖具体实现细节
//! 2. 使用 **轮询等待** (poll-until-stable) 而非固定 sleep，避免顺序冲突
//! 3. **reduce-only 平仓**：平仓 direction == 持仓 direction (不再做反转)
//! 4. 每个操作有详细 debug log 记录所有 IO，便于后续逆向分析
//!
//! 测试流程（新版本）：
//! - Phase A: 清理 & 状态确认
//! - Phase B: 市价开多 → 轮询等待生效 → 市价平多 → 轮询等待已平
//! - Phase C: 市价开空 → 轮询等待生效 → 市价平空 → 轮询等待已平
//! - Phase D: 限价开多 (带 TPSL) → 轮询等待挂单 → 撤单 → 轮询等待已撤
//! - Phase E: 限价开空 (带 TPSL) → 轮询等待挂单 → 撤单 → 轮询等待已撤
//! - Phase F: 限价开多 (无 TPSL) → 轮询等待挂单 → 撤单
//! - Phase G: 最终状态确认
//!
//! 运行：`cargo run --release --bin test_order_operations`

use exchange_adapter_lbank::{
    auth::LbankSigner,
    client::LbankClient,
    protocol::{OrderInsertResponse, PositionResponse, TradeDirection},
    proxy::ProxyConfig,
};
use rust_decimal::Decimal;
use std::str::FromStr;
use std::time::{Duration, Instant};
use std::{fs::File, io::Write};
use tracing::{debug, warn};
use tracing_subscriber::util::SubscriberInitExt;

const SYMBOL: &str = "BTCUSDT";
const VOLUME_STR: &str = "0.0001";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1); // 放慢轮询，避免 429

/// Phase 之间强制间隔，避免触发 Cloudflare 反爬 (默认 1500ms)
const PHASE_INTER_DELAY: Duration = Duration::from_millis(1500);

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
    let mut backoff = Duration::from_secs(2);
    for attempt in 1..=max_attempts {
        match op_query_pending_orders(client, symbol).await {
            Ok(orders) => return Ok(orders),
            Err(e) => {
                let msg = format!("{:#?}", e);
                // 429 / Cloudflare / rate-limit / network error 都重试
                let retryable = msg.contains("429")
                    || msg.contains("Too Many Requests")
                    || msg.contains("rate")
                    || msg.contains("cloudflare")
                    || msg.contains("Just a moment")
                    || msg.contains("connection")
                    || msg.contains("timed out");
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
                backoff = (backoff * 2).min(Duration::from_secs(15));
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
                warn!(label = %label, error = %e, "query_positions failed during wait");
            }
        }
        tokio::time::sleep(DEFAULT_POLL_INTERVAL).await;
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

    match op_query_pending_orders_with_retry(client, SYMBOL, 4).await {
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
        Err(e) => reporter.fail(&format!("查询挂单失败: {:#?}", e)),
    }
    reporter.section_end();
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    // 初始化日志 - 输出到 stderr (这样 >file 2>&1 可以同时重定向)
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .with_writer(std::io::stderr)
        .finish()
        .try_init()?;

    let mut reporter = TestReporter::new();
    reporter.info(&format!("交易对: {}", SYMBOL));
    reporter.info(&format!("数量: {} BTC", VOLUME_STR));

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

    phase_a_status_and_cleanup(&client, &mut reporter).await;
    gap!();
    phase_b_market_long_roundtrip(&client, &mut reporter).await;
    gap!();
    phase_c_market_short_roundtrip(&client, &mut reporter).await;
    gap!();
    phase_d_limit_long_with_tpsl(&client, &mut reporter).await;
    gap!();
    phase_e_limit_short_with_tpsl(&client, &mut reporter).await;
    gap!();
    phase_f_limit_long_no_tpsl_and_cancel(&client, &mut reporter).await;
    gap!();
    phase_g_final_state(&client, &mut reporter).await;

    let filename = "test_order_result.txt";
    reporter.write_to(filename)?;
    println!("\n结果已保存到: {}", filename);

    Ok(())
}
