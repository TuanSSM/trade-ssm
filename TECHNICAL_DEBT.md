# Technical Debt Remediation Plan — trade-ssm

## Context

The trade-ssm codebase is a professional Rust crypto trading suite that has grown across 13 crates and 8 services. While the core architecture is sound (proper Decimal usage, clean crate separation, anti-repainting awareness), significant technical debt has accumulated across testing, error handling, resilience, and completeness. This plan catalogs all identified debt and provides a phased implementation roadmap. Correctness and reliability are paramount for a trading system.

---

## Technical Debt Inventory

### TD-01: Missing Test Coverage (Critical)

70+ files lack tests. For a trading system, this is the highest-risk debt.

**ssm-indicators** (Critical — anti-repainting rules require append-one-candle tests):
- [x] `atr.rs` — add unit tests + append-one-candle anti-repainting test
- [x] `bollinger.rs` — add unit tests + append-one-candle anti-repainting test
- [x] `liquidations.rs` — add unit tests + append-one-candle anti-repainting test
- [x] `macd.rs` — add unit tests + append-one-candle anti-repainting test
- [x] `obv.rs` — add unit tests + append-one-candle anti-repainting test
- [x] `plot.rs` — add unit tests
- [x] `rsi.rs` — add unit tests + append-one-candle anti-repainting test
- [x] `vwap.rs` — add unit tests + append-one-candle anti-repainting test

**ssm-orderflow** (Critical — orderflow signals drive trading decisions):
- [x] `absorption.rs` — add unit tests
- [x] `delta.rs` — add unit tests
- [x] `footprint.rs` — add unit tests
- [x] `imbalance.rs` — add unit tests
- [x] `market_profile.rs` — add unit tests
- [x] `sweep.rs` — add unit tests

**ssm-ai** (High — RL training correctness unverified):
- [x] `config.rs` — add unit tests
- [x] `continuous_env.rs` — add unit tests
- [x] `correlated_features.rs` — add unit tests
- [x] `edge.rs` — add unit tests
- [x] `env.rs` — add unit tests
- [x] `episode_sampler.rs` — add unit tests
- [x] `features.rs` — add unit tests
- [x] `hyperopt.rs` — add unit tests
- [x] `metrics.rs` — add unit tests
- [x] `model.rs` — add unit tests
- [x] `model_manager.rs` — add unit tests
- [x] `multi_feature.rs` — add unit tests
- [x] `multi_timeframe.rs` — add unit tests
- [x] `normalize.rs` — add unit tests
- [x] `optimizer.rs` — add unit tests
- [x] `outlier.rs` — add unit tests
- [x] `ppo.rs` — add unit tests
- [x] `replay_buffer.rs` — add unit tests
- [x] `reward.rs` — add unit tests
- [x] `trainer.rs` — add unit tests
- [x] `vectorized_env.rs` — add unit tests

**ssm-strategy** (High — strategy logic untested):
- [x] `ai_strategy.rs` — add unit tests
- [x] `cvd_momentum.rs` — add unit tests
- [x] `composite.rs` — add unit tests
- [x] `orderflow_strategy.rs` — add unit tests

**ssm-execution** (High — money-critical paths):
- [x] `position_tracker.rs` — add unit tests
- [x] `backtest.rs` — add unit tests
- [x] `protections.rs` — add unit tests

**ssm-exchange** (Medium):
- [x] `websocket.rs` — add unit tests
- [x] `bybit.rs` — add unit tests
- [x] `history.rs` — add unit tests
- [x] `pairlist.rs` — add unit tests

**ssm-notify** (Low):
- [x] `telegram.rs` — add unit tests
- [x] `telegram_bot.rs` — add unit tests
- [x] `webhook.rs` — add unit tests

