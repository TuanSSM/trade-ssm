# Freqtrade Gap Analysis & TODO Definition of Done

Competitive analysis of [freqtrade](https://www.freqtrade.io/en/stable/) vs trade-ssm,
with prioritized TODO items and Definition of Done for each gap.

---

## Current trade-ssm Capabilities (Baseline)

| Area | What We Have |
|------|-------------|
| **Indicators** | CVD, RSI, ATR, EMA, MACD, Bollinger Bands, OBV, VWAP, liquidation analysis |
| **Strategies** | CVD momentum, order flow (delta divergence/imbalance), composite weighted voting |
| **Order Types** | Market, Limit, StopMarket, StopLimit, TakeProfitMarket, TakeProfitLimit, TrailingStop |
| **Execution** | Paper engine (full), Live engine (stubbed) |
| **AI/ML** | Feature pipeline (22 features), TradingEnv with hedge mode, TableModel, RL policy gradient |
| **Data** | Binance REST + WebSocket, candle download, trade aggregation |
| **Backtesting** | Offline indicator replay on historical candles |
| **Notifications** | Telegram alerts (formatted HTML) |
| **Infra** | NATS message bus, Docker, CI/CD, risk management (position limits, circuit breaker) |

---

## Gap Analysis: Freqtrade Features We're Missing

### Priority Legend
- **P0** — Critical for parity / production readiness
- **P1** — High value, significant competitive gap
- **P2** — Nice to have, improves developer/trader experience
- **P3** — Future consideration

---

## P0 — Critical

### TODO-001: Live Exchange Execution

**Gap:** Live order submission is stubbed (`tracing::warn!("live execution not yet implemented")`).
Freqtrade has full live trading with order lifecycle management.

**Location:** `crates/ssm-execution/src/engine.rs:77-81`

**Definition of Done:**
- [ ] `ExecutionEngine::submit_order()` calls `LiveEngine` methods for `ExecutionMode::Live`
- [ ] Orders placed on Binance Futures via authenticated REST API
- [ ] Order status polling / WebSocket fill updates implemented
- [ ] Order cancellation support (`cancel_order`)
- [ ] Partial fill handling with position updates
- [ ] Balance and position fetching before order placement (pre-flight checks)
- [ ] Rate limiting / retry logic for exchange API calls
- [ ] Integration test with Binance testnet (not mainnet)
- [ ] Paper vs Live mode switching via `EXECUTION_MODE` env var verified end-to-end
- [ ] `just ci` passes
- [ ] Error handling: network failures, insufficient balance, rejected orders

---

### TODO-002: Dynamic Stoploss & Take-Profit

**Gap:** We have static StopMarket/StopLimit/TakeProfitMarket/TakeProfitLimit order types.
Freqtrade supports dynamic trailing stoploss per trade (time-based, stepped, indicator-based, per-pair),
custom ROI tables, and stoploss-on-exchange.

**Definition of Done:**
- [ ] `Strategy` trait extended with `custom_stoploss(&self, trade: &Trade, candles: &[Candle]) -> Option<Decimal>` callback
- [ ] Time-based stoploss tightening (e.g., move to breakeven after N candles)
- [ ] Indicator-based stoploss (e.g., ATR-based trailing stop)
- [ ] Stepped stoploss (discrete levels based on profit thresholds)
- [ ] Stoploss-on-exchange: place actual stop orders on Binance (for live mode)
- [ ] ROI table support: configurable profit targets at time intervals
- [ ] Anti-repainting test: stoploss values must not change retroactively
- [ ] Paper execution tests for dynamic stoploss scenarios
- [ ] `just ci` passes

---

### TODO-003: Trade Lifecycle Callbacks

**Gap:** Our `Strategy` trait has a single `analyze()` method. Freqtrade has 13+ callbacks:
`custom_exit`, `confirm_trade_entry`, `confirm_trade_exit`, `adjust_trade_position`,
`order_filled`, `custom_stake_amount`, etc.

**Definition of Done:**
- [ ] `Strategy` trait extended with default-implemented callbacks:
  - `on_trade_enter(&self, signal: &Signal, position: &Position) -> bool` (confirm entry)
  - `on_trade_exit(&self, position: &Position, candles: &[Candle]) -> Option<ExitReason>` (custom exit)
  - `on_order_filled(&self, order: &Order, position: &Position)` (react to fills)
  - `custom_position_size(&self, signal: &Signal, balance: Decimal) -> Decimal` (dynamic sizing)
  - `should_adjust_position(&self, position: &Position, candles: &[Candle]) -> Option<Decimal>` (DCA)
- [ ] All callbacks have no-op default implementations (backward compatible)
- [ ] Execution engine invokes callbacks at appropriate lifecycle points
- [ ] At least one built-in strategy uses callbacks (e.g., CVD momentum with DCA)
- [ ] Unit tests for each callback path
- [ ] `just ci` passes

---

## P1 — High Value

### TODO-004: Hyperparameter Optimization

**Gap:** No hyperopt capability. Freqtrade has full hyperopt with configurable loss functions
(Sharpe, Sortino, Calmar, profit, drawdown), parameter spaces, and epoch-based search.

**Definition of Done:**
- [ ] `HyperParam` derive macro or builder for strategy parameters with search ranges
- [ ] `HyperoptRunner` that runs backtests across parameter combinations
- [ ] At least 2 loss functions: `SharpeRatio` and `MaxDrawdown`
- [ ] Grid search and random search modes
- [ ] Results serialized to JSON with ranked parameter sets
- [ ] CLI entrypoint: `just hyperopt <strategy> <data_file> [--epochs N]`
- [ ] Best parameters loadable by strategy at runtime
- [ ] `just ci` passes

---

### TODO-005: Backtesting Enhancements

**Gap:** Current backtest is indicator replay only. Freqtrade backtesting simulates full trade lifecycle
with PnL tracking, drawdown analysis, win rate, trade duration stats, funding fees, and leverage.

**Definition of Done:**
- [ ] Backtest engine simulates order fills against historical candles
- [ ] Position tracking with entry/exit matching and PnL calculation
- [ ] Trade statistics: total trades, win rate, avg profit, avg duration, best/worst trade
- [ ] Drawdown analysis: max drawdown (absolute and percentage), drawdown duration
- [ ] Sharpe ratio, Sortino ratio, profit factor calculations
- [ ] Funding fee simulation for futures positions
- [ ] Leverage support in backtest (margin impact on PnL)
- [ ] Results output as structured JSON + human-readable summary
- [ ] Anti-repainting enforced (only closed candles used for signals)
- [ ] `just ci` passes

---

### TODO-006: Multi-Pair / Portfolio Support

**Gap:** trade-ssm operates on a single symbol. Freqtrade handles pair lists, pair filters,
and portfolio-level management across many pairs simultaneously.

**Definition of Done:**
- [ ] `PairList` config: static list or dynamic filter (volume, price, spread)
- [ ] Analyzer service processes multiple symbols concurrently
- [ ] Portfolio-level position limits (max open trades, max exposure)
- [ ] Per-pair and portfolio-level risk tracking
- [ ] Backtest supports multi-pair replay with shared capital
- [ ] Correlation-aware position sizing (optional)
- [ ] `just ci` passes

---

### TODO-007: REST API for Bot Control

**Gap:** No API to monitor or control a running bot. Freqtrade has a full REST API with
trade management, performance analytics, config reload, force-enter/exit, and WebSocket streaming.

**Definition of Done:**
- [ ] HTTP server (axum/actix) embedded in analyzer service
- [ ] Endpoints: `GET /status`, `GET /trades`, `GET /profit`, `GET /balance`
- [ ] Control endpoints: `POST /forceexit`, `POST /reload_config`, `POST /start`, `POST /stop`
- [ ] JWT or API key authentication
- [ ] CORS configuration for web dashboard access
- [ ] Health check endpoint for Docker/k8s liveness probes
- [ ] OpenAPI spec generated or maintained
- [ ] Integration test for each endpoint
- [ ] `just ci` passes

---

### TODO-008: Advanced AI/ML Pipeline

**Gap:** We have a basic feature pipeline (22 features) and TableModel. Freqtrade FreqAI has:
automatic feature expansion across timeframes/pairs, sliding-window retraining,
outlier detection (DI, SVM, DBSCAN), model expiration, continual learning, and
multiple model backends (LightGBM, CatBoost, XGBoost, PyTorch).

**Definition of Done:**
- [ ] Feature expansion: auto-generate features across multiple timeframes
- [ ] Correlated pair features: include features from related symbols (e.g., ETHUSDT for BTCUSDT)
- [ ] Sliding-window retraining: periodic model refresh with configurable window
- [ ] Model expiration: predictions rejected if model is older than `expiration_hours`
- [ ] Outlier detection: DI (Dissimilarity Index) to flag low-confidence predictions
- [ ] Model persistence: save/load for all model types (not just TableModel)
- [ ] At least one additional model backend (e.g., XGBoost via FFI or ONNX)
- [ ] Training metrics tracked and logged
- [ ] `just ci` passes
- [ ] AI model trait implemented for new models

---

## P2 — Nice to Have

### TODO-009: Webhook Notifications

**Gap:** We only have Telegram. Freqtrade supports webhooks (generic HTTP POST),
Discord, custom strategy messages, and template variables.

**Definition of Done:**
- [ ] `WebhookNotifier` that sends HTTP POST on trade events
- [ ] Configurable payload templates with variables (`{pair}`, `{profit}`, `{action}`, etc.)
- [ ] Discord webhook preset (rich embed format)
- [ ] Strategy-initiated custom messages via notification channel
- [ ] Retry logic with exponential backoff
- [ ] `just ci` passes

---

### TODO-010: Plotting & Visualization

**Gap:** No charting or visualization. Freqtrade has interactive Plotly charts with
candlesticks, indicators, buy/sell markers, and profit charts.

**Definition of Done:**
- [ ] Backtest results exportable as JSON with OHLCV + indicator + signal data
- [ ] HTML chart generator (Plotly.js or similar) from backtest output
- [ ] Candlestick chart with overlay indicators (EMA, Bollinger Bands)
- [ ] Sub-charts for volume indicators (CVD, OBV)
- [ ] Entry/exit markers on chart with profit annotation
- [ ] Equity curve chart
- [ ] CLI command: `just plot <backtest_results.json>`
- [ ] `just ci` passes

---

### TODO-011: Telegram Interactive Bot

**Gap:** Current Telegram is one-way alerts. Freqtrade's Telegram bot supports
interactive commands: `/status`, `/profit`, `/balance`, `/forceexit`, `/performance`,
`/daily`, `/start`, `/stop`.

**Definition of Done:**
- [ ] Telegram bot with command handler (polling or webhook)
- [ ] Commands: `/status` (open positions), `/profit` (PnL summary), `/balance`
- [ ] Control commands: `/start`, `/stop`, `/forceexit <pair>`
- [ ] `/daily` and `/weekly` profit summaries
- [ ] Command authorization (allowed chat IDs only)
- [ ] `just ci` passes

---

### TODO-012: CLI Utility Commands

**Gap:** Limited CLI. Freqtrade has `list-exchanges`, `list-pairs`, `list-timeframes`,
`test-pairlist`, `new-strategy` scaffolding, `show-config`, `convert-db`.

**Definition of Done:**
- [ ] `trade-ssm list-pairs` — query available pairs from Binance
- [ ] `trade-ssm list-timeframes` — show supported intervals
- [ ] `trade-ssm show-config` — display current config with secrets redacted
- [ ] `trade-ssm new-strategy <name>` — scaffold a strategy from template
- [ ] `trade-ssm show-trades <db_or_json>` — inspect trade history
- [ ] Help text and `--help` for all subcommands
- [ ] `just ci` passes

---

### TODO-013: Producer/Consumer Multi-Bot Architecture

**Gap:** No multi-bot coordination. Freqtrade supports producer/consumer mode where
one bot computes indicators and broadcasts to consumers via WebSocket.

**Definition of Done:**
- [ ] NATS-based pub/sub for indicator broadcasts (leverage existing NATS infra)
- [ ] Producer mode: publish analyzed candle data to NATS topic
- [ ] Consumer mode: subscribe to producer's indicator data, merge with local analysis
- [ ] Consumer can use producer signals directly or apply own strategy
- [ ] Multiple producers supported per consumer
- [ ] Authentication/authorization on NATS channels
- [ ] Integration test with producer + consumer services
- [ ] `just ci` passes

---

## P3 — Future Consideration

### TODO-014: Multi-Exchange Support

**Gap:** Binance-only. Freqtrade supports 30+ exchanges via ccxt.

**Definition of Done:**
- [ ] `Exchange` trait abstraction over exchange-specific REST/WS APIs
- [ ] At least one additional exchange (e.g., Bybit or OKX)
- [ ] Exchange-agnostic order types and position types in ssm-core
- [ ] Exchange selection via config/env var
- [ ] `just ci` passes

---

### TODO-015: Edge Positioning / Risk-Adjusted Sizing

**Gap:** No statistical edge analysis. Freqtrade's Edge calculates win rate, risk-reward ratio,
and expectancy per pair to dynamically adjust position sizes and filter out low-edge pairs.

**Definition of Done:**
- [ ] `EdgeAnalyzer` that computes per-strategy stats from backtest results:
  win rate, avg win, avg loss, expectancy, risk-reward ratio
- [ ] Position sizing based on Kelly criterion or fixed-risk-per-trade
- [ ] Pair filtering: skip trades where expectancy < threshold
- [ ] Integration with backtest results and live trading
- [ ] `just ci` passes

---

### TODO-016: Leverage & Margin Mode Support

**Gap:** Basic leverage exists in types but no dynamic leverage selection per trade.
Freqtrade supports per-pair leverage via callback, liquidation buffer, isolated/cross margin.

**Definition of Done:**
- [ ] `Strategy` callback: `leverage(&self, pair: &str, signal: &Signal) -> Decimal`
- [ ] Liquidation price calculation and buffer enforcement
- [ ] Isolated vs cross margin mode configuration
- [ ] Funding rate tracking for PnL accuracy
- [ ] Backtest accounts for leverage impact on PnL and liquidation risk
- [ ] `just ci` passes

---

### TODO-017: Advanced RL Agents

**Gap:** Single policy-gradient RL agent. Freqtrade/FreqAI supports multiple RL algorithms.

**Definition of Done:**
- [ ] PPO (Proximal Policy Optimization) agent implementation
- [ ] Configurable reward function (not just PnL — include drawdown penalty, Sharpe-based)
- [ ] Experience replay buffer for off-policy methods
- [ ] Training checkpointing and resume
- [ ] Comparison benchmarks: TableModel vs PPO on same data
- [ ] `just ci` passes
- [ ] AI model trait implemented for new agents

---

### TODO-018: Protection Plugins

**Gap:** No automated circuit-breaker plugins beyond basic position limits. Freqtrade has:
StoplossGuard (halt after N stoplosses), MaxDrawdown (halt when drawdown exceeded),
CooldownPeriod (lock pair after exit), LowProfitPairs (lock underperformers).

**Definition of Done:**
- [ ] `Protection` trait with `should_lock(&self, trades: &[Trade], pair: &str) -> Option<Duration>`
- [ ] `StoplossGuard`: halt trading after N stoplosses within configurable time window
- [ ] `MaxDrawdown`: pause trading when portfolio drawdown exceeds threshold
- [ ] `CooldownPeriod`: lock pair for N candles after exit to prevent re-entry churn
- [ ] `LowProfitPairs`: lock pairs with negative performance over lookback window
- [ ] Protection stack: multiple protections composable in sequence
- [ ] Pair lock state tracked in execution engine
- [ ] Locks surfaced in Telegram notifications
- [ ] Unit tests for each protection type
- [ ] `just ci` passes

---

### TODO-019: Pairlist Plugins & Filters

**Gap:** No dynamic pair selection. Freqtrade has VolumePairList, PercentChangePairList,
MarketCapPairList, plus filters (AgeFilter, SpreadFilter, VolatilityFilter, PriceFilter, etc.).

**Definition of Done:**
- [ ] `PairListProvider` trait: `fn pairs(&self) -> Result<Vec<String>>`
- [ ] `StaticPairList`: config-defined list with regex wildcard support
- [ ] `VolumePairList`: sort/filter by 24h volume from exchange API
- [ ] `PairFilter` trait: `fn filter(&self, pairs: &[String]) -> Vec<String>`
- [ ] At least 3 filters: `PriceFilter`, `SpreadFilter`, `VolatilityFilter`
- [ ] Filter chain: composable pipeline of filters applied sequentially
- [ ] Refresh interval for dynamic pair lists in live mode
- [ ] `just ci` passes

---

## Summary Matrix

| ID | Feature | Priority | Effort | Status |
|----|---------|----------|--------|--------|
| TODO-001 | Live Exchange Execution | P0 | Large | **Done** — LiveEngine wired to ExecutionEngine, async submit/cancel/query, preflight checks, testnet support, retry logic |
| TODO-002 | Dynamic Stoploss & Take-Profit | P0 | Medium | **Done** — StoplossManager (Fixed/ATR/TimeBased/Stepped), ROI table, anti-repainting tests |
| TODO-003 | Trade Lifecycle Callbacks | P0 | Medium | **Done** — Strategy trait extended with 7 callbacks, CvdMomentum demonstrates all callbacks, unit tests per path |
| TODO-004 | Hyperparameter Optimization | P1 | Large | **Done** — HyperoptRunner (grid + random search), 5 loss functions, ranked JSON results |
| TODO-005 | Backtesting Enhancements | P1 | Large | **Done** — BacktestEngine with full trade sim, PnL/fees/drawdown/Sharpe/Sortino/profit factor, funding fees, leverage |
| TODO-006 | Multi-Pair / Portfolio | P1 | Large | **Done** — PortfolioManager with per-pair + total exposure limits, max trades, correlation check, PnL tracking |
| TODO-007 | REST API for Bot Control | P1 | Medium | **Done** — axum-based API service with status/trades/profit/balance/forceexit/start/stop, API key auth, CORS, health endpoint |
| TODO-008 | Advanced AI/ML Pipeline | P1 | XL | **Done** — MultiTimeframeFeatures, CorrelatedPairFeatures, ModelManager (sliding-window retraining + expiration), DissimilarityIndex outlier detection |
| TODO-009 | Webhook Notifications | P2 | Small | **Done** — WebhookNotifier (HTTP POST + templates + retries), DiscordNotifier (rich embeds) |
| TODO-010 | Plotting & Visualization | P2 | Medium | **Done** — ChartData with Plotly.js HTML generation, candlestick/indicator/signal/equity charts |
| TODO-011 | Telegram Interactive Bot | P2 | Medium | **Done** — InteractiveTelegramBot with command parsing (/status, /profit, /balance, /forceexit, etc.), authorization |
| TODO-012 | CLI Utility Commands | P2 | Small | **Done** — trade-ssm CLI with list-pairs, list-timeframes, show-config, new-strategy, show-trades, plot, help |
| TODO-013 | Producer/Consumer Multi-Bot | P2 | Large | **Done** — NATS-based Producer/Consumer with AnalyzedData broadcast, multi-producer subscription |
| TODO-014 | Multi-Exchange Support | P3 | XL | **Done** — Exchange trait, BinanceClient + BybitClient implementations, create_exchange() factory |
| TODO-015 | Edge Positioning | P3 | Medium | **Done** — EdgeAnalyzer (win rate, expectancy, risk-reward), pair filtering, Kelly sizing |
| TODO-016 | Leverage & Margin Modes | P3 | Medium | **Done** — LeverageManager (margin calc, liquidation price, buffer check, funding fees), Isolated/Cross modes |
| TODO-017 | Advanced RL Agents | P3 | Large | **Done** — PpoAgent (softmax policy, value function, GAE, clipped updates), ReplayBuffer, AIModel impl |
| TODO-018 | Protection Plugins | P1 | Medium | **Done** — StoplossGuard, MaxDrawdownProtection, CooldownPeriod, LowProfitPairs, ProtectionStack |
| TODO-019 | Pairlist Plugins & Filters | P2 | Medium | **Done** — StaticPairList (regex), VolumePairList, PriceFilter, SpreadFilter, VolatilityFilter, FilterChain |

---

## Recommended Implementation Order

1. **TODO-005** (Backtest Enhancements) — Foundation for validating everything else
2. **TODO-001** (Live Execution) — Core production requirement
3. **TODO-002** (Dynamic Stoploss) — Critical for risk management
4. **TODO-003** (Lifecycle Callbacks) — Enables strategy flexibility
5. **TODO-004** (Hyperopt) — Leverages improved backtesting
6. **TODO-007** (REST API) — Operational monitoring
7. **TODO-006** (Multi-Pair) — Scale out
8. **TODO-008** (Advanced ML) — Competitive AI edge
