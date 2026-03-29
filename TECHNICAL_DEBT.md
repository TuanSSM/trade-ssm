# Technical Debt Remediation Plan — trade-ssm

## Context

The trade-ssm codebase is a professional Rust crypto trading suite that has grown across 13 crates and 8 services. While the core architecture is sound (proper Decimal usage, clean crate separation, anti-repainting awareness), significant technical debt has accumulated across testing, error handling, resilience, and completeness. This plan catalogs all identified debt and provides a phased implementation roadmap. Correctness and reliability are paramount for a trading system.

---

## Technical Debt Inventory

### TD-01: Missing Test Coverage (Critical)

70+ files lack tests. For a trading system, this is the highest-risk debt.

**ssm-indicators** (Critical — anti-repainting rules require append-one-candle tests):
- [ ] `atr.rs` — add unit tests + append-one-candle anti-repainting test
- [ ] `bollinger.rs` — add unit tests + append-one-candle anti-repainting test
- [ ] `liquidations.rs` — add unit tests + append-one-candle anti-repainting test
- [ ] `macd.rs` — add unit tests + append-one-candle anti-repainting test
- [ ] `obv.rs` — add unit tests + append-one-candle anti-repainting test
- [ ] `plot.rs` — add unit tests
- [ ] `rsi.rs` — add unit tests + append-one-candle anti-repainting test
- [ ] `vwap.rs` — add unit tests + append-one-candle anti-repainting test

**ssm-orderflow** (Critical — orderflow signals drive trading decisions):
- [ ] `absorption.rs` — add unit tests
- [ ] `delta.rs` — add unit tests
- [ ] `footprint.rs` — add unit tests
- [ ] `imbalance.rs` — add unit tests
- [ ] `market_profile.rs` — add unit tests
- [ ] `sweep.rs` — add unit tests

**ssm-ai** (High — RL training correctness unverified):
- [ ] `config.rs` — add unit tests
- [ ] `continuous_env.rs` — add unit tests
- [ ] `correlated_features.rs` — add unit tests
- [ ] `edge.rs` — add unit tests
- [ ] `env.rs` — add unit tests
- [ ] `episode_sampler.rs` — add unit tests
- [ ] `features.rs` — add unit tests
- [ ] `hyperopt.rs` — add unit tests
- [ ] `metrics.rs` — add unit tests
- [ ] `model.rs` — add unit tests
- [ ] `model_manager.rs` — add unit tests
- [ ] `multi_feature.rs` — add unit tests
- [ ] `multi_timeframe.rs` — add unit tests
- [ ] `normalize.rs` — add unit tests
- [ ] `optimizer.rs` — add unit tests
- [ ] `outlier.rs` — add unit tests
- [ ] `ppo.rs` — add unit tests
- [ ] `replay_buffer.rs` — add unit tests
- [ ] `reward.rs` — add unit tests
- [ ] `trainer.rs` — add unit tests
- [ ] `vectorized_env.rs` — add unit tests

**ssm-strategy** (High — strategy logic untested):
- [ ] `ai_strategy.rs` — add unit tests
- [ ] `cvd_momentum.rs` — add unit tests
- [ ] `composite.rs` — add unit tests
- [ ] `orderflow_strategy.rs` — add unit tests

**ssm-execution** (High — money-critical paths):
- [ ] `position_tracker.rs` — add unit tests
- [ ] `backtest.rs` — add unit tests
- [ ] `protections.rs` — add unit tests

**ssm-exchange** (Medium):
- [ ] `websocket.rs` — add unit tests
- [ ] `bybit.rs` — add unit tests
- [ ] `history.rs` — add unit tests
- [ ] `pairlist.rs` — add unit tests

**ssm-notify** (Low):
- [ ] `telegram.rs` — add unit tests
- [ ] `telegram_bot.rs` — add unit tests
- [ ] `webhook.rs` — add unit tests

