use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Market data
// ---------------------------------------------------------------------------

/// OHLCV candle from an exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub open_time: i64,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
    pub close_time: i64,
    pub quote_volume: Decimal,
    pub trades: u64,
    pub taker_buy_volume: Decimal,
    pub taker_sell_volume: Decimal,
}

/// Single trade tick (aggr.trade-inspired in-candle resolution).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub symbol: String,
    pub price: Decimal,
    pub quantity: Decimal,
    pub side: Side,
    pub timestamp: i64,
    pub is_liquidation: bool,
}

/// Liquidation event from futures exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Liquidation {
    pub symbol: String,
    pub side: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str", rename = "origQty")]
    pub quantity: Decimal,
    pub time: i64,
}

/// Binance force order API response shape.
#[derive(Debug, Deserialize)]
pub struct ForceOrderResponse {
    pub symbol: String,
    pub side: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str", rename = "origQty")]
    pub orig_qty: Decimal,
    pub time: i64,
}

// ---------------------------------------------------------------------------
// Trading primitives
// ---------------------------------------------------------------------------

/// Trade / position direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Buy => "BUY",
            Self::Sell => "SELL",
        })
    }
}

/// Every order type a professional trading suite must support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderType {
    Market,
    Limit,
    StopMarket,
    StopLimit,
    TakeProfitMarket,
    TakeProfitLimit,
    TrailingStop,
}

impl std::fmt::Display for OrderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Market => "MARKET",
            Self::Limit => "LIMIT",
            Self::StopMarket => "STOP_MARKET",
            Self::StopLimit => "STOP_LIMIT",
            Self::TakeProfitMarket => "TAKE_PROFIT_MARKET",
            Self::TakeProfitLimit => "TAKE_PROFIT_LIMIT",
            Self::TrailingStop => "TRAILING_STOP",
        })
    }
}

/// Time-in-force for limit orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeInForce {
    Gtc, // Good Till Cancel
    Ioc, // Immediate Or Cancel
    Fok, // Fill Or Kill
    Gtd, // Good Till Date
}

/// Full order specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: String,
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    pub quantity: Decimal,
    pub price: Option<Decimal>,
    pub stop_price: Option<Decimal>,
    pub trailing_delta: Option<Decimal>,
    pub time_in_force: Option<TimeInForce>,
    pub reduce_only: bool,
    pub status: OrderStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    Pending,
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
    Expired,
}

/// Open position state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub side: Side,
    pub entry_price: Decimal,
    pub quantity: Decimal,
    pub unrealized_pnl: Decimal,
    pub realized_pnl: Decimal,
    pub leverage: u32,
    pub opened_at: i64,
}

// ---------------------------------------------------------------------------
// Strategy + AI action space (FreqAI-inspired)
// ---------------------------------------------------------------------------

/// Discrete action space for RL agents (FreqAI Base5Action equivalent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AIAction {
    Neutral,    // 0 — hold / do nothing
    EnterLong,  // 1 — open long position
    ExitLong,   // 2 — close long position
    EnterShort, // 3 — open short position
    ExitShort,  // 4 — close short position
}

impl AIAction {
    pub fn from_index(i: u8) -> Self {
        match i {
            1 => Self::EnterLong,
            2 => Self::ExitLong,
            3 => Self::EnterShort,
            4 => Self::ExitShort,
            _ => Self::Neutral,
        }
    }

    pub fn to_index(self) -> u8 {
        match self {
            Self::Neutral => 0,
            Self::EnterLong => 1,
            Self::ExitLong => 2,
            Self::EnterShort => 3,
            Self::ExitShort => 4,
        }
    }
}

/// Feature row fed to ML/RL models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureRow {
    pub timestamp: i64,
    pub features: Vec<f64>,
    pub label: Option<f64>,
}

/// Signal produced by a strategy (bot or AI).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub timestamp: i64,
    pub symbol: String,
    pub action: AIAction,
    pub confidence: f64,
    pub source: String,
    pub metadata: std::collections::HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Liquidation tiers
// ---------------------------------------------------------------------------

/// Liquidation tier classification (aggr.trade-inspired thresholds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiquidationTier {
    Small,   // >= $1K
    Medium,  // >= $10K
    Large,   // >= $30K
    Massive, // >= $100K
}

impl LiquidationTier {
    pub fn classify(usd_value: Decimal) -> Option<Self> {
        if usd_value >= Decimal::from(100_000) {
            Some(Self::Massive)
        } else if usd_value >= Decimal::from(30_000) {
            Some(Self::Large)
        } else if usd_value >= Decimal::from(10_000) {
            Some(Self::Medium)
        } else if usd_value >= Decimal::from(1_000) {
            Some(Self::Small)
        } else {
            None
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Small => "$1K+",
            Self::Medium => "$10K+",
            Self::Large => "$30K+",
            Self::Massive => "$100K+",
        }
    }
}

// ---------------------------------------------------------------------------
// Execution mode
// ---------------------------------------------------------------------------

