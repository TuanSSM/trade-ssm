use rust_decimal::Decimal;

use crate::exchange::types::Candle;

/// CVD analysis result for a window of candles
#[derive(Debug, Clone)]
pub struct CvdAnalysis {
    /// Per-candle delta values (buy_vol - sell_vol)
    pub deltas: Vec<Decimal>,
    /// Cumulative sum over the window
    pub cumulative: Vec<Decimal>,
    /// Total CVD for the window
    pub total_cvd: Decimal,
    /// Trend classification
    pub trend: CvdTrend,
    /// Number of candles analyzed
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
        match self {
            Self::Bullish => write!(f, "BULLISH"),
            Self::Bearish => write!(f, "BEARISH"),
            Self::Neutral => write!(f, "NEUTRAL"),
        }
    }
}

/// Calculate CVD over the last `window` closed candles.
///
/// Anti-repainting: this function excludes the last candle if it's still forming.
/// Pass only closed candles to this function.
pub fn analyze_cvd(candles: &[Candle], window: usize) -> CvdAnalysis {
    // Take the last `window` candles (these should all be closed)
    let start = candles.len().saturating_sub(window);
    let window_candles = &candles[start..];

    let mut deltas = Vec::with_capacity(window_candles.len());
    let mut cumulative = Vec::with_capacity(window_candles.len());
    let mut running = Decimal::ZERO;

    for candle in window_candles {
        let delta = candle.taker_buy_volume - candle.taker_sell_volume;
        running += delta;
        deltas.push(delta);
        cumulative.push(running);
    }

    let total_cvd = running;

    // Determine trend: compare first half CVD to second half
    let trend = if window_candles.len() < 2 {
        CvdTrend::Neutral
    } else {
        let mid = cumulative.len() / 2;
        let first_half_end = cumulative[mid];
        let second_half_change = total_cvd - first_half_end;

        // If second half continues in same direction and total is significant
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
        window_size: window_candles.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn make_candle(buy_vol: &str, sell_vol: &str) -> Candle {
        Candle {
            open_time: 0,
            open: Decimal::ZERO,
            high: Decimal::ZERO,
            low: Decimal::ZERO,
            close: Decimal::ZERO,
            volume: Decimal::from_str(buy_vol).unwrap() + Decimal::from_str(sell_vol).unwrap(),
            close_time: 0,
            quote_volume: Decimal::ZERO,
            trades: 0,
            taker_buy_volume: Decimal::from_str(buy_vol).unwrap(),
            taker_sell_volume: Decimal::from_str(sell_vol).unwrap(),
        }
    }

    #[test]
    fn test_bullish_cvd() {
        // Consistently more buying than selling
        let candles: Vec<Candle> = (0..15)
            .map(|_| make_candle("60", "40"))
            .collect();

        let analysis = analyze_cvd(&candles, 15);
        assert_eq!(analysis.trend, CvdTrend::Bullish);
        assert!(analysis.total_cvd > Decimal::ZERO);
        assert_eq!(analysis.window_size, 15);
    }

    #[test]
    fn test_bearish_cvd() {
        let candles: Vec<Candle> = (0..15)
            .map(|_| make_candle("30", "70"))
            .collect();

        let analysis = analyze_cvd(&candles, 15);
        assert_eq!(analysis.trend, CvdTrend::Bearish);
        assert!(analysis.total_cvd < Decimal::ZERO);
    }

    #[test]
    fn test_window_smaller_than_candles() {
        let candles: Vec<Candle> = (0..30)
            .map(|_| make_candle("55", "45"))
            .collect();

        let analysis = analyze_cvd(&candles, 15);
        assert_eq!(analysis.window_size, 15);
        assert_eq!(analysis.deltas.len(), 15);
    }
}