**services/** (Medium — no integration tests):
- [ ] `analyzer` — add integration tests
- [ ] `backtest` — add integration tests
- [ ] `download-data` — add integration tests
- [ ] `data-feed` — add integration tests
- [ ] `signal` — add integration tests
- [ ] `execution` — add integration tests
- [ ] `rl-backtest` — add integration tests

### TD-02: Production Panic Points (Critical)

4 strategies call `.last().unwrap()` on candles in production `analyze()` methods:

- [ ] `crates/ssm-strategy/src/cvd_momentum.rs:66` — replace `candles.last().unwrap()` with safe alternative
- [ ] `crates/ssm-strategy/src/ai_strategy.rs:53` — replace `candles.last().unwrap()` with safe alternative
- [ ] `crates/ssm-strategy/src/composite.rs:89` — replace `candles.last().unwrap()` with safe alternative
- [ ] `crates/ssm-strategy/src/orderflow_strategy.rs:116` — replace `candles.last().unwrap()` with safe alternative
- [ ] `crates/ssm-execution/src/live.rs:510` — replace `HmacSha256::new_from_slice().expect()` with `?`

**Fix:** Replace with `.ok_or_else(|| anyhow!("empty candles"))?` or guard with early return.

### TD-03: Domain Error Types Missing (High)

Library crates use `anyhow` instead of proper domain errors. This makes error matching impossible for callers.

- [ ] Create `crates/ssm-exchange/src/error.rs` — `ExchangeError { ApiError, Timeout, Unimplemented, UnknownExchange }`
  - [ ] Replace `binance.rs` (4x `anyhow::bail!()`) with `ExchangeError`
  - [ ] Replace `bybit.rs` (6x `anyhow::bail!()`) with `ExchangeError`
  - [ ] Replace `exchange_trait.rs` (1x `anyhow::bail!()`) with `ExchangeError`
- [ ] Create `crates/ssm-execution/src/error.rs` — `ExecutionError { InvalidOrder, NoLiveEngine, PositionConflict }`
  - [ ] Replace `engine.rs` (9x `anyhow::bail!()`/`anyhow!()`) with `ExecutionError`
- [ ] Create `crates/ssm-strategy/src/error.rs` — `StrategyError { InsufficientData, AnalysisFailed }`
  - [ ] Replace anyhow usage in strategy trait impls with `StrategyError`

### TD-04: Network Resilience (High)

- [ ] `crates/ssm-exchange/src/binance.rs` — set default timeout on `reqwest::Client` builder (30s)
- [ ] `crates/ssm-execution/src/live.rs` — set default timeout on `reqwest::Client` builder (30s)
- [ ] `crates/ssm-execution/src/live.rs` — implement exponential backoff with jitter in retry loop (currently fixed delay)
- [ ] `services/data-feed/src/main.rs` — properly await/join spawned WebSocket task handle

### TD-05: Scattered Configuration (Medium)

8 services independently parse env vars with duplicated defaults. Create shared config module.

- [ ] Create shared config struct in `ssm-core` or new `ssm-config` crate with validation
- [ ] Migrate `services/analyzer/src/main.rs` to centralized config
- [ ] Migrate `services/download-data/src/main.rs` to centralized config
- [ ] Migrate `services/backtest/src/main.rs` to centralized config
- [ ] Migrate `services/rl-backtest/src/main.rs` to centralized config
- [ ] Migrate `services/data-feed/src/main.rs` to centralized config
- [ ] Migrate `services/signal/src/main.rs` to centralized config
- [ ] Migrate `services/execution/src/main.rs` to centralized config

### TD-06: Incomplete TODO Items (Medium)

- [ ] TODO-001: Live Exchange Execution tests (`crates/ssm-execution/src/engine.rs:641`)
- [ ] TODO-002: Dynamic stoploss & take-profit (`crates/ssm-strategy/src/traits.rs:48`)
- [ ] TODO-003: Trade lifecycle callbacks (`crates/ssm-strategy/src/traits.rs:21`)
- [ ] TODO-005: Trade lifecycle types (`crates/ssm-core/src/types.rs:261`)
- [ ] Trailing stop fill logic in PaperEngine (`crates/ssm-execution/src/paper.rs` — currently returns `Open` but never fills)
- [ ] `list_pairs` implementation for Binance (`crates/ssm-exchange/src/binance.rs:152`)
- [ ] `list_pairs` implementation for Bybit (`crates/ssm-exchange/src/bybit.rs:228`)

### TD-07: Architecture Issues (Medium)

- [ ] Fix crate boundary violation: `ssm-strategy` depends on `ssm-ai` — violates documented dependency graph
- [ ] Add compile-time anti-repainting: create `ClosedCandles` newtype wrapper enforcing last-candle-excluded rule
- [ ] Unify duplicate signal paths: analyzer calls indicators directly vs signal service uses Strategy via NATS
- [ ] Integrate leverage module: `leverage.rs` exists but `LiveEngine.submit_order()` has no leverage param
- [ ] Version NATS schema: topics use flat naming without schema validation

### TD-08: Docker/CI Gaps (Medium)

- [ ] Update `Dockerfile` to build all service binaries (currently only 4: analyzer, download-data, backtest, rl-backtest)
- [ ] Update `docker-compose.yml` to match Dockerfile binary list
- [ ] Add `deny.toml` for supply chain security (`cargo-deny`)
- [ ] Add `[workspace.lints]` configuration to root `Cargo.toml`

### TD-09: Code Quality (Low)

- [ ] Reduce excessive `.clone()` calls (~45) — use references/Cow in hot paths (execution, indicators)
- [ ] Split large files: `types.rs` (1520L), `env.rs` (1377L), `backtest.rs` (956L), `engine.rs` (864L), `ppo.rs` (832L)
- [ ] Add `///` doc comments to 100+ public API items lacking documentation
- [ ] Create test fixture builders to eliminate repetitive `Decimal::from_str().unwrap()` in tests
- [ ] Remove hardcoded `"BTCUSDT"` in `cvd_momentum.rs:65` — pass symbol from context

---

## Implementation Plan

### Phase 1: Safety & Correctness (Week 1)

**Goal:** Eliminate production panic paths and add critical indicator tests.

- [ ] **1. Fix production unwraps** (TD-02)
  - [ ] Replace 4x `candles.last().unwrap()` with proper error handling in all strategy files
  - [ ] Replace `expect()` with `?` in `live.rs:510`
  - Files: `crates/ssm-strategy/src/{cvd_momentum,ai_strategy,composite,orderflow_strategy}.rs`, `crates/ssm-execution/src/live.rs`

- [ ] **2. Add anti-repainting tests for all indicators** (TD-01 partial)
  - [ ] `atr.rs` — append-one-candle test
  - [ ] `bollinger.rs` — append-one-candle test
  - [ ] `rsi.rs` — append-one-candle test
  - [ ] `macd.rs` — append-one-candle test
  - [ ] `ema.rs` — append-one-candle test
  - [ ] `obv.rs` — append-one-candle test
  - [ ] `vwap.rs` — append-one-candle test
  - [ ] `liquidations.rs` — append-one-candle test
  - Verify: values at `[0..N]` don't change when candle `N+1` is added

- [ ] **3. Add orderflow module tests** (TD-01 partial)
  - [ ] `delta.rs` — unit tests
  - [ ] `footprint.rs` — unit tests
  - [ ] `imbalance.rs` — unit tests
  - [ ] `absorption.rs` — unit tests
  - [ ] `sweep.rs` — unit tests
  - [ ] `market_profile.rs` — unit tests

### Phase 2: Error Handling & Resilience (Week 2)

**Goal:** Replace anyhow in libraries with domain errors; add network resilience.

- [ ] **4. Define domain error types** (TD-03)
  - [ ] Create `ExchangeError` enum and migrate ssm-exchange
  - [ ] Create `ExecutionError` enum and migrate ssm-execution
  - [ ] Create `StrategyError` enum and migrate ssm-strategy

- [ ] **5. Add network timeouts** (TD-04)
  - [ ] Configure `reqwest::Client` with default timeout (30s) in BinanceClient
  - [ ] Configure `reqwest::Client` with default timeout (30s) in LiveEngine
  - [ ] Implement exponential backoff with jitter in LiveEngine retry loop
  - [ ] Fix spawned task join in data-feed service

### Phase 3: Strategy & Execution Tests (Week 3)

**Goal:** Test all strategy implementations and critical execution paths.

- [ ] **6. Strategy tests** (TD-01 partial)
  - [ ] `cvd_momentum.rs` — unit tests
  - [ ] `ai_strategy.rs` — unit tests
  - [ ] `composite.rs` — unit tests
  - [ ] `orderflow_strategy.rs` — unit tests

- [ ] **7. Execution tests** (TD-01 partial)
  - [ ] `position_tracker.rs` — unit tests
  - [ ] `backtest.rs` — unit tests
  - [ ] `protections.rs` — unit tests
  - [ ] Implement trailing stop fill logic in PaperEngine (TD-06)

### Phase 4: AI/ML Tests & Config (Week 4)

**Goal:** Test AI modules and centralize configuration.

- [ ] **8. AI module tests** (TD-01 partial, priority order)
  - [ ] `features.rs` — unit tests
  - [ ] `env.rs` — unit tests
  - [ ] `ppo.rs` — unit tests
  - [ ] `reward.rs` — unit tests
  - [ ] `normalize.rs` — unit tests
  - [ ] `model.rs` — unit tests
  - [ ] Remaining 15 modules

- [ ] **9. Centralize configuration** (TD-05)
  - [ ] Create shared config structs with validation
  - [ ] Migrate all 7 services to use centralized config

### Phase 5: Architecture & CI (Week 5+)

**Goal:** Fix structural issues and improve CI.

- [ ] **10. Fix crate boundary violation** (TD-07)
  - [ ] Remove `ssm-ai` dependency from `ssm-strategy` or justify and document it

- [ ] **11. Add compile-time anti-repainting** (TD-07)
  - [ ] Create `ClosedCandles` newtype wrapper
  - [ ] Update all indicator crate callers to use `ClosedCandles`

- [ ] **12. Docker & CI improvements** (TD-08)
  - [ ] Update Dockerfile to build all service binaries
  - [ ] Add `deny.toml` for supply chain security
  - [ ] Add workspace lints to root `Cargo.toml`

- [ ] **13. Resolve TODOs** (TD-06)
  - [ ] Implement TODO-002 (dynamic stoploss)
  - [ ] Implement TODO-003 (lifecycle callbacks)
  - [ ] Implement TODO-005 (lifecycle types)
  - [ ] Implement `list_pairs` for Binance and Bybit

---

## Verification

After each phase:
- [ ] `just ci` passes (fmt, clippy, test)
- [ ] `cargo test --workspace` — all new tests pass
- [ ] `just docker-build` — Docker builds successfully (after Phase 5)
- [ ] `just backtest user_data/<sample>.json` — backtest completes without errors

For domain error changes (Phase 2): verify that existing service binaries still compile and handle errors correctly.

For anti-repainting tests (Phase 1): verify append-one-candle property — values at indices `[0..N]` must not change when candle `N+1` is appended.
