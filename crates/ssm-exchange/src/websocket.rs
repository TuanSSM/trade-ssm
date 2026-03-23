use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use ssm_core::{Candle, Liquidation, Side, Trade};
use std::str::FromStr;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

const FUTURES_STREAM_BASE: &str = "wss://fstream.binance.com/stream?streams=";

/// Events emitted by the WebSocket client.
#[derive(Debug, Clone)]
pub enum WsEvent {
    Trade(Trade),
    Liquidation(Liquidation),
    Kline(Candle),
}

/// Configuration for the WebSocket client.
#[derive(Debug, Clone)]
pub struct WsConfig {
    pub symbol: String,
    pub kline_interval: String,
    /// Maximum reconnect attempts before giving up (0 = infinite).
    pub max_reconnects: u32,
    /// Base delay for exponential backoff in milliseconds.
    pub reconnect_base_ms: u64,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            symbol: "btcusdt".to_string(),
            kline_interval: "15m".to_string(),
            max_reconnects: 0,
            reconnect_base_ms: 1000,
        }
    }
}

/// Binance Futures WebSocket client.
///
/// Connects to aggTrade, forceOrder, and kline streams.
/// Emits parsed events through an mpsc channel.
pub struct BinanceWsClient {
    config: WsConfig,
}

impl BinanceWsClient {
    pub fn new(config: WsConfig) -> Self {
        Self { config }
    }

    /// Build the combined stream URL for all subscribed streams.
    fn stream_url(&self) -> String {
        let sym = self.config.symbol.to_lowercase();
        let interval = &self.config.kline_interval;
        format!("{FUTURES_STREAM_BASE}{sym}@aggTrade/{sym}@forceOrder/{sym}@kline_{interval}")
    }

    /// Start the WebSocket connection and emit events.
    ///
    /// This function runs indefinitely, reconnecting on failures.
    /// Send events to the provided channel. Returns only on fatal error
    /// or if max_reconnects is exceeded.
    pub async fn run(&self, tx: mpsc::Sender<WsEvent>) -> Result<()> {
        let mut attempt = 0u32;

        loop {
            let url = self.stream_url();
            tracing::info!(%url, attempt, "connecting to Binance WebSocket");

            match self.connect_and_stream(&url, &tx).await {
                Ok(()) => {
                    tracing::info!("WebSocket stream ended cleanly");
                    return Ok(());
                }
                Err(e) => {
                    attempt += 1;
                    tracing::warn!(error = %e, attempt, "WebSocket connection failed");

                    if self.config.max_reconnects > 0 && attempt >= self.config.max_reconnects {
                        return Err(e).context("max reconnect attempts exceeded");
                    }

                    let delay = self.config.reconnect_base_ms * 2u64.pow(attempt.min(6));
                    tracing::info!(delay_ms = delay, "reconnecting after backoff");
                    tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                }
            }
        }
    }

