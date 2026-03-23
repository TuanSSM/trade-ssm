use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ssm_core::{Candle, Side};

/// An absorption event — large volume traded with minimal price impact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbsorptionEvent {
    /// Candle index where absorption was detected.
    pub index: usize,
    /// Which side was absorbed (Buy = aggressive buying absorbed, Sell = aggressive selling absorbed).
    pub absorbed_side: Side,
    /// Total volume absorbed.
    pub volume: Decimal,
    /// Price range during absorption (high - low).
    pub price_range: Decimal,
    /// Ratio of volume to price range — higher = stronger absorption.
    pub absorption_strength: Decimal,
}

/// Configuration for absorption detection.
#[derive(Debug, Clone)]
pub struct AbsorptionConfig {
    /// Minimum volume (in base asset) for a candle to be considered high-volume.
    pub min_volume_threshold: Decimal,
    /// Maximum price range as percentage of price for absorption.
    /// e.g., 0.001 = 0.1% max range.
    pub max_range_pct: Decimal,
    /// Volume multiple over average to qualify.
    pub volume_multiple: Decimal,
}

impl Default for AbsorptionConfig {
    fn default() -> Self {
        Self {
            min_volume_threshold: Decimal::from(10),
            max_range_pct: Decimal::new(2, 3), // 0.002 = 0.2%
            volume_multiple: Decimal::from(2),
        }
    }
}

