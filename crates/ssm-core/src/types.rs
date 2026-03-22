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
#[derive(Debug, Clone, Deserialize)]
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
}
