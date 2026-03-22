# trade-ssm

Rust crypto trading system. CVD + liquidation tracking with Telegram alerts.
Workspace microservices architecture via Docker Compose.

## Architecture

```
trade-ssm/
├── crates/
│   ├── ssm-core/          # Shared types: Candle, Liquidation, LiquidationTier
│   ├── ssm-exchange/      # Exchange connectors (Binance REST)
│   ├── ssm-indicators/    # Pure indicators: CVD, liquidation analysis
│   └── ssm-notify/        # Notification dispatch (Telegram bot)
├── services/
│   └── analyzer/          # Main service binary (polling loop)
├── Dockerfile             # Multi-stage build for analyzer
├── docker-compose.yml     # Service orchestration
└── Makefile               # Dev workflow shortcuts
```

### Dependency graph

```
ssm-core          ← zero external deps (types only)
ssm-exchange      ← ssm-core, reqwest
ssm-indicators    ← ssm-core, rust_decimal
ssm-notify        ← ssm-core, ssm-indicators, reqwest
analyzer          ← all crates above
```

### Crate boundaries

| Crate | Concern | AI context hint |
|-------|---------|-----------------|
| `ssm-core` | Domain types shared across all crates | Read first — <200 lines |
| `ssm-exchange` | HTTP calls to exchange REST APIs | Binance only for MVP |
| `ssm-indicators` | Pure math on `&[Candle]` slices | No I/O, no async |
| `ssm-notify` | Telegram message formatting + sending | Depends on indicator types |
| `analyzer` | Wiring: fetch → analyze → notify loop | Thin main, ~60 lines |

## Quick reference

```bash
make ci           # fmt-check + clippy + test (run before commit)
make run          # cargo run --bin analyzer
make docker-up    # build + start via docker compose
make test         # cargo test --workspace
make lint         # cargo clippy --workspace -- -D warnings
```

## Env vars

| Var | Required | Default |
|-----|----------|---------|
| `TELEGRAM_BOT_TOKEN` | yes | — |
| `TELEGRAM_CHAT_ID` | yes | — |
| `SYMBOL` | no | BTCUSDT |
| `INTERVAL` | no | 15m |
| `CHECK_INTERVAL_SECS` | no | 60 |

Copy `.env.example` to `.env` and fill in values.

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

## Definition of done

- [ ] `make ci` passes (fmt, clippy, test)
- [ ] Anti-repainting test for any new indicator
- [ ] Telegram message formats correctly for new signals
- [ ] Docker builds successfully

## Exchange API reference

```bash
# Binance futures klines (15m, last 16 candles)
curl "https://fapi.binance.com/fapi/v1/klines?symbol=BTCUSDT&interval=15m&limit=16"

# Binance futures liquidations
curl "https://fapi.binance.com/fapi/v1/forceOrders?symbol=BTCUSDT&limit=100"
```

## Links

- [CCXT Rust](https://github.com/Praying/ccxt-rust)
- [freqtrade](https://github.com/freqtrade/freqtrade) — strategy arch inspiration
- [aggr.trade](https://github.com/Tucsky/aggr) — aggregation inspiration
- [Binance API](https://binance-docs.github.io/apidocs/)
