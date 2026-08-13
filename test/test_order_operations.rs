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

use exchange_adapter_lbank::{
    auth::LbankSigner,
    client::LbankClient,
    protocol::{
        OrderInsertResponse, PositionResponse, TradeDirection,
    },
    proxy::ProxyConfig,
};
use rust_decimal::Decimal;
use std::str::FromStr;
use std::time::{Duration, Instant};
use std::{fs::File, io::Write, sync::Arc};
use tracing::{debug, warn};
use tracing_subscriber::util::SubscriberInitExt;

const SYMBOL: &str = "BTCUSDT";
const VOLUME_STR: &str = "0.0001";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(500);

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
pub async fn poll_until<F, Fut>(
    rt: &tokio::runtime::Runtime,
    label: &str,
    predicate: F,
    timeout: Duration,
    interval: Duration,
) -> bool
where
    F: Fn() -> Fut,
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
        rt.block_on(async {
            tokio::time::sleep(interval).await;
        });
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

/// **op_market_close**：市价平仓 (reduce-only，direction = 持仓方向，不再反转)
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
        "op_market_close start (direction=持仓方向，reduce-only)"
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

/// **op_query_pending_orders**：查询当前挂单
pub async fn op_query_pending_orders(
    client: &LbankClient,
    symbol: &str,
) -> anyhow::Result<Vec<exchange_adapter_lbank::protocol::OrderResponse>> {
    debug!(symbol = %symbol, "op_query_pending_orders start");
    let orders = client.query_orders(Some(symbol)).await?;
    debug!(count = orders.len(), "op_query_pending_orders done");
    Ok(orders)
}

/// **op_wait_for_position**：轮询等到指定方向和数量的持仓出现
pub async fn op_wait_for_position(
    rt: &tokio::runtime::Runtime,
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
        rt.block_on(async {
            tokio::time::sleep(DEFAULT_POLL_INTERVAL).await;
        });
    }
    warn!(label = %label, attempts = attempts, "wait_for_position timeout");
    None
}

