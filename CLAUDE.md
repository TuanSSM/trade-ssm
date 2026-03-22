# trade-ssm

Rust crypto trading system. CVD + liquidation tracking with Telegram alerts.
Workspace microservices architecture via Docker Compose.

## Architecture

```
trade-ssm/
├── crates/
│   ├── ssm-core/          # Shared types: Candle, Liquidation, LiquidationTier
│   ├── ssm-exchange/      # Exchange connectors (Binance REST) + history download
│   ├── ssm-indicators/    # Pure indicators: CVD, liquidation analysis
│   └── ssm-notify/        # Notification dispatch (Telegram bot)
├── services/
│   ├── analyzer/          # Live polling service (fetch → analyze → notify)
│   ├── download-data/     # Historical data fetcher (like freqtrade download-data)
│   └── backtest/          # Offline indicator replay (like freqtrade backtesting)
├── .github/workflows/     # CI: check → test → build → backtest pipeline
├── Dockerfile             # Multi-stage build (all 3 binaries)
├── docker-compose.yml     # Service orchestration + tool profiles
└── Makefile               # Dev workflow shortcuts
```

### Dependency graph

```
ssm-core          ← zero external deps (types only)
ssm-exchange      ← ssm-core, reqwest, tokio
ssm-indicators    ← ssm-core, rust_decimal
ssm-notify        ← ssm-core, ssm-indicators, reqwest
analyzer          ← all crates above
download-data     ← ssm-exchange
backtest          ← ssm-core, ssm-exchange, ssm-indicators
```

### Crate boundaries

| Crate | Concern | AI context hint |
|-------|---------|-----------------|
| `ssm-core` | Domain types shared across all crates | Read first — <200 lines |
| `ssm-exchange` | HTTP calls + history download/load | Binance only for MVP |
| `ssm-indicators` | Pure math on `&[Candle]` slices | No I/O, no async |
| `ssm-notify` | Telegram message formatting + sending | Depends on indicator types |
| `analyzer` | Wiring: fetch → analyze → notify loop | Thin main, ~60 lines |
| `download-data` | Paginated historical kline fetcher | Saves JSON to user_data/ |
| `backtest` | Sliding-window CVD replay on saved data | Outputs .backtest.json |

## Quick reference

```bash
# Dev workflow
make ci                # fmt-check + clippy + test (run before commit)
make run               # cargo run --bin analyzer
make test              # cargo test --workspace
make lint              # cargo clippy --workspace -- -D warnings

# Data pipeline (freqtrade-inspired)
make download-data                          # fetch 30d BTCUSDT 15m candles
make backtest DATAFILE=user_data/file.json  # replay indicators offline

# Docker commands (freqtrade-inspired)
make docker-build          # build all binaries
make docker-up             # start live analyzer
make docker-download       # download historical data
make docker-backtest DATAFILE=/app/user_data/file.json
make docker-logs           # tail analyzer logs
```

## Env vars

| Var | Service | Required | Default |
|-----|---------|----------|---------|
| `TELEGRAM_BOT_TOKEN` | analyzer | yes | — |
| `TELEGRAM_CHAT_ID` | analyzer | yes | — |
| `SYMBOL` | all | no | BTCUSDT |
| `INTERVAL` | all | no | 15m |
| `CHECK_INTERVAL_SECS` | analyzer | no | 60 |
| `DAYS` | download-data | no | 30 |
| `DATADIR` | download-data | no | user_data |
| `DATAFILE` | backtest | yes | — |
| `CVD_WINDOW` | backtest | no | 15 |

Copy `.env.example` to `.env` and fill in values.

## CI Pipeline (.github/workflows/ci.yml)

```
check (fmt + clippy) → test → build → backtest (7d live data)
```

Runs on push to `main`, `develop`, `claude/**` and PRs to `main`.
Uploads binaries and backtest results as artifacts.

## Anti-repainting rules

1. Never signal on the forming (current) candle
2. Indicators receive only closed candles via `&candles[..len-1]`
3. Append-one-candle test: values at `[0..N]` must not change when candle `N+1` is added
4. CVD `analyze_cvd()` is a pure function — same input = same output

## Conventions

- `rust_decimal::Decimal` for all prices/volumes, never `f64`
- `anyhow::Result` in binaries, domain errors in libraries
- All I/O is async (tokio), indicators are sync pure functions
- One test file per module, inline `#[cfg(test)]` blocks
- Structured logging via `tracing` with field-level context

## Definition of done

- [ ] `make ci` passes (fmt, clippy, test)
- [ ] Anti-repainting test for any new indicator
- [ ] Telegram message formats correctly for new signals
- [ ] Docker builds successfully
- [ ] Backtest runs on sample data without errors

## Exchange API reference

```bash
# Binance futures klines (15m, last 16 candles)
curl "https://fapi.binance.com/fapi/v1/klines?symbol=BTCUSDT&interval=15m&limit=16"

# Binance futures klines with time range (historical download)
curl "https://fapi.binance.com/fapi/v1/klines?symbol=BTCUSDT&interval=15m&limit=1000&startTime=MS&endTime=MS"

# Binance futures liquidations
curl "https://fapi.binance.com/fapi/v1/forceOrders?symbol=BTCUSDT&limit=100"
```

## Links

- [freqtrade](https://github.com/freqtrade/freqtrade) — strategy arch inspiration
- [aggr.trade](https://github.com/Tucsky/aggr) — aggregation inspiration
- [Binance API](https://binance-docs.github.io/apidocs/)
