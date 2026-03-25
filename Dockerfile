FROM rust:1.85-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifests first for better layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY services services

# Build all binaries in one layer for cache efficiency
RUN cargo build --release --bin analyzer --bin download-data --bin backtest --bin rl-backtest

FROM debian:bookworm-slim

LABEL org.opencontainers.image.source="https://github.com/TuanSSM/trade-ssm"
LABEL org.opencontainers.image.description="Professional Rust crypto trading suite"
LABEL org.opencontainers.image.licenses="MIT"

RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/analyzer /usr/local/bin/analyzer
COPY --from=builder /app/target/release/download-data /usr/local/bin/download-data
COPY --from=builder /app/target/release/backtest /usr/local/bin/backtest
COPY --from=builder /app/target/release/rl-backtest /usr/local/bin/rl-backtest

RUN mkdir -p /app/user_data
WORKDIR /app

ENV RUST_LOG=info

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD test -f /proc/1/status || exit 1
