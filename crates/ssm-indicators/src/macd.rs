use rust_decimal::Decimal;
use ssm_core::Candle;

use crate::ema::ema;

/// MACD analysis result.
#[derive(Debug, Clone)]
pub struct MacdResult {
    /// MACD line (fast EMA - slow EMA).
    pub macd: Vec<Decimal>,
    /// Signal line (EMA of MACD line).
    pub signal: Vec<Decimal>,
    /// Histogram (MACD - signal).
    pub histogram: Vec<Decimal>,
}

/// Compute MACD (Moving Average Convergence Divergence).
///
/// Default parameters: fast=12, slow=26, signal=9.
pub fn macd(candles: &[Candle], fast: usize, slow: usize, signal_period: usize) -> MacdResult {
    let fast_ema = ema(candles, fast);
    let slow_ema = ema(candles, slow);

    if fast_ema.is_empty() || slow_ema.is_empty() {
        return MacdResult {
            macd: vec![],
            signal: vec![],
            histogram: vec![],
        };
    }

    // Align: fast_ema starts at index (fast-1), slow_ema at (slow-1)
    // MACD starts when both are available
    let offset = slow - fast; // how many more fast_ema values exist before slow_ema starts
    let macd_values: Vec<Decimal> = fast_ema[offset..]
        .iter()
        .zip(slow_ema.iter())
        .map(|(f, s)| f - s)
        .collect();

    // Signal line = EMA of MACD values
    if macd_values.len() < signal_period || signal_period == 0 {
        return MacdResult {
            macd: macd_values,
            signal: vec![],
            histogram: vec![],
        };
    }

    let multiplier = Decimal::from(2) / Decimal::from((signal_period + 1) as u64);
    let one_minus = Decimal::ONE - multiplier;

    let initial_sma: Decimal =
        macd_values[..signal_period].iter().sum::<Decimal>() / Decimal::from(signal_period as u64);

    let mut signal_line = Vec::with_capacity(macd_values.len() - signal_period + 1);
    signal_line.push(initial_sma);

    let mut prev = initial_sma;
    for val in &macd_values[signal_period..] {
        let s = *val * multiplier + prev * one_minus;
        signal_line.push(s);
        prev = s;
    }

    // Histogram: MACD - signal (aligned from where signal starts)
    let hist_offset = macd_values.len() - signal_line.len();
    let histogram: Vec<Decimal> = macd_values[hist_offset..]
        .iter()
        .zip(signal_line.iter())
        .map(|(m, s)| m - s)
        .collect();

    MacdResult {
        macd: macd_values,
        signal: signal_line,
        histogram,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn candle_close(close: &str) -> Candle {
        let c = Decimal::from_str(close).unwrap();
        Candle {
            open_time: 0,
            open: c,
            high: c,
            low: c,
            close: c,
            volume: Decimal::from(100),
            close_time: 0,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: Decimal::from(50),
            taker_sell_volume: Decimal::from(50),
        }
    }

    #[test]
    fn macd_basic() {
        let candles: Vec<_> = (0..50)
            .map(|i| candle_close(&format!("{}", 100 + i)))
            .collect();
        let result = macd(&candles, 12, 26, 9);
        assert!(!result.macd.is_empty());
        assert!(!result.signal.is_empty());
        assert!(!result.histogram.is_empty());
        assert_eq!(result.signal.len(), result.histogram.len());
    }

    #[test]
    fn macd_uptrend_positive() {
        let candles: Vec<_> = (0..50)
            .map(|i| candle_close(&format!("{}", 100 + i * 2)))
            .collect();
        let result = macd(&candles, 12, 26, 9);
        // In an uptrend, MACD should be positive (fast EMA > slow EMA)
        let last_macd = result.macd.last().unwrap();
        assert!(
            *last_macd > Decimal::ZERO,
            "MACD should be positive in uptrend"
        );
    }

    #[test]
    fn insufficient_candles() {
        let candles: Vec<_> = (0..10)
            .map(|i| candle_close(&format!("{}", 100 + i)))
            .collect();
        let result = macd(&candles, 12, 26, 9);
        assert!(result.macd.is_empty());
    }

    #[test]
    fn no_repainting_macd() {
        let short: Vec<_> = (0..50)
            .map(|i| candle_close(&format!("{}", 100 + i)))
            .collect();
        let mut long = short.clone();
        long.push(candle_close("200"));

        let r_short = macd(&short, 12, 26, 9);
        let r_long = macd(&long, 12, 26, 9);

        for i in 0..r_short.macd.len() {
            assert_eq!(r_short.macd[i], r_long.macd[i], "MACD repainting at {i}");
        }
    }
}
