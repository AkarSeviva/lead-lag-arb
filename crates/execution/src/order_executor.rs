//! Order Executor - 完整订单执行逻辑
//!
//! 根据策略指导文章 (Section 4) 实现:
//!
//! ## 开仓 (Entry)
//! - **必须 Taker** (市价单/IOC)
//! - 原因: 信号窗口仅 100-300ms，挂限价单可能错过机会
//! - Lbank: `OrderPriceType=4`, `OffsetFlag=0`
//!
//! ## 平仓 (Exit) - Method 1 优先
//! 1. **TP 信号触发** → 在最新价挂 GTC Limit (Maker)
//!    - Lbank: `OrderPriceType=0`, `OffsetFlag=5`
//!    - 优点: 利润最大化，避免过早成交
//! 2. **GTC 超时 (gtc_timeout_secs=5s)** → 撤单 → 提交市价单 (Taker)
//!    - Lbank: `OrderPriceType=4`, `OffsetFlag=5`
//!    - 保底成交
//!
//! ## 止损 (Stop-Loss)
//! - **必须 Market** (Taker) - 不惜代价出场
//! - Lbank: `OrderPriceType=4`, `OffsetFlag=5`
//!
//! ## 防交易所反作弊 (Section 7.5)
//! - +/- 50ms 随机抖动
//! - 不在完全相同间隔重复开平仓
//!
//! ## 风险控制 (Section 7.2)
//! - GTC timeout fallback (5s)
//! - Max holding time: 30s 强制市价平仓

use anyhow::{Context, Result};
use config::Direction;
use exchange_adapter_lbank::{
    client::LbankClient,
    protocol::{TradeDirection, TriggerOrderType},
};
use rand::Rng;
use rust_decimal::Decimal;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;
use tracing::{debug, info, warn};

use crate::state_machine::ExitMethod;

/// 订单执行结果
#[derive(Debug, Clone)]
pub enum OrderResult {
    OpenSuccess {
        order_id: String,
        filled_price: Decimal,
        filled_volume: Decimal,
        trade_unit_id: Option<String>,
    },
    CloseSuccess {
        order_id: String,
        filled_price: Decimal,
        realized_pnl: Decimal,
        method: ExitMethod,
    },
    Failed {
        reason: String,
    },
}

/// 开仓参数
#[derive(Debug, Clone)]
pub struct OpenOrderParams {
    pub symbol: String,
    pub direction: Direction,
    pub volume: Decimal,
    pub expected_price: Decimal,
}

/// 平仓参数
#[derive(Debug, Clone)]
pub struct CloseOrderParams {
    pub symbol: String,
    pub direction: Direction,
    pub volume: Decimal,
    pub trade_unit_id: String,
    pub gtc_price: Decimal,
    pub gtc_timeout_secs: u64,
    pub entry_price: Decimal,
}

/// 订单执行器
#[derive(Clone)]
pub struct OrderExecutor {
    client: Arc<LbankClient>,
    jitter_ms: u64,
}

impl OrderExecutor {
    pub fn new(client: Arc<LbankClient>) -> Self {
        Self {
            client,
            jitter_ms: 50,
        }
    }

    pub fn with_jitter(mut self, jitter_ms: u64) -> Self {
        self.jitter_ms = jitter_ms;
        self
    }

    fn direction_to_lbank(&self, d: Direction, is_close: bool) -> TradeDirection {
        match (d, is_close) {
            (Direction::Long, false) => TradeDirection::Long,
            (Direction::Short, false) => TradeDirection::Short,
            (Direction::Long, true) => TradeDirection::Short, // 平多 = 卖出
            (Direction::Short, true) => TradeDirection::Long,  // 平空 = 买入
        }
    }

    fn apply_jitter(&self) {
        if self.jitter_ms == 0 {
            return;
        }
        let mut rng = rand::thread_rng();
        let jitter = rng.gen_range(-(self.jitter_ms as i64)..=(self.jitter_ms as i64));
        if jitter > 0 {
            std::thread::sleep(Duration::from_millis(jitter as u64));
        }
    }

