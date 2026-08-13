# 代码问题清单

本文档跟踪代码中需要修复的问题。

---

## 一、高优先级 (阻塞运行)

### 1. WebSocket 协议解析完全错误

**文件:** `crates/exchange-adapter-lbank/src/ws.rs`

**问题:** 消息格式与实际协议不匹配

**当前代码期望:**
```rust
{"table":"swap/orderBook","data":[...]}
```

**实际协议:**
```rust
{"d":[...],"e":"{...}","x":4,"y":"4000000001","z":3}
```

**修复方向:**
- `x=4` 表示深度数据推送
- `d` 数组包含成交记录: `{a:品种, b:成交量, c:价格, d:方向, e:时间, f:订单ID}`
- 需要从成交推送重建订单簿

**状态:** ❌ 未修复

---

### 2. 缺少认证模块

**问题:** 无法发送认证请求

**需要:**
1. 登录获取 `ex-token`、`ex-uid`、`apiSecret`
2. 生成 `ex-signature`
3. 设置所有认证头

**状态:** ❌ 未实现

---

### 3. 缺少账户余额查询

**文件:** `crates/exchange-adapter-lbank/src/client.rs`

**需要:** 解析 `/cfd/agg/v1/sendQryAll` 响应中的 `assetBalance.available`

**状态:** ❌ 未实现

---

## 二、中优先级 (功能缺失)

### 4. 止盈止损 (计划委托) ✅ 已逆向完成

**API:** `POST /cfd/action/v1.0/SendCloseOrderInsert`

**请求字段:**
```json
{
    "InstrumentID": "BTCUSDT",
    "ExchangeID": "Exchange",
    "Direction": "0",
    "OffsetFlag": "0",
    "OrderPriceType": "0",
    "OrderType": "0",
    "Price": 63790.6,
    "Volume": 0.0001,
    "CloseSLTriggerPrice": "60000",
    "CloseSLTriggerPriceType": "0",
    "CloseTPTriggerPrice": "68000",
    "CloseTPTriggerPriceType": "0",
    "TriggerOrderType": "2"
}
```

**代码缺失:**
```rust
// 需要添加 TriggerOrderRequest 结构体
// 需要添加 create_trigger_order() 方法
```

**状态:** ❌ 文档已完成，❌ 代码未实现

---

### 5. 撤单 ✅ 已逆向完成

**API:** `POST /cfd/action/v1.0/SendOrderAction`

**请求:**
```json
{
    "OrderSysID": "1007986500073684",
    "ActionFlag": "1"
}
```

**状态:** ❌ 文档已完成，❌ 代码未实现

---

### 6. 查询触发单 ✅ 已逆向完成

**API:** `GET /cfd/query/v1.0/TriggerOrder?TriggerOrderType=12`

**状态:** ❌ 未实现

---

### 7. 历史成交记录 ✅ 已逆向完成

**API:** `GET /cfd/query/v1.0/Trade`

**参数:**
```
ProductGroup=SwapU
orderType=price
pageNo=1
pageSize=20
startTime=...
endTime=...
```

**状态:** ❌ 未实现

---

### 8. 限价单下单 ✅ 已逆向完成

**OrderPriceType:**
- `"0"` = 限价单 (需要 `Price` 字段)
- `"4"` = 市价单

**状态:** ❌ 未实现

---

### 8.1 历史委托查询 ✅ 已逆向完成

**API:** `GET /cfd/order/v1/historyAllOrderPage`

**参数:**
```
pageNo=1
pageSize=20
fakeOnePage=1
startTime=...
endTime=...
orderBusType=4
```

**状态:** ❌ 未实现

---

### 9. 深度数据重建订单簿

**问题:** WebSocket 只推送成交记录，不是完整的订单簿快照

**需要实现:**
- 增量更新订单簿
- 定期获取快照并对比
- 处理订单簿重建

**状态:** ❌ 未实现

---

## 三、低优先级 (可优化)

### 10. API 限流处理

**状态:** ⚠️ 仅跟踪，未实现

---

### 11. 断线重连后订单状态同步

**状态:** ⚠️ 仅跟踪，未实现

---

## 六、认证模块 (源码确认)

### HMAC-SHA256 签名 ✅

**签名算法:**
```
signString = [METHOD][PATH][TIMESTAMP][USER_AGENT][VERSION_CODE][CHANNEL][CLIENT_TYPE][DEVICE_ID]
signature = Base64(HMAC-SHA256(secretKey, signString))
```

**硬编码 Secret Key:**
```javascript
const SECRET_KEY_BASE64 = 'MjNiZWM0Zjg0ODkxMDk2ZTExMjgxMmMzMmM3YzMxYjM='
// 解码: '23bec4f8489109e112812c2c2c7c31b3'
```

**认证头:**
- [x] `ex-timestamp` - Date.now()
- [x] `ex-signature` - 签名算法确认
- [x] `ex-client-version-code` - `20251120`
- [x] `ex-client-type` - `WEB`
- [x] `ex-client-channel` - `WEB`
- [x] `ex-client-source` - `WEB`
- [ ] `ex-token` - 需登录获取
- [ ] `ex-uid` - 需登录获取
- [ ] `ex-device-id` - 需生成规则

**已完成:**
- [x] 签名算法
- [x] Secret Key (硬编码)
- [ ] deviceId 生成规则
- [ ] token/uid 获取

---

## 八、WebSocket Topic 逆向 (实测确认)

### R2: Topic=3 订单簿推送 ✅ 已解决

**问题:** 代码注释说 Topic=3 是 Market，但实际是订单簿

**实测数据:**
```json
{"b":[["63825.5","40.6320"],...], "s":[["63825.6","14.8587"],...], "x":3}
```

**结论:** `x=3` = OrderBook (订单簿)，`x=4` = Deal (成交)

---

### R3: 订单簿数据格式 ✅ 已解决

**问题:** 需要从 Deal 数据重建订单簿

**实测数据:**
```json
// 订单簿推送 (x=3)
{
  "b": [["价格","数量"], ...],  // bids 买单
  "s": [["价格","数量"], ...],  // asks 卖单
  "x": 3
}

// 成交推送 (x=4)
{
  "d": {
    "a": "BTCUSDT",    // 品种
    "b": "0.003",      // 成交量
    "c": "63825.5",    // 价格
    "d": "1",          // 方向: "0"=买入, "1"=卖出
    "e": "1786601299", // 时间戳
    "f": "1007932288764238"  // 订单ID
  },
  "x": 4
}
```

**结论:**
- 直接订阅 `x=3` 可获取完整订单簿 (25档)
- 无需从 Deal 数据重建

**状态:** ✅ 已解决 (2026-08-13 下午)

---

## 七、待实现代码

### 需要添加的 API 方法

1. [x] 签名算法 - ✅ 已确认
2. [ ] deviceId 生成
3. [ ] 登录模块

### 需要修复的模块

1. [x] WebSocket 消息解析 - ✅ 协议已文档化
2. [x] Topic=3 订单簿确认 - ✅ 实测确认 x=3 是订单簿
3. [x] 订单簿数据格式 - ✅ b=bids, s=asks
4. [ ] deviceId 生成规则
