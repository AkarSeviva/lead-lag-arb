# Lead-Lag Arbitrage 系统测试计划

## 测试策略

本系统的测试分为三个层次，**重点是交易所集成测试**：

```
┌─────────────────────────────────────────────────────────────────┐
│                      测试金字塔                                  │
│                                                                 │
│                        ┌─────┐                                  │
│                       │ E2E │  端到端 (少量)                    │
│                      ┌──────┴──────┐                           │
│                     │  Integration │ 交易所集成测试 (重要)         │
│                    ┌─────────────────┴┐                          │
│                   │    Unit Tests    │ 单元测试 (基础)            │
│                  ┌─────────────────────┴┐                        │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 测试分层

| 层级 | 目的 | 运行环境 | 运行频率 |
|------|------|----------|----------|
| **单元测试** | 验证核心算法逻辑 | 本地 | 每次提交 |
| **集成测试** | 验证与交易所 API 交互 | 需要 API 密钥 | PR 前必须 |
| **端到端测试** | 验证完整交易流程 | 回测/测试网 | 发布前 |

### 集成测试重点

交易所适配器的集成测试必须验证：

1. **WebSocket 连接** - 与真实交易所建立连接
2. **数据解析** - 验证接收的数据格式正确
3. **认证签名** - Lbank HMAC-SHA256 签名正确
4. **Rate Limit** - 正确处理 API 限流
5. **错误恢复** - 网络异常时的重连逻辑

---

## 项目概述

本系统是一个基于 Rust 的延迟套利交易系统，用于捕捉不同交易所（如 Binance 和 Lbank）之间的价格延迟。

## 模块架构

```
┌─────────────────────────────────────────────────────────────────┐
│                        live-trader                              │
│                      (应用入口)                                 │
└─────────────────────────────────────────────────────────────────┘
           │                    │                    │
           ▼                    ▼                    ▼
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│   md-gateway    │  │  risk-gate      │  │   execution     │
│   (行情网关)     │  │   (风控)        │  │   (执行引擎)     │
└─────────────────┘  └─────────────────┘  └─────────────────┘
      │                    │                    │
      ▼                    ▼                    ▼
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│exchange-adapter │  │  signal-engine  │  │   orderbook     │
│   -binance      │  │   (信号引擎)     │  │   (订单簿)      │
│   (Lead 行情)   │  │                 │  │                 │
└─────────────────┘  └─────────────────┘  └─────────────────┘
      │                    │
      ▼                    ▼
┌─────────────────┐  ┌─────────────────┐
│exchange-adapter │  │   clock-sync   │
│   -lbank        │  │   (时钟同步)    │
│   (Follower)    │  │                │
└─────────────────┘  └─────────────────┘

支持模块:
┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐
│   config    │  │  fee-pnl    │  │  metrics    │  │symbol-selector│
│   (配置)     │  │  (费用PnL)  │  │  (指标)     │  │ (币种筛选)   │
└─────────────┘  └─────────────┘  └─────────────┘  └─────────────┘
                                                                  │
                                                                  ▼
                                                         ┌─────────────┐
                                                         │ persistence │
                                                         │   (持久化)  │
                                                         └─────────────┘
```

---

## Symbol Selector 模块 (symbol-selector)

**职责**: 多交易所币种筛选、Rate Limit 预算分配、轮询策略

**核心理念**: 在 Rate Limit 约束下，平衡「高优先级监控」和「全覆盖扫描」

### Rate Limit 分配策略

```
┌─────────────────────────────────────────────────────────┐
│  Rate Limit Budget: e.g., 100 req/s                    │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  ┌─────────────────────┐     ┌─────────────────────┐  │
│  │   Active Pool (10) │     │  Rotating Pool (90) │  │
│  │   正在交易/监控     │     │  轮动扫描列表       │  │
│  │                     │     │                     │  │
│  │  BTCUSDT ★  活跃   │     │  DOGEUSDT          │  │
│  │  ETHUSDT ★  活跃   │     │  XRPUSDT           │  │
│  │  SOLUSDT ★  活跃   │     │  ADAUSDT           │  │
│  │  ...                │     │  ...               │  │
│  │                     │     │  (每批轮换)        │  │
│  └─────────────────────┘     └─────────────────────┘  │
│         ↑                            ↑                  │
│    高优先级保证                  低优先级轮换            │
│    实时性要求                  扫描全覆盖             │
└─────────────────────────────────────────────────────────┘
```

**配置示例**:

```toml
[symbol_selector]
strategy = "rate_limit_aware"

