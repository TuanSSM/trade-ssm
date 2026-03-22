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
}