/// Detect absorption events in a sequence of candles.
///
/// Absorption = high volume with minimal price movement.
/// This indicates large resting orders absorbing aggressive flow.
pub fn detect_absorption(candles: &[Candle], config: &AbsorptionConfig) -> Vec<AbsorptionEvent> {
    if candles.is_empty() {
        return Vec::new();
    }

    // Calculate average volume
    let avg_volume =
        candles.iter().map(|c| c.volume).sum::<Decimal>() / Decimal::from(candles.len() as u64);

    let volume_threshold = avg_volume * config.volume_multiple;

    let mut events = Vec::new();

    for (i, c) in candles.iter().enumerate() {
        if c.volume < config.min_volume_threshold || c.volume < volume_threshold {
            continue;
        }

        let range = c.high - c.low;
        let range_pct = if c.open > Decimal::ZERO {
            range / c.open
        } else {
            continue;
        };

        if range_pct > config.max_range_pct {
            continue;
        }

        // Volume is high but price barely moved — absorption detected
        let delta = c.taker_buy_volume - c.taker_sell_volume;
        let absorbed_side = if delta > Decimal::ZERO {
            Side::Buy // Aggressive buying was absorbed (sellers held)
        } else {
            Side::Sell // Aggressive selling was absorbed (buyers held)
        };

        let absorption_strength = if range > Decimal::ZERO {
            c.volume / range
        } else {
            c.volume // infinite strength (zero range)
        };

        events.push(AbsorptionEvent {
            index: i,
            absorbed_side,
            volume: c.volume,
            price_range: range,
            absorption_strength,
        });
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn candle_with(
        open: &str,
        high: &str,
        low: &str,
        close: &str,
        vol: &str,
        buy: &str,
        sell: &str,
    ) -> Candle {
        Candle {
            open_time: 0,
            open: Decimal::from_str(open).unwrap(),
            high: Decimal::from_str(high).unwrap(),
            low: Decimal::from_str(low).unwrap(),
            close: Decimal::from_str(close).unwrap(),
            volume: Decimal::from_str(vol).unwrap(),
            close_time: 0,
            quote_volume: Decimal::ZERO,
            trades: 100,
            taker_buy_volume: Decimal::from_str(buy).unwrap(),
            taker_sell_volume: Decimal::from_str(sell).unwrap(),
        }
    }

    #[test]
    fn detect_high_vol_tight_range() {
        let candles = vec![
            candle_with("50000", "50010", "49990", "50005", "10", "5", "5"), // normal
            candle_with("50005", "50010", "49995", "50000", "10", "5", "5"), // normal
            candle_with("50000", "50005", "49998", "50002", "100", "80", "20"), // HIGH vol, tight range
        ];
        let config = AbsorptionConfig {
            min_volume_threshold: Decimal::from(5),
            max_range_pct: Decimal::new(5, 3), // 0.5%
            volume_multiple: Decimal::from(2),
        };
        let events = detect_absorption(&candles, &config);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].index, 2);
        assert_eq!(events[0].absorbed_side, Side::Buy); // aggressive buying absorbed
    }

    #[test]
    fn no_absorption_wide_range() {
        let candles = vec![
            candle_with("50000", "51000", "49000", "50500", "100", "60", "40"), // wide range
        ];
        let config = AbsorptionConfig {
            min_volume_threshold: Decimal::from(5),
            max_range_pct: Decimal::new(2, 3), // 0.2%
            volume_multiple: Decimal::from(1),
        };
        let events = detect_absorption(&candles, &config);
        assert!(events.is_empty());
    }

    #[test]
    fn empty_candles() {
        let events = detect_absorption(&[], &AbsorptionConfig::default());
        assert!(events.is_empty());
    }

    #[test]
    fn default_config_values() {
        let config = AbsorptionConfig::default();
        assert_eq!(config.min_volume_threshold, Decimal::from(10));
        assert_eq!(config.max_range_pct, Decimal::new(2, 3));
        assert_eq!(config.volume_multiple, Decimal::from(2));
    }

    #[test]
    fn single_candle_below_min_volume_threshold() {
        // Volume is below min_volume_threshold — no absorption
        let candles = vec![
            candle_with("50000", "50001", "49999", "50000", "5", "3", "2"),
        ];
        let config = AbsorptionConfig {
            min_volume_threshold: Decimal::from(10),
            max_range_pct: Decimal::new(5, 3),
            volume_multiple: Decimal::from(1),
        };
        let events = detect_absorption(&candles, &config);
        assert!(events.is_empty());
    }

    #[test]
    fn absorption_sell_side_detected() {
        // Aggressive selling absorbed (sell_volume > buy_volume, tight range, high vol)
        let candles = vec![
            candle_with("50000", "50010", "49990", "50005", "10", "5", "5"), // normal
            candle_with("50005", "50010", "49995", "50000", "10", "5", "5"), // normal
            candle_with("50000", "50005", "49998", "50002", "100", "20", "80"), // HIGH vol, sell-heavy
        ];
        let config = AbsorptionConfig {
            min_volume_threshold: Decimal::from(5),
            max_range_pct: Decimal::new(5, 3),
            volume_multiple: Decimal::from(2),
        };
        let events = detect_absorption(&candles, &config);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].absorbed_side, Side::Sell); // aggressive selling absorbed
    }

    #[test]
    fn zero_range_candle_absorption_strength() {
        // When range is zero, absorption_strength = volume (special case)
        let candles = vec![
            candle_with("50000", "50000", "50000", "50000", "5", "2", "3"), // normal low vol
            candle_with("50000", "50000", "50000", "50000", "100", "60", "40"), // high vol, zero range
        ];
        let config = AbsorptionConfig {
            min_volume_threshold: Decimal::from(5),
            max_range_pct: Decimal::new(5, 3),
            volume_multiple: Decimal::from(1), // avg_vol = 52.5, threshold = 52.5
        };
        let events = detect_absorption(&candles, &config);
        assert_eq!(events.len(), 1);
        // Zero range means absorption_strength = volume
        assert_eq!(events[0].absorption_strength, Decimal::from(100));
    }

    #[test]
    fn single_candle_high_vol_tight_range_detected() {
        // Single candle — avg_volume == candle volume, so volume_multiple must be <= 1
        let candles = vec![
            candle_with("50000", "50005", "49998", "50002", "100", "70", "30"),
        ];
        let config = AbsorptionConfig {
            min_volume_threshold: Decimal::from(5),
            max_range_pct: Decimal::new(5, 3),
            volume_multiple: Decimal::from(1), // must be <= 1 for single candle
        };
        let events = detect_absorption(&candles, &config);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].absorbed_side, Side::Buy);
    }

    #[test]
    fn volume_below_average_multiple_not_detected() {
        // All candles have similar volume — none exceeds 2x average
        let candles = vec![
            candle_with("50000", "50005", "49998", "50002", "50", "30", "20"),
            candle_with("50000", "50005", "49998", "50002", "52", "28", "24"),
            candle_with("50000", "50005", "49998", "50002", "48", "25", "23"),
        ];
        let config = AbsorptionConfig {
            min_volume_threshold: Decimal::from(5),
            max_range_pct: Decimal::new(5, 3),
            volume_multiple: Decimal::from(2), // need 2x avg (~50), so need 100+
        };
        let events = detect_absorption(&candles, &config);
        assert!(events.is_empty());
    }

    #[test]
    fn multiple_absorption_events_detected() {
        // Two high-volume tight-range candles among normals
        // avg_vol = (10 + 300 + 10 + 300) / 4 = 155, threshold = 155 * 1.5 = 232.5
        let candles = vec![
            candle_with("50000", "50010", "49990", "50005", "10", "5", "5"),
            candle_with("50000", "50003", "49999", "50001", "300", "200", "100"), // absorption
            candle_with("50000", "50010", "49990", "50005", "10", "5", "5"),
            candle_with("50000", "50002", "49999", "50001", "300", "80", "220"), // absorption
        ];
        let config = AbsorptionConfig {
            min_volume_threshold: Decimal::from(5),
            max_range_pct: Decimal::new(5, 3),
            volume_multiple: Decimal::new(15, 1), // 1.5x
        };
        let events = detect_absorption(&candles, &config);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].index, 1);
        assert_eq!(events[0].absorbed_side, Side::Buy);
        assert_eq!(events[1].index, 3);
        assert_eq!(events[1].absorbed_side, Side::Sell);
    }

    #[test]
    fn zero_open_price_candle_skipped() {
        // open == 0 causes range_pct division to be skipped
        let candles = vec![
            candle_with("0", "10", "0", "5", "100", "60", "40"),
        ];
        let config = AbsorptionConfig {
            min_volume_threshold: Decimal::from(1),
            max_range_pct: Decimal::new(999, 0), // very lenient
            volume_multiple: Decimal::from(1),
        };
        let events = detect_absorption(&candles, &config);
        // open is zero, so range_pct calculation hits continue
        assert!(events.is_empty());
    }
}
