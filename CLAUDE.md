# trade-ssm

Professional Rust crypto trading suite. CVD + liquidation tracking, in-candle trade aggregation,
bot strategies, RL/ML model interface, paper/live execution, Telegram alerts.
Inspired by aggr.trade, freqtrade/FreqAI, and RIFEBTC patterns.

## Architecture

```
trade-ssm/
├── crates/
│   ├── ssm-core/          # Domain types: Candle, Trade, Order, Position, Signal, AIAction
│   ├── ssm-exchange/      # Binance REST + WebSocket trade aggregation (aggr-inspired)
│   ├── ssm-indicators/    # Pure indicators: CVD, liquidation analysis
│   ├── ssm-notify/        # Telegram notification dispatch
│   ├── ssm-execution/     # Order engine: paper + live, position tracker
│   ├── ssm-strategy/      # Strategy trait + built-in CVD momentum strategy
│   └── ssm-ai/            # AI model trait, RL environment, feature pipeline
├── services/
│   ├── analyzer/          # Live polling service
│   ├── download-data/     # Historical data fetcher (freqtrade download-data)
│   └── backtest/          # Offline indicator replay (freqtrade backtesting)
├── .github/workflows/
│   ├── ci.yml             # Push CI: check → test → build → backtest → notify
│   ├── pr.yml             # PR gates: check → test → build → summary
│   └── release.yml        # Tag release: validate → build matrix → Docker → GitHub Release
├── Dockerfile             # Multi-stage build (all binaries)
├── docker-compose.yml     # Services + tool profiles
└── justfile               # Dev workflow recipes
```

### Dependency graph

```
ssm-core          ← zero external service deps (shared types)
ssm-exchange      ← ssm-core, reqwest, tokio
ssm-indicators    ← ssm-core, rust_decimal
ssm-notify        ← ssm-core, ssm-indicators, reqwest
ssm-execution     ← ssm-core, rust_decimal, chrono
ssm-strategy      ← ssm-core, ssm-indicators
ssm-ai            ← ssm-core, ssm-indicators, rust_decimal
analyzer          ← all crates
download-data     ← ssm-exchange
backtest          ← ssm-core, ssm-exchange, ssm-indicators
```

### Crate boundaries

| Crate | Concern | AI context hint |
|-------|---------|-----------------|
| `ssm-core` | All shared domain types | Read first — enums, structs, traits |
| `ssm-exchange` | REST + trade aggregation | Binance futures API + aggr-style aggregator |
| `ssm-indicators` | Pure math on `&[Candle]` | No I/O, no async, deterministic |
| `ssm-notify` | Telegram formatting + send | Depends on indicator types |
| `ssm-execution` | Paper/live order engine | Position tracking, all order types |
| `ssm-strategy` | Strategy trait + builtins | CVD momentum; implement `Strategy` for custom |
| `ssm-ai` | ML/RL model interface | FreqAI-inspired: features, env, model trait |

## Order types supported

Market, Limit, StopMarket, StopLimit, TakeProfitMarket, TakeProfitLimit, TrailingStop

## AI action space (FreqAI Base5Action)

| Index | Action | Description |
|-------|--------|-------------|
| 0 | Neutral | Hold / do nothing |
| 1 | EnterLong | Open long position |
| 2 | ExitLong | Close long position |
| 3 | EnterShort | Open short position |
| 4 | ExitShort | Close short position |

## Quick reference

```bash
just              # list all recipes
just ci           # fmt-check + clippy + test (run before commit)
just run          # start live analyzer
just test         # cargo test --workspace
just lint         # cargo clippy -- -D warnings

# Data pipeline
just download-data                          # fetch 30d candles
just backtest user_data/file.json           # replay indicators

# Docker
just docker-build                           # build all images
just docker-up                              # start live analyzer
just docker-download                        # download historical data
just docker-backtest /app/user_data/f.json  # run backtest in Docker
just docker-logs                            # tail analyzer logs

# Docker integration tests
just docker-integration-test                # full Docker build + validation
just docker-validate                        # validate docker-compose.yml syntax
```

## CI/CD Workflows

### `ci.yml` — Push to main/develop + manual dispatch

