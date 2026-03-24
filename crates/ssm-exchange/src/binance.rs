use anyhow::{Context, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use ssm_core::{Candle, ForceOrderResponse, Liquidation};
use std::str::FromStr;

use crate::exchange_trait::{Exchange, PairInfo};

const FUTURES_BASE: &str = "https://fapi.binance.com";

pub struct BinanceClient {
    client: reqwest::Client,
}

impl BinanceClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// Fetch OHLCV candles from Binance futures.
    /// `interval`: 1m, 3m, 5m, 15m, 1h, 4h, 1d
    pub async fn fetch_futures_klines(
        &self,
        symbol: &str,
        interval: &str,
        limit: u32,
    ) -> Result<Vec<Candle>> {
        let url = format!("{FUTURES_BASE}/fapi/v1/klines");
        let resp = self
            .client
            .get(&url)
            .query(&[
                ("symbol", symbol),
                ("interval", interval),
                ("limit", &limit.to_string()),
            ])
            .send()
            .await
            .context("Failed to fetch Binance futures klines")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Binance futures klines API returned {status}: {body}");
        }

        let raw: Vec<Vec<serde_json::Value>> = resp.json().await?;
        raw.into_iter().map(|k| parse_kline(&k)).collect()
    }

    /// Fetch futures klines with explicit time range (for historical download).
    pub async fn fetch_futures_klines_range(
        &self,
        symbol: &str,
        interval: &str,
        limit: u32,
        start_time: i64,
        end_time: i64,
    ) -> Result<Vec<Candle>> {
        let url = format!("{FUTURES_BASE}/fapi/v1/klines");
        let resp = self
            .client
            .get(&url)
            .query(&[
                ("symbol", symbol),
                ("interval", interval),
                ("limit", &limit.to_string()),
                ("startTime", &start_time.to_string()),
                ("endTime", &end_time.to_string()),
            ])
            .send()
            .await
            .context("Failed to fetch Binance futures klines range")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Binance futures klines range API returned {status}: {body}");
        }

        let raw: Vec<Vec<serde_json::Value>> = resp.json().await?;
        raw.into_iter().map(|k| parse_kline(&k)).collect()
    }

    /// Fetch recent forced liquidation orders from Binance futures.
    pub async fn fetch_liquidations(&self, symbol: &str, limit: u32) -> Result<Vec<Liquidation>> {
        let url = format!("{FUTURES_BASE}/fapi/v1/forceOrders");
        let resp = self
            .client
            .get(&url)
            .query(&[("symbol", symbol), ("limit", &limit.to_string())])
            .send()
            .await
            .context("Failed to fetch Binance liquidations")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Binance liquidations API returned {status}: {body}");
        }

        let orders: Vec<ForceOrderResponse> = resp.json().await?;
        Ok(orders
            .into_iter()
            .map(|o| Liquidation {
                symbol: o.symbol,
                side: o.side,
                price: o.price,
                quantity: o.orig_qty,
                time: o.time,
            })
            .collect())
    }
}

impl Default for BinanceClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Exchange for BinanceClient {
    fn name(&self) -> &str {
        "binance"
    }

    async fn fetch_klines(&self, symbol: &str, interval: &str, limit: u32) -> Result<Vec<Candle>> {
        self.fetch_futures_klines(symbol, interval, limit).await
    }

    async fn fetch_klines_range(
        &self,
        symbol: &str,
        interval: &str,
        limit: u32,
        start_time: i64,
        end_time: i64,
    ) -> Result<Vec<Candle>> {
        self.fetch_futures_klines_range(symbol, interval, limit, start_time, end_time)
            .await
    }

    async fn fetch_liquidations(&self, symbol: &str, limit: u32) -> Result<Vec<Liquidation>> {
        self.fetch_liquidations(symbol, limit).await
    }

    async fn list_pairs(&self) -> Result<Vec<PairInfo>> {
        // Stub: would call /fapi/v1/exchangeInfo
        anyhow::bail!("list_pairs not yet implemented for Binance")
    }

    fn supported_timeframes(&self) -> Vec<&str> {
        vec![
            "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h", "1d", "3d", "1w",
            "1M",
        ]
    }
}

