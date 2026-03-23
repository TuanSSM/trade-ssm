use rust_decimal::Decimal;
use ssm_core::Candle;

/// Compute On-Balance Volume (OBV).
///
/// OBV accumulates volume: +volume when close > prev_close, -volume when close < prev_close.
/// Leading indicator — OBV divergence from price can signal reversals.
pub fn obv(candles: &[Candle]) -> Vec<Decimal> {
    if candles.is_empty() {
        return vec![];
    }

    let mut result = Vec::with_capacity(candles.len());
    result.push(Decimal::ZERO); // OBV starts at 0

    let mut running = Decimal::ZERO;
    for i in 1..candles.len() {
        if candles[i].close > candles[i - 1].close {
            running += candles[i].volume;
        } else if candles[i].close < candles[i - 1].close {
            running -= candles[i].volume;
        }
        // If close == prev_close, OBV unchanged
        result.push(running);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn candle_cv(close: &str, vol: &str) -> Candle {
        let c = Decimal::from_str(close).unwrap();
        let v = Decimal::from_str(vol).unwrap();
        Candle {
            open_time: 0,
            open: c,
            high: c,
            low: c,
            close: c,
            volume: v,
            close_time: 0,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: v / Decimal::from(2),
            taker_sell_volume: v / Decimal::from(2),
        }
    }

    #[test]
    fn obv_uptrend() {
        let candles = vec![
            candle_cv("100", "1000"),
            candle_cv("105", "1500"), // up → +1500
            candle_cv("110", "1200"), // up → +2700
        ];
        let result = obv(&candles);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], Decimal::ZERO);
        assert_eq!(result[1], Decimal::from(1500));
        assert_eq!(result[2], Decimal::from(2700));
    }

    #[test]
    fn obv_downtrend() {
        let candles = vec![
            candle_cv("110", "1000"),
            candle_cv("105", "1500"), // down → -1500
            candle_cv("100", "1200"), // down → -2700
        ];
        let result = obv(&candles);
        assert_eq!(result[2], Decimal::from(-2700));
    }

    #[test]
    fn obv_flat() {
        let candles = vec![
            candle_cv("100", "1000"),
            candle_cv("100", "500"), // flat → unchanged
        ];
        let result = obv(&candles);
        assert_eq!(result[1], Decimal::ZERO);
    }

    #[test]
    fn no_repainting_obv() {
        let short = vec![candle_cv("100", "1000"), candle_cv("105", "500")];
        let mut long = short.clone();
        long.push(candle_cv("110", "800"));

        let r_short = obv(&short);
        let r_long = obv(&long);

        for i in 0..r_short.len() {
            assert_eq!(r_short[i], r_long[i], "OBV repainting at {i}");
        }
    }

    #[test]
    fn test_obv_empty() {
        let result = obv(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_obv_single_candle() {
        let candles = vec![candle_cv("100", "1000")];
        let result = obv(&candles);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], Decimal::ZERO);
    }

    #[test]
    fn test_obv_mixed_trend() {
        let candles = vec![
            candle_cv("100", "1000"),
            candle_cv("110", "500"),  // up → +500
            candle_cv("105", "300"),  // down → +500 - 300 = +200
            candle_cv("115", "800"),  // up → +200 + 800 = +1000
        ];
        let result = obv(&candles);
        assert_eq!(result[0], Decimal::ZERO);
        assert_eq!(result[1], Decimal::from(500));
        assert_eq!(result[2], Decimal::from(200));
        assert_eq!(result[3], Decimal::from(1000));
    }

    #[test]
    fn test_obv_output_length() {
        let candles = vec![
            candle_cv("100", "1000"),
            candle_cv("105", "500"),
            candle_cv("110", "300"),
            candle_cv("108", "200"),
        ];
        let result = obv(&candles);
        assert_eq!(result.len(), candles.len());
    }

    #[test]
    fn test_obv_alternating_up_down() {
        let candles = vec![
            candle_cv("100", "1000"),
            candle_cv("110", "500"),  // up: +500
            candle_cv("100", "500"),  // down: +500 - 500 = 0
            candle_cv("110", "500"),  // up: 0 + 500 = +500
            candle_cv("100", "500"),  // down: +500 - 500 = 0
        ];
        let result = obv(&candles);
        assert_eq!(result[0], Decimal::ZERO);
        assert_eq!(result[1], Decimal::from(500));
        assert_eq!(result[2], Decimal::ZERO);
        assert_eq!(result[3], Decimal::from(500));
        assert_eq!(result[4], Decimal::ZERO);
    }

    #[test]
    fn test_obv_zero_volume() {
        let candles = vec![
            candle_cv("100", "0"),
            candle_cv("110", "0"),  // up but zero volume => +0
            candle_cv("105", "0"),  // down but zero volume => -0
        ];
        let result = obv(&candles);
        assert_eq!(result.len(), 3);
        for val in &result {
            assert_eq!(*val, Decimal::ZERO, "OBV should be 0 with zero volume");
        }
    }

    #[test]
    fn test_obv_alternating_with_different_volumes() {
        let candles = vec![
            candle_cv("100", "1000"),
            candle_cv("110", "2000"),  // up: +2000
            candle_cv("105", "500"),   // down: +2000 - 500 = +1500
            candle_cv("108", "1000"),  // up: +1500 + 1000 = +2500
            candle_cv("107", "300"),   // down: +2500 - 300 = +2200
        ];
        let result = obv(&candles);
        assert_eq!(result[0], Decimal::ZERO);
        assert_eq!(result[1], Decimal::from(2000));
        assert_eq!(result[2], Decimal::from(1500));
        assert_eq!(result[3], Decimal::from(2500));
        assert_eq!(result[4], Decimal::from(2200));
    }

    #[test]
    fn test_obv_multiple_flat_candles() {
        // All same close => OBV stays at 0
        let candles = vec![
            candle_cv("100", "1000"),
            candle_cv("100", "2000"),
            candle_cv("100", "500"),
            candle_cv("100", "800"),
        ];
        let result = obv(&candles);
        for val in &result {
            assert_eq!(*val, Decimal::ZERO, "OBV should be 0 for flat prices");
        }
    }
}
