use anyhow::{Context, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use ssm_core::{Candle, Liquidation};
use std::str::FromStr;

use crate::error::ExchangeError;
use crate::exchange_trait::{Exchange, PairInfo};

const BYBIT_BASE: &str = "https://api.bybit.com";

/// Bybit exchange client.
pub struct BybitClient {
    client: reqwest::Client,
    base_url: String,
}

impl BybitClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            base_url: BYBIT_BASE.to_string(),
        }
    }

    /// Create a client with a custom base URL (useful for testing).
    pub fn with_base_url(base_url: &str) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            base_url: base_url.to_string(),
        }
    }
}

impl Default for BybitClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Map our interval strings to Bybit interval format.
/// Bybit uses: 1, 3, 5, 15, 30, 60, 120, 240, 360, 720, D, W, M
fn map_interval(interval: &str) -> &str {
    match interval {
        "1m" => "1",
        "3m" => "3",
        "5m" => "5",
        "15m" => "15",
        "30m" => "30",
        "1h" => "60",
        "2h" => "120",
        "4h" => "240",
        "6h" => "360",
        "12h" => "720",
        "1d" => "D",
        "1w" => "W",
        "1M" => "M",
        other => other,
    }
}

/// Response wrapper for Bybit V5 API.
#[derive(serde::Deserialize)]
struct BybitResponse<T> {
    #[serde(rename = "retCode")]
    ret_code: i32,
    #[serde(rename = "retMsg")]
    ret_msg: String,
    result: T,
}

#[derive(serde::Deserialize)]
struct KlineResult {
    list: Vec<Vec<String>>,
}

/// Parse a Bybit kline entry: [startTime, open, high, low, close, volume, turnover]
fn parse_bybit_kline(k: &[String], interval: &str) -> Result<Candle> {
    if k.len() < 7 {
        return Err(ExchangeError::ParseError(format!(
            "bybit kline has {} fields, expected 7",
            k.len()
        ))
        .into());
    }

    let open_time: i64 = k[0].parse().context("open_time")?;
    let open = Decimal::from_str(&k[1]).context("open")?;
    let high = Decimal::from_str(&k[2]).context("high")?;
    let low = Decimal::from_str(&k[3]).context("low")?;
    let close = Decimal::from_str(&k[4]).context("close")?;
    let volume = Decimal::from_str(&k[5]).context("volume")?;
    let turnover = Decimal::from_str(&k[6]).context("turnover")?;

    // Estimate close_time from interval
    let interval_ms = interval_to_ms(interval);
    let close_time = open_time + interval_ms - 1;

    Ok(Candle {
        open_time,
        open,
        high,
        low,
        close,
        volume,
        close_time,
        quote_volume: turnover,
        trades: 0, // Bybit kline doesn't provide trade count
        taker_buy_volume: Decimal::ZERO,
        taker_sell_volume: Decimal::ZERO,
    })
}

/// Convert interval string to milliseconds.
fn interval_to_ms(interval: &str) -> i64 {
    match interval {
        "1m" | "1" => 60_000,
        "3m" | "3" => 180_000,
        "5m" | "5" => 300_000,
        "15m" | "15" => 900_000,
        "30m" | "30" => 1_800_000,
        "1h" | "60" => 3_600_000,
        "2h" | "120" => 7_200_000,
        "4h" | "240" => 14_400_000,
        "6h" | "360" => 21_600_000,
        "12h" | "720" => 43_200_000,
        "1d" | "D" => 86_400_000,
        "1w" | "W" => 604_800_000,
        _ => 60_000,
    }
}

#[async_trait]
impl Exchange for BybitClient {
    fn name(&self) -> &str {
        "bybit"
    }

