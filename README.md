# Lead-Lag Arbitrage System

A Rust implementation of a lead-lag latency arbitrage strategy for cryptocurrency markets.

## Architecture

```
lead-lag-arb/
├── Cargo.toml                 # Workspace definition
├── crates/
│   ├── config/               # Strategy configuration
│   ├── exchange-adapter-lbank/ # LBank API adapter
│   ├── md-gateway/           # Market data gateway (WebSocket)
│   ├── clock-sync/           # Clock synchronization
│   ├── orderbook/            # Order book reconstruction
│   ├── signal-engine/        # Arbitrage signal calculation
│   ├── risk-gate/            # Pre-trade risk management
│   ├── execution/            # Order execution engine
│   ├── fee-pnl/              # Fee and PnL calculation
│   ├── persistence/          # Data persistence
│   └── metrics/              # Prometheus metrics
├── apps/
│   └── live-trader/          # Main trading application
└── deploy/                   # Deployment scripts
```

## Modules

### config
Strategy parameters, entry/exit thresholds, risk limits.

### exchange-adapter-lbank
- REST API client with HMAC-SHA256 authentication
- WebSocket client for real-time data
- Order management and position tracking

### md-gateway
- Multi-exchange WebSocket connection management
- Automatic reconnection with exponential backoff
- Message routing and normalization

### clock-sync
- NTP-like clock synchronization with exchanges
- Latency measurement and statistics

### orderbook
- L2 order book reconstruction
- Price level aggregation
- Spread and imbalance calculation

### signal-engine
- Lead-lag spread calculation
- Entry filter chain (duration, depth, volatility, cooldown)
- Signal state machine

### risk-gate
- Position limits and exposure checks
- Circuit breaker for consecutive losses
- API rate limiting

### execution
- Order state machine
- TP/SL/timeout exit handling
- Position monitoring

## Quick Start

### Prerequisites

- Rust 1.86+
- Internet connection to exchange APIs

### Build

```bash
git clone https://github.com/YOUR_USERNAME/lead-lag-arb.git
cd lead-lag-arb
cargo build --release
```

### Configure

1. Copy the environment template:
```bash
cp .env.example .env
```

2. Edit `.env` with your API keys:
```bash
# Binance
BINANCE_API_KEY=your_binance_key_here
BINANCE_SECRET_KEY=your_binance_secret_here

# Lbank
LBANK_API_KEY=your_lbank_key_here
LBANK_SECRET_KEY=your_lbank_secret_here
```

3. Edit `config/strategy.toml` with your strategy parameters.

### Run

```bash
# Single symbol
cargo run --bin live-trader -- --symbol BTCUSDT

# Or with release build
./target/release/live-trader --symbol BTCUSDT
```

## VPS Deployment

### Option 1: Direct Deployment

```bash
# 1. Connect to your VPS
ssh user@your-vps-ip

# 2. Install Rust (if not installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 3. Clone the repository
git clone https://github.com/YOUR_USERNAME/lead-lag-arb.git
cd lead-lag-arb

# 4. Build release
cargo build --release

# 5. Configure environment
cp .env.example .env
nano .env  # Add your API keys

# 6. Create systemd service
sudo cp deploy/live-trader.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable live-trader

# 7. Start the service
sudo systemctl start live-trader
sudo journalctl -u live-trader -f  # View logs
```

### Option 2: Using Deployment Script

```bash
# From your local machine
./deploy/deploy.sh user@your-vps-ip

# The script will:
# 1. Copy the project to VPS
# 2. Build the release binary
# 3. Setup systemd service
# 4. Start and monitor the service
```

### Option 3: Docker Deployment

```bash
# Build image
docker build -t lead-lag-arb .

# Run container
docker run -d \
  --name lead-lag-arb \
  --env-file .env \
  -v $(pwd)/config:/app/config \
  -v $(pwd)/logs:/app/logs \
  lead-lag-arb
```

## Systemd Service Configuration

The service runs as a background daemon with automatic restart:

```ini
[Unit]
Description=Lead-Lag Arbitrage Trader
After=network.target

[Service]
Type=simple
User=ubuntu
WorkingDirectory=/opt/lead-lag-arb
Environment=RUST_LOG=info
ExecStart=/opt/lead-lag-arb/target/release/live-trader --symbol BTCUSDT
Restart=always
RestartSec=10

# Security
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true

[Install]
WantedBy=multi-user.target
```

## Monitoring

### View Logs

```bash
# Systemd journal
sudo journalctl -u live-trader -f

# Or from the project directory
tail -f logs/trader.log
```

### Prometheus Metrics

Metrics are exposed at `http://localhost:9090/metrics`:

- `arb_signals_total` - Total arbitrage signals generated
- `arb_trades_total` - Total trades executed
- `arb_pnl_total` - Cumulative PnL
- `arb_positions_active` - Current active positions
- `arb_latency_ms` - Order execution latency histogram

### Health Check

```bash
curl http://localhost:9090/health
```

## Configuration

Edit `config/strategy.toml`:

```toml
# Entry/Exit Parameters
entry_threshold = 0.001      # 0.1% spread threshold
tp_ratio = 1.0              # Take profit at full convergence
sl_ratio = 1.0              # Stop loss at double spread
max_holding_secs = 30       # Maximum position holding time

# Filters
[filters]
min_spread_duration_ms = 100
min_leader_depth_usd = 100000
max_volatility = 0.02

# Risk Management
[risk]
max_concurrent_positions = 3
max_position_usd = 1000
circuit_breaker_losses = 5

# Network
[network]
proxy_enabled = false
proxy_url = "http://127.0.0.1:7890"
```

## Development

```bash
# Run all tests
cargo test --workspace

# Run with coverage
cargo test --workspace --coverage

# Format code
cargo fmt

# Lint
cargo clippy --workspace -- -D warnings
```

## Security Notes

1. **Never commit API keys** - Use `.env` file (already in `.gitignore`)
2. **Use read-only API keys** when possible for market data
3. **Enable 2FA** on your exchange accounts
4. **Start with small position sizes** for testing
5. **Monitor the circuit breaker** - It pauses trading after consecutive losses

## License

MIT License - see LICENSE file for details.

## Disclaimer

This software is for educational purposes. Cryptocurrency trading involves substantial risk of loss. Use at your own risk.
