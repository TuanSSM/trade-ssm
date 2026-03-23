use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use ssm_core::Candle;

use crate::ema::sma;

/// Bollinger Bands result.
#[derive(Debug, Clone)]
pub struct BollingerBands {
    pub middle: Vec<Decimal>,
    pub upper: Vec<Decimal>,
    pub lower: Vec<Decimal>,
    /// %B = (price - lower) / (upper - lower). Ranges 0-1 within bands.
    pub pct_b: Vec<Decimal>,
    /// Bandwidth = (upper - lower) / middle.
    pub bandwidth: Vec<Decimal>,
}

/// Compute Bollinger Bands.
///
/// Default parameters: period=20, std_dev_multiplier=2.0
pub fn bollinger_bands(candles: &[Candle], period: usize, std_dev_mult: Decimal) -> BollingerBands {
    let middle = sma(candles, period);
    if middle.is_empty() {
        return BollingerBands {
            middle: vec![],
            upper: vec![],
            lower: vec![],
            pct_b: vec![],
            bandwidth: vec![],
        };
    }

    let mut upper = Vec::with_capacity(middle.len());
    let mut lower = Vec::with_capacity(middle.len());
    let mut pct_b = Vec::with_capacity(middle.len());
    let mut bandwidth = Vec::with_capacity(middle.len());

    for (i, &mid) in middle.iter().enumerate() {
        let start = i; // middle[i] corresponds to candles[i..i+period]
        let slice = &candles[start..start + period];

        // Standard deviation
        let mean_f64 = mid.to_f64().unwrap_or(0.0);
        let variance: f64 = slice
            .iter()
            .map(|c| {
                let diff = c.close.to_f64().unwrap_or(0.0) - mean_f64;
                diff * diff
            })
            .sum::<f64>()
            / period as f64;
        let std_dev = Decimal::try_from(variance.sqrt()).unwrap_or(Decimal::ZERO);

        let band_width = std_dev * std_dev_mult;
        let u = mid + band_width;
        let l = mid - band_width;

        let close = candles[start + period - 1].close;
        let b = if u != l {
            (close - l) / (u - l)
        } else {
            Decimal::new(5, 1) // 0.5 if bands are zero width
        };

        let bw = if mid > Decimal::ZERO {
            (u - l) / mid
        } else {
            Decimal::ZERO
        };

        upper.push(u);
        lower.push(l);
        pct_b.push(b);
        bandwidth.push(bw);
    }

    BollingerBands {
        middle,
        upper,
        lower,
        pct_b,
        bandwidth,
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
    fn bands_contain_price() {
        let candles: Vec<_> = (0..30)
            .map(|i| candle_close(&format!("{}", 100 + (i % 10))))
            .collect();
        let bb = bollinger_bands(&candles, 20, Decimal::from(2));
        assert!(!bb.upper.is_empty());

        for i in 0..bb.middle.len() {
            assert!(bb.upper[i] >= bb.middle[i]);
            assert!(bb.lower[i] <= bb.middle[i]);
        }
    }

    #[test]
    fn constant_price_zero_bandwidth() {
        let candles: Vec<_> = (0..25).map(|_| candle_close("100")).collect();
        let bb = bollinger_bands(&candles, 20, Decimal::from(2));
        assert!(!bb.bandwidth.is_empty());
        // Constant price → zero std dev → zero bandwidth
        for bw in &bb.bandwidth {
            assert_eq!(*bw, Decimal::ZERO);
        }
    }

    #[test]
    fn pct_b_within_bands() {
        let candles: Vec<_> = (0..25).map(|_| candle_close("100")).collect();
        let bb = bollinger_bands(&candles, 20, Decimal::from(2));
        for b in &bb.pct_b {
            let v = b.to_f64().unwrap();
            assert!((-0.5..=1.5).contains(&v), "%B out of expected range: {v}");
        }
    }

    #[test]
    fn no_repainting_bollinger() {
        let short: Vec<_> = (0..25)
            .map(|i| candle_close(&format!("{}", 100 + i % 5)))
            .collect();
        let mut long = short.clone();
        long.push(candle_close("110"));

        let r_short = bollinger_bands(&short, 20, Decimal::from(2));
        let r_long = bollinger_bands(&long, 20, Decimal::from(2));

        for i in 0..r_short.middle.len() {
            assert_eq!(r_short.middle[i], r_long.middle[i], "BB repainting at {i}");
        }
    }

    #[test]
    fn test_bollinger_insufficient_data() {
        let candles: Vec<_> = (0..10).map(|_| candle_close("100")).collect();
        let bb = bollinger_bands(&candles, 20, Decimal::from(2));
        assert!(bb.middle.is_empty());
        assert!(bb.upper.is_empty());
        assert!(bb.lower.is_empty());
        assert!(bb.pct_b.is_empty());
        assert!(bb.bandwidth.is_empty());
    }

    #[test]
    fn test_bollinger_output_length() {
        let candles: Vec<_> = (0..30)
            .map(|i| candle_close(&format!("{}", 100 + i)))
            .collect();
        let bb = bollinger_bands(&candles, 20, Decimal::from(2));
        assert_eq!(bb.upper.len(), bb.middle.len());
        assert_eq!(bb.lower.len(), bb.middle.len());
        assert_eq!(bb.pct_b.len(), bb.middle.len());
        assert_eq!(bb.bandwidth.len(), bb.middle.len());
    }

    #[test]
    fn test_bollinger_upper_above_lower() {
        let candles: Vec<_> = (0..30)
            .map(|i| candle_close(&format!("{}", 100 + (i % 10))))
            .collect();
        let bb = bollinger_bands(&candles, 20, Decimal::from(2));
        assert!(!bb.upper.is_empty());
        for i in 0..bb.upper.len() {
            assert!(
                bb.upper[i] >= bb.lower[i],
                "Upper band {} should be >= lower band {} at index {i}",
                bb.upper[i],
                bb.lower[i]
            );
        }
    }

    #[test]
    fn test_bollinger_very_small_period() {
        // Period = 2 (minimum meaningful period for std dev)
        let candles = vec![
            candle_close("100"),
            candle_close("110"),
            candle_close("105"),
            candle_close("115"),
        ];
        let bb = bollinger_bands(&candles, 2, Decimal::from(2));
        assert!(!bb.middle.is_empty());
        assert_eq!(bb.middle.len(), 3); // 4 - 2 + 1 = 3
                                        // First middle = (100+110)/2 = 105
        assert_eq!(bb.middle[0], Decimal::from(105));
        // Bands should exist and be ordered
        for i in 0..bb.upper.len() {
            assert!(bb.upper[i] >= bb.middle[i]);
            assert!(bb.lower[i] <= bb.middle[i]);
        }
    }

    #[test]
    fn test_bollinger_bandwidth_precision() {
        // Known data: period=3, prices [100, 110, 120]
        // SMA = 110, std_dev = sqrt(((100-110)^2 + (110-110)^2 + (120-110)^2)/3) = sqrt(200/3)
        // bandwidth = (upper - lower) / middle = 2 * 2 * std_dev / middle
        let candles = vec![
            candle_close("100"),
            candle_close("110"),
            candle_close("120"),
        ];
        let bb = bollinger_bands(&candles, 3, Decimal::from(2));
        assert_eq!(bb.middle.len(), 1);
        assert_eq!(bb.middle[0], Decimal::from(110));
        // bandwidth = (upper - lower) / middle
        let bw = bb.bandwidth[0];
        // upper - lower = 4 * std_dev, std_dev = sqrt(200/3) ~ 8.165
        // bandwidth ~ 4 * 8.165 / 110 ~ 0.2969
        let bw_f64 = bw.to_f64().unwrap();
        assert!(
            (bw_f64 - 0.2969).abs() < 0.01,
            "Bandwidth should be ~0.297, got {bw_f64}"
        );
    }

    #[test]
    fn test_bollinger_period_1() {
        // Period=1: SMA is just the close price, std_dev = 0
        let candles = vec![
            candle_close("100"),
            candle_close("110"),
            candle_close("120"),
        ];
        let bb = bollinger_bands(&candles, 1, Decimal::from(2));
        assert_eq!(bb.middle.len(), 3);
        // Each middle should equal the close
        assert_eq!(bb.middle[0], Decimal::from(100));
        assert_eq!(bb.middle[1], Decimal::from(110));
        assert_eq!(bb.middle[2], Decimal::from(120));
        // std_dev = 0 => upper == lower == middle
        for i in 0..bb.middle.len() {
            assert_eq!(bb.upper[i], bb.middle[i]);
            assert_eq!(bb.lower[i], bb.middle[i]);
            assert_eq!(bb.bandwidth[i], Decimal::ZERO);
        }
    }

    #[test]
    fn test_bollinger_high_volatility_wider_bands() {
        // High volatility data should produce wider bandwidth than low volatility
        let low_vol_candles: Vec<_> = (0..10).map(|_| candle_close("100")).collect();
        let high_vol_candles: Vec<_> = (0..10)
            .map(|i| {
                if i % 2 == 0 {
                    candle_close("80")
                } else {
                    candle_close("120")
                }
            })
            .collect();
        let bb_low = bollinger_bands(&low_vol_candles, 5, Decimal::from(2));
        let bb_high = bollinger_bands(&high_vol_candles, 5, Decimal::from(2));
        assert!(!bb_low.bandwidth.is_empty());
        assert!(!bb_high.bandwidth.is_empty());
        let low_bw = *bb_low.bandwidth.last().unwrap();
        let high_bw = *bb_high.bandwidth.last().unwrap();
        assert!(
            high_bw > low_bw,
            "High volatility bandwidth {high_bw} should be > low volatility {low_bw}"
        );
    }
}
