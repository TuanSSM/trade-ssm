use rust_decimal::Decimal;
use ssm_core::Candle;

/// Compute Average True Range (ATR) using Wilder's smoothing.
///
/// True Range = max(high-low, |high-prev_close|, |low-prev_close|)
/// ATR = smoothed average of True Range over `period`.
pub fn atr(candles: &[Candle], period: usize) -> Vec<Decimal> {
    if candles.len() <= period || period == 0 {
        return vec![];
    }

    let period_dec = Decimal::from(period as u64);
    let one = Decimal::ONE;

    // Calculate true ranges
    let mut true_ranges = Vec::with_capacity(candles.len() - 1);
    for i in 1..candles.len() {
        let high_low = candles[i].high - candles[i].low;
        let high_prev_close = (candles[i].high - candles[i - 1].close).abs();
        let low_prev_close = (candles[i].low - candles[i - 1].close).abs();
        true_ranges.push(high_low.max(high_prev_close).max(low_prev_close));
    }

    if true_ranges.len() < period {
        return vec![];
    }

    // Initial ATR = average of first `period` true ranges
    let initial: Decimal = true_ranges[..period].iter().sum::<Decimal>() / period_dec;

    let mut result = Vec::with_capacity(true_ranges.len() - period + 1);
    result.push(initial);

    // Wilder's smoothing
    let mut prev = initial;
    for tr in &true_ranges[period..] {
        let val = (prev * (period_dec - one) + *tr) / period_dec;
        result.push(val);
        prev = val;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn candle_ohlc(open: &str, high: &str, low: &str, close: &str) -> Candle {
        Candle {
            open_time: 0,
            open: Decimal::from_str(open).unwrap(),
            high: Decimal::from_str(high).unwrap(),
            low: Decimal::from_str(low).unwrap(),
            close: Decimal::from_str(close).unwrap(),
            volume: Decimal::from(100),
            close_time: 0,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: Decimal::from(50),
            taker_sell_volume: Decimal::from(50),
        }
    }

    #[test]
    fn atr_basic() {
        let candles: Vec<_> = (0..20)
            .map(|i| {
                let base = 100 + i;
                candle_ohlc(
                    &format!("{}", base),
                    &format!("{}", base + 5),
                    &format!("{}", base - 3),
                    &format!("{}", base + 2),
                )
            })
            .collect();
        let result = atr(&candles, 14);
        assert!(!result.is_empty());
        // All ATR values should be positive
        for val in &result {
            assert!(*val > Decimal::ZERO);
        }
    }

    #[test]
    fn atr_constant_range() {
        // Same range every candle → ATR should converge to that range
        let candles: Vec<_> = (0..30)
            .map(|i| {
                let base = 100 + i;
                candle_ohlc(
                    &format!("{}", base),
                    &format!("{}", base + 10),
                    &format!("{}", base),
                    &format!("{}", base + 5),
                )
            })
            .collect();
        let result = atr(&candles, 14);
        assert!(!result.is_empty());
        // ATR should be approximately 10 (the constant range)
        let last = *result.last().unwrap();
        assert!(last >= Decimal::from(9) && last <= Decimal::from(11));
    }

    #[test]
    fn no_repainting_atr() {
        let short: Vec<_> = (0..20)
            .map(|i| {
                candle_ohlc(
                    &format!("{}", 100 + i),
                    &format!("{}", 108 + i),
                    &format!("{}", 95 + i),
                    &format!("{}", 103 + i),
                )
            })
            .collect();
        let mut long = short.clone();
        long.push(candle_ohlc("121", "130", "115", "125"));

        let r_short = atr(&short, 14);
        let r_long = atr(&long, 14);

        for i in 0..r_short.len() {
            assert_eq!(r_short[i], r_long[i], "ATR repainting at {i}");
        }
    }

    #[test]
    fn test_atr_insufficient_data() {
        let candles: Vec<_> = (0..5)
            .map(|i| {
                candle_ohlc(
                    &format!("{}", 100 + i),
                    &format!("{}", 105 + i),
                    &format!("{}", 95 + i),
                    &format!("{}", 102 + i),
                )
            })
            .collect();
        let result = atr(&candles, 14);
        assert!(result.is_empty());
    }

    #[test]
    fn test_atr_zero_period() {
        let candles: Vec<_> = (0..10)
            .map(|i| {
                candle_ohlc(
                    &format!("{}", 100 + i),
                    &format!("{}", 105 + i),
                    &format!("{}", 95 + i),
                    &format!("{}", 102 + i),
                )
            })
            .collect();
        let result = atr(&candles, 0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_atr_output_length() {
        let candles: Vec<_> = (0..20)
            .map(|i| {
                candle_ohlc(
                    &format!("{}", 100 + i),
                    &format!("{}", 108 + i),
                    &format!("{}", 95 + i),
                    &format!("{}", 103 + i),
                )
            })
            .collect();
        let period = 14;
        let result = atr(&candles, period);
        // true_ranges has len candles.len()-1 = 19, ATR output = 19 - 14 + 1 = 6
        assert_eq!(result.len(), candles.len() - 1 - period + 1);
    }

    #[test]
    fn test_atr_all_positive() {
        let candles: Vec<_> = (0..30)
            .map(|i| {
                let base = 100 + (i as i64 % 10) * 3;
                candle_ohlc(
                    &format!("{}", base),
                    &format!("{}", base + 7),
                    &format!("{}", base - 4),
                    &format!("{}", base + 2),
                )
            })
            .collect();
        let result = atr(&candles, 14);
        assert!(!result.is_empty());
        for val in &result {
            assert!(*val > Decimal::ZERO, "ATR must be positive, got {val}");
        }
    }

    #[test]
    fn test_atr_trending_market_increasing() {
        // Increasing volatility: each candle has a wider range
        let candles: Vec<_> = (0..25)
            .map(|i| {
                let base = 100 + i * 2;
                let spread = 2 + i; // increasing range
                candle_ohlc(
                    &format!("{}", base),
                    &format!("{}", base + spread),
                    &format!("{}", base - spread),
                    &format!("{}", base + 1),
                )
            })
            .collect();
        let result = atr(&candles, 5);
        assert!(result.len() >= 2);
        // ATR should generally increase as volatility grows
        let first = result[0];
        let last = *result.last().unwrap();
        assert!(
            last > first,
            "ATR should increase with rising volatility: first={first}, last={last}"
        );
    }

    #[test]
    fn test_atr_declining_volatility() {
        // Decreasing volatility: range shrinks over time
        let candles: Vec<_> = (0..25)
            .map(|i| {
                let base = 100;
                let spread = 20i64 - i as i64; // spread from 20 down to ~0
                let spread = spread.max(1); // keep at least 1
                candle_ohlc(
                    &format!("{}", base),
                    &format!("{}", base as i64 + spread),
                    &format!("{}", base as i64 - spread),
                    &format!("{}", base),
                )
            })
            .collect();
        let result = atr(&candles, 5);
        assert!(result.len() >= 2);
        let first = result[0];
        let last = *result.last().unwrap();
        assert!(
            last < first,
            "ATR should decrease with declining volatility: first={first}, last={last}"
        );
    }

    #[test]
    fn test_atr_gap_up_true_range() {
        // Test that gaps are captured by true range (not just high-low)
        let candles = vec![
            candle_ohlc("100", "110", "90", "105"),   // normal
            candle_ohlc("100", "110", "90", "100"),    // normal
            candle_ohlc("100", "110", "90", "100"),    // close at 100
            candle_ohlc("130", "140", "125", "135"),   // gap up: low(125) > prev_close(100)
        ];
        // With period=2, we need > 2 candles
        let result = atr(&candles, 2);
        assert!(!result.is_empty());
        // The last TR should include the gap: |high(140) - prev_close(100)| = 40
        // which is larger than high-low = 15
        let last = *result.last().unwrap();
        assert!(
            last > Decimal::from(15),
            "ATR should capture the gap, got {last}"
        );
    }
}
