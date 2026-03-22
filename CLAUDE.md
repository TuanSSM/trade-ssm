# trade-ssm — Trade State Space Model

A high-performance Rust-based crypto trading system inspired by freqtrade and aggr.trade. Designed for low-latency signal generation, multi-exchange data aggregation, and automated Telegram notifications.

## Project Goals

- **Rust-native performance**: Microsecond execution, zero GC pauses, minimal memory footprint
- **Multi-exchange aggregation**: Aggregate order flow across Binance, Bybit, Coinbase, Deribit, Hyperliquid, OKX, Kraken, Bitget (inspired by [aggr.trade](https://aggr.trade))
- **Anti-repainting**: Only signal on closed candles — never look ahead
- **Telegram-first alerts**: Real-time CVD, liquidation, and strategy signals via Telegram bot
- **Pluggable strategies**: Trait-based strategy framework similar to freqtrade's `populate_indicators` / `populate_entry_trend` / `populate_exit_trend`

## Architecture

```
src/
├── main.rs              # Entry point, CLI, scheduler
├── exchange/
│   ├── mod.rs           # Exchange trait + registry
│   ├── binance.rs       # Binance REST + WebSocket
│   └── types.rs         # Candle, Trade, Liquidation types
├── indicators/
│   ├── mod.rs           # Indicator trait
│   ├── cvd.rs           # Cumulative Volume Delta
│   ├── liquidations.rs  # Tiered liquidation tracking
│   ├── premium.rs       # Spot vs perp premium
│   └── keltner.rs       # Keltner Channel
├── strategy/
│   ├── mod.rs           # Strategy trait
│   └── cvd_liq.rs       # CVD + liquidation strategy
├── signals/
│   ├── mod.rs           # Signal types
│   └── telegram.rs      # Telegram bot integration
├── aggregation/
│   └── mod.rs           # Multi-exchange aggregator
├── backtest/
│   └── mod.rs           # Backtesting engine
└── data/
    └── mod.rs           # Historical data management
```

## Key Concepts

### Candle Anti-Repainting Rules
1. **Never act on the current (open) candle** — wait for candle close
2. **Indicators must use `[..len-1]`** — exclude the forming candle from calculations
3. **No look-ahead bias** — backtesting must process candles sequentially
4. **CVD resets** — clearly document when CVD accumulator resets vs. continues

### Cumulative Volume Delta (CVD)
Tracks directional volume flow: `CVD += buy_volume - sell_volume` per candle. Aggregated across exchanges for stronger signal. Analyzed over configurable windows (default: last 15 candles).

### Liquidation Tiers (from aggr.trade)
- Tier 1: > $1,000
- Tier 2: > $10,000
- Tier 3: > $30,000
- Tier 4: > $100,000+

### Telegram Signals
Messages include: symbol, timeframe, CVD trend (bullish/bearish/neutral), liquidation summary, premium delta, and timestamp.

## Exchange Data Fetching

### Binance Klines (OHLCV)
```bash
# 15-minute candles, last 15
curl "https://api.binance.com/api/v3/klines?symbol=BTCUSDT&interval=15m&limit=15"

# 1-hour candles
curl "https://api.binance.com/api/v3/klines?symbol=BTCUSDT&interval=1h&limit=15"

# 4-hour candles
curl "https://api.binance.com/api/v3/klines?symbol=BTCUSDT&interval=4h&limit=15"
```

### Binance Futures Liquidations
```bash
# Recent liquidation orders
curl "https://fapi.binance.com/fapi/v1/forceOrders?symbol=BTCUSDT&limit=100"
```

### Bybit Klines
```bash
curl "https://api.bybit.com/v5/market/kline?category=linear&symbol=BTCUSDT&interval=15&limit=15"
```

## Development Workflow

```bash
# Build
cargo build

# Run (requires TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID env vars)
TELEGRAM_BOT_TOKEN=xxx TELEGRAM_CHAT_ID=yyy cargo run

# Test
cargo test

# Format
cargo fmt

# Lint
cargo clippy -- -D warnings

# Benchmark (when criterion benchmarks exist)
cargo bench
```

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `TELEGRAM_BOT_TOKEN` | Yes | Telegram bot API token from @BotFather |
| `TELEGRAM_CHAT_ID` | Yes | Target chat/channel ID for notifications |
| `BINANCE_API_KEY` | No | For authenticated endpoints (not needed for public market data) |
| `CHECK_INTERVAL_SECS` | No | Polling interval in seconds (default: 60) |

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `tokio` | Async runtime |
| `reqwest` | HTTP client for exchange REST APIs |
| `serde` / `serde_json` | JSON serialization |
| `rust_decimal` | Precise decimal arithmetic for prices |
| `chrono` | Timestamps and time handling |
| `tokio-tungstenite` | WebSocket connections (future) |
| `ta` | Technical analysis indicators |

## Conventions

- **Error handling**: Use `anyhow::Result` for application errors, `thiserror` for library errors
- **Async**: All I/O operations are async via tokio
- **Naming**: snake_case for functions/variables, PascalCase for types, SCREAMING_SNAKE for constants
- **Decimal precision**: Use `rust_decimal::Decimal` for all price/volume values — never `f64`
- **Logging**: Use `tracing` crate with structured logging

## Definition of Done

A feature is complete when:
1. Unit tests pass (`cargo test`)
2. No candle repainting — verified by sequential-only data access
3. Clippy clean (`cargo clippy -- -D warnings`)
4. Formatted (`cargo fmt --check`)
5. Telegram notification works for the feature's signals
6. Documented in this file if it changes architecture

## External References

- [CCXT Rust](https://github.com/Praying/ccxt-rust) — Exchange abstraction
- [freqtrade](https://github.com/freqtrade/freqtrade) — Strategy architecture inspiration
- [aggr.trade](https://github.com/Tucsky/aggr) — Multi-exchange aggregation inspiration
- [aggr-templates](https://github.com/cryptorife/aggr-templates) — Dashboard templates
- [Barter-rs](https://docs.rs/barter) — Rust trading framework reference
- [ta-rs](https://docs.rs/ta) — Technical analysis in Rust
- [Binance API](https://binance-docs.github.io/apidocs/) — Exchange REST/WS docs
