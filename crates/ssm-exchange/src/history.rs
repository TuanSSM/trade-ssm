use anyhow::{Context, Result};
use ssm_core::Candle;
use std::path::Path;

use crate::binance::BinanceClient;

/// Fetch historical klines in paginated batches via Binance `startTime` param.
/// Returns all closed candles between `start_ms` and `end_ms` (epoch millis).
pub async fn download_candles(
    client: &BinanceClient,
    symbol: &str,
    interval: &str,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<Candle>> {
    let mut all = Vec::new();
    let mut cursor = start_ms;
    let batch_size: u32 = 1000; // Binance max per request

    tracing::info!(
        symbol,
        interval,
        start_ms,
        end_ms,
        "downloading historical candles"
    );

    loop {
        let batch = client
            .fetch_futures_klines_range(symbol, interval, batch_size, cursor, end_ms)
            .await?;

        if batch.is_empty() {
            break;
        }

        let last_close_time = batch.last().unwrap().close_time;
        let count = batch.len();
        all.extend(batch);

        tracing::info!(
            fetched = count,
            total = all.len(),
            cursor_ms = last_close_time,
            "batch downloaded"
        );

        // Advance cursor past the last candle's close
        cursor = last_close_time + 1;
        if cursor >= end_ms {
            break;
        }

        // Respect Binance rate limits
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    // Drop the last candle if it hasn't closed yet
    if let Some(last) = all.last() {
        let now_ms = chrono::Utc::now().timestamp_millis();
        if last.close_time > now_ms {
            all.pop();
        }
    }

    tracing::info!(total = all.len(), "download complete");
    Ok(all)
}

/// Save candles to a JSON file at `path`.
pub fn save_candles(candles: &[Candle], path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("creating data directory")?;
    }
    let file = std::fs::File::create(path).context("creating candle file")?;
    serde_json::to_writer(std::io::BufWriter::new(file), candles)
        .context("writing candles JSON")?;
    tracing::info!(path = %path.display(), count = candles.len(), "candles saved");
    Ok(())
}

/// Load candles from a JSON file.
pub fn load_candles(path: &Path) -> Result<Vec<Candle>> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let candles: Vec<Candle> =
        serde_json::from_reader(std::io::BufReader::new(file)).context("parsing candles JSON")?;
    tracing::info!(path = %path.display(), count = candles.len(), "candles loaded");
    Ok(candles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn sample_candle(time: i64) -> Candle {
        Candle {
            open_time: time,
            open: Decimal::from_str("100").unwrap(),
            high: Decimal::from_str("105").unwrap(),
            low: Decimal::from_str("95").unwrap(),
            close: Decimal::from_str("102").unwrap(),
            volume: Decimal::from_str("1000").unwrap(),
            close_time: time + 900_000, // 15m candle
            quote_volume: Decimal::from_str("100000").unwrap(),
            trades: 500,
            taker_buy_volume: Decimal::from_str("600").unwrap(),
            taker_sell_volume: Decimal::from_str("400").unwrap(),
        }
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("ssm_test_history");
        let path = dir.join("test_candles.json");

        let candles = vec![sample_candle(1000000), sample_candle(1900000)];
        save_candles(&candles, &path).unwrap();

        let loaded = load_candles(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].open_time, 1000000);
        assert_eq!(loaded[1].close, Decimal::from_str("102").unwrap());

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_save_creates_directory() {
        let dir = std::env::temp_dir().join("ssm_test_nested_dir/a/b/c");
        let path = dir.join("candles.json");

        // Ensure it doesn't exist beforehand
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("ssm_test_nested_dir"));

        let candles = vec![sample_candle(1000000)];
        save_candles(&candles, &path).unwrap();

        assert!(path.exists());
        let loaded = load_candles(&path).unwrap();
        assert_eq!(loaded.len(), 1);

        // Cleanup
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("ssm_test_nested_dir"));
    }

    #[test]
    fn test_load_nonexistent_file_errors() {
        let path = std::env::temp_dir().join("ssm_test_nonexistent_file_xyz.json");
        let result = load_candles(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_save_empty_candles() {
        let dir = std::env::temp_dir().join("ssm_test_empty_candles");
        let path = dir.join("empty.json");

        let candles: Vec<Candle> = vec![];
        save_candles(&candles, &path).unwrap();

        let loaded = load_candles(&path).unwrap();
        assert!(loaded.is_empty());

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_roundtrip_preserves_all_fields() {
        let dir = std::env::temp_dir().join("ssm_test_all_fields");
        let path = dir.join("all_fields.json");

        let candle = sample_candle(5000000);
        save_candles(&[candle.clone()], &path).unwrap();

        let loaded = load_candles(&path).unwrap();
        assert_eq!(loaded.len(), 1);

        let c = &loaded[0];
        assert_eq!(c.open_time, candle.open_time);
        assert_eq!(c.open, candle.open);
        assert_eq!(c.high, candle.high);
        assert_eq!(c.low, candle.low);
        assert_eq!(c.close, candle.close);
        assert_eq!(c.volume, candle.volume);
        assert_eq!(c.close_time, candle.close_time);
        assert_eq!(c.quote_volume, candle.quote_volume);
        assert_eq!(c.trades, candle.trades);
        assert_eq!(c.taker_buy_volume, candle.taker_buy_volume);
        assert_eq!(c.taker_sell_volume, candle.taker_sell_volume);

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }
}