/// Controls whether orders hit a real exchange or stay local.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionMode {
    Paper, // simulated fills
    Live,  // real exchange API
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier_classify() {
        assert_eq!(LiquidationTier::classify(Decimal::from(500)), None);
        assert_eq!(
            LiquidationTier::classify(Decimal::from(1_000)),
            Some(LiquidationTier::Small)
        );
        assert_eq!(
            LiquidationTier::classify(Decimal::from(10_000)),
            Some(LiquidationTier::Medium)
        );
        assert_eq!(
            LiquidationTier::classify(Decimal::from(30_000)),
            Some(LiquidationTier::Large)
        );
        assert_eq!(
            LiquidationTier::classify(Decimal::from(100_000)),
            Some(LiquidationTier::Massive)
        );
    }

    #[test]
    fn test_tier_labels() {
        assert_eq!(LiquidationTier::Small.label(), "$1K+");
        assert_eq!(LiquidationTier::Massive.label(), "$100K+");
    }

    #[test]
    fn test_side_display() {
        assert_eq!(Side::Buy.to_string(), "BUY");
        assert_eq!(Side::Sell.to_string(), "SELL");
    }

    #[test]
    fn test_order_type_display() {
        assert_eq!(OrderType::Market.to_string(), "MARKET");
        assert_eq!(OrderType::StopLimit.to_string(), "STOP_LIMIT");
        assert_eq!(OrderType::TrailingStop.to_string(), "TRAILING_STOP");
    }

    #[test]
    fn test_ai_action_roundtrip() {
        for i in 0..=4 {
            let action = AIAction::from_index(i);
            assert_eq!(action.to_index(), i);
        }
        // Out-of-range defaults to Neutral
        assert_eq!(AIAction::from_index(99), AIAction::Neutral);
    }

    #[test]
    fn test_order_status_variants() {
        let statuses = [
            OrderStatus::Pending,
            OrderStatus::Open,
            OrderStatus::PartiallyFilled,
            OrderStatus::Filled,
            OrderStatus::Cancelled,
            OrderStatus::Rejected,
            OrderStatus::Expired,
        ];
        assert_eq!(statuses.len(), 7);
    }

    // ------------------------------------------------------------------
    // New comprehensive tests
    // ------------------------------------------------------------------

    use std::collections::HashMap;
    use std::str::FromStr;

    #[test]
    fn test_tier_boundary_values() {
        assert_eq!(LiquidationTier::classify(Decimal::from(999)), None);
        assert_eq!(
            LiquidationTier::classify(Decimal::from(1_000)),
            Some(LiquidationTier::Small)
        );
        assert_eq!(
            LiquidationTier::classify(Decimal::from(9_999)),
            Some(LiquidationTier::Small)
        );
        assert_eq!(
            LiquidationTier::classify(Decimal::from(10_000)),
            Some(LiquidationTier::Medium)
        );
        assert_eq!(
            LiquidationTier::classify(Decimal::from(29_999)),
            Some(LiquidationTier::Medium)
        );
        assert_eq!(
            LiquidationTier::classify(Decimal::from(30_000)),
            Some(LiquidationTier::Large)
        );
        assert_eq!(
            LiquidationTier::classify(Decimal::from(99_999)),
            Some(LiquidationTier::Large)
        );
        assert_eq!(
            LiquidationTier::classify(Decimal::from(100_000)),
            Some(LiquidationTier::Massive)
        );
    }

    #[test]
    fn test_tier_large_values() {
        assert_eq!(
            LiquidationTier::classify(Decimal::from(1_000_000)),
            Some(LiquidationTier::Massive)
        );
        assert_eq!(
            LiquidationTier::classify(Decimal::from(10_000_000)),
            Some(LiquidationTier::Massive)
        );
    }

    #[test]
    fn test_side_equality() {
        assert_eq!(Side::Buy, Side::Buy);
        assert_eq!(Side::Sell, Side::Sell);
        assert_ne!(Side::Buy, Side::Sell);
    }

    #[test]
    fn test_order_type_all_display() {
        assert_eq!(OrderType::Market.to_string(), "MARKET");
        assert_eq!(OrderType::Limit.to_string(), "LIMIT");
        assert_eq!(OrderType::StopMarket.to_string(), "STOP_MARKET");
        assert_eq!(OrderType::StopLimit.to_string(), "STOP_LIMIT");
        assert_eq!(
            OrderType::TakeProfitMarket.to_string(),
            "TAKE_PROFIT_MARKET"
        );
        assert_eq!(
            OrderType::TakeProfitLimit.to_string(),
            "TAKE_PROFIT_LIMIT"
        );
        assert_eq!(OrderType::TrailingStop.to_string(), "TRAILING_STOP");
    }

    #[test]
    fn test_ai_action_identity() {
        assert_eq!(AIAction::from_index(0), AIAction::Neutral);
        assert_eq!(AIAction::Neutral.to_index(), 0);
        assert_eq!(AIAction::from_index(1), AIAction::EnterLong);
        assert_eq!(AIAction::EnterLong.to_index(), 1);
        assert_eq!(AIAction::from_index(2), AIAction::ExitLong);
        assert_eq!(AIAction::ExitLong.to_index(), 2);
        assert_eq!(AIAction::from_index(3), AIAction::EnterShort);
        assert_eq!(AIAction::EnterShort.to_index(), 3);
        assert_eq!(AIAction::from_index(4), AIAction::ExitShort);
        assert_eq!(AIAction::ExitShort.to_index(), 4);
    }

    #[test]
    fn test_ai_action_out_of_range_values() {
        assert_eq!(AIAction::from_index(5), AIAction::Neutral);
        assert_eq!(AIAction::from_index(10), AIAction::Neutral);
        assert_eq!(AIAction::from_index(255), AIAction::Neutral);
    }

    #[test]
    fn test_execution_mode_equality() {
        assert_eq!(ExecutionMode::Paper, ExecutionMode::Paper);
        assert_eq!(ExecutionMode::Live, ExecutionMode::Live);
        assert_ne!(ExecutionMode::Paper, ExecutionMode::Live);
    }

    #[test]
    fn test_candle_serde_roundtrip() {
        let candle = Candle {
            open_time: 1_700_000_000_000,
            open: Decimal::from_str("42000.50").unwrap(),
            high: Decimal::from_str("42500.00").unwrap(),
            low: Decimal::from_str("41800.25").unwrap(),
            close: Decimal::from_str("42200.75").unwrap(),
            volume: Decimal::from_str("1234.567").unwrap(),
            close_time: 1_700_000_060_000,
            quote_volume: Decimal::from_str("51000000.00").unwrap(),
            trades: 9876,
            taker_buy_volume: Decimal::from_str("600.123").unwrap(),
            taker_sell_volume: Decimal::from_str("634.444").unwrap(),
        };

        let json = serde_json::to_string(&candle).unwrap();
        let restored: Candle = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.open_time, candle.open_time);
        assert_eq!(restored.open, candle.open);
        assert_eq!(restored.high, candle.high);
        assert_eq!(restored.low, candle.low);
        assert_eq!(restored.close, candle.close);
        assert_eq!(restored.volume, candle.volume);
        assert_eq!(restored.close_time, candle.close_time);
        assert_eq!(restored.quote_volume, candle.quote_volume);
        assert_eq!(restored.trades, candle.trades);
        assert_eq!(restored.taker_buy_volume, candle.taker_buy_volume);
        assert_eq!(restored.taker_sell_volume, candle.taker_sell_volume);
    }

    #[test]
    fn test_trade_serde_roundtrip() {
        let trade = Trade {
            symbol: "BTCUSDT".to_string(),
            price: Decimal::from_str("42000.50").unwrap(),
            quantity: Decimal::from_str("0.123").unwrap(),
            side: Side::Buy,
            timestamp: 1_700_000_000_000,
            is_liquidation: true,
        };

        let json = serde_json::to_string(&trade).unwrap();
        let restored: Trade = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.symbol, "BTCUSDT");
        assert_eq!(restored.price, trade.price);
        assert_eq!(restored.quantity, trade.quantity);
        assert_eq!(restored.side, Side::Buy);
        assert_eq!(restored.timestamp, trade.timestamp);
        assert!(restored.is_liquidation);
    }

    #[test]
    fn test_signal_serde_roundtrip() {
        let mut metadata = HashMap::new();
        metadata.insert("cvd_delta".to_string(), "150.5".to_string());
        metadata.insert("trend".to_string(), "bullish".to_string());

        let signal = Signal {
            timestamp: 1_700_000_000_000,
            symbol: "ETHUSDT".to_string(),
            action: AIAction::EnterLong,
            confidence: 0.85,
            source: "cvd_momentum".to_string(),
            metadata,
        };

        let json = serde_json::to_string(&signal).unwrap();
        let restored: Signal = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.timestamp, signal.timestamp);
        assert_eq!(restored.symbol, "ETHUSDT");
        assert_eq!(restored.action, AIAction::EnterLong);
        assert!((restored.confidence - 0.85).abs() < f64::EPSILON);
        assert_eq!(restored.source, "cvd_momentum");
        assert_eq!(restored.metadata.get("cvd_delta").unwrap(), "150.5");
        assert_eq!(restored.metadata.get("trend").unwrap(), "bullish");
    }

    #[test]
    fn test_order_serde_roundtrip() {
        let order = Order {
            id: "ord-001".to_string(),
            symbol: "BTCUSDT".to_string(),
            side: Side::Sell,
            order_type: OrderType::StopLimit,
            quantity: Decimal::from_str("0.5").unwrap(),
            price: Some(Decimal::from_str("41000.00").unwrap()),
            stop_price: Some(Decimal::from_str("41500.00").unwrap()),
            trailing_delta: Some(Decimal::from_str("100.0").unwrap()),
            time_in_force: Some(TimeInForce::Gtc),
            reduce_only: true,
            status: OrderStatus::Open,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_060_000,
        };

        let json = serde_json::to_string(&order).unwrap();
        let restored: Order = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.id, "ord-001");
        assert_eq!(restored.symbol, "BTCUSDT");
        assert_eq!(restored.side, Side::Sell);
        assert_eq!(restored.order_type, OrderType::StopLimit);
        assert_eq!(restored.quantity, Decimal::from_str("0.5").unwrap());
        assert_eq!(
            restored.price,
            Some(Decimal::from_str("41000.00").unwrap())
        );
        assert_eq!(
            restored.stop_price,
            Some(Decimal::from_str("41500.00").unwrap())
        );
        assert_eq!(
            restored.trailing_delta,
            Some(Decimal::from_str("100.0").unwrap())
        );
        assert_eq!(restored.time_in_force, Some(TimeInForce::Gtc));
        assert!(restored.reduce_only);
        assert_eq!(restored.status, OrderStatus::Open);
        assert_eq!(restored.created_at, 1_700_000_000_000);
        assert_eq!(restored.updated_at, 1_700_000_060_000);
    }

    #[test]
    fn test_position_serde_roundtrip() {
        let position = Position {
            symbol: "ETHUSDT".to_string(),
            side: Side::Buy,
            entry_price: Decimal::from_str("3200.00").unwrap(),
            quantity: Decimal::from_str("2.5").unwrap(),
            unrealized_pnl: Decimal::from_str("150.00").unwrap(),
            realized_pnl: Decimal::from_str("0.00").unwrap(),
            leverage: 10,
            opened_at: 1_700_000_000_000,
        };

        let json = serde_json::to_string(&position).unwrap();
        let restored: Position = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.symbol, "ETHUSDT");
        assert_eq!(restored.side, Side::Buy);
        assert_eq!(restored.entry_price, Decimal::from_str("3200.00").unwrap());
        assert_eq!(restored.quantity, Decimal::from_str("2.5").unwrap());
        assert_eq!(
            restored.unrealized_pnl,
            Decimal::from_str("150.00").unwrap()
        );
        assert_eq!(
            restored.realized_pnl,
            Decimal::from_str("0.00").unwrap()
        );
        assert_eq!(restored.leverage, 10);
        assert_eq!(restored.opened_at, 1_700_000_000_000);
    }

    #[test]
    fn test_feature_row_serde() {
        let row = FeatureRow {
            timestamp: 1_700_000_000_000,
            features: vec![0.5, -1.2, 3.14, 0.0, 99.9],
            label: Some(1.0),
        };

        let json = serde_json::to_string(&row).unwrap();
        let restored: FeatureRow = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.timestamp, row.timestamp);
        assert_eq!(restored.features, vec![0.5, -1.2, 3.14, 0.0, 99.9]);
        assert_eq!(restored.label, Some(1.0));
    }

    #[test]
    fn test_liquidation_serde() {
        let liq = Liquidation {
            symbol: "BTCUSDT".to_string(),
            side: "SELL".to_string(),
            price: Decimal::from_str("42000.00").unwrap(),
            quantity: Decimal::from_str("1.5").unwrap(),
            time: 1_700_000_000_000,
        };

        let json = serde_json::to_string(&liq).unwrap();
        let restored: Liquidation = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.symbol, "BTCUSDT");
        assert_eq!(restored.side, "SELL");
        assert_eq!(restored.price, Decimal::from_str("42000.00").unwrap());
        assert_eq!(restored.quantity, Decimal::from_str("1.5").unwrap());
        assert_eq!(restored.time, 1_700_000_000_000);
    }

    #[test]
    fn test_order_status_equality() {
        assert_eq!(OrderStatus::Pending, OrderStatus::Pending);
        assert_eq!(OrderStatus::Open, OrderStatus::Open);
        assert_eq!(OrderStatus::PartiallyFilled, OrderStatus::PartiallyFilled);
        assert_eq!(OrderStatus::Filled, OrderStatus::Filled);
        assert_eq!(OrderStatus::Cancelled, OrderStatus::Cancelled);
        assert_eq!(OrderStatus::Rejected, OrderStatus::Rejected);
        assert_eq!(OrderStatus::Expired, OrderStatus::Expired);
        assert_ne!(OrderStatus::Pending, OrderStatus::Filled);
    }

    #[test]
    fn test_time_in_force_variants() {
        let variants = [
            TimeInForce::Gtc,
            TimeInForce::Ioc,
            TimeInForce::Fok,
            TimeInForce::Gtd,
        ];
        assert_eq!(variants.len(), 4);
        // All variants are distinct
        for i in 0..variants.len() {
            for j in (i + 1)..variants.len() {
                assert_ne!(variants[i], variants[j]);
            }
        }
    }

    #[test]
    fn test_cvd_trend_display() {
        // Verify Side Display covers both variants
        assert_eq!(format!("{}", Side::Buy), "BUY");
        assert_eq!(format!("{}", Side::Sell), "SELL");

        // Verify OrderType Display covers all variants
        let all_types = [
            (OrderType::Market, "MARKET"),
            (OrderType::Limit, "LIMIT"),
            (OrderType::StopMarket, "STOP_MARKET"),
            (OrderType::StopLimit, "STOP_LIMIT"),
            (OrderType::TakeProfitMarket, "TAKE_PROFIT_MARKET"),
            (OrderType::TakeProfitLimit, "TAKE_PROFIT_LIMIT"),
            (OrderType::TrailingStop, "TRAILING_STOP"),
        ];
        for (ot, expected) in &all_types {
            assert_eq!(format!("{}", ot), *expected);
        }
    }

    #[test]
    fn test_side_clone_copy() {
        let a = Side::Buy;
        let b = a; // Copy
        let c = a.clone(); // Clone
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(b, c);

        let x = Side::Sell;
        let y = x;
        assert_eq!(x, y);
    }

    // ------------------------------------------------------------------
    // Additional coverage tests
    // ------------------------------------------------------------------

    #[test]
    fn test_candle_debug() {
        let candle = Candle {
            open_time: 1_000,
            open: Decimal::from(100),
            high: Decimal::from(110),
            low: Decimal::from(90),
            close: Decimal::from(105),
            volume: Decimal::from(50),
            close_time: 2_000,
            quote_volume: Decimal::from(5000),
            trades: 42,
            taker_buy_volume: Decimal::from(30),
            taker_sell_volume: Decimal::from(20),
        };
        let debug = format!("{:?}", candle);
        assert!(debug.contains("Candle"));
        assert!(debug.contains("open_time: 1000"));
        assert!(debug.contains("trades: 42"));
    }

    #[test]
    fn test_candle_clone() {
        let candle = Candle {
            open_time: 1_000,
            open: Decimal::from(100),
            high: Decimal::from(110),
            low: Decimal::from(90),
            close: Decimal::from(105),
            volume: Decimal::from(50),
            close_time: 2_000,
            quote_volume: Decimal::from(5000),
            trades: 42,
            taker_buy_volume: Decimal::from(30),
            taker_sell_volume: Decimal::from(20),
        };
        let cloned = candle.clone();
        assert_eq!(cloned.open_time, candle.open_time);
        assert_eq!(cloned.open, candle.open);
        assert_eq!(cloned.high, candle.high);
        assert_eq!(cloned.low, candle.low);
        assert_eq!(cloned.close, candle.close);
        assert_eq!(cloned.volume, candle.volume);
        assert_eq!(cloned.close_time, candle.close_time);
        assert_eq!(cloned.quote_volume, candle.quote_volume);
        assert_eq!(cloned.trades, candle.trades);
        assert_eq!(cloned.taker_buy_volume, candle.taker_buy_volume);
        assert_eq!(cloned.taker_sell_volume, candle.taker_sell_volume);
    }

    #[test]
    fn test_candle_serde_from_json_object() {
        let json = r#"{
            "open_time": 1700000000000,
            "open": "50000.00",
            "high": "51000.00",
            "low": "49000.00",
            "close": "50500.00",
            "volume": "100.0",
            "close_time": 1700000060000,
            "quote_volume": "5050000.00",
            "trades": 500,
            "taker_buy_volume": "60.0",
            "taker_sell_volume": "40.0"
        }"#;
        let candle: Candle = serde_json::from_str(json).unwrap();
        assert_eq!(candle.open, Decimal::from_str("50000.00").unwrap());
        assert_eq!(candle.trades, 500);
    }

    #[test]
    fn test_trade_debug_and_clone() {
        let trade = Trade {
            symbol: "ETHUSDT".to_string(),
            price: Decimal::from(3000),
            quantity: Decimal::from(1),
            side: Side::Sell,
            timestamp: 1_700_000_000_000,
            is_liquidation: false,
        };
        let debug = format!("{:?}", trade);
        assert!(debug.contains("Trade"));
        assert!(debug.contains("ETHUSDT"));

        let cloned = trade.clone();
        assert_eq!(cloned.symbol, trade.symbol);
        assert_eq!(cloned.price, trade.price);
        assert_eq!(cloned.side, trade.side);
        assert!(!cloned.is_liquidation);
    }

    #[test]
    fn test_trade_serde_sell_side() {
        let trade = Trade {
            symbol: "SOLUSDT".to_string(),
            price: Decimal::from_str("150.25").unwrap(),
            quantity: Decimal::from_str("10.0").unwrap(),
            side: Side::Sell,
            timestamp: 1_700_000_000_000,
            is_liquidation: false,
        };
        let json = serde_json::to_string(&trade).unwrap();
        let restored: Trade = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.side, Side::Sell);
        assert!(!restored.is_liquidation);
    }

    #[test]
    fn test_side_serde_roundtrip() {
        let buy_json = serde_json::to_string(&Side::Buy).unwrap();
        let sell_json = serde_json::to_string(&Side::Sell).unwrap();
        assert_eq!(serde_json::from_str::<Side>(&buy_json).unwrap(), Side::Buy);
        assert_eq!(
            serde_json::from_str::<Side>(&sell_json).unwrap(),
            Side::Sell
        );
    }

    #[test]
    fn test_side_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Side::Buy);
        set.insert(Side::Sell);
        set.insert(Side::Buy); // duplicate
        assert_eq!(set.len(), 2);
        assert!(set.contains(&Side::Buy));
        assert!(set.contains(&Side::Sell));
    }

    #[test]
    fn test_side_debug() {
        assert_eq!(format!("{:?}", Side::Buy), "Buy");
        assert_eq!(format!("{:?}", Side::Sell), "Sell");
    }

    #[test]
    fn test_order_type_equality_and_hash() {
        use std::collections::HashSet;
        let all = [
            OrderType::Market,
            OrderType::Limit,
            OrderType::StopMarket,
            OrderType::StopLimit,
            OrderType::TakeProfitMarket,
            OrderType::TakeProfitLimit,
            OrderType::TrailingStop,
        ];
        let set: HashSet<OrderType> = all.iter().copied().collect();
        assert_eq!(set.len(), 7);
    }

    #[test]
    fn test_order_type_serde_roundtrip() {
        let variants = [
            OrderType::Market,
            OrderType::Limit,
            OrderType::StopMarket,
            OrderType::StopLimit,
            OrderType::TakeProfitMarket,
            OrderType::TakeProfitLimit,
            OrderType::TrailingStop,
        ];
        for variant in &variants {
            let json = serde_json::to_string(variant).unwrap();
            let restored: OrderType = serde_json::from_str(&json).unwrap();
            assert_eq!(*variant, restored);
        }
    }

    #[test]
    fn test_order_type_debug() {
        assert_eq!(format!("{:?}", OrderType::Market), "Market");
        assert_eq!(format!("{:?}", OrderType::TrailingStop), "TrailingStop");
    }

    #[test]
    fn test_order_with_none_optional_fields() {
        let order = Order {
            id: "ord-minimal".to_string(),
            symbol: "BTCUSDT".to_string(),
            side: Side::Buy,
            order_type: OrderType::Market,
            quantity: Decimal::from(1),
            price: None,
            stop_price: None,
            trailing_delta: None,
            time_in_force: None,
            reduce_only: false,
            status: OrderStatus::Pending,
            created_at: 0,
            updated_at: 0,
        };
        let json = serde_json::to_string(&order).unwrap();
        let restored: Order = serde_json::from_str(&json).unwrap();
        assert!(restored.price.is_none());
        assert!(restored.stop_price.is_none());
        assert!(restored.trailing_delta.is_none());
        assert!(restored.time_in_force.is_none());
        assert!(!restored.reduce_only);
    }

    #[test]
    fn test_order_clone_and_debug() {
        let order = Order {
            id: "ord-debug".to_string(),
            symbol: "BTCUSDT".to_string(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            quantity: Decimal::from(2),
            price: Some(Decimal::from(40000)),
            stop_price: None,
            trailing_delta: None,
            time_in_force: Some(TimeInForce::Gtc),
            reduce_only: false,
            status: OrderStatus::Open,
            created_at: 100,
            updated_at: 200,
        };
        let debug = format!("{:?}", order);
        assert!(debug.contains("Order"));
        assert!(debug.contains("ord-debug"));

        let cloned = order.clone();
        assert_eq!(cloned.id, "ord-debug");
        assert_eq!(cloned.order_type, OrderType::Limit);
    }

    #[test]
    fn test_order_status_serde_roundtrip() {
        let statuses = [
            OrderStatus::Pending,
            OrderStatus::Open,
            OrderStatus::PartiallyFilled,
            OrderStatus::Filled,
            OrderStatus::Cancelled,
            OrderStatus::Rejected,
            OrderStatus::Expired,
        ];
        for status in &statuses {
            let json = serde_json::to_string(status).unwrap();
            let restored: OrderStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(*status, restored);
        }
    }

    #[test]
    fn test_order_status_debug() {
        assert_eq!(format!("{:?}", OrderStatus::Pending), "Pending");
        assert_eq!(format!("{:?}", OrderStatus::PartiallyFilled), "PartiallyFilled");
        assert_eq!(format!("{:?}", OrderStatus::Expired), "Expired");
    }

    #[test]
    fn test_order_status_clone_copy() {
        let s = OrderStatus::Filled;
        let s2 = s; // Copy
        let s3 = s.clone(); // Clone
        assert_eq!(s, s2);
        assert_eq!(s, s3);
    }

    #[test]
    fn test_position_clone_and_debug() {
        let pos = Position {
            symbol: "BTCUSDT".to_string(),
            side: Side::Sell,
            entry_price: Decimal::from(45000),
            quantity: Decimal::from_str("0.01").unwrap(),
            unrealized_pnl: Decimal::from_str("-50.00").unwrap(),
            realized_pnl: Decimal::from(0),
            leverage: 20,
            opened_at: 1_700_000_000_000,
        };
        let debug = format!("{:?}", pos);
        assert!(debug.contains("Position"));
        assert!(debug.contains("BTCUSDT"));

        let cloned = pos.clone();
        assert_eq!(cloned.side, Side::Sell);
        assert_eq!(cloned.leverage, 20);
        assert_eq!(
            cloned.unrealized_pnl,
            Decimal::from_str("-50.00").unwrap()
        );
    }

    #[test]
    fn test_position_with_negative_pnl() {
        let pos = Position {
            symbol: "ETHUSDT".to_string(),
            side: Side::Buy,
            entry_price: Decimal::from(3500),
            quantity: Decimal::from(5),
            unrealized_pnl: Decimal::from_str("-200.50").unwrap(),
            realized_pnl: Decimal::from_str("100.25").unwrap(),
            leverage: 5,
            opened_at: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&pos).unwrap();
        let restored: Position = serde_json::from_str(&json).unwrap();
        assert_eq!(
            restored.unrealized_pnl,
            Decimal::from_str("-200.50").unwrap()
        );
        assert_eq!(
            restored.realized_pnl,
            Decimal::from_str("100.25").unwrap()
        );
    }

    #[test]
    fn test_ai_action_serde_roundtrip() {
        let actions = [
            AIAction::Neutral,
            AIAction::EnterLong,
            AIAction::ExitLong,
            AIAction::EnterShort,
            AIAction::ExitShort,
        ];
        for action in &actions {
            let json = serde_json::to_string(action).unwrap();
            let restored: AIAction = serde_json::from_str(&json).unwrap();
            assert_eq!(*action, restored);
        }
    }

    #[test]
    fn test_ai_action_debug() {
        assert_eq!(format!("{:?}", AIAction::Neutral), "Neutral");
        assert_eq!(format!("{:?}", AIAction::EnterLong), "EnterLong");
        assert_eq!(format!("{:?}", AIAction::ExitLong), "ExitLong");
        assert_eq!(format!("{:?}", AIAction::EnterShort), "EnterShort");
        assert_eq!(format!("{:?}", AIAction::ExitShort), "ExitShort");
    }

    #[test]
    fn test_ai_action_clone_copy_hash() {
        use std::collections::HashSet;
        let a = AIAction::EnterLong;
        let b = a; // Copy
        let c = a.clone(); // Clone
        assert_eq!(a, b);
        assert_eq!(a, c);

        let mut set = HashSet::new();
        set.insert(AIAction::Neutral);
        set.insert(AIAction::EnterLong);
        set.insert(AIAction::ExitLong);
        set.insert(AIAction::EnterShort);
        set.insert(AIAction::ExitShort);
        set.insert(AIAction::Neutral); // duplicate
        assert_eq!(set.len(), 5);
    }

    #[test]
    fn test_ai_action_from_index_zero() {
        // 0 explicitly maps to Neutral (not just the default fallback)
        let action = AIAction::from_index(0);
        assert_eq!(action, AIAction::Neutral);
        assert_eq!(action.to_index(), 0);
    }

    #[test]
    fn test_signal_with_empty_metadata() {
        let signal = Signal {
            timestamp: 0,
            symbol: "BTCUSDT".to_string(),
            action: AIAction::Neutral,
            confidence: 0.0,
            source: "test".to_string(),
            metadata: HashMap::new(),
        };
        let json = serde_json::to_string(&signal).unwrap();
        let restored: Signal = serde_json::from_str(&json).unwrap();
        assert!(restored.metadata.is_empty());
        assert_eq!(restored.confidence, 0.0);
    }

    #[test]
    fn test_signal_all_action_variants() {
        let actions = [
            AIAction::Neutral,
            AIAction::EnterLong,
            AIAction::ExitLong,
            AIAction::EnterShort,
            AIAction::ExitShort,
        ];
        for action in &actions {
            let signal = Signal {
                timestamp: 1_000,
                symbol: "BTCUSDT".to_string(),
                action: *action,
                confidence: 0.5,
                source: "test".to_string(),
                metadata: HashMap::new(),
            };
            let json = serde_json::to_string(&signal).unwrap();
            let restored: Signal = serde_json::from_str(&json).unwrap();
            assert_eq!(restored.action, *action);
        }
    }

    #[test]
    fn test_signal_debug_and_clone() {
        let signal = Signal {
            timestamp: 1_000,
            symbol: "BTCUSDT".to_string(),
            action: AIAction::ExitShort,
            confidence: 0.99,
            source: "rl_agent".to_string(),
            metadata: HashMap::new(),
        };
        let debug = format!("{:?}", signal);
        assert!(debug.contains("Signal"));
        assert!(debug.contains("ExitShort"));

        let cloned = signal.clone();
        assert_eq!(cloned.action, AIAction::ExitShort);
        assert_eq!(cloned.source, "rl_agent");
    }

    #[test]
    fn test_liquidation_tier_debug_and_clone() {
        let tier = LiquidationTier::Large;
        let debug = format!("{:?}", tier);
        assert_eq!(debug, "Large");
        let cloned = tier.clone();
        assert_eq!(tier, cloned);
    }

    #[test]
    fn test_liquidation_tier_all_labels() {
        assert_eq!(LiquidationTier::Small.label(), "$1K+");
        assert_eq!(LiquidationTier::Medium.label(), "$10K+");
        assert_eq!(LiquidationTier::Large.label(), "$30K+");
        assert_eq!(LiquidationTier::Massive.label(), "$100K+");
    }

    #[test]
    fn test_liquidation_tier_classify_zero_and_negative() {
        assert_eq!(LiquidationTier::classify(Decimal::from(0)), None);
        assert_eq!(LiquidationTier::classify(Decimal::from(-1000)), None);
    }

    #[test]
    fn test_liquidation_tier_classify_fractional_boundaries() {
        // Just below 1000
        assert_eq!(
            LiquidationTier::classify(Decimal::from_str("999.99").unwrap()),
            None
        );
        // Just at 1000
        assert_eq!(
            LiquidationTier::classify(Decimal::from_str("1000.00").unwrap()),
            Some(LiquidationTier::Small)
        );
        // Just above 1000
        assert_eq!(
            LiquidationTier::classify(Decimal::from_str("1000.01").unwrap()),
            Some(LiquidationTier::Small)
        );
        // Just below 10000
        assert_eq!(
            LiquidationTier::classify(Decimal::from_str("9999.99").unwrap()),
            Some(LiquidationTier::Small)
        );
        // Just below 30000
        assert_eq!(
            LiquidationTier::classify(Decimal::from_str("29999.99").unwrap()),
            Some(LiquidationTier::Medium)
        );
        // Just below 100000
        assert_eq!(
            LiquidationTier::classify(Decimal::from_str("99999.99").unwrap()),
            Some(LiquidationTier::Large)
        );
    }

    #[test]
    fn test_liquidation_tier_copy() {
        let tier = LiquidationTier::Massive;
        let tier2 = tier; // Copy
        assert_eq!(tier, tier2);
    }

    #[test]
    fn test_liquidation_clone_and_debug() {
        let liq = Liquidation {
            symbol: "ETHUSDT".to_string(),
            side: "BUY".to_string(),
            price: Decimal::from(3000),
            quantity: Decimal::from_str("0.5").unwrap(),
            time: 1_700_000_000_000,
        };
        let debug = format!("{:?}", liq);
        assert!(debug.contains("Liquidation"));
        assert!(debug.contains("ETHUSDT"));

        let cloned = liq.clone();
        assert_eq!(cloned.symbol, "ETHUSDT");
        assert_eq!(cloned.side, "BUY");
    }

    #[test]
    fn test_force_order_response_debug() {
        let json = r#"{
            "symbol": "BTCUSDT",
            "side": "SELL",
            "price": "42000.00",
            "origQty": "1.5",
            "time": 1700000000000
        }"#;
        let resp: ForceOrderResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.symbol, "BTCUSDT");
        assert_eq!(resp.side, "SELL");
        assert_eq!(resp.price, Decimal::from_str("42000.00").unwrap());
        assert_eq!(resp.orig_qty, Decimal::from_str("1.5").unwrap());
        assert_eq!(resp.time, 1_700_000_000_000);

        let debug = format!("{:?}", resp);
        assert!(debug.contains("ForceOrderResponse"));
    }

    #[test]
    fn test_execution_mode_serde_roundtrip() {
        let paper_json = serde_json::to_string(&ExecutionMode::Paper).unwrap();
        let live_json = serde_json::to_string(&ExecutionMode::Live).unwrap();
        assert_eq!(
            serde_json::from_str::<ExecutionMode>(&paper_json).unwrap(),
            ExecutionMode::Paper
        );
        assert_eq!(
            serde_json::from_str::<ExecutionMode>(&live_json).unwrap(),
            ExecutionMode::Live
        );
    }

    #[test]
    fn test_execution_mode_debug_clone_copy() {
        let mode = ExecutionMode::Paper;
        assert_eq!(format!("{:?}", mode), "Paper");
        assert_eq!(format!("{:?}", ExecutionMode::Live), "Live");

        let mode2 = mode; // Copy
        let mode3 = mode.clone(); // Clone
        assert_eq!(mode, mode2);
        assert_eq!(mode, mode3);
    }

    #[test]
    fn test_time_in_force_serde_roundtrip() {
        let variants = [
            TimeInForce::Gtc,
            TimeInForce::Ioc,
            TimeInForce::Fok,
            TimeInForce::Gtd,
        ];
        for v in &variants {
            let json = serde_json::to_string(v).unwrap();
            let restored: TimeInForce = serde_json::from_str(&json).unwrap();
            assert_eq!(*v, restored);
        }
    }

    #[test]
    fn test_time_in_force_debug_clone_copy() {
        let tif = TimeInForce::Fok;
        assert_eq!(format!("{:?}", tif), "Fok");
        let tif2 = tif; // Copy
        let tif3 = tif.clone(); // Clone
        assert_eq!(tif, tif2);
        assert_eq!(tif, tif3);
    }

    #[test]
    fn test_feature_row_with_no_label() {
        let row = FeatureRow {
            timestamp: 1_000,
            features: vec![1.0, 2.0, 3.0],
            label: None,
        };
        let json = serde_json::to_string(&row).unwrap();
        let restored: FeatureRow = serde_json::from_str(&json).unwrap();
        assert!(restored.label.is_none());
        assert_eq!(restored.features.len(), 3);
    }

    #[test]
    fn test_feature_row_empty_features() {
        let row = FeatureRow {
            timestamp: 0,
            features: vec![],
            label: None,
        };
        let json = serde_json::to_string(&row).unwrap();
        let restored: FeatureRow = serde_json::from_str(&json).unwrap();
        assert!(restored.features.is_empty());
    }

    #[test]
    fn test_feature_row_debug_and_clone() {
        let row = FeatureRow {
            timestamp: 1_000,
            features: vec![0.5, -1.0],
            label: Some(0.0),
        };
        let debug = format!("{:?}", row);
        assert!(debug.contains("FeatureRow"));

        let cloned = row.clone();
        assert_eq!(cloned.timestamp, 1_000);
        assert_eq!(cloned.features, vec![0.5, -1.0]);
        assert_eq!(cloned.label, Some(0.0));
    }

    #[test]
    fn test_liquidation_serde_with_renamed_field() {
        // Test that origQty is properly deserialized via serde rename
        let json = r#"{"symbol":"BTCUSDT","side":"BUY","price":"50000","origQty":"2.0","time":100}"#;
        let liq: Liquidation = serde_json::from_str(json).unwrap();
        assert_eq!(liq.quantity, Decimal::from(2));

        // Serialization should produce origQty in the output
        let serialized = serde_json::to_string(&liq).unwrap();
        assert!(serialized.contains("origQty"));
    }

    #[test]
    fn test_order_all_order_types_serde() {
        // Create an order for each OrderType and verify serde roundtrip
        let order_types = [
            OrderType::Market,
            OrderType::Limit,
            OrderType::StopMarket,
            OrderType::StopLimit,
            OrderType::TakeProfitMarket,
            OrderType::TakeProfitLimit,
            OrderType::TrailingStop,
        ];
        for ot in &order_types {
            let order = Order {
                id: format!("ord-{}", ot),
                symbol: "BTCUSDT".to_string(),
                side: Side::Buy,
                order_type: *ot,
                quantity: Decimal::from(1),
                price: None,
                stop_price: None,
                trailing_delta: None,
                time_in_force: None,
                reduce_only: false,
                status: OrderStatus::Pending,
                created_at: 0,
                updated_at: 0,
            };
            let json = serde_json::to_string(&order).unwrap();
            let restored: Order = serde_json::from_str(&json).unwrap();
            assert_eq!(restored.order_type, *ot);
        }
    }

    #[test]
    fn test_order_all_statuses_serde_in_order() {
        // Simulate a lifecycle: Pending -> Open -> PartiallyFilled -> Filled
        let statuses = [
            OrderStatus::Pending,
            OrderStatus::Open,
            OrderStatus::PartiallyFilled,
            OrderStatus::Filled,
        ];
        for (i, status) in statuses.iter().enumerate() {
            let order = Order {
                id: "ord-lifecycle".to_string(),
                symbol: "BTCUSDT".to_string(),
                side: Side::Buy,
                order_type: OrderType::Limit,
                quantity: Decimal::from(1),
                price: Some(Decimal::from(40000)),
                stop_price: None,
                trailing_delta: None,
                time_in_force: Some(TimeInForce::Gtc),
                reduce_only: false,
                status: *status,
                created_at: 0,
                updated_at: i as i64,
            };
            let json = serde_json::to_string(&order).unwrap();
            let restored: Order = serde_json::from_str(&json).unwrap();
            assert_eq!(restored.status, *status);
            assert_eq!(restored.updated_at, i as i64);
        }
    }

    #[test]
    fn test_order_rejected_and_cancelled_statuses() {
        for status in &[OrderStatus::Cancelled, OrderStatus::Rejected, OrderStatus::Expired] {
            let order = Order {
                id: "ord-terminal".to_string(),
                symbol: "BTCUSDT".to_string(),
                side: Side::Sell,
                order_type: OrderType::Market,
                quantity: Decimal::from(1),
                price: None,
                stop_price: None,
                trailing_delta: None,
                time_in_force: None,
                reduce_only: true,
                status: *status,
                created_at: 0,
                updated_at: 0,
            };
            let json = serde_json::to_string(&order).unwrap();
            let restored: Order = serde_json::from_str(&json).unwrap();
            assert_eq!(restored.status, *status);
            assert!(restored.reduce_only);
        }
    }

    #[test]
    fn test_position_sell_side_serde() {
        let pos = Position {
            symbol: "BTCUSDT".to_string(),
            side: Side::Sell,
            entry_price: Decimal::from(45000),
            quantity: Decimal::from_str("0.5").unwrap(),
            unrealized_pnl: Decimal::from(0),
            realized_pnl: Decimal::from(0),
            leverage: 1,
            opened_at: 0,
        };
        let json = serde_json::to_string(&pos).unwrap();
        let restored: Position = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.side, Side::Sell);
        assert_eq!(restored.leverage, 1);
    }
}