[symbol_selector.rate_limit]
total = 100                    # 总 Rate Limit
active_reserved = 10           # 保留给活跃监控
rotating_batch_size = 20       # 每批轮询数量
rotation_interval_secs = 5      # 轮询间隔

watchlist = ["BTCUSDT", "ETHUSDT", "SOLUSDT"]  # 可选：观察列表
require_both_exchanges = true   # 是否需要两所都支持
```

---

## 模块测试计划

### 1. config (配置模块)

**职责**: 管理策略参数、过滤条件、风控参数、资本配置、网络配置

**文件**:
- `src/strategy.rs` - 策略配置结构
- `src/types.rs` - 通用类型定义

**测试用例**:

| ID | 测试项 | 输入 | 预期结果 | 测试类型 | 状态 |
|----|-------|------|---------|---------|------|
| C-01 | 默认配置加载 | `StrategyConfig::default()` | 所有字段有默认值 | 单元测试 | ✅ 通过 |
| C-02 | TOML 反序列化 | 有效的 TOML 字符串 | 正确解析为结构体 | 单元测试 | ✅ 通过 |
| C-03 | 网络配置开关 | `proxy_enabled: true/false` | 代理启用/禁用 | 单元测试 | ✅ 通过 |
| C-04 | 风险参数验证 | 超出限制的值 | 正确识别越界 | 单元测试 | ✅ 通过 |
| C-05 | 过滤器配置 | 自定义 filter 参数 | 正确覆盖默认值 | 单元测试 | ✅ 通过 |
| C-06 | 环境变量加载 | `.env` 文件 | 正确读取 API 密钥 | 单元测试 | ✅ 通过 |
| C-07 | 环境变量默认值 | 缺失的变量 | 返回默认值 | 单元测试 | ✅ 通过 |

**测试命令**:
```bash
cargo test -p config
```

---

### 2. exchange-adapter-binance (Binance 交易所适配器)

**职责**: Binance 交易所的 WebSocket 行情接收，作为 Lead 信号来源

**子模块**:
- `types.rs` - 类型定义和归一化
- `market.rs` - 市场数据适配

**测试用例**:

| ID | 测试项 | 输入 | 预期结果 | 测试类型 | 状态 |
|----|-------|------|---------|---------|------|
| B-01 | 深度更新解析 | Binance depthUpdate JSON | 正确解析价格档位 | 单元测试 | ✅ 手动通过 |
| B-02 | 成交解析 | Binance trade JSON | 正确解析成交数据 | 单元测试 | ✅ 手动通过 |
| B-03 | 归一化订单簿 | DepthUpdate | 正确计算 spread 和 depth | 单元测试 | ✅ 手动通过 |
| B-04 | **WebSocket 连接** | **连接 Binance 测试网** | **连接成功并接收实时数据** | **集成测试** | ⏳ 待测试 |
| B-05 | Testnet 配置 | testnet = true | 使用 testnet 端点 | 单元测试 | ✅ 手动通过 |
| B-06 | **多符号订阅** | **BTCUSDT, ETHUSDT** | **同时接收多个品种数据** | **集成测试** | ⏳ 待测试 |
| B-07 | **数据完整性验证** | **接收到的深度数据** | ** bids/asks 价格有序、spread 合理** | **集成测试** | ⏳ 待测试 |
| B-08 | **断线重连** | **模拟断开连接** | **自动重连并恢复订阅** | **集成测试** | ⏳ 待测试 |

**测试命令**:
```bash
# 单元测试
cargo test -p exchange-adapter-binance -- --ignored

