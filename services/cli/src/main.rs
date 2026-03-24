use std::env;
use std::fs;

use rust_decimal::prelude::ToPrimitive;
use ssm_core::Candle;
use ssm_indicators::plot::{ChartData, ChartType};

fn main() {
    let args: Vec<String> = env::args().collect();
    let command = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match command {
        "list-pairs" => list_pairs(),
        "list-timeframes" => list_timeframes(),
        "show-config" => show_config(),
        "new-strategy" => new_strategy(&args),
        "show-trades" => show_trades(&args),
        "plot" => plot(&args),
        "help" | "--help" | "-h" => print_help(),
        _ => {
            eprintln!("Unknown command: {command}");
            print_help();
        }
    }
}

fn list_pairs() {
    let pairs = [
        "BTCUSDT",
        "ETHUSDT",
        "BNBUSDT",
        "SOLUSDT",
        "XRPUSDT",
        "DOGEUSDT",
        "ADAUSDT",
        "AVAXUSDT",
        "DOTUSDT",
        "MATICUSDT",
        "LINKUSDT",
        "LTCUSDT",
        "BCHUSDT",
        "ATOMUSDT",
        "NEARUSDT",
        "APTUSDT",
        "ARBUSDT",
        "OPUSDT",
        "SUIUSDT",
        "PEPEUSDT",
    ];
    println!("Supported trading pairs:");
    for pair in &pairs {
        println!("  {pair}");
    }
}

pub fn list_timeframes() {
    let timeframes = supported_timeframes();
    println!("Supported timeframes:");
    for tf in &timeframes {
        println!("  {tf}");
    }
}

pub fn supported_timeframes() -> Vec<&'static str> {
    vec![
        "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h", "1d", "3d", "1w",
    ]
}

fn show_config() {
    println!("Current configuration:");
    println!();

    let vars = [
        ("SYMBOL", "BTCUSDT"),
        ("INTERVAL", "15m"),
        ("CHECK_INTERVAL_SECS", "60"),
        ("EXECUTION_MODE", "paper"),
        ("DAYS", "30"),
        ("DATADIR", "user_data"),
        ("CVD_WINDOW", "15"),
    ];

    for (name, default) in &vars {
        let value = env::var(name).unwrap_or_else(|_| format!("{default} (default)"));
        println!("  {name:.<30} {value}");
    }

    // Sensitive vars — redact values
    let secret_vars = ["TELEGRAM_BOT_TOKEN", "TELEGRAM_CHAT_ID"];
    for name in &secret_vars {
        let value = match env::var(name) {
            Ok(_) => "****".to_string(),
            Err(_) => "(not set)".to_string(),
        };
        println!("  {name:.<30} {value}");
    }

    // DATAFILE — not secret but optional
    let datafile = env::var("DATAFILE").unwrap_or_else(|_| "(not set)".to_string());
    println!("  {name:.<30} {val}", name = "DATAFILE", val = datafile);
}

fn new_strategy(args: &[String]) {
    let name = match args.get(2) {
        Some(n) => n,
        None => {
            eprintln!("Usage: trade-ssm new-strategy <name>");
            eprintln!("Example: trade-ssm new-strategy my_custom_strategy");
            std::process::exit(1);
        }
    };

    let struct_name = to_pascal_case(name);

    print!(
        r#"use anyhow::Result;
use ssm_core::{{Candle, Signal, SignalDirection}};
use ssm_strategy::Strategy;

/// Custom strategy: {name}
pub struct {struct_name};

impl Strategy for {struct_name} {{
    fn name(&self) -> &str {{
        "{name}"
    }}

    fn analyze(&self, candles: &[Candle]) -> Result<Option<Signal>> {{
        // Anti-repainting: only use closed candles
        if candles.len() < 2 {{
            return Ok(None);
        }}
        let closed = &candles[..candles.len() - 1];
        let last = closed.last().unwrap();

        // TODO: implement your strategy logic here
        // Example: simple price change check
        // if last.close > last.open {{
        //     return Ok(Some(Signal {{
        //         direction: SignalDirection::Long,
        //         strength: rust_decimal::Decimal::ONE,
        //         reason: "bullish candle".to_string(),
        //     }}));
        // }}

        Ok(None)
    }}
}}

#[cfg(test)]
mod tests {{
    use super::*;

    #[test]
    fn test_{name}_returns_none_on_empty() {{
        let strategy = {struct_name};
        let result = strategy.analyze(&[]).unwrap();
        assert!(result.is_none());
    }}
}}
"#
    );
}

fn show_trades(args: &[String]) {
    let path = match args.get(2) {
        Some(p) => p,
        None => {
            eprintln!("Usage: trade-ssm show-trades <json_file>");
            std::process::exit(1);
        }
    };

    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading file '{path}': {e}");
            std::process::exit(1);
        }
    };

    let candles: Vec<Candle> = match serde_json::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error parsing JSON: {e}");
            eprintln!("Expected an array of candle/trade records.");
            std::process::exit(1);
        }
    };

    println!(
        "{:<24} {:>12} {:>12} {:>12} {:>12} {:>14}",
        "Time (ms)", "Open", "High", "Low", "Close", "Volume"
    );
    println!("{}", "-".repeat(100));

    for candle in &candles {
        println!(
            "{:<24} {:>12} {:>12} {:>12} {:>12} {:>14}",
            candle.open_time, candle.open, candle.high, candle.low, candle.close, candle.volume,
        );
    }
    println!();
    println!("Total records: {}", candles.len());
}

