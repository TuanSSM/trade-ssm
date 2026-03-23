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

    #[test]
    fn empty_candles_no_sweeps() {
        let events = detect_sweeps(&[], &SweepConfig::default());
        assert!(events.is_empty());
    }

    #[test]
    fn default_config_values() {
        let config = SweepConfig::default();
        assert_eq!(config.min_wick_ratio, Decimal::from(2));
        assert_eq!(config.min_wick_pct, Decimal::new(1, 3));
    }

    #[test]
    fn doji_candle_with_large_wicks_detects_both_sweeps() {
        // Doji: open == close, but large wicks both directions
        // open=close=100, high=120, low=80 => body=0, upper_wick=20, lower_wick=20
        let candles = vec![candle_ohlc("100", "120", "80", "100")];
        let config = SweepConfig {
            min_wick_ratio: Decimal::from(2),
            min_wick_pct: Decimal::new(1, 3),
        };
        let events = detect_sweeps(&candles, &config);
        // Both bearish and bullish sweeps should be detected
        let bearish = events.iter().filter(|e| e.sweep_type == SweepType::BearishSweep).count();
        let bullish = events.iter().filter(|e| e.sweep_type == SweepType::BullishSweep).count();
        assert_eq!(bearish, 1);
        assert_eq!(bullish, 1);
    }

    #[test]
    fn wick_below_min_pct_threshold_not_detected() {
        // Tiny wick relative to price — below min_wick_pct
        // price ~1000, wick = 0.1 => wick_pct = 0.0001 < 0.001
        let candles = vec![candle_ohlc("1000", "1000.1", "999.95", "1000.05")];
        let config = SweepConfig {
            min_wick_ratio: Decimal::from(1),
            min_wick_pct: Decimal::new(1, 3), // 0.1%
        };
        let events = detect_sweeps(&candles, &config);
        assert!(events.is_empty());
    }

    #[test]
    fn single_candle_only_upper_sweep() {
        // Large upper wick, no lower wick
        // open=100, high=120, low=100, close=101 => upper_wick=19, lower_wick=0, body=1
        let candles = vec![candle_ohlc("100", "120", "100", "101")];
        let config = SweepConfig {
            min_wick_ratio: Decimal::from(2),
            min_wick_pct: Decimal::new(1, 3),
        };
        let events = detect_sweeps(&candles, &config);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sweep_type, SweepType::BearishSweep);
    }

    #[test]
    fn high_wick_ratio_threshold_filters_moderate_wicks() {
        // Moderate wick that passes low ratio but fails high ratio
        // open=100, high=108, low=95, close=103 => body=3, upper=5, lower=5
        let candles = vec![candle_ohlc("100", "108", "95", "103")];
        let lenient = SweepConfig {
            min_wick_ratio: Decimal::from(1),
            min_wick_pct: Decimal::new(1, 3),
        };
        let strict = SweepConfig {
            min_wick_ratio: Decimal::from(5),
            min_wick_pct: Decimal::new(1, 3),
        };
        let events_lenient = detect_sweeps(&candles, &lenient);
        let events_strict = detect_sweeps(&candles, &strict);
        assert!(events_lenient.len() > events_strict.len(),
            "strict config should detect fewer sweeps");
    }

    #[test]
    fn multiple_candles_independent_detection() {
        // Two candles, each with different sweep types
        let candles = vec![
            candle_ohlc("100", "120", "99", "101"), // bearish sweep (upper wick=19, body=1)
            candle_ohlc("100", "101", "80", "99"),  // bullish sweep (lower wick=19, body=1)
        ];
        let config = SweepConfig {
            min_wick_ratio: Decimal::from(2),
            min_wick_pct: Decimal::new(1, 3),
        };
        let events = detect_sweeps(&candles, &config);
        let bearish: Vec<_> = events.iter().filter(|e| e.sweep_type == SweepType::BearishSweep).collect();
        let bullish: Vec<_> = events.iter().filter(|e| e.sweep_type == SweepType::BullishSweep).collect();
        assert!(!bearish.is_empty(), "should detect bearish sweep on first candle");
        assert!(!bullish.is_empty(), "should detect bullish sweep on second candle");
        assert_eq!(bearish[0].index, 0);
        assert_eq!(bullish[0].index, 1);
    }
}