# 集成测试 (需要 API 密钥)
cargo test -p exchange-adapter-binance --test integration
```

**依赖项**:
- 网络连接访问 Binance WebSocket
- 测试网 API Key (可选，用于某些需要认证的接口)

---

### 3. exchange-adapter-lbank (Lbank 交易所适配器)

**职责**: Lbank 交易所的 REST API 调用、WebSocket 接收、认证签名

**子模块**:
- `auth.rs` - HMAC-SHA256 签名生成
- `client.rs` - REST API 客户端
- `ws.rs` - WebSocket 客户端
- `market.rs` - 市场数据适配
- `orders.rs` - 订单管理
- `protocol.rs` - 协议编解码
- `proxy.rs` - 代理配置

**测试用例**:

| ID | 测试项 | 输入 | 预期结果 | 测试类型 |
|----|-------|------|---------|---------|
| L-01 | 签名生成 | API密钥 + 私钥 + 请求体 | 生成正确的 HMAC-SHA256 签名 | 单元测试 |
| L-02 | 请求头构造 | 签名 + 时间戳 | 包含正确格式的 headers | 单元测试 |
| L-03 | 响应解析 | JSON 响应字符串 | 正确反序列化 | 单元测试 |
| L-04 | 代理客户端构建 | `ProxyConfig { enabled: true }` | reqwest 客户端配置代理 | 单元测试 |
| L-05 | 代理关闭状态 | `ProxyConfig { enabled: false }` | 不设置代理 | 单元测试 |
| L-06 | WebSocket 消息解析 | Lbank 格式的 JSON | 正确转换为内部类型 | 单元测试 |
| L-07 | **WebSocket 连接** | **连接 Lbank** | **连接成功并接收数据** | **集成测试** |
| L-08 | **REST API 签名** | **真实 API 请求** | **服务器返回签名验证成功** | **集成测试** |
| L-09 | **订单簿数据流** | **接收实时深度** | **数据格式正确、实时更新** | **集成测试** |
| L-10 | **代理连接** | **通过代理访问** | **成功建立连接** | **集成测试** |
| L-11 | **认证流程** | **完整签名 + 请求** | **API 返回有效响应** | **集成测试** |

**测试命令**:
```bash
# 单元测试
cargo test -p exchange-adapter-lbank -- --ignored