/// Parse Binance kline array: [open_time, O, H, L, C, vol, close_time,
/// quote_vol, trades, taker_buy_base_vol, taker_buy_quote_vol, ignore]
fn parse_kline(k: &[serde_json::Value]) -> Result<Candle> {
    let dec = |v: &serde_json::Value| -> Result<Decimal> {
        Decimal::from_str(v.as_str().context("expected string for decimal")?)
            .context("invalid decimal")
    };

    let volume = dec(&k[5])?;
    let taker_buy_volume = dec(&k[9])?;

    Ok(Candle {
        open_time: k[0].as_i64().context("open_time")?,
        open: dec(&k[1])?,
        high: dec(&k[2])?,
        low: dec(&k[3])?,
        close: dec(&k[4])?,
        volume,
        close_time: k[6].as_i64().context("close_time")?,
        quote_volume: dec(&k[7])?,
        trades: k[8].as_u64().context("trades")?,
        taker_buy_volume,
        taker_sell_volume: volume - taker_buy_volume,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_kline() {
        let raw = vec![
            serde_json::json!(1609459200000i64),
            serde_json::json!("29000.00"),
            serde_json::json!("29500.00"),
            serde_json::json!("28800.00"),
            serde_json::json!("29200.00"),
            serde_json::json!("100.50"),
            serde_json::json!(1609462800000i64),
            serde_json::json!("2920000.00"),
            serde_json::json!(5000u64),
            serde_json::json!("60.30"),
            serde_json::json!("1752000.00"),
            serde_json::json!("0"),
        ];

        let candle = parse_kline(&raw).unwrap();
        assert_eq!(candle.open, Decimal::from_str("29000.00").unwrap());
        assert_eq!(candle.taker_buy_volume, Decimal::from_str("60.30").unwrap());
        assert_eq!(
            candle.taker_sell_volume,
            Decimal::from_str("40.20").unwrap()
        );
    }

    #[test]
    fn test_parse_kline_sell_volume_calculation() {
        let raw = vec![
            serde_json::json!(1000000i64),
            serde_json::json!("50000.00"),
            serde_json::json!("51000.00"),
            serde_json::json!("49000.00"),
            serde_json::json!("50500.00"),
            serde_json::json!("200.00"), // total volume
            serde_json::json!(1899999i64),
            serde_json::json!("10000000.00"),
            serde_json::json!(1000u64),
            serde_json::json!("120.00"), // taker_buy_volume
            serde_json::json!("6000000.00"),
            serde_json::json!("0"),
        ];

        let candle = parse_kline(&raw).unwrap();
        // taker_sell_volume = volume - taker_buy_volume = 200 - 120 = 80
        assert_eq!(candle.volume, Decimal::from_str("200.00").unwrap());
        assert_eq!(
            candle.taker_buy_volume,
            Decimal::from_str("120.00").unwrap()
        );
        assert_eq!(
            candle.taker_sell_volume,
            Decimal::from_str("80.00").unwrap()
        );
        assert_eq!(
            candle.taker_sell_volume,
            candle.volume - candle.taker_buy_volume
        );
    }

    #[test]
    fn test_parse_kline_all_fields() {
        let raw = vec![
            serde_json::json!(1609459200000i64), // open_time
            serde_json::json!("29000.00"),       // open
            serde_json::json!("29500.00"),       // high
            serde_json::json!("28800.00"),       // low
            serde_json::json!("29200.00"),       // close
            serde_json::json!("100.50"),         // volume
            serde_json::json!(1609462799999i64), // close_time
            serde_json::json!("2920000.00"),     // quote_volume
            serde_json::json!(5000u64),          // trades
            serde_json::json!("60.30"),          // taker_buy_volume
            serde_json::json!("1752000.00"),     // taker_buy_quote_volume (unused)
            serde_json::json!("0"),              // ignore
        ];

        let candle = parse_kline(&raw).unwrap();
        assert_eq!(candle.open_time, 1609459200000);
        assert_eq!(candle.open, Decimal::from_str("29000.00").unwrap());
        assert_eq!(candle.high, Decimal::from_str("29500.00").unwrap());
        assert_eq!(candle.low, Decimal::from_str("28800.00").unwrap());
        assert_eq!(candle.close, Decimal::from_str("29200.00").unwrap());
        assert_eq!(candle.volume, Decimal::from_str("100.50").unwrap());
        assert_eq!(candle.close_time, 1609462799999);
        assert_eq!(
            candle.quote_volume,
            Decimal::from_str("2920000.00").unwrap()
        );
        assert_eq!(candle.trades, 5000);
        assert_eq!(candle.taker_buy_volume, Decimal::from_str("60.30").unwrap());
        assert_eq!(
            candle.taker_sell_volume,
            Decimal::from_str("40.20").unwrap()
        );
    }

    #[test]
    fn test_binance_client_default() {
        // BinanceClient::default() should work the same as ::new()
        // Both create a client with a default reqwest::Client
        let _client_new = BinanceClient::new();
        let _client_default = BinanceClient::default();
        // If we get here without panicking, both constructors work
    }
}