**services/** (Medium — no integration tests):
- [x] `analyzer` — add integration tests
- [x] `backtest` — add integration tests
- [x] `download-data` — add integration tests
- [x] `data-feed` — add integration tests
- [x] `signal` — add integration tests
- [x] `execution` — add integration tests
- [x] `rl-backtest` — add integration tests

### TD-02: Production Panic Points (Critical)

4 strategies call `.last().unwrap()` on candles in production `analyze()` methods:

- [x] `crates/ssm-strategy/src/cvd_momentum.rs:66` — replace `candles.last().unwrap()` with safe match
- [x] `crates/ssm-strategy/src/ai_strategy.rs:53` — replace `candles.last().unwrap()` with safe match
- [x] `crates/ssm-strategy/src/composite.rs:89` — replace `candles.last().unwrap()` with safe match
- [x] `crates/ssm-strategy/src/orderflow_strategy.rs:116` — replace `candles.last().unwrap()` with safe match
- [x] `crates/ssm-execution/src/live.rs:510` — replace `HmacSha256::new_from_slice().expect()` with `Result` + `?`

### TD-03: Domain Error Types Missing (High)

Library crates use `anyhow` instead of proper domain errors. This makes error matching impossible for callers.

- [x] Create `crates/ssm-exchange/src/error.rs` — `ExchangeError { ApiError, ExchangeApiError, ParseError, Unimplemented, UnknownExchange, Network }`
  - [x] Replace `binance.rs` (4x `anyhow::bail!()`) with `ExchangeError`
  - [x] Replace `bybit.rs` (6x `anyhow::bail!()`) with `ExchangeError`
  - [x] Replace `exchange_trait.rs` (1x `anyhow::bail!()`) with `ExchangeError`
- [x] Create `crates/ssm-execution/src/error.rs` — `ExecutionError { NeutralAction, NoLiveEngine, PreflightFailed, OrderFailed, SigningError }`
  - [x] Replace `engine.rs` (9x `anyhow::bail!()`/`anyhow!()`) with `ExecutionError`
- [x] Create `crates/ssm-strategy/src/error.rs` — `StrategyError { InsufficientData, AnalysisFailed, PredictionFailed }`
  - [x] Add tests for all error type variants

### TD-04: Network Resilience (High)

- [x] `crates/ssm-exchange/src/binance.rs` — set default timeout on `reqwest::Client` builder (30s)
- [x] `crates/ssm-execution/src/live.rs` — set default timeout on `reqwest::Client` builder (30s)
- [x] `crates/ssm-execution/src/live.rs` — implement exponential backoff with jitter in retry loop
- [x] `services/data-feed/src/main.rs` — properly await/join spawned WebSocket task handle

### TD-05: Scattered Configuration (Medium)

8 services independently parse env vars with duplicated defaults. Create shared config module.

- [x] Create shared config module in `ssm-core/src/config.rs` with `ServiceConfig`, `env_or()`, `env_parse()`, `interval_to_ms()`
- [x] Migrate `services/analyzer/src/main.rs` to centralized config
- [x] Migrate `services/download-data/src/main.rs` to centralized config
- [x] Migrate `services/backtest/src/main.rs` to centralized config
- [x] Migrate `services/data-feed/src/main.rs` to centralized config
- [x] Migrate `services/signal/src/main.rs` to centralized config
- [x] Migrate `services/execution/src/main.rs` to centralized config

### TD-06: Incomplete TODO Items (Medium)

- [x] Trailing stop fill logic in PaperEngine (`crates/ssm-execution/src/paper.rs` — tracks best price per order, triggers on callback rate retrace)
- [ ] TODO-001: Live Exchange Execution tests (`crates/ssm-execution/src/engine.rs:641`) — requires live exchange credentials
- [ ] TODO-002: Dynamic stoploss & take-profit (`crates/ssm-strategy/src/traits.rs:48`) — deferred to future sprint
- [ ] TODO-003: Trade lifecycle callbacks (`crates/ssm-strategy/src/traits.rs:21`) — deferred to future sprint
- [ ] TODO-005: Trade lifecycle types (`crates/ssm-core/src/types.rs:261`) — deferred to future sprint
- [ ] `list_pairs` implementation for Binance (`crates/ssm-exchange/src/binance.rs:152`) — requires API integration
- [ ] `list_pairs` implementation for Bybit (`crates/ssm-exchange/src/bybit.rs:228`) — requires API integration

### TD-07: Architecture Issues (Medium)

- [x] Add compile-time anti-repainting: created `ClosedCandles` newtype wrapper in `ssm-core/src/types.rs`
- [ ] Fix crate boundary violation: `ssm-strategy` depends on `ssm-ai` — needs architectural decision
- [ ] Unify duplicate signal paths: analyzer calls indicators directly vs signal service uses Strategy via NATS
- [ ] Integrate leverage module: `leverage.rs` exists but `LiveEngine.submit_order()` has no leverage param
- [ ] Version NATS schema: topics use flat naming without schema validation

### TD-08: Docker/CI Gaps (Medium)

- [x] Update `Dockerfile` to build all 8 service binaries (analyzer, download-data, backtest, rl-backtest, rl-trainer, data-feed, signal-service, execution-service)
- [x] Add `deny.toml` for supply chain security (`cargo-deny`)
- [x] Add `[workspace.lints.clippy]` configuration to root `Cargo.toml` with pedantic + sensible allows

### TD-09: Code Quality (Low)

- [x] Remove hardcoded `"BTCUSDT"` in `cvd_momentum.rs` — added `with_symbol()` builder method
- [ ] Reduce excessive `.clone()` calls (~45) — use references/Cow in hot paths
- [ ] Split large files: `types.rs` (1520L), `env.rs` (1377L), `backtest.rs` (956L), `engine.rs` (864L), `ppo.rs` (832L)
- [ ] Add `///` doc comments to 100+ public API items lacking documentation
- [ ] Create test fixture builders to eliminate repetitive `Decimal::from_str().unwrap()` in tests

---

## Implementation Plan

### Phase 1: Safety & Correctness (Week 1) ✅

**Goal:** Eliminate production panic paths and add critical indicator tests.

- [x] **1. Fix production unwraps** (TD-02)
  - [x] Replace 4x `candles.last().unwrap()` with safe match in all strategy files
  - [x] Replace `expect()` with `Result` + `?` in `live.rs` sign method
  - Files: `crates/ssm-strategy/src/{cvd_momentum,ai_strategy,composite,orderflow_strategy}.rs`, `crates/ssm-execution/src/live.rs`

- [x] **2. Add anti-repainting tests for all indicators** (TD-01 partial)
  - [x] `atr.rs` — append-one-candle test
  - [x] `bollinger.rs` — append-one-candle test
  - [x] `rsi.rs` — append-one-candle test
  - [x] `macd.rs` — append-one-candle test
  - [x] `ema.rs` — append-one-candle test
  - [x] `obv.rs` — append-one-candle test
  - [x] `vwap.rs` — append-one-candle test
  - [x] `liquidations.rs` — append-one-candle test
  - Verify: values at `[0..N]` don't change when candle `N+1` is added

- [x] **3. Add orderflow module tests** (TD-01 partial)
  - [x] `delta.rs` — unit tests
  - [x] `footprint.rs` — unit tests
  - [x] `imbalance.rs` — unit tests
  - [x] `absorption.rs` — unit tests
  - [x] `sweep.rs` — unit tests
  - [x] `market_profile.rs` — unit tests

### Phase 2: Error Handling & Resilience (Week 2) ✅

**Goal:** Replace anyhow in libraries with domain errors; add network resilience.

- [x] **4. Define domain error types** (TD-03)
  - [x] Create `ExchangeError` enum and migrate ssm-exchange
  - [x] Create `ExecutionError` enum and migrate ssm-execution
  - [x] Create `StrategyError` enum and migrate ssm-strategy

- [x] **5. Add network timeouts** (TD-04)
  - [x] Configure `reqwest::Client` with default timeout (30s) in BinanceClient
  - [x] Configure `reqwest::Client` with default timeout (30s) in LiveEngine
  - [x] Implement exponential backoff with jitter in LiveEngine retry loop
  - [x] Fix spawned task join in data-feed service

### Phase 3: Strategy & Execution Tests (Week 3) ✅

**Goal:** Test all strategy implementations and critical execution paths.

- [x] **6. Strategy tests** (TD-01 partial)
  - [x] `cvd_momentum.rs` — unit tests
  - [x] `ai_strategy.rs` — unit tests
  - [x] `composite.rs` — unit tests
  - [x] `orderflow_strategy.rs` — unit tests

- [x] **7. Execution tests** (TD-01 partial)
  - [x] `position_tracker.rs` — unit tests
  - [x] `backtest.rs` — unit tests
  - [x] `protections.rs` — unit tests
  - [x] Implement trailing stop fill logic in PaperEngine (TD-06)

### Phase 4: AI/ML Tests & Config (Week 4) ✅

**Goal:** Test AI modules and centralize configuration.

- [x] **8. AI module tests** (TD-01 partial, all 21 modules)
  - [x] `features.rs` — unit tests
  - [x] `env.rs` — unit tests
  - [x] `ppo.rs` — unit tests
  - [x] `reward.rs` — unit tests
  - [x] `normalize.rs` — unit tests
  - [x] `model.rs` — unit tests
  - [x] `config.rs` — unit tests
  - [x] `continuous_env.rs` — unit tests
  - [x] `correlated_features.rs` — unit tests
  - [x] `edge.rs` — unit tests
  - [x] `episode_sampler.rs` — unit tests
  - [x] `hyperopt.rs` — unit tests
  - [x] `metrics.rs` — unit tests
  - [x] `model_manager.rs` — unit tests
  - [x] `multi_feature.rs` — unit tests
  - [x] `multi_timeframe.rs` — unit tests
  - [x] `optimizer.rs` — unit tests
  - [x] `outlier.rs` — unit tests
  - [x] `replay_buffer.rs` — unit tests
  - [x] `trainer.rs` — unit tests
  - [x] `vectorized_env.rs` — unit tests

- [x] **9. Centralize configuration** (TD-05)
  - [x] Create shared config structs with validation in `ssm-core/src/config.rs`
  - [x] Migrate all services to use centralized config

### Phase 5: Architecture & CI (Week 5+) — Partial

**Goal:** Fix structural issues and improve CI.

- [x] **11. Add compile-time anti-repainting** (TD-07)
  - [x] Create `ClosedCandles` newtype wrapper

- [x] **12. Docker & CI improvements** (TD-08)
  - [x] Update Dockerfile to build all service binaries
  - [x] Add `deny.toml` for supply chain security
  - [x] Add workspace lints to root `Cargo.toml`

- [ ] **10. Fix crate boundary violation** (TD-07) — deferred: requires architectural decision
- [ ] **13. Resolve remaining TODOs** (TD-06) — deferred: requires live exchange integration or design work

---

## Verification

- [x] `cargo fmt --check` passes
- [x] `cargo clippy --workspace` passes (0 warnings)
- [x] `cargo test --workspace` passes (1,047 tests, 0 failures)
- [ ] `just docker-build` — Docker builds successfully (requires Docker runtime)
- [ ] `just backtest user_data/<sample>.json` — backtest completes (requires sample data)

## Summary of Changes

| Category | Items | Completed | Deferred |
|----------|-------|-----------|----------|
| TD-01: Test Coverage | 70+ modules | 70+ | 0 |
| TD-02: Panic Points | 5 | 5 | 0 |
| TD-03: Domain Errors | 3 error types | 3 | 0 |
| TD-04: Network Resilience | 4 items | 4 | 0 |
| TD-05: Config Centralization | 7 services | 7 | 0 |
| TD-06: TODO Items | 7 items | 1 | 6 (require live APIs/design) |
| TD-07: Architecture | 5 items | 1 | 4 (require design decisions) |
| TD-08: Docker/CI | 3 items | 3 | 0 |
| TD-09: Code Quality | 5 items | 1 | 4 (low priority) |

**Total: 93+ items completed, 14 deferred (require external dependencies, live APIs, or architectural decisions).**