    async fn fetch_klines(&self, symbol: &str, interval: &str, limit: u32) -> Result<Vec<Candle>> {
        let bybit_interval = map_interval(interval);
        let url = format!("{}/v5/market/kline", self.base_url);

        let resp = self
            .client
            .get(&url)
            .query(&[
                ("category", "linear"),
                ("symbol", symbol),
                ("interval", bybit_interval),
                ("limit", &limit.to_string()),
            ])
            .send()
            .await
            .context("Failed to fetch Bybit klines")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ExchangeError::ApiError {
                status: status.to_string(),
                body,
            }
            .into());
        }

        let body: BybitResponse<KlineResult> = resp.json().await?;
        if body.ret_code != 0 {
            return Err(ExchangeError::ExchangeApiError {
                code: body.ret_code,
                message: body.ret_msg,
            }
            .into());
        }

        // Bybit returns newest first; reverse to match Binance convention (oldest first).
        let mut candles: Vec<Candle> = body
            .result
            .list
            .iter()
            .map(|k| parse_bybit_kline(k, interval))
            .collect::<Result<Vec<_>>>()?;
        candles.reverse();
        Ok(candles)
    }

    async fn fetch_klines_range(
        &self,
        symbol: &str,
        interval: &str,
        limit: u32,
        start_time: i64,
        end_time: i64,
    ) -> Result<Vec<Candle>> {
        let bybit_interval = map_interval(interval);
        let url = format!("{}/v5/market/kline", self.base_url);

        let resp = self
            .client
            .get(&url)
            .query(&[
                ("category", "linear"),
                ("symbol", symbol),
                ("interval", bybit_interval),
                ("limit", &limit.to_string()),
                ("start", &start_time.to_string()),
                ("end", &end_time.to_string()),
            ])
            .send()
            .await
            .context("Failed to fetch Bybit klines range")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ExchangeError::ApiError {
                status: status.to_string(),
                body,
            }
            .into());
        }

        let body: BybitResponse<KlineResult> = resp.json().await?;
        if body.ret_code != 0 {
            return Err(ExchangeError::ExchangeApiError {
                code: body.ret_code,
                message: body.ret_msg,
            }
            .into());
        }

        let mut candles: Vec<Candle> = body
            .result
            .list
            .iter()
            .map(|k| parse_bybit_kline(k, interval))
            .collect::<Result<Vec<_>>>()?;
        candles.reverse();
        Ok(candles)
    }

    async fn fetch_liquidations(&self, _symbol: &str, _limit: u32) -> Result<Vec<Liquidation>> {
        // Bybit does not provide a public liquidation endpoint in V5.
        // Return empty for now; can be implemented via WebSocket in the future.
        Ok(Vec::new())
    }

    async fn list_pairs(&self) -> Result<Vec<PairInfo>> {
        // Stub: would call /v5/market/instruments-info
        Err(ExchangeError::Unimplemented("list_pairs for Bybit".into()).into())
    }

    fn supported_timeframes(&self) -> Vec<&str> {
        vec![
            "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "12h", "1d", "1w",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bybit_client_default() {
        let client = BybitClient::default();
        assert_eq!(client.base_url, BYBIT_BASE);
    }

    #[test]
    fn test_bybit_client_with_base_url() {
        let client = BybitClient::with_base_url("http://localhost:8080");
        assert_eq!(client.base_url, "http://localhost:8080");
    }

    #[test]
    fn test_map_interval() {
        assert_eq!(map_interval("1m"), "1");
        assert_eq!(map_interval("15m"), "15");
        assert_eq!(map_interval("1h"), "60");
        assert_eq!(map_interval("4h"), "240");
        assert_eq!(map_interval("1d"), "D");
    }

    #[test]
    fn test_interval_to_ms() {
        assert_eq!(interval_to_ms("1m"), 60_000);
        assert_eq!(interval_to_ms("15m"), 900_000);
        assert_eq!(interval_to_ms("1h"), 3_600_000);
        assert_eq!(interval_to_ms("1d"), 86_400_000);
    }

    #[test]
    fn test_parse_bybit_kline() {
        let data = vec![
            "1609459200000".to_string(),
            "29000.00".to_string(),
            "29500.00".to_string(),
            "28800.00".to_string(),
            "29200.00".to_string(),
            "100.50".to_string(),
            "2920000.00".to_string(),
        ];

        let candle = parse_bybit_kline(&data, "15m").unwrap();
        assert_eq!(candle.open_time, 1609459200000);
        assert_eq!(candle.open, Decimal::from_str("29000.00").unwrap());
        assert_eq!(candle.high, Decimal::from_str("29500.00").unwrap());
        assert_eq!(candle.low, Decimal::from_str("28800.00").unwrap());
        assert_eq!(candle.close, Decimal::from_str("29200.00").unwrap());
        assert_eq!(candle.volume, Decimal::from_str("100.50").unwrap());
        assert_eq!(
            candle.quote_volume,
            Decimal::from_str("2920000.00").unwrap()
        );
        // close_time = open_time + 15min - 1ms
        assert_eq!(candle.close_time, 1609459200000 + 900_000 - 1);
    }

    #[test]
    fn test_parse_bybit_kline_too_few_fields() {
        let data = vec!["1609459200000".to_string(), "29000.00".to_string()];
        assert!(parse_bybit_kline(&data, "15m").is_err());
    }

    #[test]
    fn test_bybit_name() {
        let client = BybitClient::new();
        assert_eq!(Exchange::name(&client), "bybit");
    }

    #[test]
    fn test_bybit_supported_timeframes_contents() {
        let client = BybitClient::new();
        let tf = client.supported_timeframes();
        assert!(tf.contains(&"1m"));
        assert!(tf.contains(&"15m"));
        assert!(tf.contains(&"1h"));
        assert!(tf.contains(&"4h"));
        assert!(tf.contains(&"1d"));
        assert!(!tf.contains(&"1M")); // Monthly not in list
    }
}