    async fn connect_and_stream(&self, url: &str, tx: &mpsc::Sender<WsEvent>) -> Result<()> {
        let (ws_stream, _) = tokio_tungstenite::connect_async(url)
            .await
            .context("WebSocket connect failed")?;

        tracing::info!("WebSocket connected");

        let (mut write, mut read) = ws_stream.split();

        loop {
            tokio::select! {
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            if let Some(event) = parse_combined_stream(&text) {
                                if tx.send(event).await.is_err() {
                                    tracing::info!("receiver dropped, shutting down WS");
                                    return Ok(());
                                }
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            write.send(Message::Pong(data)).await?;
                        }
                        Some(Ok(Message::Close(_))) => {
                            tracing::info!("server sent close frame");
                            return Ok(());
                        }
                        Some(Err(e)) => {
                            return Err(e).context("WebSocket read error");
                        }
                        None => {
                            return Ok(());
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

/// Parse a combined stream message from Binance.
///
/// Combined stream format: `{ "stream": "btcusdt@aggTrade", "data": { ... } }`
fn parse_combined_stream(text: &str) -> Option<WsEvent> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let stream = value.get("stream")?.as_str()?;
    let data = value.get("data")?;

    if stream.contains("@aggTrade") {
        parse_agg_trade(data)
    } else if stream.contains("@forceOrder") {
        parse_force_order(data)
    } else if stream.contains("@kline") {
        parse_kline_event(data)
    } else {
        None
    }
}

/// Parse aggTrade event into a `Trade`.
///
/// ```json
/// { "e": "aggTrade", "s": "BTCUSDT", "p": "50000.00", "q": "0.100",
///   "T": 1234567890, "m": false }
/// ```
fn parse_agg_trade(data: &serde_json::Value) -> Option<WsEvent> {
    let symbol = data.get("s")?.as_str()?.to_string();
    let price = Decimal::from_str(data.get("p")?.as_str()?).ok()?;
    let quantity = Decimal::from_str(data.get("q")?.as_str()?).ok()?;
    let timestamp = data.get("T")?.as_i64()?;
    // "m" = true means the buyer is the market maker → taker is seller
    let is_buyer_maker = data.get("m")?.as_bool()?;
    let side = if is_buyer_maker {
        Side::Sell
    } else {
        Side::Buy
    };

    Some(WsEvent::Trade(Trade {
        symbol,
        price,
        quantity,
        side,
        timestamp,
        is_liquidation: false,
    }))
}

/// Parse forceOrder event into a `Liquidation`.
///
/// ```json
/// { "e": "forceOrder", "o": { "s": "BTCUSDT", "S": "SELL", "p": "50000",
///   "q": "0.100", "T": 1234567890 } }
/// ```
fn parse_force_order(data: &serde_json::Value) -> Option<WsEvent> {
    let order = data.get("o")?;
    let symbol = order.get("s")?.as_str()?.to_string();
    let side = order.get("S")?.as_str()?.to_string();
    let price = Decimal::from_str(order.get("p")?.as_str()?).ok()?;
    let quantity = Decimal::from_str(order.get("q")?.as_str()?).ok()?;
    let time = order.get("T")?.as_i64()?;

    Some(WsEvent::Liquidation(Liquidation {
        symbol,
        side,
        price,
        quantity,
        time,
    }))
}

/// Parse kline event into a `Candle`.
///
/// Only emits when the kline is closed (`x: true`).
fn parse_kline_event(data: &serde_json::Value) -> Option<WsEvent> {
    let kline = data.get("k")?;
    let is_closed = kline.get("x")?.as_bool()?;
    if !is_closed {
        return None;
    }

    let open_time = kline.get("t")?.as_i64()?;
    let close_time = kline.get("T")?.as_i64()?;
    let open = Decimal::from_str(kline.get("o")?.as_str()?).ok()?;
    let high = Decimal::from_str(kline.get("h")?.as_str()?).ok()?;
    let low = Decimal::from_str(kline.get("l")?.as_str()?).ok()?;
    let close = Decimal::from_str(kline.get("c")?.as_str()?).ok()?;
    let volume = Decimal::from_str(kline.get("v")?.as_str()?).ok()?;
    let quote_volume = Decimal::from_str(kline.get("q")?.as_str()?).ok()?;
    let trades = kline.get("n")?.as_u64()?;
    let taker_buy_volume = Decimal::from_str(kline.get("V")?.as_str()?).ok()?;
    let taker_sell_volume = volume - taker_buy_volume;

    Some(WsEvent::Kline(Candle {
        open_time,
        open,
        high,
        low,
        close,
        volume,
        close_time,
        quote_volume,
        trades,
        taker_buy_volume,
        taker_sell_volume,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agg_trade_event() {
        let json = r#"{
            "stream": "btcusdt@aggTrade",
            "data": {
                "e": "aggTrade", "s": "BTCUSDT",
                "p": "50000.00", "q": "0.100",
                "T": 1234567890, "m": false
            }
        }"#;
        let event = parse_combined_stream(json).unwrap();
        match event {
            WsEvent::Trade(t) => {
                assert_eq!(t.symbol, "BTCUSDT");
                assert_eq!(t.price, Decimal::from_str("50000.00").unwrap());
                assert_eq!(t.quantity, Decimal::from_str("0.100").unwrap());
                assert_eq!(t.side, Side::Buy);
                assert_eq!(t.timestamp, 1234567890);
                assert!(!t.is_liquidation);
            }
            _ => panic!("expected Trade event"),
        }
    }

    #[test]
    fn parse_agg_trade_seller() {
        let json = r#"{
            "stream": "btcusdt@aggTrade",
            "data": {
                "e": "aggTrade", "s": "BTCUSDT",
                "p": "49000.00", "q": "0.500",
                "T": 1234567891, "m": true
            }
        }"#;
        let event = parse_combined_stream(json).unwrap();
        match event {
            WsEvent::Trade(t) => assert_eq!(t.side, Side::Sell),
            _ => panic!("expected Trade event"),
        }
    }

    #[test]
    fn parse_force_order_event() {
        let json = r#"{
            "stream": "btcusdt@forceOrder",
            "data": {
                "e": "forceOrder",
                "o": {
                    "s": "BTCUSDT", "S": "SELL",
                    "p": "48000.00", "q": "2.000", "T": 9999999
                }
            }
        }"#;
        let event = parse_combined_stream(json).unwrap();
        match event {
            WsEvent::Liquidation(l) => {
                assert_eq!(l.symbol, "BTCUSDT");
                assert_eq!(l.side, "SELL");
                assert_eq!(l.price, Decimal::from_str("48000.00").unwrap());
                assert_eq!(l.quantity, Decimal::from_str("2.000").unwrap());
            }
            _ => panic!("expected Liquidation event"),
        }
    }

    #[test]
    fn parse_kline_closed() {
        let json = r#"{
            "stream": "btcusdt@kline_15m",
            "data": {
                "e": "kline",
                "k": {
                    "t": 1000000, "T": 1899999,
                    "o": "50000", "h": "51000", "l": "49000", "c": "50500",
                    "v": "100.5", "q": "5050000", "n": 5000,
                    "V": "60.3", "x": true
                }
            }
        }"#;
        let event = parse_combined_stream(json).unwrap();
        match event {
            WsEvent::Kline(c) => {
                assert_eq!(c.open_time, 1000000);
                assert_eq!(c.open, Decimal::from_str("50000").unwrap());
                assert_eq!(c.taker_buy_volume, Decimal::from_str("60.3").unwrap());
                assert_eq!(
                    c.taker_sell_volume,
                    Decimal::from_str("100.5").unwrap() - Decimal::from_str("60.3").unwrap()
                );
            }
            _ => panic!("expected Kline event"),
        }
    }

    #[test]
    fn skip_unclosed_kline() {
        let json = r#"{
            "stream": "btcusdt@kline_15m",
            "data": {
                "e": "kline",
                "k": {
                    "t": 1000000, "T": 1899999,
                    "o": "50000", "h": "51000", "l": "49000", "c": "50500",
                    "v": "100.5", "q": "5050000", "n": 5000,
                    "V": "60.3", "x": false
                }
            }
        }"#;
        assert!(parse_combined_stream(json).is_none());
    }

    #[test]
    fn unknown_stream_returns_none() {
        let json = r#"{ "stream": "unknown@stream", "data": {} }"#;
        assert!(parse_combined_stream(json).is_none());
    }

    #[test]
    fn ws_config_default() {
        let cfg = WsConfig::default();
        assert_eq!(cfg.symbol, "btcusdt");
        assert_eq!(cfg.kline_interval, "15m");
    }

    #[test]
    fn stream_url_format() {
        let client = BinanceWsClient::new(WsConfig {
            symbol: "ethusdt".to_string(),
            kline_interval: "1h".to_string(),
            ..Default::default()
        });
        let url = client.stream_url();
        assert!(url.contains("ethusdt@aggTrade"));
        assert!(url.contains("ethusdt@forceOrder"));
        assert!(url.contains("ethusdt@kline_1h"));
    }

    #[test]
    fn test_invalid_json_returns_none() {
        let result = parse_combined_stream("this is not json at all");
        assert!(result.is_none());
    }

    #[test]
    fn test_missing_stream_field_returns_none() {
        let json = r#"{ "data": { "e": "aggTrade", "s": "BTCUSDT", "p": "50000", "q": "1", "T": 123, "m": false } }"#;
        assert!(parse_combined_stream(json).is_none());
    }

    #[test]
    fn test_missing_data_field_returns_none() {
        let json = r#"{ "stream": "btcusdt@aggTrade" }"#;
        assert!(parse_combined_stream(json).is_none());
    }

    #[test]
    fn test_kline_taker_sell_volume() {
        let json = r#"{
            "stream": "btcusdt@kline_15m",
            "data": {
                "e": "kline",
                "k": {
                    "t": 1000000, "T": 1899999,
                    "o": "50000", "h": "51000", "l": "49000", "c": "50500",
                    "v": "250.0", "q": "12500000", "n": 8000,
                    "V": "150.0", "x": true
                }
            }
        }"#;
        let event = parse_combined_stream(json).unwrap();
        match event {
            WsEvent::Kline(c) => {
                // taker_sell_volume = volume - taker_buy_volume = 250.0 - 150.0 = 100.0
                assert_eq!(c.volume, Decimal::from_str("250.0").unwrap());
                assert_eq!(c.taker_buy_volume, Decimal::from_str("150.0").unwrap());
                assert_eq!(c.taker_sell_volume, Decimal::from_str("100.0").unwrap());
                assert_eq!(c.taker_sell_volume, c.volume - c.taker_buy_volume);
            }
            _ => panic!("expected Kline event"),
        }
    }

    #[test]
    fn test_stream_url_lowercases_symbol() {
        let client = BinanceWsClient::new(WsConfig {
            symbol: "BTCUSDT".to_string(),
            kline_interval: "15m".to_string(),
            ..Default::default()
        });
        let url = client.stream_url();
        assert!(url.contains("btcusdt@aggTrade"));
        assert!(url.contains("btcusdt@forceOrder"));
        assert!(url.contains("btcusdt@kline_15m"));
        // Should NOT contain uppercase
        assert!(!url.contains("BTCUSDT"));
    }

    #[test]
    fn test_ws_config_custom() {
        let cfg = WsConfig {
            symbol: "ETHUSDT".to_string(),
            kline_interval: "1h".to_string(),
            max_reconnects: 5,
            reconnect_base_ms: 2000,
        };
        assert_eq!(cfg.symbol, "ETHUSDT");
        assert_eq!(cfg.kline_interval, "1h");
        assert_eq!(cfg.max_reconnects, 5);
        assert_eq!(cfg.reconnect_base_ms, 2000);
    }
}
