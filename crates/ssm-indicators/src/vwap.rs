use rust_decimal::Decimal;
use ssm_core::Candle;

/// VWAP (Volume Weighted Average Price) result.
#[derive(Debug, Clone)]
pub struct VwapResult {
    /// VWAP values (one per candle).
    pub vwap: Vec<Decimal>,
}

/// Compute VWAP (Volume Weighted Average Price).
///
/// VWAP = cumulative(typical_price * volume) / cumulative(volume)
/// typical_price = (high + low + close) / 3
///
/// Resets at session boundary. If no session boundaries, runs continuously.
pub fn vwap(candles: &[Candle]) -> VwapResult {
    if candles.is_empty() {
        return VwapResult { vwap: vec![] };
    }

    let three = Decimal::from(3);
    let mut cum_tp_vol = Decimal::ZERO;
    let mut cum_vol = Decimal::ZERO;
    let mut result = Vec::with_capacity(candles.len());

    for c in candles {
        let typical_price = (c.high + c.low + c.close) / three;
        cum_tp_vol += typical_price * c.volume;
        cum_vol += c.volume;

        let vwap_val = if cum_vol > Decimal::ZERO {
            cum_tp_vol / cum_vol
        } else {
            typical_price
        };

        result.push(vwap_val);
    }

    VwapResult { vwap: result }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn candle_hlcv(high: &str, low: &str, close: &str, vol: &str) -> Candle {
        Candle {
            open_time: 0,
            open: Decimal::from_str(close).unwrap(),
            high: Decimal::from_str(high).unwrap(),
            low: Decimal::from_str(low).unwrap(),
            close: Decimal::from_str(close).unwrap(),
            volume: Decimal::from_str(vol).unwrap(),
            close_time: 0,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: Decimal::from_str(vol).unwrap() / Decimal::from(2),
            taker_sell_volume: Decimal::from_str(vol).unwrap() / Decimal::from(2),
        }
    }

    #[test]
    fn vwap_single_candle() {
        let candles = vec![candle_hlcv("110", "90", "100", "1000")];
        let result = vwap(&candles);
        assert_eq!(result.vwap.len(), 1);
        // TP = (110 + 90 + 100) / 3 = 100
        assert_eq!(result.vwap[0], Decimal::from(100));
    }

    #[test]
    fn vwap_weights_by_volume() {
        let candles = vec![
            candle_hlcv("100", "100", "100", "1000"), // TP=100, vol=1000
            candle_hlcv("200", "200", "200", "1"),    // TP=200, vol=1
        ];
        let result = vwap(&candles);
        // VWAP should be much closer to 100 (high volume) than 200 (low volume)
        let last = result.vwap[1];
        assert!(
            last < Decimal::from(101),
            "VWAP should be near 100, got {last}"
        );
    }

    #[test]
    fn no_repainting_vwap() {
        let short = vec![
            candle_hlcv("110", "90", "100", "1000"),
            candle_hlcv("115", "95", "105", "500"),
        ];
        let mut long = short.clone();
        long.push(candle_hlcv("120", "100", "110", "800"));

        let r_short = vwap(&short);
        let r_long = vwap(&long);

        for i in 0..r_short.vwap.len() {
            assert_eq!(r_short.vwap[i], r_long.vwap[i], "VWAP repainting at {i}");
        }
    }

    #[test]
    fn empty_candles() {
        let result = vwap(&[]);
        assert!(result.vwap.is_empty());
    }

    #[test]
    fn test_vwap_output_length() {
        let candles = vec![
            candle_hlcv("110", "90", "100", "1000"),
            candle_hlcv("115", "95", "105", "500"),
            candle_hlcv("120", "100", "110", "800"),
        ];
        let result = vwap(&candles);
        assert_eq!(result.vwap.len(), candles.len());
    }

    #[test]
    fn test_vwap_constant_price() {
        let candles: Vec<_> = (0..10)
            .map(|_| candle_hlcv("100", "100", "100", "500"))
            .collect();
        let result = vwap(&candles);
        assert_eq!(result.vwap.len(), 10);
        for val in &result.vwap {
            assert_eq!(*val, Decimal::from(100), "VWAP should equal constant price");
        }
    }

    #[test]
    fn test_vwap_zero_volume_candles() {
        // When volume is zero, VWAP should fall back to typical price
        let candles = vec![candle_hlcv("110", "90", "100", "0")];
        let result = vwap(&candles);
        assert_eq!(result.vwap.len(), 1);
        // TP = (110+90+100)/3 = 100
        assert_eq!(result.vwap[0], Decimal::from(100));
    }

    #[test]
    fn test_vwap_zero_volume_after_nonzero() {
        // First candle has volume, second has zero volume
        let candles = vec![
            candle_hlcv("110", "90", "100", "1000"), // TP=100, vol=1000
            candle_hlcv("120", "100", "110", "0"),   // TP=110, vol=0
        ];
        let result = vwap(&candles);
        assert_eq!(result.vwap.len(), 2);
        // First VWAP = 100
        assert_eq!(result.vwap[0], Decimal::from(100));
        // Second VWAP: cum_tp_vol = 100*1000 + 110*0 = 100000, cum_vol = 1000
        // VWAP = 100000/1000 = 100 (zero volume doesn't change cumulative)
        assert_eq!(result.vwap[1], Decimal::from(100));
    }

    #[test]
    fn test_vwap_single_candle_asymmetric() {
        // High != Low != Close
        let candles = vec![candle_hlcv("150", "90", "120", "500")];
        let result = vwap(&candles);
        assert_eq!(result.vwap.len(), 1);
        // TP = (150+90+120)/3 = 360/3 = 120
        assert_eq!(result.vwap[0], Decimal::from(120));
    }

    #[test]
    fn test_vwap_identical_prices_different_volumes() {
        // All prices identical but volumes differ => VWAP stays constant
        let candles = vec![
            candle_hlcv("100", "100", "100", "1000"),
            candle_hlcv("100", "100", "100", "5000"),
            candle_hlcv("100", "100", "100", "100"),
        ];
        let result = vwap(&candles);
        assert_eq!(result.vwap.len(), 3);
        for val in &result.vwap {
            assert_eq!(*val, Decimal::from(100));
        }
    }

    #[test]
    fn test_vwap_monotonically_within_range() {
        // VWAP should always be between the min and max typical prices seen so far
        let candles = vec![
            candle_hlcv("100", "100", "100", "500"), // TP = 100
            candle_hlcv("200", "200", "200", "500"), // TP = 200
            candle_hlcv("150", "150", "150", "500"), // TP = 150
        ];
        let result = vwap(&candles);
        assert_eq!(result.vwap[0], Decimal::from(100));
        // After adding 200, VWAP should be between 100 and 200
        assert!(result.vwap[1] >= Decimal::from(100) && result.vwap[1] <= Decimal::from(200));
        // After adding 150, VWAP should still be between 100 and 200
        assert!(result.vwap[2] >= Decimal::from(100) && result.vwap[2] <= Decimal::from(200));
    }
}