# 集成测试 (需要 API 密钥)
cargo test -p exchange-adapter-lbank --test integration
```

**依赖项**:
- 有效的 Lbank API 密钥
- 代理服务 (如果需要访问)

---

### 4. orderbook (订单簿模块)

**职责**: Level 2 订单簿重建、增量更新、深度计算

**文件**:
- `src/book.rs` - 订单簿核心逻辑
- `src/snapshot.rs` - 快照数据结构
- `src/diff.rs` - 增量数据结构

**测试用例**:

| ID | 测试项 | 输入 | 预期结果 | 测试类型 |
|----|-------|------|---------|---------|
| OB-01 | 订单簿初始化 | 空 |  bids/asks 都为空 | 单元测试 |
| OB-02 | 快照更新 | 完整的买卖盘 | bids/asks 正确填充 | 单元测试 |
| OB-03 | 增量插入 | 新价格 + 数量 | 正确插入到对应位置 | 单元测试 |
| OB-04 | 增量更新 | 已存在价格 + 新数量 | 数量正确更新 | 单元测试 |
| OB-05 | 增量删除 | 存在的价格 | 从订单簿移除 | 单元测试 |
| OB-06 | 价格排序 | 多个价格 | bids 降序, asks 升序 | 单元测试 |
| OB-07 | 深度计算 | 指定档位 | 正确累计深度 | 单元测试 |
| OB-08 | 最佳买卖价差 | 买卖盘 | 计算正确的 spread | 单元测试 |
| OB-09 | 订单簿重建 | 快照 + 多个增量 | 最终状态正确 | 集成测试 |

**测试命令**:
```bash
cargo test -p orderbook
```

---

### 5. clock-sync (时钟同步模块)

**职责**: 与交易所服务器时钟同步，测量网络延迟

**文件**:
- `src/sync.rs` - 同步逻辑
- `src/types.rs` - 时钟偏移类型

**测试用例**:

| ID | 测试项 | 输入 | 预期结果 | 测试类型 |
|----|-------|------|---------|---------|
| CS-01 | 时钟偏移测量 | 模拟 HTTP 响应 | 计算出偏移值 | 单元测试 |
| CS-02 | 多样本中位数 | 多个偏移样本 | 返回中位数 | 单元测试 |
| CS-03 | 同步状态机 | 正常/异常网络 | 正确状态转换 | 单元测试 |
| CS-04 | 延迟统计 | 多次测量 | p50/p99 统计正确 | 单元测试 |
| CS-05 | NTP 服务器连接 | 真实 NTP 服务器 | 获取正确时间 | 集成测试 |

**测试命令**:
```bash
cargo test -p clock-sync
```

---

### 6. md-gateway (行情网关)

**职责**: WebSocket 连接管理、自动重连、消息路由

**子模块**:
- `connection.rs` - 连接管理
- `router.rs` - 消息路由
- `message.rs` - 原始消息解析

**测试用例**:

| ID | 测试项 | 输入 | 预期结果 | 测试类型 |
|----|-------|------|---------|---------|
| MG-01 | 连接建立 | 有效的 WebSocket URL | 连接成功 | 集成测试 |
| MG-02 | 订阅处理 | 订阅请求 | 正确发送到服务器 | 单元测试 |
| MG-03 | 消息路由 | 不同 topic 的消息 | 路由到正确处理器 | 单元测试 |
| MG-04 | 断线重连 | 模拟断开 | 自动重连并恢复订阅 | 集成测试 |
| MG-05 | 心跳检测 | ping/pong | 保持连接活跃 | 集成测试 |
| MG-06 | 背压处理 | 消息洪泛 | 不会内存溢出 | 压力测试 |

**测试命令**:
```bash
cargo test -p md-gateway
```

---

### 7. signal-engine (信号引擎)

**职责**: 套利信号计算、过滤链、条件判断

**子模块**:
- `context.rs` - 信号上下文
- `filters.rs` - 过滤器实现
- `signal.rs` - 信号类型
- `state.rs` - 状态管理

**测试用例**:

| ID | 测试项 | 输入 | 预期结果 | 测试类型 | 状态 |
|----|-------|------|---------|---------|------|
| SE-01 | 价差计算 | 两个订单簿 | 正确计算 spread | 单元测试 | ✅ 通过 |
| SE-02 | 方向判断 | bid_a > ask_b | 识别 Long 信号 | 单元测试 | ✅ 通过 |
| SE-03 | 深度过滤 | 深度不足 | 过滤掉信号 | 单元测试 | ✅ 通过 |
| SE-04 | 波动率过滤 | 高波动市场 | 过滤掉信号 | 单元测试 | ✅ 通过 |
| SE-05 | 持续时间过滤 | 持续时间不足 | 过滤掉信号 | 单元测试 | ✅ 通过 |
| SE-06 | 冷却期检查 | 刚触发 SL | 阻止新信号 | 单元测试 | ✅ 通过 |
| SE-07 | 信号有效性 | 满足所有条件 | 产生有效信号 | 集成测试 | ⏳ 待测试 |
| SE-08 | 信号状态机 | 信号的整个生命周期 | 状态正确转换 | 单元测试 | ✅ 通过 |

**测试命令**:
```bash
cargo test -p signal-engine
```

---

### 8. risk-gate (风控门)

**职责**: 交易前风控检查、持仓管理、断路器

**子模块**:
- `gate.rs` - 风控核心
- `circuit_breaker.rs` - 断路器
- `position.rs` - 持仓追踪

**测试用例**:

| ID | 测试项 | 输入 | 预期结果 | 测试类型 |
|----|-------|------|---------|---------|
| RG-01 | 持仓限额检查 | 低于限额 | 允许交易 | 单元测试 |
| RG-02 | 持仓限额检查 | 超过限额 | 拒绝交易 | 单元测试 |
| RG-03 | 总暴露检查 | 未超限 | 允许交易 | 单元测试 |
| RG-04 | 总暴露检查 | 超过限额 | 拒绝交易 | 单元测试 |
| RG-05 | 断路器触发 | 连续亏损 N 次 | 暂停交易 | 单元测试 |
| RG-06 | 断路器恢复 | 冷却期后 | 恢复交易 | 单元测试 |
| RG-07 | SL 后冷却 | 刚触发 SL | 阻止开仓 | 单元测试 |
| RG-08 | 并发交易限制 | 已达上限 | 拒绝新开仓 | 单元测试 |
| RG-09 | 持仓更新 | 开仓/平仓事件 | 正确更新追踪 | 单元测试 |

**测试命令**:
```bash
cargo test -p risk-gate
```

---

### 9. execution (执行引擎)

**职责**: 订单生命周期管理、TP/SL 触发、退出策略

**子模块**:
- `state_machine.rs` - 订单状态机
- `exit.rs` - 退出策略
- `monitor.rs` - 持仓监控

**测试用例**:

| ID | 测试项 | 输入 | 预期结果 | 测试类型 |
|----|-------|------|---------|---------|
| EX-01 | 订单状态转换 | pending → filled | 状态正确 | 单元测试 |
| EX-02 | TP 触发检查 | 价格达到 TP | 触发止盈 | 单元测试 |
| EX-03 | SL 触发检查 | 价格达到 SL | 触发止损 | 单元测试 |
| EX-04 | 最大持仓时间 | 超时 | 自动平仓 | 单元测试 |
| EX-05 | GTC 超时 | 订单未成交 + 超时 | 取消并重试市价 | 单元测试 |
| EX-06 | 退出策略选择 | 不同条件 | 正确选择退出方式 | 单元测试 |
| EX-07 | 完整交易流程 | 开仓 → 持有 → 平仓 | 全流程正确 | 集成测试 |

**测试命令**:
```bash
cargo test -p execution
```

---

### 10. fee-pnl (费用与盈亏)

**职责**: 交易费用计算、期望值计算、盈亏追踪

**文件**:
- `src/lib.rs` - 费用计算器和 EV 计算器

**测试用例**:

| ID | 测试项 | 输入 | 预期结果 | 测试类型 |
|----|-------|------|---------|---------|
| FP-01 | 双向手续费 | 名义价值 $10,000 | 正确计算总费用 | 单元测试 |
| FP-02 | 手续费返还 | 80% 返还比例 | 正确扣除返还 | 单元测试 |
| FP-03 | 期望值计算 | p=0.6, R=2, L=1 | E > 0 表示盈利策略 | 单元测试 |
| FP-04 | 盈亏平衡胜率 | R=2, L=1 | 正确计算 breakeven | 单元测试 |
| FP-05 | 盈亏记录 | 多笔交易 | 正确累计 PnL | 单元测试 |
| FP-06 | 胜率计算 | 10 笔交易 6 胜 | 60% 胜率 | 单元测试 |

**测试命令**:
```bash
cargo test -p fee-pnl
```

---

### 11. metrics (指标模块)

**职责**: Prometheus 指标收集与导出

**文件**:
- `src/lib.rs` - 指标注册表

**测试用例**:

| ID | 测试项 | 输入 | 预期结果 | 测试类型 |
|----|-------|------|---------|---------|
| MT-01 | 计数器递增 | 信号数量 +1 | 正确增加 | 单元测试 |
| MT-02 | 仪表盘更新 | PnL 值变化 | 正确更新 | 单元测试 |
| MT-03 | 直方图记录 | 延迟值 | 正确分桶统计 | 单元测试 |
| MT-04 | 指标导出 | Prometheus 格式 | 正确格式化输出 | 单元测试 |
| MT-05 | 自定义指标 | 动态添加 gauge | 可查询到 | 单元测试 |

**测试命令**:
```bash
cargo test -p metrics
```

---

### 12. persistence (持久化模块)

**职责**: Tick 数据、订单日志、盈亏记录持久化

**文件**:
- `src/lib.rs` - 异步文件写入器

**测试用例**:

| ID | 测试项 | 输入 | 预期结果 | 测试类型 |
|----|-------|------|---------|---------|
| PS-01 | 目录创建 | 新目录路径 | 自动创建目录 | 集成测试 |
| PS-02 | Tick 写入 | TickRecord | JSONL 格式追加 | 集成测试 |
| PS-03 | 订单日志写入 | OrderLogRecord | JSONL 格式追加 | 集成测试 |
| PS-04 | PnL 写入 | PnlLogRecord | JSONL 格式追加 | 集成测试 |
| PS-05 | 文件格式验证 | 写入后读取 | 每行一个有效 JSON | 集成测试 |

**测试命令**:
```bash
cargo test -p persistence
```

---

### 13. symbol-selector (币种筛选模块)

**职责**: 多交易所币种交集计算、Rate Limit 预算分配、轮询策略管理

**文件**:
- `src/types.rs` - 核心类型 (Symbol, Exchange, RateLimitBudget)
- `src/config.rs` - 配置结构 (SelectorConfig, Strategy)
- `src/pool.rs` - 池管理 (ActivePool, RotatingPool, PoolManager)
- `src/selector.rs` - 主选择器 (SymbolSelector, SelectionEvent)

**测试用例**:

| ID | 测试项 | 输入 | 预期结果 | 测试类型 |
|----|-------|------|---------|---------|
| SS-01 | 币种交集计算 | Binance + Lbank 列表 | 正确计算交集 | 单元测试 |
| SS-02 | Active Pool 添加/移除 | BTCUSDT | 正确管理活跃池 | 单元测试 |
| SS-03 | Rotating Pool 轮换 | 6个symbol, batch=2 | 正确分批轮换 | 单元测试 |
| SS-04 | Rate Limit 预算分配 | total=100, reserved=10 | 正确分配 90 给轮换 | 单元测试 |
| SS-05 | 观察列表过滤 | watchlist=[BTC,ETH] | 只保留观察列表 | 单元测试 |
| SS-06 | 轮换时间间隔 | 间隔内再次调用 | 不触发轮换 | 单元测试 |
| SS-07 | 活跃symbol排斥 | BTC加入活跃池 | BTC不在轮换池中 | 单元测试 |
| SS-08 | 异步轮换任务 | 定时触发 | 正确后台轮换 | 集成测试 |
| SS-09 | 事件通知 | 添加活跃symbol | 发送 SelectionEvent | 单元测试 |
| SS-10 | 配置反序列化 | TOML 格式配置 | 正确解析所有字段 | 单元测试 |
| SS-11 | **币种列表更新** | **从交易所获取列表** | **正确更新交集** | **集成测试** |
| SS-12 | **实时轮换** | **模拟交易所数据** | **轮换策略正确执行** | **集成测试** |

**测试命令**:
```bash
# 单元测试
cargo test -p symbol-selector -- --ignored

