use anyhow::Result;
use ssm_core::{
    env_or, env_parse, DEFAULT_DATADIR, DEFAULT_DOWNLOAD_DAYS, DEFAULT_INTERVAL, DEFAULT_SYMBOL,
};
use ssm_exchange::binance::BinanceClient;
use ssm_exchange::history;
use std::path::PathBuf;

/// Usage: download-data [--symbol BTCUSDT] [--interval 15m] [--days 30] [--datadir user_data]
#[tokio::main]
async fn main() -> Result<()> {
    ssm_core::init_logging();

    let symbol = env_or("SYMBOL", DEFAULT_SYMBOL);
    let interval = env_or("INTERVAL", DEFAULT_INTERVAL);
    let days: i64 = env_parse("DAYS", DEFAULT_DOWNLOAD_DAYS as i64);
    let datadir = env_or("DATADIR", DEFAULT_DATADIR);

    let end = chrono::Utc::now();
    let start = end - chrono::Duration::days(days);

    tracing::info!(
        %symbol,
        %interval,
        days,
        start = %start.format("%Y-%m-%d"),
        end = %end.format("%Y-%m-%d"),
        "downloading historical data"
    );

    let correlation_pairs: Vec<String> = std::env::var("CORRELATION_PAIRS")
        .map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default();

    if !correlation_pairs.is_empty() {
        tracing::info!(pairs = ?correlation_pairs, "will also download correlated pair data");
    }

    let client = BinanceClient::new();
    let start_ms = start.timestamp_millis();
    let end_ms = end.timestamp_millis();

    let candles = history::download_candles(&client, &symbol, &interval, start_ms, end_ms).await?;

    if candles.is_empty() {
        tracing::warn!("no candles downloaded");
        return Ok(());
    }

    let first_date = format_epoch_date(candles.first().unwrap().open_time);
    let last_date = format_epoch_date(candles.last().unwrap().open_time);

    let filename = format!("{symbol}-{interval}-{first_date}-{last_date}.json");
    let path = PathBuf::from(&datadir).join(&filename);

    history::save_candles(&candles, &path)?;

    tracing::info!(
        candles = candles.len(),
        file = %path.display(),
        "download complete"
    );

    // Download correlated pair data using the same interval and date range
    for corr_symbol in &correlation_pairs {
        tracing::info!(symbol = %corr_symbol, "downloading correlated pair data");
        let corr_candles =
            history::download_candles(&client, corr_symbol, &interval, start_ms, end_ms).await?;

        if corr_candles.is_empty() {
            tracing::warn!(symbol = %corr_symbol, "no candles downloaded for correlated pair");
            continue;
        }

        let corr_first = format_epoch_date(corr_candles.first().unwrap().open_time);
        let corr_last = format_epoch_date(corr_candles.last().unwrap().open_time);
        let corr_filename = format!("{corr_symbol}-{interval}-{corr_first}-{corr_last}.json");
        let corr_path = PathBuf::from(&datadir).join(&corr_filename);

        history::save_candles(&corr_candles, &corr_path)?;

        tracing::info!(
            symbol = %corr_symbol,
            candles = corr_candles.len(),
            file = %corr_path.display(),
            "correlated pair download complete"
        );
    }

    Ok(())
}

fn format_epoch_date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .format("%Y%m%d")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_epoch_date() {
        // 2021-01-01 00:00:00 UTC
        assert_eq!(format_epoch_date(1609459200000), "20210101");
    }

    #[test]
    fn test_format_epoch_date_zero() {
        assert_eq!(format_epoch_date(0), "19700101");
    }

    #[test]
    fn config_defaults() {
        let symbol = env_or("__NONEXISTENT__", DEFAULT_SYMBOL);
        assert_eq!(symbol, "BTCUSDT");
        let days: i64 = env_parse("__NONEXISTENT__", DEFAULT_DOWNLOAD_DAYS as i64);
        assert_eq!(days, 30);
    }
}
