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

# Run RL backtest with trained model
rl-model-backtest datafile model_path="models/table_model_best.json":
    DATAFILE={{ datafile }} RL_MODE=model MODEL_PATH={{ model_path }} cargo run --bin rl-backtest

# === Docker RL (training & deployment) ===

# Start RL training pipeline (data-feed + NATS + rl-trainer)
docker-rl-train:
    docker compose --profile rl up -d

# Deploy RL model for paper trading (data-feed + signal + execution)
docker-rl-deploy-paper:
    EXECUTION_MODE=paper STRATEGY_MODE=ai docker compose --profile deploy up -d

# Deploy RL model for live trading (data-feed + signal + execution)
docker-rl-deploy-live:
    EXECUTION_MODE=live STRATEGY_MODE=ai docker compose --profile deploy up -d

# Run RL backtest via Docker
docker-rl-backtest datafile:
    DATAFILE={{ datafile }} docker compose run --rm rl-backtest

# Tail RL training logs
docker-rl-logs:
    docker compose logs -f rl-trainer

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

# === Docker Integration Tests ===

# Run Docker integration tests locally (builds + validates image)
docker-integration-test:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Building Docker image..."
    docker build -t trade-ssm:integration-test .
    echo ""
    echo "=== Binary Verification ==="
    for bin in analyzer download-data backtest rl-backtest rl-trainer data-feed signal-service execution-service; do
        if docker run --rm trade-ssm:integration-test sh -c "test -x /usr/local/bin/$bin"; then
            echo "  ✓ $bin"
        else
            echo "  ✗ $bin MISSING" && exit 1
        fi
    done
    echo ""
    echo "=== Runtime Checks ==="
    docker run --rm trade-ssm:integration-test sh -c "test -d /app/user_data && echo '  ✓ /app/user_data exists'"
    docker run --rm trade-ssm:integration-test sh -c "test -d /etc/ssl/certs && echo '  ✓ SSL certificates present'"
    echo ""
    echo "=== Compose Validation ==="
    docker compose config --quiet && echo "  ✓ docker-compose.yml valid"
    echo ""
    echo "=== Image Size ==="
    docker images trade-ssm:integration-test --format "  {{.Size}}"
    echo ""
    echo "All Docker integration tests passed"

# Validate docker-compose.yml syntax
docker-validate:
    docker compose config --quiet && echo "docker-compose.yml is valid"

# === Cleanup ===

# Remove build artifacts
clean:
    cargo clean