```
check (fmt + clippy) → test → build → backtest + rl-backtest → notify
                        ↘ audit (parallel)
                        ↘ docs  (parallel)
```

- Runs on push to `main`, `develop` with concurrency control (cancels in-progress)
- Manual dispatch with input: `cvd_window`
- `RUSTFLAGS: -D warnings` — clippy warnings via compiler flag (consistent across jobs)
- **Security audit**: `rustsec/audit-check` runs in parallel (known vulnerability scanning)
- **Doc check**: `cargo doc --workspace -D warnings` catches broken doc links
- **Binary verification**: post-build step confirms all 4 binaries exist with size reporting
- Backtest + RL backtest reuse build artifacts (no rebuild)
- Telegram notification on main — includes audit + docs status in grid
- All jobs have `timeout-minutes` to prevent runaway builds

### `pr.yml` — Pull request gates

```
check → test → build → pr-report + summary
  ↘ audit (parallel, advisory/non-blocking)
  ↘ docs  (parallel, blocking)
```

- Triggers on PR open/sync/reopen to `main`, `develop`
- Concurrency: cancels in-progress runs for same PR
- Read-only cache (`save-if: false`) to avoid polluting main branch cache
- **Security audit**: runs as advisory (`continue-on-error: true`) — visible but non-blocking
- **Doc check**: blocking — prevents merging PRs with broken documentation
- **PR Report**: sticky comment includes audit + docs status in results table
- **Summary gate**: check + test + build + docs must all pass (required status check)
- Uploads test output and build artifacts with binary verification

### `pr-docker.yml` — Docker integration on PR

```
changes → docker-test (build + all integration) → pr-comment + docker-status
```

- Triggers on PR open/sync/reopen to `main`, `develop`
- **Smart filtering**: only runs when Dockerfile, docker-compose, or Rust source changes
- **Single job design**: build + integration tests in one job (eliminated DinD overhead)
  - Buildx build with GHA cache
  - Binary verification (all 4 binaries executable)
  - Image size reporting with >200MB warning threshold
  - Container start smoke test (dummy credentials, proves binary runs)
  - Docker Compose config validation
  - Runtime environment checks (SSL certs, working dir, root user warning)
- **PR Comment**: sticky comment with collapsible build + test details
- **Status gate**: `Docker PR Status` — auto-passes if no relevant changes

### `release.yml` — Tag-triggered releases

```
validate (version check + CI) → build (x86_64 + aarch64) → Docker (GHCR) → release → notify
```

- Triggers on `v*.*.*` tags
- **Version validation**: tag must match `Cargo.toml` version (prevents mismatched releases)
- **Security audit**: runs during validation (non-blocking warning)
- Cross-compiles for linux x86_64 and aarch64 with binary verification
- **SHA256 checksums**: generated per platform tarball, included in release notes
- **Categorized changelog**: commits sorted into Features/Fixes/Other sections
- Docker Buildx pushes to `ghcr.io` with semver tags
- Creates GitHub Release with changelog, tarballs, and checksums

### Dependency management

**Dependabot** (`.github/dependabot.yml`) keeps dependencies current:

| Ecosystem | Schedule | Grouping |
|-----------|----------|----------|
| GitHub Actions | Weekly (Monday) | Individual PRs |
| Cargo (Rust) | Weekly (Monday) | Minor/patch grouped |
| Docker | Weekly (Monday) | Individual PRs |

### CI cache & artifact strategy

| Workflow | Cache key | Save policy | Artifacts |
|----------|-----------|-------------|-----------|
| `ci.yml` | `ci-check`, `ci-test`, `ci-release`, `ci-docs` | Always (default branch) | Binaries (14d), backtest results (30d) |
| `pr.yml` | `pr-check`, `pr-test`, `pr-build`, `pr-docs` | Never (`save-if: false`) | Test output (7d), build (3d) |
| `pr-docker.yml` | `pr-docker` (GHA Buildx) | Min (GHA cache) | Docker report (7d) |
| `release.yml` | `release-validate`, `release-{target}` | Always | Platform tarballs + checksums (5d) |

- **Rust cache**: `Swatinem/rust-cache@v2` with `shared-key` per job for isolation
- **Docker cache**: GitHub Actions cache backend via `docker/build-push-action` `cache-from`/`cache-to`
- **Artifact reuse**: CI backtest downloads pre-built binaries instead of rebuilding

