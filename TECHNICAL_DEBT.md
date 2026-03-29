# Technical Debt Remediation Plan — trade-ssm

## Context

The trade-ssm codebase is a professional Rust crypto trading suite that has grown across 13 crates and 8 services. While the core architecture is sound (proper Decimal usage, clean crate separation, anti-repainting awareness), significant technical debt has accumulated across testing, error handling, resilience, and completeness. This plan catalogs all identified debt and provides a phased implementation roadmap. Correctness and reliability are paramount for a trading system.

---

## Technical Debt Inventory

### TD-01: Missing Test Coverage (Critical)

70+ files lack tests. For a trading system, this is the highest-risk debt.

| Crate | Untested Modules | Risk |
|-------|-----------------|------|
| **ssm-indicators** | atr, bollinger, liquidations, macd, obv, plot, rsi, vwap | Critical — anti-repainting rules require append-one-candle tests |
| **ssm-orderflow** | ALL: absorption, delta, footprint, imbalance, market_profile, sweep | Critical — orderflow signals drive trading decisions |
| **ssm-ai** | ALL 21 modules: config, continuous_env, correlated_features, edge, env, episode_sampler, features, hyperopt, metrics, model, model_manager, multi_feature, multi_timeframe, normalize, optimizer, outlier, ppo, replay_buffer, reward, trainer, vectorized_env | High — RL training correctness unverified |
| **ssm-strategy** | ai_strategy, cvd_momentum, composite, orderflow_strategy | High — strategy logic untested |
| **ssm-execution** | position_tracker, backtest, protections | High — money-critical paths |
| **ssm-exchange** | websocket, bybit, history, pairlist | Medium |
| **ssm-notify** | telegram, telegram_bot, webhook | Low |
| **services/** | No integration tests for any service binary | Medium |

**Files to modify:** Every module listed above needs `#[cfg(test)] mod tests { ... }` blocks.

### TD-02: Production Panic Points (Critical)

4 strategies call `.last().unwrap()` on candles in production `analyze()` methods:

| File | Line | Issue |
|------|------|-------|
| `crates/ssm-strategy/src/cvd_momentum.rs` | 66 | `candles.last().unwrap()` |
| `crates/ssm-strategy/src/ai_strategy.rs` | 53 | `candles.last().unwrap()` |
| `crates/ssm-strategy/src/composite.rs` | 89 | `candles.last().unwrap()` |
| `crates/ssm-strategy/src/orderflow_strategy.rs` | 116 | `candles.last().unwrap()` |
| `crates/ssm-execution/src/live.rs` | 510 | `HmacSha256::new_from_slice().expect()` |

**Fix:** Replace with `.ok_or_else(|| anyhow!("empty candles"))?` or guard with early return.

### TD-03: Domain Error Types Missing (High)

Library crates use `anyhow` instead of proper domain errors. This makes error matching impossible for callers.

| Crate | Locations | Proposed Error Type |
|-------|-----------|-------------------|
| `ssm-exchange` | binance.rs (4x bail), bybit.rs (6x bail), exchange_trait.rs (1x bail) | `ExchangeError` |
| `ssm-execution` | engine.rs (9x bail/anyhow) | `ExecutionError` |
| `ssm-strategy` | trait impls use anyhow | `StrategyError` |

**Files to create:**
- `crates/ssm-exchange/src/error.rs` — `ExchangeError { ApiError, Timeout, Unimplemented, UnknownExchange }`
- `crates/ssm-execution/src/error.rs` — `ExecutionError { InvalidOrder, NoLiveEngine, PositionConflict }`
- `crates/ssm-strategy/src/error.rs` — `StrategyError { InsufficientData, AnalysisFailed }`

### TD-04: Network Resilience (High)

| Issue | Location | Fix |
|-------|----------|-----|
| No request timeouts | `crates/ssm-exchange/src/binance.rs` — `reqwest::Client::new()` without timeout | Set default timeout on Client builder |
| No request timeouts | `crates/ssm-execution/src/live.rs` — same | Same |
| Fixed retry delay | `crates/ssm-execution/src/live.rs` — retry loop uses `base_delay` | Implement exponential backoff with jitter |
| Spawned task not joined | `services/data-feed/src/main.rs` | Properly await/join WebSocket task handle |

### TD-05: Scattered Configuration (Medium)

8 services independently parse env vars with duplicated defaults:

| Service | Config pattern |
|---------|---------------|
| `services/analyzer/src/main.rs` | Direct `env::var()` with hardcoded defaults |
| `services/download-data/src/main.rs` | Same |
| `services/backtest/src/main.rs` | Same |
| `services/rl-backtest/src/main.rs` | TOML config file (only one) |
| `services/data-feed/src/main.rs` | Direct `env::var()` |
| `services/signal/src/main.rs` | Direct `env::var()` |
| `services/execution/src/main.rs` | Direct `env::var()` |

**Fix:** Create shared config module in `ssm-core` or new `ssm-config` crate with validated structs.

### TD-06: Incomplete TODO Items (Medium)

| ID | Description | Location |
|----|-------------|----------|
| TODO-001 | Live Exchange Execution tests | `crates/ssm-execution/src/engine.rs:641` |
| TODO-002 | Dynamic stoploss & take-profit | `crates/ssm-strategy/src/traits.rs:48` |
| TODO-003 | Trade lifecycle callbacks | `crates/ssm-strategy/src/traits.rs:21` |
| TODO-005 | Trade lifecycle types | `crates/ssm-core/src/types.rs:261` |
| — | Trailing stop never fills in PaperEngine | `crates/ssm-execution/src/paper.rs` |
| — | `list_pairs` unimplemented | `crates/ssm-exchange/src/binance.rs:152`, `bybit.rs:228` |

### TD-07: Architecture Issues (Medium)

| Issue | Detail |
|-------|--------|
| **Crate boundary violation** | `ssm-strategy` depends on `ssm-ai` — violates documented dependency graph |
| **No compile-time anti-repainting** | Callers must remember to slice `&candles[..len-1]`; no `ClosedCandles` newtype |
| **Duplicate signal paths** | Analyzer calls indicators directly; signal service uses Strategy via NATS — divergent logic |
| **Leverage not integrated** | `leverage.rs` module exists but `LiveEngine.submit_order()` has no leverage param |
| **NATS schema unversioned** | Topics use flat naming without schema validation |

### TD-08: Docker/CI Gaps (Medium)

| Issue | Location |
|-------|----------|
| Dockerfile builds only 4 binaries but compose references more | `Dockerfile`, `docker-compose.yml` |
| No `cargo-deny` for supply chain security | Missing `deny.toml` |
| No workspace lints configuration | `Cargo.toml` workspace |

### TD-09: Code Quality (Low)

| Issue | Count | Detail |
|-------|-------|--------|
| Excessive `.clone()` | ~45 | Could use references/Cow in hot paths (execution, indicators) |
| Large files | 5 | types.rs (1520L), env.rs (1377L), backtest.rs (956L), engine.rs (864L), ppo.rs (832L) |
| Missing doc comments | 100+ pub items | Public APIs lack `///` documentation |
| Repetitive test setup | Many | No test fixture builders; Decimal::from_str().unwrap() repeated everywhere |
| Hardcoded symbol | `cvd_momentum.rs:65` | `"BTCUSDT"` hardcoded in strategy |

---

## Implementation Plan

### Phase 1: Safety & Correctness (Week 1)

**Goal:** Eliminate production panic paths and add critical indicator tests.

1. **Fix production unwraps** (TD-02)
   - Replace 4x `candles.last().unwrap()` with proper error handling in all strategy files
   - Replace `expect()` with `?` in `live.rs:510`
   - Files: `crates/ssm-strategy/src/{cvd_momentum,ai_strategy,composite,orderflow_strategy}.rs`, `crates/ssm-execution/src/live.rs`

2. **Add anti-repainting tests for all indicators** (TD-01 partial)
   - Add append-one-candle tests for: atr, bollinger, rsi, macd, ema, obv, vwap, liquidations
   - Verify: values at `[0..N]` don't change when candle `N+1` is added
   - Files: `crates/ssm-indicators/src/{atr,bollinger,rsi,macd,obv,vwap,liquidations}.rs`

3. **Add orderflow module tests** (TD-01 partial)
   - Tests for: delta, footprint, imbalance, absorption, sweep, market_profile
   - Files: `crates/ssm-orderflow/src/*.rs`

### Phase 2: Error Handling & Resilience (Week 2)

**Goal:** Replace anyhow in libraries with domain errors; add network resilience.

4. **Define domain error types** (TD-03)
   - Create `ExchangeError`, `ExecutionError`, `StrategyError` enums
   - Replace all `anyhow::bail!()` in library crates
   - Files: new `error.rs` in ssm-exchange, ssm-execution, ssm-strategy

5. **Add network timeouts** (TD-04)
   - Configure `reqwest::Client` with default timeout (30s) in BinanceClient and LiveEngine
   - Implement exponential backoff with jitter in LiveEngine retry loop
   - Files: `crates/ssm-exchange/src/binance.rs`, `crates/ssm-execution/src/live.rs`

### Phase 3: Strategy & Execution Tests (Week 3)

**Goal:** Test all strategy implementations and critical execution paths.

6. **Strategy tests** (TD-01 partial)
   - Test cvd_momentum, ai_strategy, composite, orderflow_strategy
   - Files: `crates/ssm-strategy/src/*.rs`

7. **Execution tests** (TD-01 partial)
   - Test position_tracker, backtest engine, protections
   - Implement trailing stop fill logic in PaperEngine (TD-06)
   - Files: `crates/ssm-execution/src/{position_tracker,backtest,protections,paper}.rs`

### Phase 4: AI/ML Tests & Config (Week 4)

**Goal:** Test AI modules and centralize configuration.

8. **AI module tests** (TD-01 partial)
   - Priority: features, env, ppo, reward, normalize, model
   - Files: `crates/ssm-ai/src/*.rs`

9. **Centralize configuration** (TD-05)
   - Create shared config structs with validation
   - Migrate services to use centralized config
   - Files: `crates/ssm-core/src/config.rs` (or new `ssm-config` crate), all `services/*/src/main.rs`

### Phase 5: Architecture & CI (Week 5+)

**Goal:** Fix structural issues and improve CI.

10. **Fix crate boundary violation** (TD-07)
    - Remove `ssm-ai` dependency from `ssm-strategy` or justify and document it
    - Files: `crates/ssm-strategy/Cargo.toml`

11. **Add compile-time anti-repainting** (TD-07)
    - Create `ClosedCandles` newtype wrapper that enforces the last-candle-excluded rule
    - Files: `crates/ssm-core/src/types.rs`, indicator crate callers

12. **Docker & CI improvements** (TD-08)
    - Update Dockerfile to build all service binaries
    - Add `deny.toml` for supply chain security
    - Add workspace lints
    - Files: `Dockerfile`, `deny.toml`, workspace `Cargo.toml`

13. **Resolve TODOs** (TD-06)
    - Implement TODO-002 (dynamic stoploss), TODO-003 (lifecycle callbacks), TODO-005 (lifecycle types)
    - Files: `crates/ssm-strategy/src/traits.rs`, `crates/ssm-core/src/types.rs`

---

## Verification

After each phase:
1. Run `just ci` — must pass (fmt, clippy, test)
2. Run `cargo test --workspace` — all new tests pass
3. Run `just docker-build` — Docker builds successfully (after Phase 5)
4. Run `just backtest user_data/<sample>.json` — backtest completes without errors

For domain error changes (Phase 2): verify that existing service binaries still compile and handle errors correctly.

For anti-repainting tests (Phase 1): verify append-one-candle property — values at indices `[0..N]` must not change when candle `N+1` is appended.
