use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ssm_core::Candle;

/// Simplified per-candle volume delta: taker_buy - taker_sell.
///
/// Lightweight alternative to the full ssm-orderflow delta analysis.
/// Anti-repainting: caller must pass only closed candles.
pub fn simplified_volume_delta(candles: &[Candle]) -> Vec<Decimal> {
    candles
        .iter()
        .map(|c| c.taker_buy_volume - c.taker_sell_volume)
        .collect()
}

/// Result of volume spike detection for a single candle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpikeDirection {
    None,
    Bullish,
    Bearish,
}

/// Detect a volume spike at the final candle:
///   * `volume > volume_sma(period) * volume_mult`
///   * `|delta| / volume > imbalance_frac`
///
/// Returns `SpikeDirection::Bullish` when delta is positive, `Bearish` when negative.
/// `None` otherwise (or when the window is insufficient).
pub fn detect_volume_spike(
    candles: &[Candle],
    period: usize,
    volume_mult: Decimal,
    imbalance_frac: Decimal,
) -> SpikeDirection {
    if candles.len() < period || period == 0 {
        return SpikeDirection::None;
    }

    // Volume SMA (closes-based `sma` only uses close; roll our own on volume).
    let start = candles.len() - period;
    let sum: Decimal = candles[start..].iter().map(|c| c.volume).sum();
    let avg = sum / Decimal::from(period as u64);
    let last = &candles[candles.len() - 1];

    if last.volume <= avg * volume_mult {
        return SpikeDirection::None;
    }
    if last.volume.is_zero() {
        return SpikeDirection::None;
    }

    let delta = last.taker_buy_volume - last.taker_sell_volume;
    let delta_frac = delta.abs() / last.volume;
    if delta_frac < imbalance_frac {
        return SpikeDirection::None;
    }

    if delta > Decimal::ZERO {
        SpikeDirection::Bullish
    } else if delta < Decimal::ZERO {
        SpikeDirection::Bearish
    } else {
        SpikeDirection::None
    }
}

/// Compute a rolling volume SMA (sibling to `ema::sma` but on candle `volume`).
pub fn volume_sma(candles: &[Candle], period: usize) -> Vec<Decimal> {
    if candles.len() < period || period == 0 {
        return vec![];
    }
    let mut result = Vec::with_capacity(candles.len() - period + 1);
    let mut sum: Decimal = candles[..period].iter().map(|c| c.volume).sum();
    result.push(sum / Decimal::from(period as u64));
    for i in period..candles.len() {
        sum += candles[i].volume - candles[i - period].volume;
        result.push(sum / Decimal::from(period as u64));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn candle(volume: &str, buy: &str, sell: &str) -> Candle {
        let v = Decimal::from_str(volume).unwrap();
        Candle {
            open_time: 0,
            open: Decimal::ZERO,
            high: Decimal::ZERO,
            low: Decimal::ZERO,
            close: Decimal::ZERO,
            volume: v,
            close_time: 0,
            quote_volume: Decimal::ZERO,
            trades: 0,
            taker_buy_volume: Decimal::from_str(buy).unwrap(),
            taker_sell_volume: Decimal::from_str(sell).unwrap(),
        }
    }

    #[test]
    fn delta_basic() {
        let candles = vec![candle("100", "70", "30"), candle("100", "20", "80")];
        let d = simplified_volume_delta(&candles);
        assert_eq!(d, vec![Decimal::from(40), Decimal::from(-60)]);
    }

    #[test]
    fn delta_no_repainting() {
        let short = vec![candle("100", "70", "30"), candle("100", "20", "80")];
        let mut long = short.clone();
        long.push(candle("100", "50", "50"));
        let r_short = simplified_volume_delta(&short);
        let r_long = simplified_volume_delta(&long);
        for i in 0..r_short.len() {
            assert_eq!(r_short[i], r_long[i]);
        }
    }

    #[test]
    fn spike_bullish() {
        let mut candles: Vec<_> = (0..19).map(|_| candle("100", "50", "50")).collect();
        candles.push(candle("500", "400", "100")); // 5x volume, delta=+300, frac=0.6
        let dir = detect_volume_spike(
            &candles,
            20,
            Decimal::from(2),
            Decimal::new(5, 1), // 0.5
        );
        assert_eq!(dir, SpikeDirection::Bullish);
    }

    #[test]
    fn spike_bearish() {
        let mut candles: Vec<_> = (0..19).map(|_| candle("100", "50", "50")).collect();
        candles.push(candle("500", "100", "400"));
        let dir = detect_volume_spike(&candles, 20, Decimal::from(2), Decimal::new(5, 1));
        assert_eq!(dir, SpikeDirection::Bearish);
    }

    #[test]
    fn spike_rejected_when_volume_normal() {
        let candles: Vec<_> = (0..20).map(|_| candle("100", "80", "20")).collect();
        let dir = detect_volume_spike(&candles, 20, Decimal::from(2), Decimal::new(5, 1));
        assert_eq!(dir, SpikeDirection::None);
    }

    #[test]
    fn spike_rejected_when_balanced_delta() {
        let mut candles: Vec<_> = (0..19).map(|_| candle("100", "50", "50")).collect();
        candles.push(candle("500", "260", "240")); // 5x volume, delta=+20, frac=0.04
        let dir = detect_volume_spike(&candles, 20, Decimal::from(2), Decimal::new(5, 1));
        assert_eq!(dir, SpikeDirection::None);
    }

    #[test]
    fn volume_sma_basic() {
        let candles = vec![
            candle("100", "50", "50"),
            candle("200", "100", "100"),
            candle("300", "150", "150"),
        ];
        let vs = volume_sma(&candles, 2);
        assert_eq!(vs.len(), 2);
        assert_eq!(vs[0], Decimal::from(150));
        assert_eq!(vs[1], Decimal::from(250));
    }

    #[test]
    fn volume_sma_no_repainting() {
        let short: Vec<_> = (0..10)
            .map(|i| candle(&format!("{}", 100 + i * 10), "0", "0"))
            .collect();
        let mut long = short.clone();
        long.push(candle("999", "0", "0"));
        let r_short = volume_sma(&short, 5);
        let r_long = volume_sma(&long, 5);
        for i in 0..r_short.len() {
            assert_eq!(r_short[i], r_long[i]);
        }
    }
}
