use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ssm_core::Candle;

/// A sweep (stop-hunt / liquidity grab) event.
///
/// A sweep occurs when price rapidly spikes through a level then reverses,
/// "sweeping" stop-loss orders and grabbing liquidity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepEvent {
    /// Candle index where the sweep occurred.
    pub index: usize,
    pub sweep_type: SweepType,
    /// The wick size that exceeded the body (sweep portion).
    pub wick_size: Decimal,
    /// Body size of the candle.
    pub body_size: Decimal,
    /// Wick-to-body ratio (higher = more aggressive sweep).
    pub wick_ratio: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SweepType {
    /// Price spiked below a level and reversed up (long wick below = bullish sweep).
    BullishSweep,
    /// Price spiked above a level and reversed down (long wick above = bearish sweep).
    BearishSweep,
}

/// Configuration for sweep detection.
#[derive(Debug, Clone)]
pub struct SweepConfig {
    /// Minimum wick-to-body ratio for a sweep (e.g., 2.0 = wick must be 2x body).
    pub min_wick_ratio: Decimal,
    /// Minimum wick size as percentage of price.
    pub min_wick_pct: Decimal,
}

impl Default for SweepConfig {
    fn default() -> Self {
        Self {
            min_wick_ratio: Decimal::from(2),
            min_wick_pct: Decimal::new(1, 3), // 0.001 = 0.1%
        }
    }
}

/// Detect sweep events in candle data.
///
/// A sweep candle has a long wick (spike) relative to its body,
/// indicating a rapid move through a level followed by reversal.
pub fn detect_sweeps(candles: &[Candle], config: &SweepConfig) -> Vec<SweepEvent> {
    let mut events = Vec::new();

    for (i, c) in candles.iter().enumerate() {
        let body_top = c.open.max(c.close);
        let body_bottom = c.open.min(c.close);
        let body_size = body_top - body_bottom;

        let upper_wick = c.high - body_top;
        let lower_wick = body_bottom - c.low;

        // Avoid division by zero — use a small body floor
        let body_for_ratio = if body_size > Decimal::ZERO {
            body_size
        } else {
            Decimal::new(1, 10) // tiny value
        };

        let mid_price = (c.high + c.low) / Decimal::from(2);
        if mid_price.is_zero() {
            continue;
        }

        // Check upper wick (bearish sweep)
        let upper_wick_pct = upper_wick / mid_price;
        let upper_ratio = upper_wick / body_for_ratio;
        if upper_wick_pct >= config.min_wick_pct && upper_ratio >= config.min_wick_ratio {
            events.push(SweepEvent {
                index: i,
                sweep_type: SweepType::BearishSweep,
                wick_size: upper_wick,
                body_size,
                wick_ratio: upper_ratio,
            });
        }

        // Check lower wick (bullish sweep)
        let lower_wick_pct = lower_wick / mid_price;
        let lower_ratio = lower_wick / body_for_ratio;
        if lower_wick_pct >= config.min_wick_pct && lower_ratio >= config.min_wick_ratio {
            events.push(SweepEvent {
                index: i,
                sweep_type: SweepType::BullishSweep,
                wick_size: lower_wick,
                body_size,
                wick_ratio: lower_ratio,
            });
        }
    }

    events
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
    fn bullish_sweep_long_lower_wick() {
        // Candle: opened at 100, dropped to 90, recovered to close at 99
        // Lower wick = 9, body = 1, ratio = 9
        let candles = vec![candle_ohlc("100", "101", "90", "99")];
        let config = SweepConfig {
            min_wick_ratio: Decimal::from(2),
            min_wick_pct: Decimal::new(1, 3),
        };
        let events = detect_sweeps(&candles, &config);
        let bullish: Vec<_> = events
            .iter()
            .filter(|e| e.sweep_type == SweepType::BullishSweep)
            .collect();
        assert_eq!(bullish.len(), 1);
        assert_eq!(bullish[0].wick_size, Decimal::from(9));
    }

    #[test]
    fn bearish_sweep_long_upper_wick() {
        // Candle: opened at 100, spiked to 115, closed at 101
        // Upper wick = 14, body = 1, ratio = 14
        let candles = vec![candle_ohlc("100", "115", "99", "101")];
        let config = SweepConfig::default();
        let events = detect_sweeps(&candles, &config);
        let bearish: Vec<_> = events
            .iter()
            .filter(|e| e.sweep_type == SweepType::BearishSweep)
            .collect();
        assert_eq!(bearish.len(), 1);
    }

    #[test]
    fn no_sweep_normal_candle() {
        // Normal candle with balanced wicks
        let candles = vec![candle_ohlc("100", "105", "95", "103")];
        let config = SweepConfig {
            min_wick_ratio: Decimal::from(3),
            min_wick_pct: Decimal::new(5, 3), // 0.5%
        };
        let events = detect_sweeps(&candles, &config);
        assert!(events.is_empty());
    }
}