# 集成测试
cargo test -p symbol-selector --test integration
```

---

## 集成测试计划

### IT-01: 完整信号流程

**描述**: 测试从接收市场数据到产生交易信号的全流程

**步骤**:
1. 启动 md-gateway 连接到模拟/真实 WebSocket
2. orderbook 接收并重建订单簿
3. signal-engine 计算价差并检查过滤器
4. 验证信号输出

**验收标准**: 满足条件的价差能产生 ArbitrageSignal

### IT-02: 完整交易流程

**描述**: 测试从信号产生到订单成交的全流程

**步骤**:
1. 产生套利信号
2. risk-gate 进行风控检查
3. execution 发送订单
4. 监控订单状态变化
5. TP/SL 触发并平仓
6. 验证 PnL 计算

**验收标准**: 完整交易生命周期正确运行

### IT-03: 网络异常恢复

**描述**: 测试网络中断后的恢复能力

**步骤**:
1. 建立 WebSocket 连接
2. 模拟网络断开
3. 验证自动重连
4. 验证订阅恢复
5. 验证数据连续性

**验收标准**: 重连后系统恢复正常工作

### IT-04: 压力测试

**描述**: 高频市场数据下的系统表现

**步骤**:
1. 以 100+ msg/s 发送模拟数据
2. 监控 CPU/内存使用
3. 验证无数据丢失
4. 验证延迟在可接受范围

**验收标准**: 系统在高负载下稳定运行

---

## 测试执行顺序

```
Phase 1: 基础模块 (无外部依赖)
├── config          ← 先测试配置，依赖最少
├── fee-pnl         ← 纯计算逻辑，无 IO
├── metrics         ← 纯内存操作
└── symbol-selector ← 币种筛选，内存操作

