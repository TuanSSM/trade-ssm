use anyhow::Result;
use async_trait::async_trait;
use rust_decimal::Decimal;
use ssm_core::{Candle, Liquidation};

use crate::binance::BinanceClient;
use crate::bybit::BybitClient;

/// Abstract exchange interface.
#[async_trait]
pub trait Exchange: Send + Sync {
    /// Exchange name (e.g. "binance", "bybit").
    fn name(&self) -> &str;

    /// Fetch recent OHLCV candles.
    async fn fetch_klines(&self, symbol: &str, interval: &str, limit: u32) -> Result<Vec<Candle>>;

    /// Fetch OHLCV candles in a specific time range.
    async fn fetch_klines_range(
        &self,
        symbol: &str,
        interval: &str,
        limit: u32,
        start_time: i64,
        end_time: i64,
    ) -> Result<Vec<Candle>>;

    /// Fetch recent liquidation orders.
    async fn fetch_liquidations(&self, symbol: &str, limit: u32) -> Result<Vec<Liquidation>>;

    /// List available trading pairs.
    async fn list_pairs(&self) -> Result<Vec<PairInfo>>;

    /// Get supported timeframes for this exchange.
    fn supported_timeframes(&self) -> Vec<&str>;
}

/// Information about a trading pair.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PairInfo {
    pub symbol: String,
    pub base: String,
    pub quote: String,
    pub price: Option<Decimal>,
    pub volume_24h: Option<Decimal>,
}

/// Create an exchange client by name.
pub fn create_exchange(name: &str) -> Result<Box<dyn Exchange>> {
    match name {
        "binance" => Ok(Box::new(BinanceClient::new())),
        "bybit" => Ok(Box::new(BybitClient::new())),
        _ => anyhow::bail!("unknown exchange: {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_exchange_binance() {
        let exchange = create_exchange("binance").unwrap();
        assert_eq!(exchange.name(), "binance");
    }

    #[test]
    fn test_create_exchange_bybit() {
        let exchange = create_exchange("bybit").unwrap();
        assert_eq!(exchange.name(), "bybit");
    }

    #[test]
    fn test_create_exchange_unknown() {
        let result = create_exchange("unknown");
        assert!(result.is_err());
        let err_msg = format!("{}", result.err().unwrap());
        assert!(err_msg.contains("unknown exchange"));
    }

    #[test]
    fn test_binance_supported_timeframes() {
        let exchange = BinanceClient::new();
        let tf = exchange.supported_timeframes();
        assert!(tf.contains(&"15m"));
        assert!(tf.contains(&"1h"));
        assert!(tf.contains(&"1d"));
    }

    #[test]
    fn test_bybit_supported_timeframes() {
        let exchange = BybitClient::new();
        let tf = exchange.supported_timeframes();
        assert!(tf.contains(&"15m"));
    }

    #[test]
    fn test_pair_info_serialization_roundtrip() {
        let pair = PairInfo {
            symbol: "BTCUSDT".to_string(),
            base: "BTC".to_string(),
            quote: "USDT".to_string(),
            price: Some(Decimal::new(60000, 0)),
            volume_24h: Some(Decimal::new(1000000, 0)),
        };

        let json = serde_json::to_string(&pair).unwrap();
        let deserialized: PairInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.symbol, "BTCUSDT");
        assert_eq!(deserialized.base, "BTC");
        assert_eq!(deserialized.quote, "USDT");
        assert_eq!(deserialized.price, Some(Decimal::new(60000, 0)));
        assert_eq!(deserialized.volume_24h, Some(Decimal::new(1000000, 0)));
    }

    #[test]
    fn test_pair_info_serialization_with_none_fields() {
        let pair = PairInfo {
            symbol: "ETHUSDT".to_string(),
            base: "ETH".to_string(),
            quote: "USDT".to_string(),
            price: None,
            volume_24h: None,
        };

        let json = serde_json::to_string(&pair).unwrap();
        let deserialized: PairInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.symbol, "ETHUSDT");
        assert_eq!(deserialized.price, None);
        assert_eq!(deserialized.volume_24h, None);
    }
}
