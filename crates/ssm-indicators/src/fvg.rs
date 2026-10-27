use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ssm_core::Candle;

/// Fair Value Gap zone — a three-candle imbalance where price gapped past the
/// first candle's extreme without filling it on the second candle.
///
/// Bullish FVG: `candles[i].low > candles[i - 2].high`, gap below current price.
/// Bearish FVG: `candles[i].high < candles[i - 2].low`, gap above current price.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FvgZone {
    pub index: usize,
    pub fvg_type: FvgType,
    pub top: Decimal,
    pub bottom: Decimal,
    pub gap_size: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FvgType {
    Bullish,
    Bearish,
}

/// Detect fast FVG zones over closed candles, filtered by `min_gap_atr * atr`.
///
/// Vectorized fast path for the DCA strategy's level set.
/// Anti-repainting: caller must pass only closed candles.
pub fn fast_fvg(candles: &[Candle], atr: &[Decimal], min_gap_atr: Decimal) -> Vec<FvgZone> {
    if candles.len() < 3 {
        return Vec::new();
    }

    let mut zones = Vec::new();
    for i in 2..candles.len() {
        let prev2 = &candles[i - 2];
        let curr = &candles[i];

        let min_gap = atr_at(atr, candles.len(), i)
            .map(|a| a * min_gap_atr)
            .unwrap_or(Decimal::ZERO);

        // Bullish FVG: current low above the two-back high
        if curr.low > prev2.high {
            let gap = curr.low - prev2.high;
            if gap >= min_gap {
                zones.push(FvgZone {
                    index: i,
                    fvg_type: FvgType::Bullish,
                    top: curr.low,
                    bottom: prev2.high,
                    gap_size: gap,
                });
            }
        } else if curr.high < prev2.low {
            // Bearish FVG: current high below the two-back low
            let gap = prev2.low - curr.high;
            if gap >= min_gap {
                zones.push(FvgZone {
                    index: i,
                    fvg_type: FvgType::Bearish,
                    top: prev2.low,
                    bottom: curr.high,
                    gap_size: gap,
                });
            }
        }
    }
    zones
}

/// Map a candle index to the matching ATR entry.
///
/// ATR uses Wilder's smoothing: first ATR value corresponds to `candles[period]`,
/// so `atr.len()` lags `candles.len()` by `period`.
fn atr_at(atr: &[Decimal], candles_len: usize, candle_idx: usize) -> Option<Decimal> {
    if atr.is_empty() {
        return None;
    }
    let period = candles_len.saturating_sub(atr.len());
    if candle_idx < period {
        return None;
    }
    atr.get(candle_idx - period).copied()
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
    fn detects_bullish_gap() {
        // candle[0] high = 100, candle[1] ignored, candle[2] low = 110 → bullish FVG
        let candles = vec![
            candle_ohlc("95", "100", "90", "98"),
            candle_ohlc("100", "108", "99", "107"),
            candle_ohlc("110", "115", "110", "113"),
        ];
        let zones = fast_fvg(&candles, &[], Decimal::ZERO);
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].fvg_type, FvgType::Bullish);
        assert_eq!(zones[0].bottom, Decimal::from(100));
        assert_eq!(zones[0].top, Decimal::from(110));
    }

    #[test]
    fn detects_bearish_gap() {
        let candles = vec![
            candle_ohlc("110", "115", "108", "112"),
            candle_ohlc("108", "110", "100", "101"),
            candle_ohlc("100", "105", "95", "99"),
        ];
        let zones = fast_fvg(&candles, &[], Decimal::ZERO);
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].fvg_type, FvgType::Bearish);
        assert_eq!(zones[0].top, Decimal::from(108));
        assert_eq!(zones[0].bottom, Decimal::from(105));
    }

    #[test]
    fn no_gap_when_overlap() {
        let candles = vec![
            candle_ohlc("95", "105", "90", "100"),
            candle_ohlc("100", "108", "95", "105"),
            candle_ohlc("103", "110", "100", "108"),
        ];
        let zones = fast_fvg(&candles, &[], Decimal::ZERO);
        assert!(zones.is_empty());
    }

    #[test]
    fn filters_by_min_gap_atr() {
        let candles = vec![
            candle_ohlc("95", "100", "90", "98"),
            candle_ohlc("100", "108", "99", "107"),
            candle_ohlc("110", "115", "110", "113"),
        ];
        // Fake ATR values: one entry per candle (period = 0 lag for simplicity)
        let atr = vec![Decimal::from(5); 3];
        // Gap = 10, min_gap_atr = 1 → threshold = 5, passes
        let zones = fast_fvg(&candles, &atr, Decimal::from(1));
        assert_eq!(zones.len(), 1);
        // min_gap_atr = 3 → threshold = 15, rejects
        let zones = fast_fvg(&candles, &atr, Decimal::from(3));
        assert!(zones.is_empty());
    }

    #[test]
    fn empty_input() {
        assert!(fast_fvg(&[], &[], Decimal::ZERO).is_empty());
        let one = vec![candle_ohlc("100", "105", "95", "100")];
        assert!(fast_fvg(&one, &[], Decimal::ZERO).is_empty());
    }

    #[test]
    fn no_repainting_fvg() {
        let short = vec![
            candle_ohlc("95", "100", "90", "98"),
            candle_ohlc("100", "108", "99", "107"),
            candle_ohlc("110", "115", "110", "113"),
            candle_ohlc("112", "118", "111", "116"),
        ];
        let mut long = short.clone();
        long.push(candle_ohlc("115", "120", "113", "118"));

        let r_short = fast_fvg(&short, &[], Decimal::ZERO);
        let r_long = fast_fvg(&long, &[], Decimal::ZERO);
        for i in 0..r_short.len() {
            assert_eq!(r_short[i].index, r_long[i].index);
            assert_eq!(r_short[i].fvg_type, r_long[i].fvg_type);
            assert_eq!(r_short[i].top, r_long[i].top);
            assert_eq!(r_short[i].bottom, r_long[i].bottom);
        }
    }

    #[test]
    fn atr_at_handles_lag() {
        // candles.len() = 10, atr.len() = 7 → period = 3
        // candle_idx 2 → None, candle_idx 3 → atr[0]
        let atr = vec![Decimal::from(1); 7];
        assert_eq!(atr_at(&atr, 10, 2), None);
        assert_eq!(atr_at(&atr, 10, 3), Some(Decimal::from(1)));
        assert_eq!(atr_at(&atr, 10, 9), Some(Decimal::from(1)));
    }
}
