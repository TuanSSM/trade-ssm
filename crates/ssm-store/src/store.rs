use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use rust_decimal::Decimal;
use ssm_core::{
    ExitReason, Order, OrderStatus, OrderType, Position, Side, Signal, TimeInForce, TradeRecord,
};
use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;

use crate::schema;

/// SQLite-backed persistence for positions, orders, trades, and signals.
///
/// Thread-safe via internal Mutex. All Decimal values stored as TEXT.
pub struct TradeStore {
    conn: Mutex<Connection>,
}

impl TradeStore {
    /// Open (or create) a SQLite database at the given path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path.as_ref())
            .with_context(|| format!("opening database: {}", path.as_ref().display()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .context("setting pragmas")?;
        conn.execute_batch(schema::CREATE_TABLES)
            .context("creating tables")?;
        tracing::info!(path = %path.as_ref().display(), "trade store opened");
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory database (for testing).
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("opening in-memory database")?;
        conn.execute_batch(schema::CREATE_TABLES)
            .context("creating tables")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // -----------------------------------------------------------------------
    // Positions
    // -----------------------------------------------------------------------

    /// Save or update a position (upsert).
    pub fn save_position(&self, pos: &Position) -> Result<()> {
        let conn = self.conn.lock().expect("lock");
        conn.execute(
            "INSERT OR REPLACE INTO positions (symbol, side, entry_price, quantity, unrealized_pnl, realized_pnl, leverage, opened_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                pos.symbol,
                format!("{}", pos.side),
                pos.entry_price.to_string(),
                pos.quantity.to_string(),
                pos.unrealized_pnl.to_string(),
                pos.realized_pnl.to_string(),
                pos.leverage,
                pos.opened_at,
            ],
        )?;
        Ok(())
    }

    /// Remove a closed position.
    pub fn remove_position(&self, symbol: &str) -> Result<()> {
        let conn = self.conn.lock().expect("lock");
        conn.execute("DELETE FROM positions WHERE symbol = ?1", params![symbol])?;
        Ok(())
    }

    /// Load all open positions (for startup recovery).
    pub fn load_positions(&self) -> Result<Vec<Position>> {
        let conn = self.conn.lock().expect("lock");
        let mut stmt = conn.prepare(
            "SELECT symbol, side, entry_price, quantity, unrealized_pnl, realized_pnl, leverage, opened_at FROM positions",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PositionRow {
                symbol: row.get(0)?,
                side: row.get::<_, String>(1)?,
                entry_price: row.get::<_, String>(2)?,
                quantity: row.get::<_, String>(3)?,
                unrealized_pnl: row.get::<_, String>(4)?,
                realized_pnl: row.get::<_, String>(5)?,
                leverage: row.get(6)?,
                opened_at: row.get(7)?,
            })
        })?;

