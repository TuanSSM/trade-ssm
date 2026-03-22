use rust_decimal::Decimal;
use serde::Deserialize;

/// OHLCV candle from an exchange
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

/// Liquidation event from futures exchange
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

/// Wrapper for Binance force order response
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

/// Liquidation tier classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiquidationTier {
    /// > $1,000
    Small,
    /// > $10,000
    Medium,
    /// > $30,000
    Large,
    /// > $100,000
    Massive,
}

impl LiquidationTier {
    pub fn classify(usd_value: Decimal) -> Option<Self> {
        let thousand = Decimal::from(1_000);
        let ten_k = Decimal::from(10_000);
        let thirty_k = Decimal::from(30_000);
        let hundred_k = Decimal::from(100_000);

        if usd_value >= hundred_k {
            Some(Self::Massive)
        } else if usd_value >= thirty_k {
            Some(Self::Large)
        } else if usd_value >= ten_k {
            Some(Self::Medium)
        } else if usd_value >= thousand {
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
