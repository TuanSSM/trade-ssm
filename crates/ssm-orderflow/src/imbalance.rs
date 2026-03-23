use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ssm_core::Candle;

/// A volume imbalance zone — area where aggressive buying or selling dominates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImbalanceZone {
    /// Candle index where imbalance was detected.
    pub index: usize,
    pub zone_type: ImbalanceType,
    /// Buy/sell volume ratio.
    pub ratio: Decimal,
    /// The dominant volume.
    pub dominant_volume: Decimal,
    /// The weak-side volume.
    pub weak_volume: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImbalanceType {
    /// Buyers dominate (buy_vol >> sell_vol) — bullish zone.
    BuyImbalance,
    /// Sellers dominate (sell_vol >> buy_vol) — bearish zone.
    SellImbalance,
}

/// Configuration for imbalance detection.
#[derive(Debug, Clone)]
pub struct ImbalanceConfig {
    /// Minimum ratio for imbalance (e.g., 3.0 = one side must be 3x the other).
    pub min_ratio: Decimal,
    /// Minimum consecutive imbalances to form a "stacked imbalance" (stronger S/R).
    pub stacked_threshold: usize,
}

impl Default for ImbalanceConfig {
    fn default() -> Self {
        Self {
            min_ratio: Decimal::from(3),
            stacked_threshold: 3,
        }
    }
}

/// Stacked imbalance — consecutive candles with same-direction imbalance.
/// These form strong support/resistance zones.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackedImbalance {
    /// Starting candle index.
    pub start_index: usize,
    /// Number of consecutive imbalances.
    pub count: usize,
    pub zone_type: ImbalanceType,
}

/// Detect volume imbalance zones in candle data.
pub fn detect_imbalances(candles: &[Candle], config: &ImbalanceConfig) -> Vec<ImbalanceZone> {
    let mut zones = Vec::new();

    for (i, c) in candles.iter().enumerate() {
        if c.taker_buy_volume.is_zero() && c.taker_sell_volume.is_zero() {
            continue;
        }

        if c.taker_sell_volume > Decimal::ZERO {
            let buy_ratio = c.taker_buy_volume / c.taker_sell_volume;
            if buy_ratio >= config.min_ratio {
                zones.push(ImbalanceZone {
                    index: i,
                    zone_type: ImbalanceType::BuyImbalance,
                    ratio: buy_ratio,
                    dominant_volume: c.taker_buy_volume,
                    weak_volume: c.taker_sell_volume,
                });
                continue;
            }
        }

        if c.taker_buy_volume > Decimal::ZERO {
            let sell_ratio = c.taker_sell_volume / c.taker_buy_volume;
            if sell_ratio >= config.min_ratio {
                zones.push(ImbalanceZone {
                    index: i,
                    zone_type: ImbalanceType::SellImbalance,
                    ratio: sell_ratio,
                    dominant_volume: c.taker_sell_volume,
                    weak_volume: c.taker_buy_volume,
                });
            }
        }
    }

    zones
}

