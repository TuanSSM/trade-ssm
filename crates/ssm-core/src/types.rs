use rust_decimal::Decimal;
use serde::Deserialize;

/// OHLCV candle from an exchange.
#[derive(Debug, Clone)]
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
}
