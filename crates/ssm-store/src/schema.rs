/// SQL schema for trade-ssm persistence layer.
///
/// Tables: positions, orders, trades (completed), signals, config snapshots.
/// All Decimal values stored as TEXT to preserve precision.
pub const CREATE_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS positions (
    symbol        TEXT PRIMARY KEY,
    side          TEXT NOT NULL,
    entry_price   TEXT NOT NULL,
    quantity      TEXT NOT NULL,
    unrealized_pnl TEXT NOT NULL DEFAULT '0',
    realized_pnl  TEXT NOT NULL DEFAULT '0',
    leverage      INTEGER NOT NULL DEFAULT 1,
    opened_at     INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS orders (
    id            TEXT PRIMARY KEY,
    symbol        TEXT NOT NULL,
    side          TEXT NOT NULL,
    order_type    TEXT NOT NULL,
    quantity      TEXT NOT NULL,
    price         TEXT,
    stop_price    TEXT,
    trailing_delta TEXT,
    time_in_force TEXT,
    reduce_only   INTEGER NOT NULL DEFAULT 0,
    status        TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS trades (
    id              TEXT PRIMARY KEY,
    symbol          TEXT NOT NULL,
    side            TEXT NOT NULL,
    entry_price     TEXT NOT NULL,
    exit_price      TEXT NOT NULL,
    quantity        TEXT NOT NULL,
    profit          TEXT NOT NULL,
    profit_pct      TEXT NOT NULL,
    entry_time      INTEGER NOT NULL,
    exit_time       INTEGER NOT NULL,
    duration_candles INTEGER NOT NULL,
    exit_reason     TEXT NOT NULL,
    leverage        INTEGER NOT NULL DEFAULT 1,
    fee             TEXT NOT NULL DEFAULT '0',
    strategy        TEXT NOT NULL DEFAULT '',
    confidence      TEXT NOT NULL DEFAULT '0'
);

CREATE TABLE IF NOT EXISTS signals (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp   INTEGER NOT NULL,
    symbol      TEXT NOT NULL,
    action      TEXT NOT NULL,
    confidence  TEXT NOT NULL,
    strategy    TEXT NOT NULL DEFAULT '',
    received_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_trades_symbol ON trades(symbol);
CREATE INDEX IF NOT EXISTS idx_trades_exit_time ON trades(exit_time);
CREATE INDEX IF NOT EXISTS idx_signals_timestamp ON signals(timestamp);
CREATE INDEX IF NOT EXISTS idx_orders_symbol ON orders(symbol);
"#;
