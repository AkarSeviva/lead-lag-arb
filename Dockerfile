FROM rust:1.86-slim-bookworm as builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./
COPY crates crates/
COPY apps apps/

# Build dependencies first (for caching)
RUN mkdir -p src && echo "fn main() {}" > src/main.rs
RUN cargo build --release --bin live-trader 2>/dev/null || true
RUN rm -rf src

# Copy actual source
COPY . .

# Build release binary
RUN touch src/main.rs && cargo build --release --bin live-trader
RUN strip target/release/live-trader

# Runtime image
FROM debian:bookworm-slim

# Install runtime dependencies only
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -r trader && useradd -r -g trader trader

WORKDIR /app

# Copy binary from builder
COPY --from=builder /build/target/release/live-trader /app/
COPY --from=builder /build/.env.example /app/.env.example
COPY --from=builder /build/config /app/config 2>/dev/null || true

# Create directories
RUN mkdir -p /app/logs && chown -R trader:trader /app

# Switch to non-root user
USER trader

# Environment
ENV RUST_LOG=info
ENV RUST_BACKTRACE=1

# Expose metrics port
EXPOSE 9090

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:9090/health || exit 1

ENTRYPOINT ["/app/live-trader"]
