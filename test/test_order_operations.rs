//! 订单操作测试 - 顺序测试各种下单机制
//! 
//! 测试流程:
//! 1. 查询持仓确认无持仓
//! 2. 市价开多仓 (0.0001 BTC)
//! 3. 等待1秒
//! 4. 查询持仓确认开仓成功
//! 5. 市价平多仓
//! 6. 等待1秒
//! 7. 限价开空仓 (挂单价格低于市价)
//! 8. 查询持仓确认开仓成功
//! 9. 撤单 (如果未成交)
//! 10. 限价平空仓

use exchange_adapter_lbank::{auth::LbankSigner, client::LbankClient, protocol::TradeDirection, proxy::ProxyConfig};
use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};

const SYMBOL: &str = "BTCUSDT";
const VOLUME: &str = "0.0001";

fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .finish()
        .init();

    let mut output = String::new();
    output.push_str("===========================================\n");
    output.push_str("Lbank 订单操作测试\n");
    output.push_str("===========================================\n\n");
    output.push_str(&format!("交易对: {}\n", SYMBOL));
    output.push_str(&format!("数量: {} BTC\n\n", VOLUME));

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
    let volume = VOLUME.parse().unwrap();

    // =========================================================================
    // Step 1: 查询当前持仓
    // =========================================================================
    output.push_str("【Step 1】查询当前持仓\n");
    output.push_str("----------------------------------------\n");
    match rt.block_on(client.query_positions()) {
        Ok(positions) => {
            output.push_str(&format!("✅ 查询成功! 当前持仓数: {}\n", positions.len()));
            for pos in &positions {
                output.push_str(&format!("  - {} {}: {}\n", 
                    pos.instrument_id, pos.direction, pos.volume));
            }
        }
        Err(e) => {
            output.push_str(&format!("❌ 失败: {}\n", e));
        }
    }
    output.push_str("\n");

    // =========================================================================
    // Step 2: 市价开多仓
    // =========================================================================
    output.push_str("【Step 2】市价开多仓 (Long)\n");
    output.push_str("----------------------------------------\n");
    match rt.block_on(client.market_open(SYMBOL, TradeDirection::Long, volume)) {
        Ok(resp) => {
            output.push_str(&format!("✅ 开多仓成功!\n"));
            output.push_str(&format!("  OrderID: {}\n", resp.order_id));
            output.push_str(&format!("  订单状态: {}\n", resp.status));
            if let Some(msg) = resp.error_msg {
                output.push_str(&format!("  错误信息: {}\n", msg));
            }
        }
        Err(e) => {
            output.push_str(&format!("❌ 失败: {}\n", e));
        }
    }
    output.push_str("\n");

    // 等待1秒
    output.push_str("等待 1 秒...\n\n");
    std::thread::sleep(Duration::from_secs(1));

    // =========================================================================
    // Step 3: 查询持仓确认
    // =========================================================================
    output.push_str("【Step 3】查询持仓确认\n");
    output.push_str("----------------------------------------\n");
    match rt.block_on(client.query_positions()) {
        Ok(positions) => {
            output.push_str(&format!("✅ 查询成功! 当前持仓数: {}\n", positions.len()));
            for pos in &positions {
                output.push_str(&format!("  - {} 方向:{} 数量:{}\n", 
                    pos.instrument_id, pos.posi_direction, pos.position));
                if pos.posi_direction == "0" {
                    output.push_str(&format!("    TradeUnitID: {}\n", pos.trade_unit_id));
                }
            }
        }
        Err(e) => {
            output.push_str(&format!("❌ 失败: {}\n", e));
        }
    }
    output.push_str("\n");

    // 等待1秒
    output.push_str("等待 1 秒...\n\n");
    std::thread::sleep(Duration::from_secs(1));

    // =========================================================================
    // Step 4: 市价平多仓
    // =========================================================================
    output.push_str("【Step 4】市价平多仓 (Close Long)\n");
    output.push_str("----------------------------------------\n");
    
    // 先获取持仓获取 trade_unit_id
    let trade_unit_id = rt.block_on(client.query_positions())?
        .into_iter()
        .find(|p| p.posi_direction == "0")
        .map(|p| p.trade_unit_id);

    match trade_unit_id {
        Some(tid) => {
            output.push_str(&format!("  TradeUnitID: {}\n", tid));
            match rt.block_on(client.market_close(SYMBOL, TradeDirection::Long, volume, &tid)) {
                Ok(resp) => {
                    output.push_str(&format!("✅ 平多仓成功!\n"));
                    output.push_str(&format!("  OrderID: {}\n", resp.order_id));
                    output.push_str(&format!("  订单状态: {}\n", resp.status));
                }
                Err(e) => {
                    output.push_str(&format!("❌ 失败: {}\n", e));
                }
            }
        }
        None => {
            output.push_str("❌ 未找到多仓持仓，跳过平仓\n");
        }
    }
    output.push_str("\n");

    // 等待1秒
    output.push_str("等待 1 秒...\n\n");
    std::thread::sleep(Duration::from_secs(1));

    // =========================================================================
    // Step 5: 获取当前价格
    // =========================================================================
    output.push_str("【Step 5】获取当前价格\n");
    output.push_str("----------------------------------------\n");
    let current_price = match rt.block_on(client.get_order_book(SYMBOL, 1)) {
        Ok(items) => {
            if let Some(best_bid) = items.iter().find(|i| i.direction == "1") {
                output.push_str(&format!("✅ 获取成功!\n"));
                output.push_str(&format!("  最佳买价 (Bid): {}\n", best_bid.price));
                best_bid.price.clone()
            } else {
                output.push_str("❌ 未找到买价\n");
                "0".to_string()
            }
        }
        Err(e) => {
            output.push_str(&format!("❌ 获取失败: {}\n", e));
            "0".to_string()
        }
    };
    output.push_str("\n");

    // =========================================================================
    // Step 6: 限价开空仓 (挂单价格 = 市价 - 10)
    // =========================================================================
    output.push_str("【Step 6】限价开空仓 (Short) - 挂单价格低于市价\n");
    output.push_str("----------------------------------------\n");
    
    let limit_price_short: f64 = current_price.parse().unwrap_or(0.0) - 10.0;
    output.push_str(&format!("  挂单价格: {} (市价 {} - 10)\n", limit_price_short, current_price));
    
    match rt.block_on(client.limit_open(SYMBOL, TradeDirection::Short, volume, limit_price_short.into())) {
        Ok(resp) => {
            output.push_str(&format!("✅ 限价开空仓请求已发送!\n"));
            output.push_str(&format!("  OrderID: {}\n", resp.order_id));
            output.push_str(&format!("  订单状态: {}\n", resp.status));
        }
        Err(e) => {
            output.push_str(&format!("❌ 失败: {}\n", e));
        }
    }
    output.push_str("\n");

    // 等待1秒
    output.push_str("等待 1 秒...\n\n");
    std::thread::sleep(Duration::from_secs(1));

    // =========================================================================
    // Step 7: 查询订单确认
    // =========================================================================
    output.push_str("【Step 7】查询当前订单\n");
    output.push_str("----------------------------------------\n");
    match rt.block_on(client.query_orders(Some(SYMBOL))) {
        Ok(orders) => {
            output.push_str(&format!("✅ 查询成功! 当前订单数: {}\n", orders.len()));
            for order in orders {
                output.push_str(&format!("  - OrderID:{} 价格:{} 数量:{} 方向:{} 状态:{}\n", 
                    order.order_id, order.price, order.volume, order.direction, order.status));
            }
        }
        Err(e) => {
            output.push_str(&format!("❌ 失败: {}\n", e));
        }
    }
    output.push_str("\n");

    // =========================================================================
    // Step 8: 撤单 (如果有未成交订单)
    // =========================================================================
    output.push_str("【Step 8】撤单 (如果有未成交订单)\n");
    output.push_str("----------------------------------------\n");
    
    let pending_orders = rt.block_on(client.query_orders(Some(SYMBOL)))?
        .into_iter()
        .filter(|o| o.status == "2" || o.status == "3") // 2=未成交, 3=部分成交
        .collect::<Vec<_>>();

    if pending_orders.is_empty() {
        output.push_str("  没有待成交订单，跳过撤单\n");
    } else {
        for order in pending_orders {
            output.push_str(&format!("  撤单 OrderID: {}\n", order.order_id));
            match rt.block_on(client.cancel_order(&order.order_id)) {
                Ok(resp) => {
                    output.push_str(&format!("  ✅ 撤单成功: {}\n", resp.order_id));
                }
                Err(e) => {
                    output.push_str(&format!("  ❌ 撤单失败: {}\n", e));
                }
            }
        }
    }
    output.push_str("\n");

    // 等待1秒
    output.push_str("等待 1 秒...\n\n");
    std::thread::sleep(Duration::from_secs(1));

    // =========================================================================
    // Step 9: 限价平空仓 (挂单价格 = 市价 + 10)
    // =========================================================================
    output.push_str("【Step 9】限价平空仓 (Close Short)\n");
    output.push_str("----------------------------------------\n");
    
    // 先获取空头持仓
    let short_position = rt.block_on(client.query_positions())?
        .into_iter()
        .find(|p| p.posi_direction == "1");

    match short_position {
        Some(pos) => {
            let trade_unit_id = &pos.trade_unit_id;
            let close_price: f64 = current_price.parse().unwrap_or(0.0) + 10.0;
            output.push_str(&format!("  TradeUnitID: {}\n", trade_unit_id));
            output.push_str(&format!("  平仓价格: {} (市价 {} + 10)\n", close_price, current_price));
            
            let volume: rust_decimal::Decimal = pos.position.parse().unwrap_or(0.0001.into());
            match rt.block_on(client.limit_close(SYMBOL, TradeDirection::Short, volume, close_price.into(), trade_unit_id)) {
                Ok(resp) => {
                    output.push_str(&format!("✅ 限价平空仓请求已发送!\n"));
                    output.push_str(&format!("  OrderID: {}\n", resp.order_id));
                    output.push_str(&format!("  订单状态: {}\n", resp.status));
                }
                Err(e) => {
                    output.push_str(&format!("❌ 失败: {}\n", e));
                }
            }
        }
        None => {
            output.push_str("❌ 未找到空头持仓，跳过平仓\n");
        }
    }
    output.push_str("\n");

    // 等待1秒
    output.push_str("等待 1 秒...\n\n");
    std::thread::sleep(Duration::from_secs(1));

    // =========================================================================
    // Step 10: 最终持仓确认
    // =========================================================================
    output.push_str("【Step 10】最终持仓确认\n");
    output.push_str("----------------------------------------\n");
    match rt.block_on(client.query_positions()) {
        Ok(positions) => {
            output.push_str(&format!("✅ 测试完成! 最终持仓数: {}\n", positions.len()));
            for pos in &positions {
                output.push_str(&format!("  - {} 方向:{} 数量:{}\n", 
                    pos.instrument_id, pos.direction, pos.volume));
            }
            if positions.is_empty() {
                output.push_str("  ✅ 所有仓位已平清!\n");
            }
        }
        Err(e) => {
            output.push_str(&format!("❌ 失败: {}\n", e));
        }
    }
    output.push_str("\n");

    // 写入文件
    let filename = "test_order_result.txt";
    let mut file = File::create(filename)?;
    file.write_all(output.as_bytes())?;
    
    println!("结果已保存到: {}", filename);
    println!("\n{}", output);

    Ok(())
}
