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

    #[test]
    fn test_macd_downtrend_negative() {
        let candles: Vec<_> = (0..50)
            .map(|i| candle_close(&format!("{}", 200 - i * 2)))
            .collect();
        let result = macd(&candles, 12, 26, 9);
        let last_macd = result.macd.last().unwrap();
        assert!(
            *last_macd < Decimal::ZERO,
            "MACD should be negative in downtrend, got {last_macd}"
        );
    }

    #[test]
    fn test_macd_histogram_equals_diff() {
        let candles: Vec<_> = (0..50)
            .map(|i| candle_close(&format!("{}", 100 + i)))
            .collect();
        let result = macd(&candles, 12, 26, 9);
        assert!(!result.histogram.is_empty());
        // histogram[i] = macd[offset+i] - signal[i]
        let hist_offset = result.macd.len() - result.signal.len();
        for i in 0..result.histogram.len() {
            assert_eq!(
                result.histogram[i],
                result.macd[hist_offset + i] - result.signal[i],
                "Histogram mismatch at index {i}"
            );
        }
    }

    #[test]
    fn test_macd_signal_length() {
        let candles: Vec<_> = (0..50)
            .map(|i| candle_close(&format!("{}", 100 + i)))
            .collect();
        let signal_period = 9;
        let result = macd(&candles, 12, 26, signal_period);
        assert_eq!(
            result.signal.len(),
            result.macd.len() - signal_period + 1
        );
    }

    #[test]
    fn test_macd_single_candle() {
        let candles = vec![candle_close("100")];
        let result = macd(&candles, 12, 26, 9);
        assert!(result.macd.is_empty());
        assert!(result.signal.is_empty());
        assert!(result.histogram.is_empty());
    }

    #[test]
    fn test_macd_all_same_prices() {
        // All prices identical => fast EMA == slow EMA => MACD line all zeros
        let candles: Vec<_> = (0..50).map(|_| candle_close("100")).collect();
        let result = macd(&candles, 12, 26, 9);
        assert!(!result.macd.is_empty());
        for val in &result.macd {
            assert_eq!(*val, Decimal::ZERO, "MACD should be 0 for constant price");
        }
        for val in &result.signal {
            assert_eq!(
                *val,
                Decimal::ZERO,
                "Signal should be 0 for constant price"
            );
        }
        for val in &result.histogram {
            assert_eq!(
                *val,
                Decimal::ZERO,
                "Histogram should be 0 for constant price"
            );
        }
    }

    #[test]
    fn test_macd_small_periods() {
        // Very small period values: fast=2, slow=3, signal=2
        let candles: Vec<_> = (0..10)
            .map(|i| candle_close(&format!("{}", 100 + i * 5)))
            .collect();
        let result = macd(&candles, 2, 3, 2);
        assert!(!result.macd.is_empty());
        assert!(!result.signal.is_empty());
        assert!(!result.histogram.is_empty());
    }

    #[test]
    fn test_macd_signal_line_is_ema_of_macd() {
        // First signal value should be the SMA of the first signal_period MACD values
        let candles: Vec<_> = (0..50)
            .map(|i| candle_close(&format!("{}", 100 + i * 2)))
            .collect();
        let signal_period = 9;
        let result = macd(&candles, 12, 26, signal_period);
        assert!(!result.signal.is_empty());
        // First signal = average of first 9 MACD values
        let expected_first_signal: Decimal =
            result.macd[..signal_period].iter().sum::<Decimal>()
                / Decimal::from(signal_period as u64);
        assert_eq!(
            result.signal[0], expected_first_signal,
            "First signal should be SMA of first {signal_period} MACD values"
        );
    }

    #[test]
    fn test_macd_histogram_sign_changes() {
        // Create data that goes up then down sharply to force histogram sign change
        let mut candles: Vec<Candle> = (0..50)
            .map(|i| candle_close(&format!("{}", 100 + i * 5)))
            .collect();
        // Then reverse sharply for longer
        for i in 0..50 {
            candles.push(candle_close(&format!("{}", 345 - i * 5)));
        }
        let result = macd(&candles, 12, 26, 9);
        assert!(!result.histogram.is_empty());
        // Check that histogram has both positive and negative values
        let has_positive = result.histogram.iter().any(|h| *h > Decimal::ZERO);
        let has_negative = result.histogram.iter().any(|h| *h < Decimal::ZERO);
        assert!(
            has_positive && has_negative,
            "Histogram should have sign changes in up-then-down market"
        );
    }

    #[test]
    fn test_macd_not_enough_for_signal() {
        // Enough data for MACD line but not for signal line
        // With fast=12, slow=26, we need 26 candles for first MACD value
        // With signal=9, we need 34 candles total for signal
        // 30 candles => MACD line exists but signal may be empty or short
        let candles: Vec<_> = (0..27)
            .map(|i| candle_close(&format!("{}", 100 + i)))
            .collect();
        let result = macd(&candles, 12, 26, 9);
        // Should have MACD values but no signal (only 2 MACD values, need 9 for signal)
        assert!(!result.macd.is_empty());
        assert!(
            result.signal.is_empty(),
            "Signal should be empty when not enough MACD values"
        );
        assert!(result.histogram.is_empty());
    }
}
