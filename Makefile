.PHONY: build test lint fmt fmt-check check ci run \
       download-data backtest \
       docker-build docker-up docker-down docker-download docker-backtest \
       clean

# === Development ===

build:
	cargo build --workspace

test:
	cargo test --workspace

lint:
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

check:
	cargo check --workspace --all-targets

# === Full CI pipeline (run before commit) ===

ci: fmt-check lint test
	@echo "All checks passed"

# === Run services (local) ===

run:
	cargo run --bin analyzer

download-data:
	cargo run --bin download-data

backtest:
	@if [ -z "$(DATAFILE)" ]; then \
		echo "Usage: make backtest DATAFILE=user_data/BTCUSDT-15m-*.json"; \
		exit 1; \
	fi
	DATAFILE=$(DATAFILE) cargo run --bin backtest

# === Docker commands (freqtrade-inspired) ===

docker-build:
	docker compose build

docker-up:
	docker compose up -d analyzer

docker-down:
	docker compose down

docker-download:
	docker compose run --rm download-data

docker-backtest:
	@if [ -z "$(DATAFILE)" ]; then \
		echo "Usage: make docker-backtest DATAFILE=/app/user_data/BTCUSDT-15m-*.json"; \
		exit 1; \
	fi
	DATAFILE=$(DATAFILE) docker compose run --rm backtest

docker-logs:
	docker compose logs -f analyzer

# === Cleanup ===

clean:
	cargo clean