Phase 2: 核心数据结构
├── orderbook       ← 市场数据处理核心
├── clock-sync      ← 时钟同步 (需要网络)

Phase 3: 业务逻辑
├── signal-engine   ← 信号计算
├── risk-gate       ← 风控检查
└── execution       ← 订单执行

Phase 4: 集成
├── exchange-adapter-binance ← Lead 交易所适配器
├── exchange-adapter-lbank   ← Follower 交易所适配器
├── md-gateway       ← WebSocket 网关
└── persistence      ← 持久化

Phase 5: 端到端
└── live-trader     ← 完整系统集成测试
```

---

## 测试类型说明

### 1. 单元测试 (Unit Tests)
- 本地验证代码逻辑正确性
- 不依赖外部服务
- 使用 mock 数据测试边界条件
- 覆盖核心算法和数据结构

### 2. 集成测试 (Integration Tests)
- **与真实交易所 API 交互**
- 验证数据解析和协议正确性
- 测试 WebSocket 连接和数据流
- 需要有效的 API 密钥和测试网络

### 3. 端到端测试 (E2E Tests)
- 完整交易流程验证
- 在回测环境或测试网运行
- 不使用真实资金

---

## 测试环境要求

### 本地开发
- Rust 1.86+
- tokio (async runtime)
- 代理服务 (可选, localhost:7890)

### 交易所集成测试 (必须)

| 交易所 | 必需项 | 说明 |
|-------|--------|------|
| **Binance** | API Key (测试网) | 用于 WebSocket 连接和数据订阅 |
| | WebSocket 端点 | `wss://testnet.binance.vision/ws` |
| | Rate Limit | 遵守 API 限制 |
| **Lbank** | API Key | 用于 REST API 和 WebSocket |
| | 签名验证 | 确保 HMAC-SHA256 签名正确 |
| | 代理设置 | 某些地区需要代理 |

