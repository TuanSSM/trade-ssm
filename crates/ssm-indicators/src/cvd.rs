use rust_decimal::Decimal;
use ssm_core::Candle;

/// CVD analysis result for a window of closed candles.
#[derive(Debug, Clone)]
pub struct CvdAnalysis {
    pub deltas: Vec<Decimal>,
    pub cumulative: Vec<Decimal>,
    pub total_cvd: Decimal,
    pub trend: CvdTrend,
    pub window_size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CvdTrend {
    Bullish,
    Bearish,
    Neutral,
}

impl std::fmt::Display for CvdTrend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Bullish => "BULLISH",
            Self::Bearish => "BEARISH",
            Self::Neutral => "NEUTRAL",
        })
    }
}

/// Calculate CVD over the last `window` closed candles.
///
/// Anti-repainting: caller must pass only closed candles.
pub fn analyze_cvd(candles: &[Candle], window: usize) -> CvdAnalysis {
    let start = candles.len().saturating_sub(window);
    let slice = &candles[start..];

    let mut deltas = Vec::with_capacity(slice.len());
    let mut cumulative = Vec::with_capacity(slice.len());
    let mut running = Decimal::ZERO;

    for c in slice {
        let delta = c.taker_buy_volume - c.taker_sell_volume;
        running += delta;
        deltas.push(delta);
        cumulative.push(running);
    }

    let total_cvd = running;
    let trend = if slice.len() < 2 {
        CvdTrend::Neutral
    } else {
        let mid = cumulative.len() / 2;
        let second_half_change = total_cvd - cumulative[mid];
        if total_cvd > Decimal::ZERO && second_half_change > Decimal::ZERO {
            CvdTrend::Bullish
        } else if total_cvd < Decimal::ZERO && second_half_change < Decimal::ZERO {
            CvdTrend::Bearish
        } else {
            CvdTrend::Neutral
        }
    };

    CvdAnalysis {
        deltas,
        cumulative,
        total_cvd,
        trend,
        window_size: slice.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn candle(buy: &str, sell: &str) -> Candle {
        let bv = Decimal::from_str(buy).unwrap();
        let sv = Decimal::from_str(sell).unwrap();
        Candle {
            open_time: 0,
            open: Decimal::ZERO,
            high: Decimal::ZERO,
            low: Decimal::ZERO,
            close: Decimal::ZERO,
            volume: bv + sv,
            close_time: 0,
            quote_volume: Decimal::ZERO,
            trades: 0,
            taker_buy_volume: bv,
            taker_sell_volume: sv,
        }
    }

    #[test]
    fn bullish_cvd() {
        let candles: Vec<_> = (0..15).map(|_| candle("60", "40")).collect();
        let a = analyze_cvd(&candles, 15);
        assert_eq!(a.trend, CvdTrend::Bullish);
        assert!(a.total_cvd > Decimal::ZERO);
        assert_eq!(a.window_size, 15);
    }

    #[test]
    fn bearish_cvd() {
        let candles: Vec<_> = (0..15).map(|_| candle("30", "70")).collect();
        let a = analyze_cvd(&candles, 15);
        assert_eq!(a.trend, CvdTrend::Bearish);
        assert!(a.total_cvd < Decimal::ZERO);
    }

    #[test]
    fn window_clips_to_available() {
        let candles: Vec<_> = (0..30).map(|_| candle("55", "45")).collect();
        let a = analyze_cvd(&candles, 15);
        assert_eq!(a.window_size, 15);
        assert_eq!(a.deltas.len(), 15);
    }

    #[test]
    fn no_repainting_cvd() {
        let short: Vec<_> = (0..10).map(|_| candle("55", "45")).collect();
        let mut long = short.clone();
        long.push(candle("60", "40"));

        let r_short = analyze_cvd(&short, 10);
        let r_long = analyze_cvd(&long, 11);

        // First 10 cumulative values must be identical
        for i in 0..r_short.cumulative.len() {
            assert_eq!(
                r_short.cumulative[i], r_long.cumulative[i],
                "CVD repainting at index {i}"
            );
        }
    }

    #[test]
    fn test_cvd_trend_display() {
        assert_eq!(CvdTrend::Bullish.to_string(), "BULLISH");
        assert_eq!(CvdTrend::Bearish.to_string(), "BEARISH");
        assert_eq!(CvdTrend::Neutral.to_string(), "NEUTRAL");
    }

    #[test]
    fn test_cvd_empty_candles() {
        let a = analyze_cvd(&[], 10);
        assert_eq!(a.total_cvd, Decimal::ZERO);
        assert_eq!(a.trend, CvdTrend::Neutral);
        assert_eq!(a.window_size, 0);
        assert!(a.deltas.is_empty());
        assert!(a.cumulative.is_empty());
    }

    #[test]
    fn test_cvd_single_candle() {
        let candles = vec![candle("60", "40")];
        let a = analyze_cvd(&candles, 1);
        assert_eq!(a.trend, CvdTrend::Neutral); // less than 2 candles
        assert_eq!(a.window_size, 1);
        assert_eq!(a.total_cvd, Decimal::from(20));
    }

    #[test]
    fn test_cvd_equal_buy_sell() {
        let candles: Vec<_> = (0..10).map(|_| candle("50", "50")).collect();
        let a = analyze_cvd(&candles, 10);
        assert_eq!(a.trend, CvdTrend::Neutral);
        assert_eq!(a.total_cvd, Decimal::ZERO);
        for d in &a.deltas {
            assert_eq!(*d, Decimal::ZERO);
        }
    }

    #[test]
    fn test_cvd_deltas_correct() {
        let candles = vec![
            candle("60", "40"),  // delta = 20
            candle("30", "70"),  // delta = -40
            candle("55", "45"),  // delta = 10
        ];
        let a = analyze_cvd(&candles, 3);
        assert_eq!(a.deltas[0], Decimal::from(20));
        assert_eq!(a.deltas[1], Decimal::from(-40));
        assert_eq!(a.deltas[2], Decimal::from(10));
    }

    #[test]
    fn test_cvd_cumulative_running_sum() {
        let candles = vec![
            candle("60", "40"),  // delta = 20,  cum = 20
            candle("30", "70"),  // delta = -40, cum = -20
            candle("55", "45"),  // delta = 10,  cum = -10
        ];
        let a = analyze_cvd(&candles, 3);
        assert_eq!(a.cumulative[0], Decimal::from(20));
        assert_eq!(a.cumulative[1], Decimal::from(-20));
        assert_eq!(a.cumulative[2], Decimal::from(-10));
    }

    #[test]
    fn test_cvd_window_larger_than_data() {
        let candles: Vec<_> = (0..5).map(|_| candle("60", "40")).collect();
        let a = analyze_cvd(&candles, 100);
        assert_eq!(a.window_size, 5);
        assert_eq!(a.deltas.len(), 5);
    }
}
