use anyhow::{bail, Context, Result};
use rust_decimal::prelude::ToPrimitive;
use ssm_core::Candle;
use ssm_exchange::history;
use ssm_indicators::cvd::{analyze_cvd, CvdTrend};
use std::path::PathBuf;

/// Usage: backtest --datafile user_data/BTCUSDT-15m-*.json [--window 15]
fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let datafile = std::env::var("DATAFILE").context("DATAFILE env var required")?;
    let window: usize = std::env::var("CVD_WINDOW")
        .unwrap_or_else(|_| "15".into())
        .parse()
        .context("CVD_WINDOW must be integer")?;

    let path = PathBuf::from(&datafile);
    let candles = history::load_candles(&path)?;

    if candles.len() < window + 1 {
        bail!(
            "need at least {} candles for window={}, got {}",
            window + 1,
            window,
            candles.len()
        );
    }

    tracing::info!(
        candles = candles.len(),
        window,
        file = %path.display(),
        "starting backtest"
    );

    let results = run_backtest(&candles, window);

    // Print summary
    print_summary(&results);

    // Write results JSON
    let out_path = path.with_extension("backtest.json");
    let file = std::fs::File::create(&out_path).context("creating output file")?;
    serde_json::to_writer_pretty(std::io::BufWriter::new(file), &results)
        .context("writing results")?;
    tracing::info!(file = %out_path.display(), "backtest results saved");

    Ok(())
}

#[derive(Debug, serde::Serialize)]
struct BacktestResult {
    total_windows: usize,
    bullish_count: usize,
    bearish_count: usize,
    neutral_count: usize,
    trend_changes: usize,
    signals: Vec<SignalEvent>,
}

#[derive(Debug, serde::Serialize)]
struct SignalEvent {
    candle_index: usize,
    open_time: i64,
    trend: String,
    total_cvd: f64,
    close_price: f64,
}

fn run_backtest(candles: &[Candle], window: usize) -> BacktestResult {
    let mut signals = Vec::new();
    let mut bullish = 0usize;
    let mut bearish = 0usize;
    let mut neutral = 0usize;
    let mut trend_changes = 0usize;
    let mut prev_trend: Option<CvdTrend> = None;

    // Slide window across candles, always using only closed candles
    for end in (window + 1)..=candles.len() {
        let closed = &candles[..end - 1]; // drop forming candle
        let cvd = analyze_cvd(closed, window);

        match cvd.trend {
            CvdTrend::Bullish => bullish += 1,
            CvdTrend::Bearish => bearish += 1,
            CvdTrend::Neutral => neutral += 1,
        }

        if let Some(prev) = prev_trend {
            if prev != cvd.trend {
                trend_changes += 1;
                let last_closed = &candles[end - 2];
                signals.push(SignalEvent {
                    candle_index: end - 2,
                    open_time: last_closed.open_time,
                    trend: cvd.trend.to_string(),
                    total_cvd: cvd.total_cvd.to_f64().unwrap_or(0.0),
                    close_price: last_closed.close.to_f64().unwrap_or(0.0),
                });
                tracing::debug!(
                    idx = end - 2,
                    trend = %cvd.trend,
                    cvd = cvd.total_cvd.to_f64().unwrap_or(0.0),
                    "trend change"
                );
            }
        }
        prev_trend = Some(cvd.trend);
    }

    let total_windows = bullish + bearish + neutral;
    tracing::info!(
        total_windows,
        bullish,
        bearish,
        neutral,
        trend_changes,
        "backtest complete"
    );

    BacktestResult {
        total_windows,
        bullish_count: bullish,
        bearish_count: bearish,
        neutral_count: neutral,
        trend_changes,
        signals,
    }
}

fn print_summary(r: &BacktestResult) {
    println!("=== Backtest Summary ===");
    println!("Windows analyzed: {}", r.total_windows);
    println!(
        "Bullish: {} ({:.1}%)",
        r.bullish_count,
        pct(r.bullish_count, r.total_windows)
    );
    println!(
        "Bearish: {} ({:.1}%)",
        r.bearish_count,
        pct(r.bearish_count, r.total_windows)
    );
    println!(
        "Neutral: {} ({:.1}%)",
        r.neutral_count,
        pct(r.neutral_count, r.total_windows)
    );
    println!("Trend changes: {}", r.trend_changes);
    println!("Signal events: {}", r.signals.len());
}

fn pct(part: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (part as f64 / total as f64) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn candle_at(time: i64, buy: &str, sell: &str) -> Candle {
        let bv = Decimal::from_str(buy).unwrap();
        let sv = Decimal::from_str(sell).unwrap();
        Candle {
            open_time: time,
            open: Decimal::from(100),
            high: Decimal::from(105),
            low: Decimal::from(95),
            close: Decimal::from(102),
            volume: bv + sv,
            close_time: time + 900_000,
            quote_volume: Decimal::ZERO,
            trades: 100,
            taker_buy_volume: bv,
            taker_sell_volume: sv,
        }
    }

    #[test]
    fn backtest_consistent_bullish() {
        let candles: Vec<_> = (0..20)
            .map(|i| candle_at(i * 900_000, "60", "40"))
            .collect();
        let r = run_backtest(&candles, 5);
        assert!(r.total_windows > 0);
        assert_eq!(r.bearish_count, 0);
        assert!(r.bullish_count > 0);
    }

    #[test]
    fn backtest_detects_trend_change() {
        let mut candles: Vec<_> = (0..10)
            .map(|i| candle_at(i * 900_000, "60", "40"))
            .collect();
        // Switch to bearish
        candles.extend((10..20).map(|i| candle_at(i * 900_000, "30", "70")));

        let r = run_backtest(&candles, 5);
        assert!(r.trend_changes > 0);
        assert!(r.bullish_count > 0);
        assert!(r.bearish_count > 0);
    }
}
