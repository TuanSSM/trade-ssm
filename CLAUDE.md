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
├── .github/workflows/     # CI: check → test → build → backtest + secrets
├── Dockerfile             # Multi-stage build (all binaries)
├── docker-compose.yml     # Services + tool profiles
└── Makefile               # Dev workflow shortcuts
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
# Dev workflow
make ci                # fmt-check + clippy + test (run before commit)
make run               # cargo run --bin analyzer
make test              # cargo test --workspace
make lint              # cargo clippy --workspace -- -D warnings

# Data pipeline
make download-data                          # fetch 30d candles
make backtest DATAFILE=user_data/file.json  # replay indicators

# Docker
make docker-build          # build all binaries
make docker-up             # start live analyzer
make docker-download       # download historical data
make docker-backtest DATAFILE=/app/user_data/file.json
make docker-logs           # tail analyzer logs
```

## CI/CD (.github/workflows/ci.yml)

```
check (fmt + clippy) → test → build → backtest → notify
```

**GitHub Secrets required:**
- `TELEGRAM_BOT_TOKEN` — for deploy notifications
- `TELEGRAM_CHAT_ID` — target chat

**Workflow inputs (manual dispatch):**
- `symbol` (default: BTCUSDT)
- `interval` (default: 15m)
- `backtest_days` (default: 7)
- `cvd_window` (default: 15)

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

- [ ] `make ci` passes (fmt, clippy, test)
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