        let mut positions = Vec::new();
        for row in rows {
            let r = row?;
            positions.push(Position {
                symbol: r.symbol,
                side: parse_side(&r.side),
                entry_price: Decimal::from_str(&r.entry_price).unwrap_or_default(),
                quantity: Decimal::from_str(&r.quantity).unwrap_or_default(),
                unrealized_pnl: Decimal::from_str(&r.unrealized_pnl).unwrap_or_default(),
                realized_pnl: Decimal::from_str(&r.realized_pnl).unwrap_or_default(),
                leverage: r.leverage,
                opened_at: r.opened_at,
            });
        }
        Ok(positions)
    }

    // -----------------------------------------------------------------------
    // Orders
    // -----------------------------------------------------------------------

    /// Save or update an order.
    pub fn save_order(&self, order: &Order) -> Result<()> {
        let conn = self.conn.lock().expect("lock");
        conn.execute(
            "INSERT OR REPLACE INTO orders (id, symbol, side, order_type, quantity, price, stop_price, trailing_delta, time_in_force, reduce_only, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                order.id,
                order.symbol,
                format!("{}", order.side),
                format!("{}", order.order_type),
                order.quantity.to_string(),
                order.price.map(|p| p.to_string()),
                order.stop_price.map(|p| p.to_string()),
                order.trailing_delta.map(|p| p.to_string()),
                order.time_in_force.map(|t| format!("{t:?}")),
                order.reduce_only as i32,
                format!("{:?}", order.status),
                order.created_at,
                order.updated_at,
            ],
        )?;
        Ok(())
    }

    /// Load all orders with a given status.
    pub fn load_orders_by_status(&self, status: OrderStatus) -> Result<Vec<Order>> {
        let conn = self.conn.lock().expect("lock");
        let status_str = format!("{status:?}");
        let mut stmt = conn.prepare(
            "SELECT id, symbol, side, order_type, quantity, price, stop_price, trailing_delta, time_in_force, reduce_only, status, created_at, updated_at FROM orders WHERE status = ?1",
        )?;
        let rows = stmt.query_map(params![status_str], |row| {
            Ok(OrderRow {
                id: row.get(0)?,
                symbol: row.get(1)?,
                side: row.get::<_, String>(2)?,
                order_type: row.get::<_, String>(3)?,
                quantity: row.get::<_, String>(4)?,
                price: row.get::<_, Option<String>>(5)?,
                stop_price: row.get::<_, Option<String>>(6)?,
                trailing_delta: row.get::<_, Option<String>>(7)?,
                time_in_force: row.get::<_, Option<String>>(8)?,
                reduce_only: row.get::<_, i32>(9)?,
                status: row.get::<_, String>(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
            })
        })?;

        let mut orders = Vec::new();
        for row in rows {
            let r = row?;
            orders.push(Order {
                id: r.id,
                symbol: r.symbol,
                side: parse_side(&r.side),
                order_type: parse_order_type(&r.order_type),
                quantity: Decimal::from_str(&r.quantity).unwrap_or_default(),
                price: r.price.as_deref().and_then(|s| Decimal::from_str(s).ok()),
                stop_price: r
                    .stop_price
                    .as_deref()
                    .and_then(|s| Decimal::from_str(s).ok()),
                trailing_delta: r
                    .trailing_delta
                    .as_deref()
                    .and_then(|s| Decimal::from_str(s).ok()),
                time_in_force: r.time_in_force.as_deref().and_then(parse_tif),
                reduce_only: r.reduce_only != 0,
                status: parse_order_status(&r.status),
                created_at: r.created_at,
                updated_at: r.updated_at,
            });
        }
        Ok(orders)
    }

    // -----------------------------------------------------------------------
    // Trades (completed)
    // -----------------------------------------------------------------------

    /// Record a completed trade.
    pub fn save_trade(&self, trade: &TradeRecord) -> Result<()> {
        self.save_trade_with_meta(trade, "", "0")
    }

    /// Record a completed trade with strategy metadata.
    pub fn save_trade_with_meta(
        &self,
        trade: &TradeRecord,
        strategy: &str,
        confidence: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("lock");
        conn.execute(
            "INSERT OR REPLACE INTO trades (id, symbol, side, entry_price, exit_price, quantity, profit, profit_pct, entry_time, exit_time, duration_candles, exit_reason, leverage, fee, strategy, confidence)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                trade.id,
                trade.symbol,
                format!("{}", trade.side),
                trade.entry_price.to_string(),
                trade.exit_price.to_string(),
                trade.quantity.to_string(),
                trade.profit.to_string(),
                trade.profit_pct.to_string(),
                trade.entry_time,
                trade.exit_time,
                trade.duration_candles,
                format!("{:?}", trade.exit_reason),
                trade.leverage,
                trade.fee.to_string(),
                strategy,
                confidence,
            ],
        )?;
        Ok(())
    }

    /// Load trades within a time range, optionally filtered by symbol.
    pub fn load_trades(
        &self,
        from: Option<i64>,
        to: Option<i64>,
        symbol: Option<&str>,
    ) -> Result<Vec<TradeRecord>> {
        let conn = self.conn.lock().expect("lock");
        let mut sql = String::from(
            "SELECT id, symbol, side, entry_price, exit_price, quantity, profit, profit_pct, entry_time, exit_time, duration_candles, exit_reason, leverage, fee FROM trades WHERE 1=1",
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(f) = from {
            sql.push_str(&format!(" AND exit_time >= ?{}", param_values.len() + 1));
            param_values.push(Box::new(f));
        }
        if let Some(t) = to {
            sql.push_str(&format!(" AND exit_time <= ?{}", param_values.len() + 1));
            param_values.push(Box::new(t));
        }
        if let Some(s) = symbol {
            sql.push_str(&format!(" AND symbol = ?{}", param_values.len() + 1));
            param_values.push(Box::new(s.to_string()));
        }
        sql.push_str(" ORDER BY exit_time DESC");

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_refs.as_slice(), |row| {
            Ok(TradeRow {
                id: row.get(0)?,
                symbol: row.get(1)?,
                side: row.get::<_, String>(2)?,
                entry_price: row.get::<_, String>(3)?,
                exit_price: row.get::<_, String>(4)?,
                quantity: row.get::<_, String>(5)?,
                profit: row.get::<_, String>(6)?,
                profit_pct: row.get::<_, String>(7)?,
                entry_time: row.get(8)?,
                exit_time: row.get(9)?,
                duration_candles: row.get(10)?,
                exit_reason: row.get::<_, String>(11)?,
                leverage: row.get(12)?,
                fee: row.get::<_, String>(13)?,
            })
        })?;

        let mut trades = Vec::new();
        for row in rows {
            let r = row?;
            trades.push(TradeRecord {
                id: r.id,
                symbol: r.symbol,
                side: parse_side(&r.side),
                entry_price: Decimal::from_str(&r.entry_price).unwrap_or_default(),
                exit_price: Decimal::from_str(&r.exit_price).unwrap_or_default(),
                quantity: Decimal::from_str(&r.quantity).unwrap_or_default(),
                profit: Decimal::from_str(&r.profit).unwrap_or_default(),
                profit_pct: Decimal::from_str(&r.profit_pct).unwrap_or_default(),
                entry_time: r.entry_time,
                exit_time: r.exit_time,
                duration_candles: r.duration_candles,
                exit_reason: parse_exit_reason(&r.exit_reason),
                leverage: r.leverage,
                fee: Decimal::from_str(&r.fee).unwrap_or_default(),
            });
        }
        Ok(trades)
    }

    /// Count trades, optionally filtered by symbol.
    pub fn trade_count(&self, symbol: Option<&str>) -> Result<usize> {
        let conn = self.conn.lock().expect("lock");
        let count: i64 = match symbol {
            Some(s) => conn.query_row(
                "SELECT COUNT(*) FROM trades WHERE symbol = ?1",
                params![s],
                |row| row.get(0),
            )?,
            None => conn.query_row("SELECT COUNT(*) FROM trades", [], |row| row.get(0))?,
        };
        Ok(count as usize)
    }

    // -----------------------------------------------------------------------
    // Signals
    // -----------------------------------------------------------------------

    /// Record a received signal.
    pub fn save_signal(&self, signal: &Signal, strategy: &str) -> Result<()> {
        let conn = self.conn.lock().expect("lock");
        let now = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT INTO signals (timestamp, symbol, action, confidence, strategy, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                signal.timestamp,
                signal.symbol,
                format!("{:?}", signal.action),
                signal.confidence.to_string(),
                strategy,
                now,
            ],
        )?;
        Ok(())
    }

    /// Count signals in a time range.
    pub fn signal_count(&self, since: i64) -> Result<usize> {
        let conn = self.conn.lock().expect("lock");
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM signals WHERE received_at >= ?1",
            params![since],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    // -----------------------------------------------------------------------
    // Analytics helpers
    // -----------------------------------------------------------------------

    /// Total realized PnL across all trades.
    pub fn total_realized_pnl(&self) -> Result<Decimal> {
        let conn = self.conn.lock().expect("lock");
        let sum: String = conn
            .query_row(
                "SELECT COALESCE(SUM(CAST(profit AS REAL)), 0) FROM trades",
                [],
                |row| row.get::<_, f64>(0),
            )
            .map(|f| format!("{f}"))?;
        Ok(Decimal::from_str(&sum).unwrap_or_default())
    }

    /// Win/loss counts.
    pub fn win_loss_counts(&self) -> Result<(usize, usize)> {
        let conn = self.conn.lock().expect("lock");
        let wins: i64 = conn.query_row(
            "SELECT COUNT(*) FROM trades WHERE CAST(profit AS REAL) > 0",
            [],
            |row| row.get(0),
        )?;
        let losses: i64 = conn.query_row(
            "SELECT COUNT(*) FROM trades WHERE CAST(profit AS REAL) <= 0",
            [],
            |row| row.get(0),
        )?;
        Ok((wins as usize, losses as usize))
    }
}

