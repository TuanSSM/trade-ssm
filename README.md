# trade-ssm

Rust crypto trading suite — CVD + liquidation tracking, in-candle trade aggregation, bot strategies, RL/ML model interface, paper/live execution, Telegram alerts.

## Architecture

```
crates/
  ssm-core          Domain types: Candle, Trade, Order, Position, Signal, AIAction
  ssm-exchange      Binance REST + WebSocket trade aggregation
  ssm-indicators    Pure indicators: CVD, liquidation analysis
  ssm-notify        Telegram notification dispatch
  ssm-execution     Order engine: paper + live, position tracker
  ssm-strategy      Strategy trait + built-in CVD momentum strategy
  ssm-ai            AI model trait, RL environment, feature pipeline
  ssm-nats          NATS messaging integration
  ssm-orderflow     Order flow analysis

services/
  analyzer          Live polling service
  download-data     Historical data fetcher
  backtest          Offline indicator replay
  rl-backtest       RL strategy backtesting + hyperparameter optimization
  rl-trainer        RL model training pipeline
  data-feed         WebSocket → NATS candle publisher
  signal            Strategy → signal service (CVD or AI mode)
  execution         Signal → order execution service
```

### Dependency graph

```
ssm-core ← (no deps)
ssm-exchange ← ssm-core
ssm-indicators ← ssm-core
ssm-notify ← ssm-core, ssm-indicators
ssm-execution ← ssm-core
ssm-strategy ← ssm-core, ssm-indicators
ssm-ai ← ssm-core, ssm-indicators
ssm-nats ← ssm-core
ssm-orderflow ← ssm-core
```

## Prerequisites

- Rust 1.83+
- [just](https://github.com/casey/just) (task runner)
- Docker + Docker Compose (optional, for containerized workflows)

## Quick start

```bash
# Build
just build

# Run CI checks (format + clippy + tests)
just ci

# Download 30 days of historical data
just download-data

# Run backtest
just backtest user_data/BTCUSDT-15m.json

# Start live analyzer
cp .env.example .env   # configure TELEGRAM_BOT_TOKEN, TELEGRAM_CHAT_ID
just run
```

## Development

```bash
just build       # cargo build --workspace
just test        # cargo test --workspace
just lint        # cargo clippy -- -D warnings
just fmt         # cargo fmt --all
just fmt-check   # check formatting without modifying
just ci          # fmt-check + lint + test (run before commit)
just clean       # cargo clean
```

## RL workflows

```bash
# Backtest with default RL config
just rl-backtest user_data/BTCUSDT-15m.json

# Hyperparameter optimization
just rl-optimize user_data/BTCUSDT-15m.json config/rl-default.toml

# Multi-timeframe comparison
just rl-multi-tf user_data/BTCUSDT-15m.json

# Backtest with trained model
just rl-model-backtest user_data/BTCUSDT-15m.json models/table_model_best.json
```

## Docker

### Profiles

| Profile   | Services                              | Use case              |
|-----------|---------------------------------------|-----------------------|
| (default) | analyzer                              | Live monitoring       |
| `tools`   | download-data, backtest               | Data pipeline         |
| `rl`      | data-feed, nats, rl-trainer, rl-backtest | RL training        |
| `deploy`  | data-feed, nats, signal, execution    | Paper/live trading    |

### Commands

```bash
just docker-build                # build images
just docker-up                   # start analyzer
just docker-down                 # stop all
just docker-download             # download historical data
just docker-backtest /app/user_data/BTCUSDT-15m.json

# RL
just docker-rl-train             # start RL training pipeline
just docker-rl-deploy-paper      # deploy for paper trading
just docker-rl-deploy-live       # deploy for live trading
just docker-rl-backtest user_data/BTCUSDT-15m.json
just docker-rl-logs              # tail RL trainer logs

# Validation
just docker-integration-test     # full Docker build + validation
just docker-validate             # validate docker-compose.yml
```

## Configuration

### Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `TELEGRAM_BOT_TOKEN` | — | Telegram bot API token (required for analyzer) |
| `TELEGRAM_CHAT_ID` | — | Telegram chat ID (required for analyzer) |
| `SYMBOL` | `BTCUSDT` | Trading pair |
| `INTERVAL` | `15m` | Candle interval |
| `CHECK_INTERVAL_SECS` | `60` | Analyzer polling interval |
| `EXECUTION_MODE` | `paper` | `paper` or `live` |
| `DAYS` | `30` | Days of history to download |
| `DATAFILE` | — | Path to candle JSON for backtesting |
| `CVD_WINDOW` | `15` | CVD indicator window size |
| `STRATEGY_MODE` | `cvd` | `cvd` or `ai` (signal service) |
| `MODEL_PATH` | — | Path to trained RL model |
| `NATS_URL` | `nats://nats:4222` | NATS broker URL |

## Extending

### Add a strategy

```rust
impl Strategy for MyStrategy {
    fn name(&self) -> &str { "my_strategy" }
    fn analyze(&self, candles: &[Candle]) -> Result<Option<Signal>> {
        // logic here
    }
}
```

### Add an AI model

```rust
impl AIModel for MyModel {
    fn name(&self) -> &str { "xgboost_v1" }
    fn predict(&self, features: &FeatureRow) -> Result<AIAction> { ... }
    fn train(&mut self, data: &[FeatureRow]) -> Result<TrainMetrics> { ... }
    fn save(&self, path: &Path) -> Result<()> { ... }
    fn load(&mut self, path: &Path) -> Result<()> { ... }
}
```

### AI action space

| Action | Index | Description |
|--------|-------|-------------|
| Neutral | 0 | Hold / do nothing |
| EnterLong | 1 | Open long |
| ExitLong | 2 | Close long |
| EnterShort | 3 | Open short |
| ExitShort | 4 | Close short |

## Conventions

- `rust_decimal::Decimal` for all prices/volumes — never `f64`
- `f64` only in AI feature vectors
- Anti-repainting: never signal on the forming candle; indicators receive `&candles[..len-1]`
- All I/O is async (tokio); indicators are sync pure functions
- `anyhow::Result` in binaries; domain errors in libraries

## License

See [LICENSE](LICENSE) for details.
