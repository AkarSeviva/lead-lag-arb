#!/bin/bash
#
# Local Build Script for Lead-Lag Arbitrage
#

set -e

echo "=== Lead-Lag Arbitrage Build Script ==="
echo ""

# Check Rust installation
if ! command -v rustc &> /dev/null; then
    echo "Rust not found. Installing..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    source "$HOME/.cargo/env"
fi

# Ensure stable toolchain
rustup default stable

# Build release
echo ""
echo "Building release binary..."
cargo build --release

# Check binary
BINARY="./target/release/live-trader"
if [ -f "$BINARY" ]; then
    echo ""
    echo "Build successful!"
    SIZE=$(du -h "$BINARY" | cut -f1)
    echo "Binary size: $SIZE"
    echo ""
    echo "Run with:"
    echo "  $BINARY --symbol BTCUSDT"
else
    echo "Build failed!"
    exit 1
fi
