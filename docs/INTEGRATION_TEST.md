# Lbank 合约交易集成测试计划

## 测试环境
- **平台**: VPS (正式环境)
- **交易所**: Lbank 合约交易 (https://uuapi.rerrkvifj.com)
- **资金**: 真实资金
- **认证**: 硬编码 token (见 main.rs)

---

## 已完成测试 ✅

### Phase 0: 环境验证 ✅ 已验证 (2026-08-13)
| 测试项 | 状态 | 说明 |
|--------|------|------|
| WebSocket 连接 | ✅ | 连接 `wss://uaws.rerrkvifj.com`，获取 WS token |
| REST API 连接 | ✅ | 调用 `/cfd/market/v1.0/SendQryMarketOrder` |
| 签名验证 | ✅ | 请求通过认证，返回数据 |
| 账户余额查询 | ✅ | 可用余额: 1.24971476 USDT |

### Phase 1: 订单操作 ✅ 已验证 (2026-08-13/14)
`test_order_operations` — 7 个 Phase 全部通过：
| 测试项 | Phase | 状态 | order_sys_id |
|--------|-------|------|-------------|
| 状态确认 (查询持仓) | A | ✅ | — |
| 市价开多 → 平多 | B | ✅ | 1007987552834017 |
| 市价开空 → 平空 | C | ✅ | 1007987552876360 |
| 限价开多 (TPSL) → 撤单 | D | ✅ | 1007987552916976 |
| 限价开空 → 撤单 | E | ✅ | 1007987552959618 |
| 限价开多 → 撤单 | F | ✅ | 1007987552998798 |
| 最终状态确认 (持仓+挂单) | G | ✅ | — |

**关键调试记录**：
1. **CF 1015 风控**: Phase 间隔太短 (1500ms) 触发 Cloudflare 限速 → 改为 4000ms
2. **OrderResponse PascalCase**: Lbank Order 接口用 `OrderSysID`/`InstrumentID` (大写) 而非 camelCase
3. **InstrumentID 过滤陷阱**: Order 接口带 `InstrumentID=` 参数会过滤掉 status=4 挂单

---

## 待测试接口

### 高优先 — 交易核心功能

#### 1. 止损/止盈挂单 (Stop Orders)
**对应方法**: `place_stop_order`

```rust
// 参数: symbol, direction, volume, price,
//       sl_trigger_price, tp_trigger_price, trigger_order_type
```

**测试用例**:
- [ ] 限价开多 + 设置止损 (sl_trigger_price < 买入价)
- [ ] 限价开多 + 设置止盈 (tp_trigger_price > 买入价)
- [ ] 限价开多 + 同时设置止损+止盈
- [ ] 止损触发后检查持仓自动平仓
- [ ] 止盈触发后检查持仓自动平仓
- [ ] 触发单取消

#### 2. 杠杆设置
**对应方法**: `set_leverage`, `get_max_leverage`, `init_leverage`

```rust
// get_max_leverage: 返回 (long_max, short_max)
// set_leverage: 设置杠杆
// init_leverage: 自动取最大值再设置
```

**测试用例**:
- [ ] 查询 BTCUSDT 最大杠杆 (expect: 200x long/short)
- [ ] 设置杠杆 10x (限价开多前)
- [ ] 设置杠杆 50x
- [ ] 设置杠杆 200x (最大值)
- [ ] 设置超过最大值应报错

#### 3. 历史成交查询
**对应方法**: `query_history_trades`

```rust
pub async fn query_history_trades(&self, start_time: i64, end_time: i64) -> Result<Vec<TradeResponse>>
```

**测试用例**:
- [ ] 查询最近 24h 成交
- [ ] 查询特定 order_sys_id 的成交
- [ ] 验证成交金额、手续费字段

#### 4. 历史订单查询
**对应方法**: `query_history_orders`

```rust
pub async fn query_history_orders(&self, start_time: i64, end_time: i64) -> Result<Vec<HistoryOrderResponse>>
```

**测试用例**:
- [ ] 查询最近 7 天历史委托
- [ ] 验证字段: order_sys_id, price, volume, direction, order_status, fee, close_profit

#### 5. 触发单查询
**对应方法**: `query_trigger_orders`

```rust
pub async fn query_trigger_orders(&self) -> Result<Vec<TriggerOrderResponse>>
```

**测试用例**:
- [ ] 查询所有触发单 (止损/止盈)
- [ ] 验证 trigger_status 状态
- [ ] 触发后状态变化

---

### 中优先 — 行情与账户数据

#### 6. 24h 行情 (Tickers)
**对应方法**: `get_tickers_24hr`

```rust
pub async fn get_tickers_24hr(&self) -> Result<Vec<Ticker24hr>>
```

**测试用例**:
- [ ] 查询所有合约 24h 行情
- [ ] 验证字段: open, high, low, close, volume

#### 7. 订单簿 (Order Book)
**对应方法**: `get_order_book`

```rust
pub async fn get_order_book(&self, symbol: &str, depth: usize) -> Result<Vec<MarketOrderItem>>
```

**测试用例**:
- [ ] 查询 BTCUSDT 深度 20
- [ ] 查询 BTCUSDT 深度 100
- [ ] 验证 asks(bid=1) 和 bids(bid=0) 分离

#### 8. 合约信息
**对应方法**: `get_instruments`

```rust
pub async fn get_instruments(&self) -> Result<Vec<InstrumentInfo>>
```

**测试用例**:
- [ ] 查询所有合约列表
- [ ] 验证 BTCUSDT/ETHUSDT 等合约存在

#### 9. 费率查询
**对应方法**: `get_fee_rate`

```rust
pub async fn get_fee_rate(&self, symbol: &str) -> Result<FeeRateResponse>
```

**测试用例**:
- [ ] 查询 BTCUSDT 费率 (maker/taker)
- [ ] 验证费率数值合理

#### 10. 聚合信息
**对应方法**: `get_aggregate_info`, `query_aggregate_info`

```rust
pub async fn get_aggregate_info(&self, symbol: &str) -> Result<AggregateInfo>
pub async fn query_aggregate_info(&self, symbol: &str) -> Result<AggregateInfoResponse>
```

**测试用例**:
- [ ] 查询 BTCUSDT 聚合信息
- [ ] 验证持仓限制、开仓限制

---

### 低优先 — 可后续补

#### 11. 限价平仓
**对应方法**: `limit_close`, `post_raw_market_close`

**测试用例**:
- [ ] 市价开多 → 限价平多 (指定价格)
- [ ] 验证成交价格

#### 12. WebSocket 实时推送
**对应方法**: `get_ws_token`

**测试用例**:
- [ ] 获取 WS token 并连接私有频道
- [ ] 接收持仓变化推送
- [ ] 接收订单状态推送

---

## 测试执行计划

### Step 1: 杠杆+触发单测试 (单次测试)
```bash
./target/release/test_order_operations --mode=leverage-stop
```

### Step 2: 历史数据查询 (只读，无风险)
```bash
./target/release/test_order_operations --mode=history
```

### Step 3: 行情数据测试 (只读)
```bash
./target/release/test_order_operations --mode=market
```

### Step 4: 完整 arbitrage 流程
```bash
./target/release/test_order_operations --mode=full
```

---

## 执行日志

| 时间 | 测试项 | 结果 | 备注 |
|------|--------|------|------|
| 2026-08-13 | Phase A-G 订单操作 | ✅ 全部通过 | 修复: CF风控/字段名/InstrumentID过滤 |
| 2026-08-14 | 下单测试全流程 | ✅ | |

---

## 注意事项
1. **资金安全**: 首次测试用最小仓位 (0.0001 BTC)
2. **止损**: 每次测试后确认无遗留持仓
3. **日志**: 详细记录每次操作结果
4. **CF 风控**: Phase 之间强制 4s 间隔，单请求间隔 ≥250ms
5. **字段名**: Order 接口用 PascalCase (`OrderSysID`)，下单接口用 camelCase (`orderSysID`)
