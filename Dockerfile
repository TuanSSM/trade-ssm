FROM rust:1.83-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY services services

# Build all binaries in one layer for cache efficiency
RUN cargo build --release --bin analyzer --bin download-data --bin backtest

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/analyzer /usr/local/bin/analyzer
COPY --from=builder /app/target/release/download-data /usr/local/bin/download-data
COPY --from=builder /app/target/release/backtest /usr/local/bin/backtest

ENV RUST_LOG=info
