.PHONY: build test lint fmt check ci run docker-build docker-up docker-down clean

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

# === Run ===

run:
	cargo run --bin analyzer

# === Docker ===

docker-build:
	docker compose build

docker-up:
	docker compose up -d

docker-down:
	docker compose down

# === Cleanup ===

clean:
	cargo clean