/// **op_wait_for_no_position**：轮询等到某方向持仓数量归零
pub async fn op_wait_for_no_position(
    rt: &tokio::runtime::Runtime,
    client: &LbankClient,
    expected_direction: TradeDirection,
) -> bool {
    let label = format!("wait_for_no_position_d{:?}", expected_direction);
    let dir_str = match expected_direction {
        TradeDirection::Long => "0",
        TradeDirection::Short => "1",
    };
    poll_until(
        rt,
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
    rt: &tokio::runtime::Runtime,
    client: &LbankClient,
    symbol: &str,
    expected_count: usize,
) -> Vec<exchange_adapter_lbank::protocol::OrderResponse> {
    let label = format!("wait_for_pending_orders_n{}", expected_count);
    let mut attempts = 0;
    let start = Instant::now();
    while start.elapsed() < DEFAULT_TIMEOUT {
        attempts += 1;
        match op_query_pending_orders(client, symbol).await {
            Ok(orders) => {
                let pending: Vec<_> = orders
                    .iter()
                    .filter(|o| {
                        let s = o.order_status.as_deref().unwrap_or("0");
                        s == "2" || s == "3"
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
        rt.block_on(async {
            tokio::time::sleep(DEFAULT_POLL_INTERVAL).await;
        });
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
    _rt: &tokio::runtime::Runtime,
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
    rt: &tokio::runtime::Runtime,
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
    let pos = op_wait_for_position(rt, client, TradeDirection::Long, volume).await;
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

    // B3: 市价平多 (reduce-only: direction = Long)
    match op_market_close(
        client,
        SYMBOL,
        TradeDirection::Long,
        volume,
        &trade_unit_id,
    )
    .await
    {
        Ok(r) => reporter.success(&format!(
            "市价平多成功: order_sys_id={}, close_profit={}",
            r.order_sys_id, r.close_profit
        )),
        Err(e) => reporter.fail(&format!("市价平多失败: {:#?}", e)),
    }

    // B4: 轮询等待持仓归零
    if op_wait_for_no_position(rt, client, TradeDirection::Long).await {
        reporter.success("多仓已平");
    } else {
        reporter.warn("轮询等待多仓归零超时");
    }
    reporter.section_end();
}

async fn phase_c_market_short_roundtrip(
    rt: &tokio::runtime::Runtime,
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

    let pos = op_wait_for_position(rt, client, TradeDirection::Short, volume).await;
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

    // 平空：direction = Short (持仓方向)
    match op_market_close(
        client,
        SYMBOL,
        TradeDirection::Short,
        volume,
        &trade_unit_id,
    )
    .await
    {
        Ok(r) => reporter.success(&format!(
            "市价平空成功: order_sys_id={}, close_profit={}",
            r.order_sys_id, r.close_profit
        )),
        Err(e) => reporter.fail(&format!("市价平空失败: {:#?}", e)),
    }

    if op_wait_for_no_position(rt, client, TradeDirection::Short).await {
        reporter.success("空仓已平");
    } else {
        reporter.warn("轮询等待空仓归零超时");
    }
    reporter.section_end();
}

async fn phase_d_limit_long_with_tpsl(
    _rt: &tokio::runtime::Runtime,
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

    let open_resp = match client
        .place_stop_order(
            SYMBOL,
            TradeDirection::Long,
            volume,
            limit_price,
            // SL/TP 触发价（占位；这里手动控制测试）
            "49000",
            "55000",
            exchange_adapter_lbank::protocol::TriggerOrderType::OrderStopProfitLoss,
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            reporter.fail(&format!("限价+TPSL 开多失败: {:#?}", e));
            reporter.section_end();
            return;
        }
    };
    reporter.success(&format!(
        "限价+TPSL 开多成功: order_sys_id={}, price={}, status={}",
        open_resp.order_sys_id, open_resp.price, open_resp.order_status
    ));

    // 撤单
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
    _rt: &tokio::runtime::Runtime,
    client: &LbankClient,
    reporter: &mut TestReporter,
) {
    reporter.section("E. 限价开空 (TPSL) → 撤单 (Phase E)");

    let volume = Decimal::from_str(VOLUME_STR).unwrap();
    let limit_price = match op_get_mid_price(client, SYMBOL).await {
        Ok(p) => p + Decimal::from(500), // 远高于市价
        Err(_) => Decimal::from(80000),
    };
    reporter.info(&format!(
        "挂单价格: {} (mid_price + 500)",
        limit_price
    ));

    let open_resp = match client
        .place_stop_order(
            SYMBOL,
            TradeDirection::Short,
            volume,
            limit_price,
            "75000",
            "65000",
            exchange_adapter_lbank::protocol::TriggerOrderType::OrderStopProfitLoss,
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            reporter.fail(&format!("限价+TPSL 开空失败: {:#?}", e));
            reporter.section_end();
            return;
        }
    };
    reporter.success(&format!(
        "限价+TPSL 开空成功: order_sys_id={}, price={}, status={}",
        open_resp.order_sys_id, open_resp.price, open_resp.order_status
    ));

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
    rt: &tokio::runtime::Runtime,
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

    // 轮询等待挂单出现
    let pending = op_wait_for_pending_orders(rt, client, SYMBOL, 1).await;
    if pending.is_empty() {
        reporter.warn("未检测到 pending 挂单（可能服务端尚未挂出）");
    } else {
        reporter.info(&format!("检测到 {} 条 pending 挂单", pending.len()));
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
    _rt: &tokio::runtime::Runtime,
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

    match op_query_pending_orders(client, SYMBOL).await {
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

fn main() -> anyhow::Result<()> {
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
    let client = Arc::new(LbankClient::with_base_url_and_proxy(
        signer,
        "https://uuapi.rerrkvifj.com",
        proxy_config,
    )?);

    let rt = tokio::runtime::Runtime::new()?;

    // 串行运行各 Phase (每个 phase 互相独立)
    rt.block_on(async {
        phase_a_status_and_cleanup(&rt, client.as_ref(), &mut reporter).await;
        phase_b_market_long_roundtrip(&rt, client.as_ref(), &mut reporter).await;
        phase_c_market_short_roundtrip(&rt, client.as_ref(), &mut reporter).await;
        phase_d_limit_long_with_tpsl(&rt, client.as_ref(), &mut reporter).await;
        phase_e_limit_short_with_tpsl(&rt, client.as_ref(), &mut reporter).await;
        phase_f_limit_long_no_tpsl_and_cancel(&rt, client.as_ref(), &mut reporter).await;
        phase_g_final_state(&rt, client.as_ref(), &mut reporter).await;
    });

    let filename = "test_order_result.txt";
    reporter.write_to(filename)?;
    println!("\n结果已保存到: {}", filename);

    Ok(())
}