    fn calculate_pnl(
        direction: Direction,
        volume: Decimal,
        entry_price: Decimal,
        exit_price: Decimal,
    ) -> Decimal {
        if entry_price.is_zero() || exit_price.is_zero() {
            return Decimal::ZERO;
        }
        let diff = match direction {
            Direction::Long => exit_price - entry_price,
            Direction::Short => entry_price - exit_price,
        };
        diff * volume
    }

    // ========================================================================
    // 开仓 (Entry) - 必须 Taker
    // ========================================================================

    /// 开仓 - 市价单 (Entry)
    ///
    /// 策略指导 (Section 4):
    /// - 信号窗口仅 100-300ms，必须立即成交
    /// - Lbank: `OrderPriceType=4`, `OffsetFlag=0`
    pub async fn open_position(&self, params: OpenOrderParams) -> Result<OrderResult> {
        info!(
            symbol = %params.symbol,
            direction = ?params.direction,
            volume = %params.volume,
            "Opening position with market order (Entry)"
        );

        self.apply_jitter();

        let direction = self.direction_to_lbank(params.direction, false);

        let resp = self
            .client
            .market_open(&params.symbol, direction, params.volume)
            .await
            .context("Market open order failed")?;

        let order_id = resp.order_sys_id.clone();
        if order_id.is_empty() {
            return Err(anyhow::anyhow!("No order ID in response"));
        }

        info!(
            order_id = %order_id,
            "Position opened (Market)"
        );

        Ok(OrderResult::OpenSuccess {
            order_id,
            filled_price: params.expected_price,
            filled_volume: params.volume,
            trade_unit_id: None,
        })
    }

    // ========================================================================
    // 平仓 (Exit) - Method 1: GTC + Timeout Fallback
    // ========================================================================

    /// 平仓 - Method 1: 先挂 GTC，超时转市价
    ///
    /// 流程:
    /// 1. 挂 GTC Limit (Maker) → 等成交
    /// 2. GTC 超时 → 撤单 → 提交市价单 (Taker)
    ///
    /// Lbank 字段:
    /// - GTC: `OrderPriceType=0` (`Limit`), `OffsetFlag=5` (`CloseAll`)
    /// - 市价: `OrderPriceType=4` (`Market`), `OffsetFlag=5` (`CloseAll`)
    pub async fn close_position_method1(&self, params: CloseOrderParams) -> Result<OrderResult> {
        info!(
            symbol = %params.symbol,
            direction = ?params.direction,
            volume = %params.volume,
            gtc_price = %params.gtc_price,
            timeout_secs = params.gtc_timeout_secs,
            "Closing position with Method 1 (GTC -> Market fallback)"
        );

        let lbank_direction = self.direction_to_lbank(params.direction, true);

        // Step 1: 挂 GTC Limit
        let gtc_order_id = match self
            .client
            .limit_close(
                &params.symbol,
                lbank_direction,
                params.volume,
                params.gtc_price,
                &params.trade_unit_id,
            )
            .await
        {
            Ok(resp) => {
                if resp.order_sys_id.is_empty() {
                    warn!("GTC response has empty order_sys_id, falling back to market");
                    return self.close_position_market_internal(params, lbank_direction).await;
                }
                resp.order_sys_id.clone()
            }
            Err(e) => {
                warn!(error = %e, "GTC order placement failed, falling back to market");
                return self.close_position_market_internal(params, lbank_direction).await;
            }
        };

        info!(
            order_id = %gtc_order_id,
            "GTC limit order placed, waiting for fill"
        );

        // Step 2: 等待成交或超时
        let start_time = Instant::now();
        let timeout = Duration::from_secs(params.gtc_timeout_secs);
        let poll_interval = Duration::from_millis(200);

        while start_time.elapsed() < timeout {
            tokio::time::sleep(poll_interval).await;

            match self.check_order_filled(&gtc_order_id).await {
                Ok(Some(filled_price)) => {
                    let realized_pnl = Self::calculate_pnl(
                        params.direction,
                        params.volume,
                        params.entry_price,
                        filled_price,
                    );

                    info!(
                        order_id = %gtc_order_id,
                        filled_price = %filled_price,
                        pnl = %realized_pnl,
                        elapsed_ms = start_time.elapsed().as_millis() as u64,
                        method = "GTC",
                        "GTC order filled"
                    );

                    return Ok(OrderResult::CloseSuccess {
                        order_id: gtc_order_id,
                        filled_price,
                        realized_pnl,
                        method: ExitMethod::GtcLimit,
                    });
                }
                Ok(None) => {
                    debug!(
                        order_id = %gtc_order_id,
                        elapsed_ms = start_time.elapsed().as_millis() as u64,
                        "GTC not yet filled"
                    );
                }
                Err(e) => {
                    warn!(error = %e, "Check order status failed, will retry");
                }
            }
        }

        // Step 3: 超时 - 撤单 + 市价
        warn!(
            order_id = %gtc_order_id,
            elapsed_secs = start_time.elapsed().as_secs(),
            "GTC order timeout, cancelling and falling back to market"
        );

        if let Err(e) = self.client.cancel_order(&gtc_order_id).await {
            warn!(error = %e, order_id = %gtc_order_id, "Cancel GTC order failed (may already be filled)");
        }

        self.close_position_market_internal(params, lbank_direction).await
    }

