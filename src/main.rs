mod exchange;
mod indicators;
mod signals;

use anyhow::Result;
use exchange::binance::BinanceClient;
use indicators::cvd::analyze_cvd;
use indicators::liquidations::analyze_liquidations;
use signals::telegram::{format_report, TelegramBot};

const DEFAULT_SYMBOL: &str = "BTCUSDT";
const DEFAULT_INTERVAL: &str = "15m";
const DEFAULT_CVD_WINDOW: usize = 15;
const DEFAULT_CHECK_INTERVAL_SECS: u64 = 60;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("trade-ssm starting up");

    let telegram = TelegramBot::from_env()?;
    let binance = BinanceClient::new();

    let symbol = std::env::var("SYMBOL").unwrap_or_else(|_| DEFAULT_SYMBOL.to_string());
    let interval = std::env::var("INTERVAL").unwrap_or_else(|_| DEFAULT_INTERVAL.to_string());
    let check_interval_secs: u64 = std::env::var("CHECK_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_CHECK_INTERVAL_SECS);

    tracing::info!(
        symbol = %symbol,
        interval = %interval,
        check_interval_secs = check_interval_secs,
        "Configuration loaded"
    );

    // Send startup notification
    telegram
        .send_message(&format!(
            "*trade-ssm started*\nMonitoring {} {} candles\nCVD window: {} candles\nCheck interval: {}s",
            symbol, interval, DEFAULT_CVD_WINDOW, check_interval_secs
        ))
        .await?;

    loop {
        match run_analysis(&binance, &telegram, &symbol, &interval).await {
            Ok(()) => {
                tracing::info!("Analysis cycle completed");
            }
            Err(e) => {
                tracing::error!(error = %e, "Analysis cycle failed");
                // Try to notify via Telegram about the error
                let _ = telegram
                    .send_message(&format!("*trade-ssm error:*\n`{}`", e))
                    .await;
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(check_interval_secs)).await;
    }
}

async fn run_analysis(
    binance: &BinanceClient,
    telegram: &TelegramBot,
    symbol: &str,
    interval: &str,
) -> Result<()> {
    // Fetch candles: request window + 1 so we can drop the current (forming) candle
    let candles = binance
        .fetch_futures_klines(symbol, interval, (DEFAULT_CVD_WINDOW + 1) as u32)
        .await?;

    // Anti-repainting: drop the last candle (still forming)
    let closed_candles = if candles.len() > 1 {
        &candles[..candles.len() - 1]
    } else {
        &candles
    };

    tracing::info!(
        closed_candles = closed_candles.len(),
        "Fetched candle data"
    );

    // Calculate CVD on closed candles only
    let cvd = analyze_cvd(closed_candles, DEFAULT_CVD_WINDOW);

    // Fetch liquidations
    let liquidations = binance.fetch_liquidations(symbol, 100).await?;
    let liq_summary = analyze_liquidations(&liquidations);

    tracing::info!(
        cvd_trend = %cvd.trend,
        liq_bias = %liq_summary.bias,
        liq_count = liquidations.len(),
        "Analysis complete"
    );

    // Format and send report
    let report = format_report(symbol, interval, &cvd, &liq_summary);
    telegram.send_message(&report).await?;

    Ok(())
}
