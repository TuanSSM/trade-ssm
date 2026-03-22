set dotenv-load

# List available recipes
default:
    @just --list --unsorted

# === Development ===

# Build the entire workspace
build:
    cargo build --workspace

# Run all workspace tests
test:
    cargo test --workspace

# Run clippy lints (deny warnings)
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Auto-format all code
fmt:
    cargo fmt --all

# Check formatting without modifying
fmt-check:
    cargo fmt --all -- --check

# Type-check all targets
check:
    cargo check --workspace --all-targets

# === CI pipeline (run before commit) ===

# Run full CI: format check + clippy + tests
ci: fmt-check lint test
    @echo "All checks passed"

# === Run services ===

# Start the live analyzer
run:
    cargo run --bin analyzer

# Download historical candle data
download-data:
    cargo run --bin download-data

# Run backtest on historical data
backtest datafile:
    DATAFILE={{ datafile }} cargo run --bin backtest

# Run RL backtest on historical data
rl-backtest datafile:
    DATAFILE={{ datafile }} cargo run --bin rl-backtest

# Run RL hyperparameter optimization
rl-optimize datafile config="config/rl-default.toml":
    DATAFILE={{ datafile }} RL_MODE=optimize RL_CONFIG={{ config }} cargo run --bin rl-backtest

# Run RL multi-timeframe comparison
rl-multi-tf datafile config="config/rl-default.toml":
    DATAFILE={{ datafile }} RL_MODE=multi_tf RL_CONFIG={{ config }} cargo run --bin rl-backtest

# === Docker (freqtrade-inspired) ===

# Build all Docker images
docker-build:
    docker compose build

# Start the live analyzer container
docker-up:
    docker compose up -d analyzer

# Stop all containers
docker-down:
    docker compose down

# Download historical data via Docker
docker-download:
    docker compose run --rm download-data

# Run backtest via Docker
docker-backtest datafile:
    DATAFILE={{ datafile }} docker compose run --rm backtest

# Tail analyzer logs
docker-logs:
    docker compose logs -f analyzer

# === Cleanup ===

# Remove build artifacts
clean:
    cargo clean
