use anyhow::Result;
use ssm_core::{env_or, env_parse, DEFAULT_DATADIR, DEFAULT_DOWNLOAD_DAYS, DEFAULT_INTERVAL, DEFAULT_SYMBOL};
use ssm_exchange::binance::BinanceClient;
use ssm_exchange::history;
use std::path::PathBuf;

/// Usage: download-data [--symbol BTCUSDT] [--interval 15m] [--days 30] [--datadir user_data]
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

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

    let client = BinanceClient::new();
    let candles = history::download_candles(
        &client,
        &symbol,
        &interval,
        start.timestamp_millis(),
        end.timestamp_millis(),
    )
    .await?;

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
    Ok(())
}

fn format_epoch_date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .format("%Y%m%d")
        .to_string()
}
