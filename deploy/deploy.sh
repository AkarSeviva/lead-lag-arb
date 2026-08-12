#!/bin/bash
#
# Lead-Lag Arbitrage - VPS Deployment Script
# Usage: ./deploy/deploy.sh user@your-vps-ip
#

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
PROJECT_NAME="lead-lag-arb"
INSTALL_DIR="/opt/lead-lag-arb"
SERVICE_NAME="live-trader"
REMOTE_USER=""
REMOTE_HOST=""

usage() {
    echo "Usage: $0 <user@host>"
    echo ""
    echo "Example:"
    echo "  $0 ubuntu@123.45.67.89"
    exit 1
}

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Parse arguments
if [ $# -eq 0 ]; then
    usage
fi

REMOTE_TARGET="$1"
REMOTE_USER=$(echo "$REMOTE_TARGET" | cut -d@ -f1)
REMOTE_HOST=$(echo "$REMOTE_TARGET" | cut -d@ -f2)

if [ -z "$REMOTE_HOST" ]; then
    log_error "Invalid target format. Use user@host"
    usage
fi

log_info "Deploying to $REMOTE_TARGET"
log_info "Project: $PROJECT_NAME"
log_info "Install dir: $INSTALL_DIR"

# Check if SSH key is available
if ! ssh -o BatchMode=yes -o ConnectTimeout=5 "$REMOTE_TARGET" "exit" 2>/dev/null; then
    log_warn "SSH connection test failed. Make sure you have SSH keys set up:"
    echo "  ssh-copy-id $REMOTE_TARGET"
    read -p "Continue anyway? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        exit 1
    fi
fi

# Step 1: Create remote directory
log_info "Creating remote directories..."
ssh "$REMOTE_TARGET" "sudo mkdir -p $INSTALL_DIR && sudo chown $REMOTE_USER:$REMOTE_USER $INSTALL_DIR"

# Step 2: Sync files to remote
log_info "Syncing files to remote..."
rsync -avz --delete \
    --exclude='target/' \
    --exclude='.git/' \
    --exclude='.env' \
    --exclude='logs/' \
    --exclude='*.log' \
    ./ "$REMOTE_TARGET:$INSTALL_DIR/"

# Step 3: Install Rust on remote if needed
log_info "Checking Rust installation..."
ssh "$REMOTE_TARGET" << 'ENDSSH'
if ! command -v rustc &> /dev/null; then
    echo "Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi
ENDSSH

# Step 4: Build on remote
log_info "Building release binary (this may take a few minutes)..."
ssh "$REMOTE_TARGET" << 'ENDSSH'
cd /opt/lead-lag-arb
source "$HOME/.cargo/env" 2>/dev/null || true

# Build with release profile
cargo build --release

# Create log directory
mkdir -p logs
ENDSSH

# Step 5: Setup systemd service
log_info "Setting up systemd service..."
ssh "$REMOTE_TARGET" "sudo cp $INSTALL_DIR/deploy/live-trader.service /etc/systemd/system/"
ssh "$REMOTE_TARGET" "sudo systemctl daemon-reload"

# Step 6: Create .env from template
log_info "Checking .env configuration..."
ssh "$REMOTE_TARGET" "[ -f $INSTALL_DIR/.env ] || cp $INSTALL_DIR/.env.example $INSTALL_DIR/.env"

# Step 7: Start service
log_info "Starting service..."
ssh "$REMOTE_TARGET" "sudo systemctl enable $SERVICE_NAME"
ssh "$REMOTE_TARGET" "sudo systemctl restart $SERVICE_NAME"

# Step 8: Check status
log_info "Checking service status..."
ssh "$REMOTE_TARGET" "sudo systemctl status $SERVICE_NAME --no-pager || true"

# Step 9: Show logs
log_info "Recent logs (last 20 lines):"
ssh "$REMOTE_TARGET" "sudo journalctl -u $SERVICE_NAME -n 20 --no-pager"

log_info ""
log_info "=========================================="
log_info "Deployment complete!"
log_info "=========================================="
log_info ""
log_info "Check service status:"
log_info "  ssh $REMOTE_TARGET 'sudo systemctl status $SERVICE_NAME'"
log_info ""
log_info "View logs:"
log_info "  ssh $REMOTE_TARGET 'sudo journalctl -u $SERVICE_NAME -f'"
log_info ""
log_info "Stop service:"
log_info "  ssh $REMOTE_TARGET 'sudo systemctl stop $SERVICE_NAME'"
log_info ""
log_info "Restart service:"
log_info "  ssh $REMOTE_TARGET 'sudo systemctl restart $SERVICE_NAME'"