/// Detect stacked imbalances (consecutive same-direction imbalances).
pub fn detect_stacked_imbalances(
    zones: &[ImbalanceZone],
    min_stack: usize,
) -> Vec<StackedImbalance> {
    if zones.is_empty() {
        return Vec::new();
    }

    let mut stacks = Vec::new();
    let mut start = 0;

    for i in 1..zones.len() {
        let consecutive = zones[i].index == zones[i - 1].index + 1
            && zones[i].zone_type == zones[i - 1].zone_type;

        if !consecutive {
            let count = i - start;
            if count >= min_stack {
                stacks.push(StackedImbalance {
                    start_index: zones[start].index,
                    count,
                    zone_type: zones[start].zone_type,
                });
            }
            start = i;
        }
    }

    // Handle the last run
    let count = zones.len() - start;
    if count >= min_stack {
        stacks.push(StackedImbalance {
            start_index: zones[start].index,
            count,
            zone_type: zones[start].zone_type,
        });
    }

    stacks
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn candle_vol(buy: &str, sell: &str) -> Candle {
        let bv = Decimal::from_str(buy).unwrap();
        let sv = Decimal::from_str(sell).unwrap();
        Candle {
            open_time: 0,
            open: Decimal::from(100),
            high: Decimal::from(105),
            low: Decimal::from(95),
            close: Decimal::from(102),
            volume: bv + sv,
            close_time: 0,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: bv,
            taker_sell_volume: sv,
        }
    }

    #[test]
    fn detect_buy_imbalance() {
        let candles = vec![candle_vol("90", "10")]; // 9:1 ratio
        let config = ImbalanceConfig {
            min_ratio: Decimal::from(3),
            stacked_threshold: 1,
        };
        let zones = detect_imbalances(&candles, &config);
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].zone_type, ImbalanceType::BuyImbalance);
    }

    #[test]
    fn detect_sell_imbalance() {
        let candles = vec![candle_vol("10", "90")]; // 9:1 sell ratio
        let config = ImbalanceConfig::default();
        let zones = detect_imbalances(&candles, &config);
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].zone_type, ImbalanceType::SellImbalance);
    }

    #[test]
    fn no_imbalance_balanced() {
        let candles = vec![candle_vol("50", "50")]; // 1:1
        let config = ImbalanceConfig::default();
        let zones = detect_imbalances(&candles, &config);
        assert!(zones.is_empty());
    }

    #[test]
    fn stacked_imbalances() {
        let candles: Vec<_> = (0..5).map(|_| candle_vol("90", "10")).collect();
        let config = ImbalanceConfig::default();
        let zones = detect_imbalances(&candles, &config);
        assert_eq!(zones.len(), 5);

        // All same type and consecutive
        let stacks = detect_stacked_imbalances(&zones, 3);
        assert_eq!(stacks.len(), 1);
        assert_eq!(stacks[0].count, 5);
        assert_eq!(stacks[0].zone_type, ImbalanceType::BuyImbalance);
    }

    #[test]
    fn empty_candles_no_imbalance() {
        let config = ImbalanceConfig::default();
        let zones = detect_imbalances(&[], &config);
        assert!(zones.is_empty());
    }

    #[test]
    fn zero_volume_candle_skipped() {
        let candles = vec![candle_vol("0", "0")];
        let config = ImbalanceConfig::default();
        let zones = detect_imbalances(&candles, &config);
        assert!(zones.is_empty());
    }

    #[test]
    fn default_config_values() {
        let config = ImbalanceConfig::default();
        assert_eq!(config.min_ratio, Decimal::from(3));
        assert_eq!(config.stacked_threshold, 3);
    }

    #[test]
    fn exact_ratio_threshold_triggers_imbalance() {
        // Ratio is exactly 3:1 — should trigger with min_ratio=3
        let candles = vec![candle_vol("75", "25")]; // 75/25 = 3.0
        let config = ImbalanceConfig {
            min_ratio: Decimal::from(3),
            stacked_threshold: 1,
        };
        let zones = detect_imbalances(&candles, &config);
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].zone_type, ImbalanceType::BuyImbalance);
        assert_eq!(zones[0].ratio, Decimal::from(3));
    }

    #[test]
    fn stacked_imbalances_empty_zones() {
        let stacks = detect_stacked_imbalances(&[], 3);
        assert!(stacks.is_empty());
    }

    #[test]
    fn stacked_imbalances_non_consecutive_indices() {
        // Zones at indices 0, 2, 4 — not consecutive, no stack
        let zones = vec![
            ImbalanceZone {
                index: 0,
                zone_type: ImbalanceType::BuyImbalance,
                ratio: Decimal::from(5),
                dominant_volume: Decimal::from(90),
                weak_volume: Decimal::from(10),
            },
            ImbalanceZone {
                index: 2,
                zone_type: ImbalanceType::BuyImbalance,
                ratio: Decimal::from(5),
                dominant_volume: Decimal::from(90),
                weak_volume: Decimal::from(10),
            },
            ImbalanceZone {
                index: 4,
                zone_type: ImbalanceType::BuyImbalance,
                ratio: Decimal::from(5),
                dominant_volume: Decimal::from(90),
                weak_volume: Decimal::from(10),
            },
        ];
        let stacks = detect_stacked_imbalances(&zones, 2);
        assert!(stacks.is_empty());
    }

    #[test]
    fn below_ratio_threshold_no_imbalance() {
        // Ratio just below threshold (2.9:1 with min_ratio=3)
        let candles = vec![candle_vol("74", "26")]; // 74/26 = 2.846...
        let config = ImbalanceConfig {
            min_ratio: Decimal::from(3),
            stacked_threshold: 1,
        };
        let zones = detect_imbalances(&candles, &config);
        assert!(zones.is_empty());
    }

    #[test]
    fn single_zone_below_stacked_threshold() {
        // Only 1 zone, stacked_threshold=2 — no stacked imbalance
        let zones = vec![ImbalanceZone {
            index: 0,
            zone_type: ImbalanceType::BuyImbalance,
            ratio: Decimal::from(5),
            dominant_volume: Decimal::from(90),
            weak_volume: Decimal::from(10),
        }];
        let stacks = detect_stacked_imbalances(&zones, 2);
        assert!(stacks.is_empty());
    }

    #[test]
    fn mixed_imbalance_types_break_stacking() {
        // Alternating buy/sell imbalances — should not stack
        let candles = vec![
            candle_vol("90", "10"), // BuyImbalance
            candle_vol("10", "90"), // SellImbalance
            candle_vol("90", "10"), // BuyImbalance
        ];
        let config = ImbalanceConfig {
            min_ratio: Decimal::from(3),
            stacked_threshold: 1,
        };
        let zones = detect_imbalances(&candles, &config);
        assert_eq!(zones.len(), 3);
        let stacks = detect_stacked_imbalances(&zones, 2);
        assert!(stacks.is_empty(), "alternating types should not form stacks");
    }

    #[test]
    fn one_side_zero_volume_only_other_side_checked() {
        // sell_volume == 0: buy ratio division would be division by zero, skipped
        // buy_volume > 0 but sell_ratio = 0 / buy_vol = 0 — no imbalance
        let candles = vec![candle_vol("100", "0")];
        let config = ImbalanceConfig::default();
        let zones = detect_imbalances(&candles, &config);
        // taker_sell_volume == 0 so buy ratio check skipped
        // taker_buy_volume > 0, sell_ratio = 0/100 = 0 < min_ratio
        assert!(zones.is_empty());
    }
}
