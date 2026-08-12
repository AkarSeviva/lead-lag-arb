//! Persistence Module
//!
//! Tick data, order logs, and PnL persistence.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

/// Market data record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickRecord {
    pub timestamp: i64,
    pub symbol: String,
    pub exchange: String,
    pub best_bid: String,
    pub best_ask: String,
    pub spread: String,
    pub bid_vol: String,
    pub ask_vol: String,
}

/// Order log record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderLogRecord {
    pub timestamp: i64,
    pub order_id: String,
    pub symbol: String,
    pub side: String,
    pub order_type: String,
    pub price: String,
    pub volume: String,
    pub status: String,
    pub message: Option<String>,
}

/// PnL log record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PnlLogRecord {
    pub timestamp: i64,
    pub symbol: String,
    pub direction: String,
    pub entry_price: String,
    pub exit_price: String,
    pub volume: String,
    pub gross_pnl: String,
    pub net_pnl: String,
    pub exit_reason: String,
}

/// Async file writer
pub struct AsyncWriter {
    tick_file: Option<tokio::fs::File>,
    order_file: Option<tokio::fs::File>,
    pnl_file: Option<tokio::fs::File>,
}

impl AsyncWriter {
    pub async fn new(data_dir: &Path) -> Result<Self> {
        tokio::fs::create_dir_all(data_dir).await?;

        let tick_path = data_dir.join("ticks.jsonl");
        let order_path = data_dir.join("orders.jsonl");
        let pnl_path = data_dir.join("pnl.jsonl");

        let tick_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(tick_path)
            .await?;

        let order_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(order_path)
            .await?;

        let pnl_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(pnl_path)
            .await?;

        Ok(Self {
            tick_file: Some(tick_file),
            order_file: Some(order_file),
            pnl_file: Some(pnl_file),
        })
    }

    pub async fn write_tick(&mut self, record: &TickRecord) -> Result<()> {
        if let Some(ref mut file) = self.tick_file {
            let json = serde_json::to_string(record)? + "\n";
            file.write_all(json.as_bytes()).await?;
        }
        Ok(())
    }

    pub async fn write_order(&mut self, record: &OrderLogRecord) -> Result<()> {
        if let Some(ref mut file) = self.order_file {
            let json = serde_json::to_string(record)? + "\n";
            file.write_all(json.as_bytes()).await?;
        }
        Ok(())
    }

    pub async fn write_pnl(&mut self, record: &PnlLogRecord) -> Result<()> {
        if let Some(ref mut file) = self.pnl_file {
            let json = serde_json::to_string(record)? + "\n";
            file.write_all(json.as_bytes()).await?;
        }
        Ok(())
    }
}