fn plot(args: &[String]) {
    let path = match args.get(2) {
        Some(p) => p,
        None => {
            eprintln!("Usage: trade-ssm plot <backtest_json>");
            eprintln!("Generates an HTML chart file from backtest candle data.");
            std::process::exit(1);
        }
    };

    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error reading file '{path}': {e}");
            std::process::exit(1);
        }
    };

    let candles: Vec<Candle> = match serde_json::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error parsing JSON: {e}");
            std::process::exit(1);
        }
    };

    if candles.is_empty() {
        eprintln!("No candle data found in file.");
        std::process::exit(1);
    }

    let mut chart = ChartData::from_candles(&candles);

    // Add EMA-20 if enough data
    if candles.len() >= 20 {
        let ema_values: Vec<(i64, f64)> = compute_ema(&candles, 20);
        chart.add_indicator("EMA 20", ChartType::Line, &ema_values);
    }

    let output_path = path.replace(".json", "_chart.html");
    let html = chart.to_html();

    match fs::write(&output_path, &html) {
        Ok(()) => println!("Chart written to: {output_path}"),
        Err(e) => {
            eprintln!("Error writing chart: {e}");
            std::process::exit(1);
        }
    }
}

fn print_help() {
    println!("trade-ssm — CLI utility commands for trade-ssm");
    println!();
    println!("USAGE:");
    println!("  trade-ssm <command> [args...]");
    println!();
    println!("COMMANDS:");
    println!("  list-pairs              List supported trading pairs");
    println!("  list-timeframes         List supported candle timeframes");
    println!("  show-config             Display current configuration (env vars)");
    println!("  new-strategy <name>     Generate a strategy template to stdout");
    println!("  show-trades <json>      Display trade/candle records from a JSON file");
    println!("  plot <backtest_json>    Generate an HTML chart from backtest data");
    println!("  help                    Show this help message");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().to_string() + &chars.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect()
}

/// Simple EMA calculation over candle close prices.
fn compute_ema(candles: &[Candle], period: usize) -> Vec<(i64, f64)> {
    if candles.len() < period {
        return Vec::new();
    }

    let multiplier = 2.0 / (period as f64 + 1.0);
    let mut result = Vec::with_capacity(candles.len() - period + 1);

    // SMA for first value
    let sma: f64 = candles[..period]
        .iter()
        .map(|c| c.close.to_f64().unwrap_or(0.0))
        .sum::<f64>()
        / period as f64;

    result.push((candles[period - 1].open_time, sma));

    let mut prev_ema = sma;
    for candle in &candles[period..] {
        let close = candle.close.to_f64().unwrap_or(0.0);
        let ema = (close - prev_ema) * multiplier + prev_ema;
        result.push((candle.open_time, ema));
        prev_ema = ema;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_timeframes_includes_expected() {
        let tfs = supported_timeframes();
        assert!(tfs.contains(&"1m"));
        assert!(tfs.contains(&"5m"));
        assert!(tfs.contains(&"15m"));
        assert!(tfs.contains(&"1h"));
        assert!(tfs.contains(&"4h"));
        assert!(tfs.contains(&"1d"));
        assert!(tfs.contains(&"1w"));
        assert_eq!(tfs.len(), 14);
    }

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("my_strategy"), "MyStrategy");
        assert_eq!(to_pascal_case("simple"), "Simple");
        assert_eq!(to_pascal_case("a_b_c"), "ABC");
    }

    #[test]
    fn test_supported_timeframes_order() {
        let tfs = supported_timeframes();
        assert_eq!(tfs[0], "1m");
        assert_eq!(tfs[tfs.len() - 1], "1w");
    }

    #[test]
    fn test_compute_ema_basic() {
        use rust_decimal::Decimal;

        let candles: Vec<Candle> = (0..25)
            .map(|i| Candle {
                open_time: i * 60_000,
                open: Decimal::from(100 + i),
                high: Decimal::from(105 + i),
                low: Decimal::from(95 + i),
                close: Decimal::from(100 + i),
                volume: Decimal::from(1000),
                close_time: (i + 1) * 60_000,
                quote_volume: Decimal::from(100_000),
                trades: 50,
                taker_buy_volume: Decimal::from(600),
                taker_sell_volume: Decimal::from(400),
            })
            .collect();

        let ema = compute_ema(&candles, 20);
        assert_eq!(ema.len(), 6); // 25 - 20 + 1
        assert!(ema[0].1 > 0.0);
    }

    #[test]
    fn test_compute_ema_insufficient_data() {
        let ema = compute_ema(&[], 20);
        assert!(ema.is_empty());
    }
}
