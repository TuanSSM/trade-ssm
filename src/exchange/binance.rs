use anyhow::{Context, Result};
use rust_decimal::Decimal;
use std::str::FromStr;

use super::types::{Candle, ForceOrderResponse, Liquidation};

const SPOT_BASE: &str = "https://api.binance.com";
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

    /// Fetch OHLCV candles from Binance spot.
    /// `interval`: 1m, 3m, 5m, 15m, 1h, 4h, 1d
    /// `limit`: number of candles (max 1000)
    pub async fn fetch_klines(
        &self,
        symbol: &str,
        interval: &str,
        limit: u32,
    ) -> Result<Vec<Candle>> {
        let url = format!("{}/api/v3/klines", SPOT_BASE);
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
            .context("Failed to fetch Binance klines")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Binance klines API returned {}: {}", status, body);
        }

        let raw: Vec<Vec<serde_json::Value>> = resp.json().await?;
        let candles = raw
            .into_iter()
            .map(|k| parse_kline(&k))
            .collect::<Result<Vec<_>>>()?;

        Ok(candles)
    }

    /// Fetch OHLCV candles from Binance futures.
    pub async fn fetch_futures_klines(
        &self,
        symbol: &str,
        interval: &str,
        limit: u32,
    ) -> Result<Vec<Candle>> {
        let url = format!("{}/fapi/v1/klines", FUTURES_BASE);
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
            anyhow::bail!("Binance futures klines API returned {}: {}", status, body);
        }

        let raw: Vec<Vec<serde_json::Value>> = resp.json().await?;
        let candles = raw
            .into_iter()
            .map(|k| parse_kline(&k))
            .collect::<Result<Vec<_>>>()?;

        Ok(candles)
    }

    /// Fetch recent forced liquidation orders from Binance futures.
    pub async fn fetch_liquidations(
        &self,
        symbol: &str,
        limit: u32,
    ) -> Result<Vec<Liquidation>> {
        let url = format!("{}/fapi/v1/forceOrders", FUTURES_BASE);
        let resp = self
            .client
            .get(&url)
            .query(&[
                ("symbol", symbol),
                ("limit", &limit.to_string()),
            ])
            .send()
            .await
            .context("Failed to fetch Binance liquidations")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Binance liquidations API returned {}: {}", status, body);
        }

        let orders: Vec<ForceOrderResponse> = resp.json().await?;
        let liquidations = orders
            .into_iter()
            .map(|o| Liquidation {
                symbol: o.symbol,
                side: o.side,
                price: o.price,
                quantity: o.orig_qty,
                time: o.time,
            })
            .collect();

        Ok(liquidations)
    }
}

/// Parse a Binance kline array into a Candle struct.
/// Kline format: [open_time, open, high, low, close, volume, close_time,
///                quote_volume, trades, taker_buy_base_vol, taker_buy_quote_vol, ignore]
fn parse_kline(k: &[serde_json::Value]) -> Result<Candle> {
    let dec = |v: &serde_json::Value| -> Result<Decimal> {
        let s = v.as_str().context("expected string for decimal")?;
        Decimal::from_str(s).context("invalid decimal")
    };

    let volume = dec(&k[5])?;
    let taker_buy_volume = dec(&k[9])?;
    let taker_sell_volume = volume - taker_buy_volume;

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
        taker_sell_volume,
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
}