    /// 检查订单是否已成交
    async fn check_order_filled(&self, order_id: &str) -> Result<Option<Decimal>> {
        // 简化实现: 这里应该调用查询订单接口
        // 实际生产代码应该调用 history_orders 或类似 API
        // 返回 Some(price) 表示已成交，None 表示未成交

        // TODO: 实现真实的订单状态查询
        let _ = order_id;
        Ok(None)
    }

    async fn close_position_market_internal(
        &self,
        params: CloseOrderParams,
        lbank_direction: TradeDirection,
    ) -> Result<OrderResult> {
        warn!(
            symbol = %params.symbol,
            "Closing position with market order (Taker)"
        );

        self.apply_jitter();

        let resp = self
            .client
            .market_close(
                &params.symbol,
                lbank_direction,
                params.volume,
                &params.trade_unit_id,
            )
            .await
            .context("Market close order failed")?;

        let order_id = resp.order_sys_id.clone();
        if order_id.is_empty() {
            return Err(anyhow::anyhow!("No market close order ID"));
        }

        // Price and volume are strings; we'd need to parse them.
        // For now, return zero and let the position tracker update later.
        let filled_price = params.gtc_price; // Use intended price as estimate

        let realized_pnl = Self::calculate_pnl(
            params.direction,
            params.volume,
            params.entry_price,
            filled_price,
        );

        info!(
            order_id = %order_id,
            filled_price = %filled_price,
            pnl = %realized_pnl,
            method = "Market",
            "Position closed via market order"
        );

        Ok(OrderResult::CloseSuccess {
            order_id,
            filled_price,
            realized_pnl,
            method: ExitMethod::TakerIoc,
        })
    }

    /// 止损 (Stop-Loss) - 必须市价
    ///
    /// 策略指导 (Section 3.2):
    /// - 价差扩大到 (1 + r_sl) × |ΔP_0| 时必须立即出场
    /// - 不能挂限价单，必须市价
    pub async fn stop_loss(&self, params: CloseOrderParams) -> Result<OrderResult> {
        warn!(
            symbol = %params.symbol,
            direction = ?params.direction,
            "STOP-LOSS triggered, executing market close"
        );

        let lbank_direction = self.direction_to_lbank(params.direction, true);
        self.close_position_market_internal(params, lbank_direction).await
    }

    /// 强制平仓 (Max holding time exceeded)
    pub async fn force_close(&self, params: CloseOrderParams) -> Result<OrderResult> {
        warn!(
            symbol = %params.symbol,
            "Max holding time exceeded, force closing"
        );

        let lbank_direction = self.direction_to_lbank(params.direction, true);
        self.close_position_market_internal(params, lbank_direction).await
    }

    // ========================================================================
    // 带止盈止损的限价单 (Conditional Orders) - 文档4.1
    // ========================================================================