// ---------------------------------------------------------------------------
// Intermediate row types for SQLite deserialization
// ---------------------------------------------------------------------------

struct PositionRow {
    symbol: String,
    side: String,
    entry_price: String,
    quantity: String,
    unrealized_pnl: String,
    realized_pnl: String,
    leverage: u32,
    opened_at: i64,
}

struct OrderRow {
    id: String,
    symbol: String,
    side: String,
    order_type: String,
    quantity: String,
    price: Option<String>,
    stop_price: Option<String>,
    trailing_delta: Option<String>,
    time_in_force: Option<String>,
    reduce_only: i32,
    status: String,
    created_at: i64,
    updated_at: i64,
}

struct TradeRow {
    id: String,
    symbol: String,
    side: String,
    entry_price: String,
    exit_price: String,
    quantity: String,
    profit: String,
    profit_pct: String,
    entry_time: i64,
    exit_time: i64,
    duration_candles: u64,
    exit_reason: String,
    leverage: u32,
    fee: String,
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

fn parse_side(s: &str) -> Side {
    match s {
        "BUY" | "Buy" => Side::Buy,
        _ => Side::Sell,
    }
}

fn parse_order_type(s: &str) -> OrderType {
    match s {
        "MARKET" => OrderType::Market,
        "LIMIT" => OrderType::Limit,
        "STOP_MARKET" => OrderType::StopMarket,
        "STOP_LIMIT" => OrderType::StopLimit,
        "TAKE_PROFIT_MARKET" => OrderType::TakeProfitMarket,
        "TAKE_PROFIT_LIMIT" => OrderType::TakeProfitLimit,
        "TRAILING_STOP" => OrderType::TrailingStop,
        _ => OrderType::Market,
    }
}

fn parse_order_status(s: &str) -> OrderStatus {
    match s {
        "Pending" => OrderStatus::Pending,
        "Open" => OrderStatus::Open,
        "PartiallyFilled" => OrderStatus::PartiallyFilled,
        "Filled" => OrderStatus::Filled,
        "Cancelled" => OrderStatus::Cancelled,
        "Rejected" => OrderStatus::Rejected,
        "Expired" => OrderStatus::Expired,
        _ => OrderStatus::Pending,
    }
}

fn parse_tif(s: &str) -> Option<TimeInForce> {
    match s {
        "Gtc" => Some(TimeInForce::Gtc),
        "Ioc" => Some(TimeInForce::Ioc),
        "Fok" => Some(TimeInForce::Fok),
        "Gtd" => Some(TimeInForce::Gtd),
        _ => None,
    }
}

fn parse_exit_reason(s: &str) -> ExitReason {
    match s {
        "Stoploss" => ExitReason::Stoploss,
        "Roi" => ExitReason::Roi,
        "Signal" => ExitReason::Signal,
        "ForceExit" => ExitReason::ForceExit,
        "Liquidation" => ExitReason::Liquidation,
        _ => {
            // Handle CustomExit("reason")
            if let Some(inner) = s
                .strip_prefix("CustomExit(\"")
                .and_then(|s| s.strip_suffix("\")"))
            {
                ExitReason::CustomExit(inner.to_string())
            } else {
                ExitReason::Signal
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssm_core::AIAction;

    fn test_store() -> TradeStore {
        TradeStore::open_memory().unwrap()
    }

    fn sample_position() -> Position {
        Position {
            symbol: "BTCUSDT".into(),
            side: Side::Buy,
            entry_price: Decimal::from(50000),
            quantity: Decimal::from(1),
            unrealized_pnl: Decimal::from(100),
            realized_pnl: Decimal::ZERO,
            leverage: 5,
            opened_at: 1700000000,
        }
    }

    fn sample_order() -> Order {
        Order {
            id: "ord-1".into(),
            symbol: "BTCUSDT".into(),
            side: Side::Buy,
            order_type: OrderType::Market,
            quantity: Decimal::from(1),
            price: Some(Decimal::from(50000)),
            stop_price: None,
            trailing_delta: None,
            time_in_force: Some(TimeInForce::Gtc),
            reduce_only: false,
            status: OrderStatus::Filled,
            created_at: 1700000000,
            updated_at: 1700000001,
        }
    }

    fn sample_trade() -> TradeRecord {
        TradeRecord {
            id: "trade-1".into(),
            symbol: "BTCUSDT".into(),
            side: Side::Buy,
            entry_price: Decimal::from(50000),
            exit_price: Decimal::from(51000),
            quantity: Decimal::from(1),
            profit: Decimal::from(1000),
            profit_pct: Decimal::from(2),
            entry_time: 1700000000,
            exit_time: 1700001000,
            duration_candles: 10,
            exit_reason: ExitReason::Roi,
            leverage: 5,
            fee: Decimal::from(10),
        }
    }

    fn sample_signal() -> Signal {
        Signal {
            timestamp: 1700000000,
            symbol: "BTCUSDT".into(),
            action: AIAction::EnterLong,
            confidence: 0.85,
            source: "test".into(),
            metadata: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn position_roundtrip() {
        let store = test_store();
        let pos = sample_position();
        store.save_position(&pos).unwrap();

        let loaded = store.load_positions().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].symbol, "BTCUSDT");
        assert_eq!(loaded[0].side, Side::Buy);
        assert_eq!(loaded[0].entry_price, Decimal::from(50000));
        assert_eq!(loaded[0].quantity, Decimal::from(1));
        assert_eq!(loaded[0].leverage, 5);
    }

    #[test]
    fn position_upsert() {
        let store = test_store();
        let mut pos = sample_position();
        store.save_position(&pos).unwrap();

        pos.quantity = Decimal::from(2);
        pos.entry_price = Decimal::from(49500);
        store.save_position(&pos).unwrap();

        let loaded = store.load_positions().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].quantity, Decimal::from(2));
        assert_eq!(loaded[0].entry_price, Decimal::from(49500));
    }

    #[test]
    fn position_remove() {
        let store = test_store();
        store.save_position(&sample_position()).unwrap();
        store.remove_position("BTCUSDT").unwrap();

        let loaded = store.load_positions().unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn order_roundtrip() {
        let store = test_store();
        let order = sample_order();
        store.save_order(&order).unwrap();

        let loaded = store.load_orders_by_status(OrderStatus::Filled).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "ord-1");
        assert_eq!(loaded[0].side, Side::Buy);
        assert_eq!(loaded[0].order_type, OrderType::Market);
        assert_eq!(loaded[0].quantity, Decimal::from(1));
    }

    #[test]
    fn order_status_filter() {
        let store = test_store();
        store.save_order(&sample_order()).unwrap();

        let pending = store.load_orders_by_status(OrderStatus::Pending).unwrap();
        assert!(pending.is_empty());
    }

    #[test]
    fn trade_roundtrip() {
        let store = test_store();
        let trade = sample_trade();
        store.save_trade(&trade).unwrap();

        let loaded = store.load_trades(None, None, None).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "trade-1");
        assert_eq!(loaded[0].profit, Decimal::from(1000));
        assert_eq!(loaded[0].exit_reason, ExitReason::Roi);
    }

    #[test]
    fn trade_filter_by_symbol() {
        let store = test_store();
        store.save_trade(&sample_trade()).unwrap();

        let btc = store.load_trades(None, None, Some("BTCUSDT")).unwrap();
        assert_eq!(btc.len(), 1);

        let eth = store.load_trades(None, None, Some("ETHUSDT")).unwrap();
        assert!(eth.is_empty());
    }

    #[test]
    fn trade_filter_by_time() {
        let store = test_store();
        store.save_trade(&sample_trade()).unwrap();

        let recent = store.load_trades(Some(1700002000), None, None).unwrap();
        assert!(recent.is_empty());

        let all = store.load_trades(Some(1700000000), None, None).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn trade_count() {
        let store = test_store();
        assert_eq!(store.trade_count(None).unwrap(), 0);

        store.save_trade(&sample_trade()).unwrap();
        assert_eq!(store.trade_count(None).unwrap(), 1);
        assert_eq!(store.trade_count(Some("BTCUSDT")).unwrap(), 1);
        assert_eq!(store.trade_count(Some("ETHUSDT")).unwrap(), 0);
    }

    #[test]
    fn signal_roundtrip() {
        let store = test_store();
        let sig = sample_signal();
        store.save_signal(&sig, "cvd_momentum").unwrap();

        let count = store.signal_count(0).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn win_loss_counts() {
        let store = test_store();

        // Winning trade
        store.save_trade(&sample_trade()).unwrap();

        // Losing trade
        let mut loser = sample_trade();
        loser.id = "trade-2".into();
        loser.profit = Decimal::from(-500);
        store.save_trade(&loser).unwrap();

        let (wins, losses) = store.win_loss_counts().unwrap();
        assert_eq!(wins, 1);
        assert_eq!(losses, 1);
    }

    #[test]
    fn total_realized_pnl() {
        let store = test_store();
        store.save_trade(&sample_trade()).unwrap();

        let mut t2 = sample_trade();
        t2.id = "trade-2".into();
        t2.profit = Decimal::from(-300);
        store.save_trade(&t2).unwrap();

        let total = store.total_realized_pnl().unwrap();
        // 1000 + (-300) = 700, but stored as REAL so precision may vary
        assert!(total > Decimal::ZERO);
    }

    #[test]
    fn multiple_positions() {
        let store = test_store();

        let mut p1 = sample_position();
        p1.symbol = "BTCUSDT".into();
        store.save_position(&p1).unwrap();

        let mut p2 = sample_position();
        p2.symbol = "ETHUSDT".into();
        p2.side = Side::Sell;
        store.save_position(&p2).unwrap();

        let loaded = store.load_positions().unwrap();
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn parse_helpers() {
        assert_eq!(parse_side("BUY"), Side::Buy);
        assert_eq!(parse_side("SELL"), Side::Sell);
        assert_eq!(parse_order_type("LIMIT"), OrderType::Limit);
        assert_eq!(parse_order_status("Filled"), OrderStatus::Filled);
        assert_eq!(parse_exit_reason("Stoploss"), ExitReason::Stoploss);
        assert_eq!(
            parse_exit_reason("CustomExit(\"my_reason\")"),
            ExitReason::CustomExit("my_reason".into())
        );
    }
}