### 测试网络/账户
- 使用各交易所提供的 **测试网**
- 创建专用测试账户，与生产环境隔离
- 准备足够的测试资金

---

## 交易所集成测试模块

### exchange-adapter-binance 集成测试

```rust
#[cfg(test)]
mod integration_tests {
    use super::*;

    /// 测试与 Binance 测试网 WebSocket 连接
    #[tokio::test]
    async fn test_binance_websocket_connection() {
        let config = BinanceConfig::testnet();
        let market_data = BinanceMarketData::new(config);

        let mut rx = market_data.subscribe_depth("BTCUSDT").await.unwrap();

        // 等待接收数据 (最多 5 秒)
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            rx.recv()
        ).await;

        assert!(result.is_ok(), "Should receive depth updates");
    }

    /// 测试实时价格数据解析
    #[tokio::test]
    async fn test_depth_update_parsing() {
        // 连接并验证数据格式正确
        // 验证订单簿数据结构
    }

    /// 测试 Rate Limit 处理
    #[tokio::test]
    async fn test_rate_limit_handling() {
        // 发送大量请求，验证限流处理
    }
}
```

### exchange-adapter-lbank 集成测试

```rust
#[cfg(test)]
mod integration_tests {
    /// 测试 Lbank REST API 签名
    #[tokio::test]
    async fn test_lbank_signature() {
        // 使用测试密钥验证签名算法
    }

    /// 测试 WebSocket 连接和数据流
    #[tokio::test]
    async fn test_lbank_websocket() {
        // 连接测试网 WebSocket
        // 验证订单簿数据结构
    }

    /// 测试代理连接
    #[tokio::test]
    async fn test_proxy_connection() {
        // 如果配置了代理，验证连接
    }
}
```

### symbol-selector 集成测试

```rust
#[cfg(test)]
mod integration_tests {
    /// 测试币种列表更新
    #[tokio::test]
    async fn test_symbol_refresh() {
        // 从交易所获取实时币种列表
        // 验证交集计算正确
    }

    /// 测试轮换策略
    #[tokio::test]
    async fn test_rotation_with_live_data() {
        // 使用真实币种数据测试轮换
    }
}
```