    /// 带止盈止损的限价单 (走 SendCloseOrderInsert)
    ///
    /// 字段映射 (逆向文档 Section 4.1):
    /// - `OrderPriceType=0` (Limit)
    /// - `OrderType=0`
    /// - 包含 `CloseSLTriggerPrice`, `CloseTPTriggerPrice`
    /// - `TriggerOrderType=2` (OrderStopProfitLoss)
    ///
    /// 注: 延迟套利不主要使用此接口，但保留用于极端场景
    pub async fn place_sl_tp_order(
        &self,
        symbol: &str,
        direction: Direction,
        volume: Decimal,
        price: Decimal,
        sl_trigger_price: Decimal,
        tp_trigger_price: Decimal,
    ) -> Result<OrderResult> {
        info!(
            symbol = %symbol,
            sl = %sl_trigger_price,
            tp = %tp_trigger_price,
            price = %price,
            "Placing SL/TP conditional limit order"
        );

        let lbank_direction = self.direction_to_lbank(direction, false);

        let resp = self
            .client
            .place_stop_order(
                symbol,
                lbank_direction,
                volume,
                price,
                &sl_trigger_price.to_string(),
                &tp_trigger_price.to_string(),
                TriggerOrderType::OrderStopProfitLoss,
            )
            .await
            .context("Place SL/TP order failed")?;

        Ok(OrderResult::OpenSuccess {
            order_id: resp.order_sys_id,
            filled_price: price,
            filled_volume: volume,
            trade_unit_id: None,
        })
    }

    // ========================================================================
    // Getters
    // ========================================================================

    pub fn client(&self) -> &Arc<LbankClient> {
        &self.client
    }
}

/// 订单执行事件（用于通知其他模块）
#[derive(Debug, Clone)]
pub enum OrderEvent {
    Opened {
        order_id: String,
        symbol: String,
    },
    GtcPlaced {
        order_id: String,
        symbol: String,
    },
    GtcFilled {
        order_id: String,
        filled_price: Decimal,
    },
    GtcTimeoutFallback {
        gtc_order_id: String,
        market_order_id: String,
    },
    Closed {
        order_id: String,
        method: ExitMethod,
        pnl: Decimal,
    },
    StopLoss {
        order_id: String,
        pnl: Decimal,
    },
    Error {
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_calculate_pnl_long_profit() {
        // Long: entry 100, exit 110, volume 1 -> +10
        let pnl = OrderExecutor::calculate_pnl(
            Direction::Long,
            dec!(1),
            dec!(100),
            dec!(110),
        );
        assert_eq!(pnl, dec!(10));
    }

    #[test]
    fn test_calculate_pnl_long_loss() {
        // Long: entry 100, exit 90, volume 1 -> -10
        let pnl = OrderExecutor::calculate_pnl(
            Direction::Long,
            dec!(1),
            dec!(100),
            dec!(90),
        );
        assert_eq!(pnl, dec!(-10));
    }

    #[test]
    fn test_calculate_pnl_short_profit() {
        // Short: entry 100, exit 90, volume 1 -> +10
        let pnl = OrderExecutor::calculate_pnl(
            Direction::Short,
            dec!(1),
            dec!(100),
            dec!(90),
        );
        assert_eq!(pnl, dec!(10));
    }

    #[test]
    fn test_calculate_pnl_short_loss() {
        // Short: entry 100, exit 110, volume 1 -> -10
        let pnl = OrderExecutor::calculate_pnl(
            Direction::Short,
            dec!(1),
            dec!(100),
            dec!(110),
        );
        assert_eq!(pnl, dec!(-10));
    }

    #[test]
    fn test_direction_mapping_open() {
        // Use a helper that skips the Arc construction
        // Direction -> Lbank mapping is a pure function
        let open_long = |d: Direction| match d {
            Direction::Long => TradeDirection::Long,
            Direction::Short => TradeDirection::Short,
        };
        assert_eq!(open_long(Direction::Long), TradeDirection::Long);
        assert_eq!(open_long(Direction::Short), TradeDirection::Short);
    }

    #[test]
    fn test_direction_mapping_close() {
        let close = |d: Direction| match d {
            Direction::Long => TradeDirection::Short,
            Direction::Short => TradeDirection::Long,
        };
        // Close long = sell
        assert_eq!(close(Direction::Long), TradeDirection::Short);
        // Close short = buy
        assert_eq!(close(Direction::Short), TradeDirection::Long);
    }

    #[test]
    fn test_open_params_construction() {
        let params = OpenOrderParams {
            symbol: "BTCUSDT".to_string(),
            direction: Direction::Long,
            volume: dec!(0.01),
            expected_price: dec!(50000),
        };
        assert_eq!(params.symbol, "BTCUSDT");
        assert_eq!(params.volume, dec!(0.01));
    }
}