use anyhow::Result;
use ssm_exchange::binance::BinanceClient;
use ssm_indicators::cvd::analyze_cvd;
use ssm_indicators::liquidations::analyze_liquidations;
use ssm_notify::telegram::{format_report, TelegramBot};

const DEFAULT_SYMBOL: &str = "BTCUSDT";
const DEFAULT_INTERVAL: &str = "15m";
const CVD_WINDOW: usize = 15;
const DEFAULT_CHECK_SECS: u64 = 60;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let telegram = TelegramBot::from_env()?;
    let binance = BinanceClient::new();

    let symbol = env_or("SYMBOL", DEFAULT_SYMBOL);
    let interval = env_or("INTERVAL", DEFAULT_INTERVAL);
    let check_secs: u64 = std::env::var("CHECK_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_CHECK_SECS);

    tracing::info!(%symbol, %interval, check_secs, "trade-ssm analyzer starting");

    telegram
        .send_message(&format!(
            "*trade-ssm started*\n{symbol} {interval} | CVD window: {CVD_WINDOW} | interval: {check_secs}s"
        ))
        .await?;

    loop {
        if let Err(e) = run_cycle(&binance, &telegram, &symbol, &interval).await {
            tracing::error!(error = %e, "cycle failed");
            let _ = telegram.send_message(&format!("*error:* `{e}`")).await;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(check_secs)).await;
    }
}

async fn run_cycle(
    binance: &BinanceClient,
    telegram: &TelegramBot,
    symbol: &str,
    interval: &str,
) -> Result<()> {
    let candles = binance
        .fetch_futures_klines(symbol, interval, (CVD_WINDOW + 1) as u32)
        .await?;

    // Anti-repainting: drop the forming (last) candle
    let closed = if candles.len() > 1 {
        &candles[..candles.len() - 1]
    } else {
        &candles
    };
    tracing::info!(closed = closed.len(), "fetched candles");

    let cvd = analyze_cvd(closed, CVD_WINDOW);
    let liqs = binance.fetch_liquidations(symbol, 100).await?;
    let liq_summary = analyze_liquidations(&liqs);

    tracing::info!(cvd = %cvd.trend, liq = %liq_summary.bias, "analysis done");

    telegram
        .send_message(&format_report(symbol, interval, &cvd, &liq_summary))
        .await
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