---

## 持续集成

### CI 测试策略

```yaml
# .github/workflows/test.yml
name: Tests

on: [push, pull_request]

jobs:
  unit-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Run unit tests
        run: cargo test --workspace

  integration-tests:
    runs-on: ubuntu-latest
    # 仅在 PR 时运行集成测试
    if: github.event_name == 'pull_request'
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Run integration tests
        env:
          BINANCE_API_KEY: ${{ secrets.BINANCE_TESTNET_KEY }}
          LBANK_API_KEY: ${{ secrets.LBANK_TEST_KEY_KEY }}
          LBANK_SECRET_KEY: ${{ secrets.LBANK_TEST_SECRET_KEY }}
        run: cargo test --workspace --test integration
```

### 测试命令

```bash
# 所有单元测试 (CI 必跑)
cargo test --workspace

# 单元测试 + 覆盖率
cargo test --workspace --coverage

# 集成测试 (需要 API 密钥)
cargo test --workspace --test integration

# 仅检查编译
cargo check --workspace

# 代码格式化检查
cargo fmt --check

# Lint 检查
cargo clippy --workspace -- -D warnings
```

### 环境变量和密钥管理

项目使用 `.env` 文件管理 API 密钥：

```
project/
├── .env.example     # 模板 (提交到 git)
├── .env            # 实际密钥 (已在 .gitignore 中)
└── config.toml     # 非敏感配置
```

**`.env.example`** (模板)：
```bash
# Binance
BINANCE_API_KEY=your_binance_key_here
BINANCE_SECRET_KEY=your_binance_secret_here

# Lbank
LBANK_API_KEY=your_lbank_key_here
LBANK_SECRET_KEY=your_lbank_secret_here
```

**`.gitignore`** (已配置)：
```
.env
.env.local
```

**Rust 代码加载**：
```rust
use config::env;

// 应用启动时调用
env::init();

// 读取密钥
let api_key = env::require("LBANK_API_KEY");
let secret = env::optional("LBANK_SECRET_KEY");
let ws_url = env::get("LBANK_WS_URL", "wss://www.lbank.com/ws");
```

### 环境变量参考

```bash
# 必需
LBANK_API_KEY=your_key
LBANK_SECRET_KEY=your_secret

# 可选 (有默认值)
LBANK_WS_URL=wss://www.lbank.com/ws
BINANCE_WS_URL=wss://stream.binance.com:9443/ws

# 代理 (如需要)
HTTP_PROXY=http://127.0.0.1:7890
HTTPS_PROXY=http://127.0.0.1:7890
```


---

## 测试覆盖率目标

| 模块 | 单元测试 | 集成测试 | 状态 |
|------|---------|---------|------|
| **config** | 90%+ | 基础 TOML 解析 | ✅ 完成 (7/7) |
| orderbook | 95%+ | 订单簿重建 (模拟数据) | ⏳ 进行中 |
| clock-sync | 80%+ | NTP 同步 | ⏳ 进行中 |
| signal-engine | 85%+ | 过滤链测试 | ✅ 部分完成 (7/8) |
| risk-gate | 90%+ | 风控场景 | ⏳ 进行中 |
| execution | 85%+ | 状态机测试 | ⏳ 进行中 |
| fee-pnl | 90%+ | EV 计算 | ⏳ 进行中 |
| metrics | 70%+ | 导出格式 | ⏳ 进行中 |
| persistence | 60%+ | 文件读写 | ⏳ 进行中 |
| symbol-selector | 85%+ | 币种刷新 + 轮换 | ⏳ 进行中 |
| **exchange-adapter-binance** | 70%+ | **WebSocket 连接 + 数据解析** | ✅ 部分完成 (5/8) |
| **exchange-adapter-lbank** | 70%+ | **REST API + 签名 + WebSocket** | ⏳ 进行中 |
| md-gateway | 75%+ | 重连测试 | ⏳ 进行中 |

> **重点**: 交易所适配器模块需要重点测试与真实 API 的交互
