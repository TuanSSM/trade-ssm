/// NATS topic definitions for trade-ssm microservices.
///
/// Topic naming convention: `ssm.{domain}.{symbol}[.{qualifier}]`
///
/// Raw trade ticks from exchange. Published by data-feed service.
/// Payload: `Trade` (JSON)
pub fn trades(symbol: &str) -> String {
    format!("ssm.trades.{}", symbol.to_lowercase())
}

/// Closed candles at a specific timeframe. Published by data-feed service.
/// Payload: `Candle` (JSON)
pub fn candles(symbol: &str, timeframe: &str) -> String {
    format!("ssm.candles.{}.{}", symbol.to_lowercase(), timeframe)
}

/// Liquidation events. Published by data-feed service.
/// Payload: `Liquidation` (JSON)
pub fn liquidations(symbol: &str) -> String {
    format!("ssm.liquidations.{}", symbol.to_lowercase())
}

/// Trading signals produced by strategies. Published by signal service.
/// Payload: `Signal` (JSON)
pub fn signals(symbol: &str) -> String {
    format!("ssm.signals.{}", symbol.to_lowercase())
}

/// Order execution events (fills, rejects). Published by execution service.
/// Payload: `Order` (JSON)
pub fn orders(symbol: &str) -> String {
    format!("ssm.orders.{}", symbol.to_lowercase())
}

/// Position updates. Published by execution service.
/// Payload: `Position` (JSON)
pub fn positions(symbol: &str) -> String {
    format!("ssm.positions.{}", symbol.to_lowercase())
}

/// Metrics and health. Published by all services.
/// Payload: JSON
pub fn metrics(service: &str) -> String {
    format!("ssm.metrics.{service}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_format() {
        assert_eq!(trades("BTCUSDT"), "ssm.trades.btcusdt");
        assert_eq!(candles("ETHUSDT", "15m"), "ssm.candles.ethusdt.15m");
        assert_eq!(liquidations("BTCUSDT"), "ssm.liquidations.btcusdt");
        assert_eq!(signals("BTCUSDT"), "ssm.signals.btcusdt");
        assert_eq!(orders("BTCUSDT"), "ssm.orders.btcusdt");
        assert_eq!(positions("BTCUSDT"), "ssm.positions.btcusdt");
        assert_eq!(metrics("data-feed"), "ssm.metrics.data-feed");
    }

    #[test]
    fn lowercase_symbol_stays_lowercase() {
        assert_eq!(trades("btcusdt"), "ssm.trades.btcusdt");
        assert_eq!(candles("ethusdt", "1h"), "ssm.candles.ethusdt.1h");
        assert_eq!(liquidations("btcusdt"), "ssm.liquidations.btcusdt");
        assert_eq!(signals("btcusdt"), "ssm.signals.btcusdt");
        assert_eq!(orders("btcusdt"), "ssm.orders.btcusdt");
        assert_eq!(positions("btcusdt"), "ssm.positions.btcusdt");
    }

    #[test]
    fn mixed_case_symbol_normalized_to_lowercase() {
        assert_eq!(trades("BtcUsDt"), "ssm.trades.btcusdt");
        assert_eq!(candles("EthUsDt", "5m"), "ssm.candles.ethusdt.5m");
        assert_eq!(liquidations("SoLuSdT"), "ssm.liquidations.solusdt");
    }

    #[test]
    fn metrics_preserves_service_name_case() {
        // metrics does not lowercase — service name is passed as-is
        assert_eq!(metrics("analyzer"), "ssm.metrics.analyzer");
        assert_eq!(metrics("data-feed"), "ssm.metrics.data-feed");
        assert_eq!(metrics("execution"), "ssm.metrics.execution");
    }

    #[test]
    fn candles_with_various_timeframes() {
        assert_eq!(candles("BTCUSDT", "1m"), "ssm.candles.btcusdt.1m");
        assert_eq!(candles("BTCUSDT", "5m"), "ssm.candles.btcusdt.5m");
        assert_eq!(candles("BTCUSDT", "1h"), "ssm.candles.btcusdt.1h");
        assert_eq!(candles("BTCUSDT", "4h"), "ssm.candles.btcusdt.4h");
        assert_eq!(candles("BTCUSDT", "1d"), "ssm.candles.btcusdt.1d");
    }

    #[test]
    fn empty_symbol_produces_trailing_dot() {
        // Edge case: empty symbol string
        assert_eq!(trades(""), "ssm.trades.");
        assert_eq!(liquidations(""), "ssm.liquidations.");
        assert_eq!(signals(""), "ssm.signals.");
        assert_eq!(orders(""), "ssm.orders.");
        assert_eq!(positions(""), "ssm.positions.");
    }

    #[test]
    fn candles_empty_symbol_and_timeframe() {
        assert_eq!(candles("", ""), "ssm.candles..");
        assert_eq!(candles("", "15m"), "ssm.candles..15m");
        assert_eq!(candles("BTCUSDT", ""), "ssm.candles.btcusdt.");
    }

    #[test]
    fn metrics_empty_service_name() {
        assert_eq!(metrics(""), "ssm.metrics.");
    }

    #[test]
    fn all_uppercase_symbols_normalized() {
        // Verify all topic functions normalize uppercase to lowercase
        let sym = "SOLUSDT";
        assert!(trades(sym).ends_with("solusdt"));
        assert!(liquidations(sym).ends_with("solusdt"));
        assert!(signals(sym).ends_with("solusdt"));
        assert!(orders(sym).ends_with("solusdt"));
        assert!(positions(sym).ends_with("solusdt"));
    }
}