### Telegram notifications

All workflows use `appleboy/telegram-action@v1.0.0` (pinned version) with HTML format:

| Workflow | Trigger | Content |
|----------|---------|---------|
| `ci.yml` | main push (success + failure) | Job status grid (7 jobs), commit SHA, Run + Artifacts links |
| `pr.yml` | PR failure only | PR link, job statuses, Run + Artifacts links |
| `pr-docker.yml` | Docker failure only | PR link, build/test status, Run + Artifacts links |
| `release.yml` | Release success | Version, `docker pull` command, Release Notes + Build links |

- **Format**: HTML (`<b>`, `<code>`, `<a href>`) — more reliable than Telegram Markdown v1
- **Gating**: all require `vars.TELEGRAM_NOTIFICATIONS_ENABLED == 'true'`
- **Links**: each message includes clickable "View Run" and "Artifacts" deep links

### GitHub configuration needed

**Secrets:**
- `TELEGRAM_BOT_TOKEN` — Telegram bot API token
- `TELEGRAM_CHAT_ID` — target chat for notifications

**Variables:**
- `TELEGRAM_NOTIFICATIONS_ENABLED` — set to `true` to enable notifications

**Branch protection (recommended for `main`):**
- Require PR reviews
- Require status checks: `PR Status`, `Docker PR Status`
- Require branches to be up to date

## Env vars

| Var | Service | Required | Default |
|-----|---------|----------|---------|
| `TELEGRAM_BOT_TOKEN` | analyzer | yes | — |
| `TELEGRAM_CHAT_ID` | analyzer | yes | — |
| `SYMBOL` | all | no | BTCUSDT |
| `INTERVAL` | all | no | 15m |
| `CHECK_INTERVAL_SECS` | analyzer | no | 60 |
| `EXECUTION_MODE` | analyzer | no | paper |
| `DAYS` | download-data | no | 30 |
| `DATADIR` | download-data | no | user_data |
| `DATAFILE` | backtest | yes | — |
| `CVD_WINDOW` | backtest | no | 15 |

## Anti-repainting rules

1. Never signal on the forming (current) candle
2. Indicators receive only closed candles via `&candles[..len-1]`
3. Append-one-candle test: values at `[0..N]` must not change when candle `N+1` is added
4. CVD `analyze_cvd()` is a pure function — same input = same output

## Conventions

- `rust_decimal::Decimal` for all prices/volumes, never `f64`
- `f64` only in AI feature vectors (ML libraries expect floats)
- `anyhow::Result` in binaries, domain errors in libraries
- All I/O is async (tokio), indicators are sync pure functions
- One test file per module, inline `#[cfg(test)]` blocks
- Structured logging via `tracing` with field-level context

## Definition of done

- [ ] `just ci` passes (fmt, clippy, test)
- [ ] Anti-repainting test for any new indicator
- [ ] Paper execution tests for new order types
- [ ] AI model trait implemented for new models
- [ ] Docker builds successfully
- [ ] Backtest runs on sample data without errors

## Adding a new strategy

```rust
impl Strategy for MyStrategy {
    fn name(&self) -> &str { "my_strategy" }
    fn analyze(&self, candles: &[Candle]) -> Result<Option<Signal>> {
        // Your logic here
    }
}
```

## Adding a new AI model

```rust
impl AIModel for MyModel {
    fn name(&self) -> &str { "xgboost_v1" }
    fn predict(&self, features: &FeatureRow) -> Result<AIAction> { ... }
    fn train(&mut self, data: &[FeatureRow]) -> Result<TrainMetrics> { ... }
    fn save(&self, path: &Path) -> Result<()> { ... }
    fn load(&mut self, path: &Path) -> Result<()> { ... }
}
```

## Links

- [freqtrade](https://github.com/freqtrade/freqtrade) — strategy arch
- [FreqAI RL](https://www.freqtrade.io/en/stable/freqai-reinforcement-learning/) — RL patterns
- [aggr.trade](https://github.com/Tucsky/aggr) — trade aggregation
- [Binance API](https://binance-docs.github.io/apidocs/)
