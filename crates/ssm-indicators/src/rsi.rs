use rust_decimal::Decimal;
use ssm_core::Candle;

/// Compute RSI (Relative Strength Index) using Wilder's smoothing.
///
/// RSI = 100 - (100 / (1 + RS))
/// RS = avg_gain / avg_loss
///
/// Returns one RSI value per candle starting from index `period`.
pub fn rsi(candles: &[Candle], period: usize) -> Vec<Decimal> {
    if candles.len() <= period || period == 0 {
        return vec![];
    }

    let period_dec = Decimal::from(period as u64);
    let hundred = Decimal::from(100);

    // Calculate initial average gain and loss
    let mut gains = Decimal::ZERO;
    let mut losses = Decimal::ZERO;

    for i in 1..=period {
        let change = candles[i].close - candles[i - 1].close;
        if change > Decimal::ZERO {
            gains += change;
        } else {
            losses += change.abs();
        }
    }

    let mut avg_gain = gains / period_dec;
    let mut avg_loss = losses / period_dec;

    let mut result = Vec::with_capacity(candles.len() - period);

    // First RSI value
    let rs = if avg_loss > Decimal::ZERO {
        avg_gain / avg_loss
    } else if avg_gain > Decimal::ZERO {
        Decimal::from(999) // max RSI
    } else {
        Decimal::ONE // neutral
    };
    result.push(hundred - hundred / (Decimal::ONE + rs));

    // Subsequent values using Wilder's smoothing
    let one = Decimal::ONE;
    for i in (period + 1)..candles.len() {
        let change = candles[i].close - candles[i - 1].close;
        let (gain, loss) = if change > Decimal::ZERO {
            (change, Decimal::ZERO)
        } else {
            (Decimal::ZERO, change.abs())
        };

        avg_gain = (avg_gain * (period_dec - one) + gain) / period_dec;
        avg_loss = (avg_loss * (period_dec - one) + loss) / period_dec;

        let rs = if avg_loss > Decimal::ZERO {
            avg_gain / avg_loss
        } else if avg_gain > Decimal::ZERO {
            Decimal::from(999)
        } else {
            Decimal::ONE
        };
        result.push(hundred - hundred / (Decimal::ONE + rs));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::prelude::ToPrimitive;
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
    fn rsi_all_up() {
        let candles: Vec<_> = (0..20)
            .map(|i| candle_close(&format!("{}", 100 + i)))
            .collect();
        let result = rsi(&candles, 14);
        assert!(!result.is_empty());
        // All prices going up → RSI should be high (near 100)
        let last = result.last().unwrap().to_f64().unwrap();
        assert!(last > 80.0, "RSI should be high for uptrend, got {last}");
    }

    #[test]
    fn rsi_all_down() {
        let candles: Vec<_> = (0..20)
            .map(|i| candle_close(&format!("{}", 200 - i)))
            .collect();
        let result = rsi(&candles, 14);
        assert!(!result.is_empty());
        let last = result.last().unwrap().to_f64().unwrap();
        assert!(last < 20.0, "RSI should be low for downtrend, got {last}");
    }

    #[test]
    fn rsi_range() {
        let candles: Vec<_> = (0..30)
            .map(|i| candle_close(&format!("{}", 100 + (i % 10))))
            .collect();
        let result = rsi(&candles, 14);
        for val in &result {
            let v = val.to_f64().unwrap();
            assert!((0.0..=100.0).contains(&v), "RSI out of range: {v}");
        }
    }

    #[test]
    fn no_repainting_rsi() {
        let short: Vec<_> = (0..20)
            .map(|i| candle_close(&format!("{}", 100 + i)))
            .collect();
        let mut long = short.clone();
        long.push(candle_close("125"));

        let r_short = rsi(&short, 14);
        let r_long = rsi(&long, 14);

        for i in 0..r_short.len() {
            assert_eq!(r_short[i], r_long[i], "RSI repainting at index {i}");
        }
    }

    #[test]
    fn insufficient_data() {
        let candles: Vec<_> = (0..5)
            .map(|i| candle_close(&format!("{}", 100 + i)))
            .collect();
        assert!(rsi(&candles, 14).is_empty());
    }

    #[test]
    fn test_rsi_flat_market() {
        // Constant price → no gains, no losses → RSI should be 50
        let candles: Vec<_> = (0..20).map(|_| candle_close("100")).collect();
        let result = rsi(&candles, 14);
        assert!(!result.is_empty());
        for val in &result {
            let v = val.to_f64().unwrap();
            assert!(
                (v - 50.0).abs() < 0.01,
                "RSI for flat market should be 50, got {v}"
            );
        }
    }

    #[test]
    fn test_rsi_output_length() {
        let candles: Vec<_> = (0..30)
            .map(|i| candle_close(&format!("{}", 100 + i)))
            .collect();
        let period = 14;
        let result = rsi(&candles, period);
        assert_eq!(result.len(), candles.len() - period);
    }

    #[test]
    fn test_rsi_zero_period() {
        let candles: Vec<_> = (0..10)
            .map(|i| candle_close(&format!("{}", 100 + i)))
            .collect();
        let result = rsi(&candles, 0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_rsi_extreme_overbought() {
        // All candles strictly increasing => RSI near 100
        let candles: Vec<_> = (0..30)
            .map(|i| candle_close(&format!("{}", 100 + i * 10)))
            .collect();
        let result = rsi(&candles, 14);
        assert!(!result.is_empty());
        for val in &result {
            let v = val.to_f64().unwrap();
            assert!(
                v > 95.0,
                "RSI should be near 100 for extreme uptrend, got {v}"
            );
        }
    }

    #[test]
    fn test_rsi_extreme_oversold() {
        // All candles strictly decreasing => RSI near 0
        let candles: Vec<_> = (0..30)
            .map(|i| candle_close(&format!("{}", 500 - i * 10)))
            .collect();
        let result = rsi(&candles, 14);
        assert!(!result.is_empty());
        for val in &result {
            let v = val.to_f64().unwrap();
            assert!(
                v < 5.0,
                "RSI should be near 0 for extreme downtrend, got {v}"
            );
        }
    }

    #[test]
    fn test_rsi_equal_gains_losses() {
        // Alternating up and down by same amount => RSI should be near 50
        let mut candles = Vec::new();
        for i in 0..30 {
            let price = if i % 2 == 0 { 100 } else { 110 };
            candles.push(candle_close(&format!("{}", price)));
        }
        let result = rsi(&candles, 14);
        assert!(!result.is_empty());
        let last = result.last().unwrap().to_f64().unwrap();
        assert!(
            (last - 50.0).abs() < 5.0,
            "RSI should be near 50 for equal gains/losses, got {last}"
        );
    }

    #[test]
    fn test_rsi_period_larger_than_data() {
        let candles: Vec<_> = (0..10)
            .map(|i| candle_close(&format!("{}", 100 + i)))
            .collect();
        let result = rsi(&candles, 20);
        assert!(result.is_empty());
    }

    #[test]
    fn test_rsi_period_equals_data_len() {
        // period == candles.len() => candles.len() <= period => empty
        let candles: Vec<_> = (0..14)
            .map(|i| candle_close(&format!("{}", 100 + i)))
            .collect();
        let result = rsi(&candles, 14);
        assert!(result.is_empty());
    }

    #[test]
    fn test_rsi_small_period() {
        let candles: Vec<_> = (0..10)
            .map(|i| candle_close(&format!("{}", 100 + i * 2)))
            .collect();
        let result = rsi(&candles, 2);
        assert!(!result.is_empty());
        // All up => RSI high
        for val in &result {
            let v = val.to_f64().unwrap();
            assert!(
                v > 80.0,
                "RSI with small period in uptrend should be high, got {v}"
            );
        }
    }
}
