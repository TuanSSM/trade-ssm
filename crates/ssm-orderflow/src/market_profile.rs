use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ssm_core::Candle;
use std::collections::BTreeMap;

/// A single point in the market profile (Time-Price-Opportunity).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileLevel {
    pub price: Decimal,
    /// Number of time periods that traded at this level.
    pub tpo_count: u32,
    /// Total volume at this level.
    pub volume: Decimal,
}

/// Market profile for a session or time period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketProfile {
    pub tick_size: Decimal,
    pub levels: Vec<ProfileLevel>,
    /// Point of Control — price level with highest volume.
    pub poc: Decimal,
    /// Value Area High — upper boundary of 70% volume area.
    pub vah: Decimal,
    /// Value Area Low — lower boundary of 70% volume area.
    pub val: Decimal,
    /// Total volume in the profile.
    pub total_volume: Decimal,
}

/// Build a market profile from candle data.
///
/// Each candle distributes its volume across its price range.
/// `tick_size` determines the price level granularity.
pub fn build_profile(candles: &[Candle], tick_size: Decimal) -> MarketProfile {
    if candles.is_empty() || tick_size.is_zero() {
        return MarketProfile {
            tick_size,
            levels: Vec::new(),
            poc: Decimal::ZERO,
            vah: Decimal::ZERO,
            val: Decimal::ZERO,
            total_volume: Decimal::ZERO,
        };
    }

    let mut level_data: BTreeMap<Decimal, (u32, Decimal)> = BTreeMap::new();

    for c in candles {
        let low_level = (c.low / tick_size).floor() * tick_size;
        let high_level = (c.high / tick_size).floor() * tick_size;

        // Count price levels this candle touches
        let mut level = low_level;
        let mut level_count = Decimal::ZERO;
        let mut tmp = level;
        while tmp <= high_level {
            level_count += Decimal::ONE;
            tmp += tick_size;
        }

        // Distribute volume across levels
        let vol_per_level = if level_count > Decimal::ZERO {
            c.volume / level_count
        } else {
            c.volume
        };

        while level <= high_level {
            let entry = level_data.entry(level).or_insert((0, Decimal::ZERO));
            entry.0 += 1; // TPO count
            entry.1 += vol_per_level;
            level += tick_size;
        }
    }

    let levels: Vec<ProfileLevel> = level_data
        .into_iter()
        .map(|(price, (tpo_count, volume))| ProfileLevel {
            price,
            tpo_count,
            volume,
        })
        .collect();

    let total_volume: Decimal = levels.iter().map(|l| l.volume).sum();

    // Find POC (max volume level)
    let poc = levels
        .iter()
        .max_by(|a, b| a.volume.cmp(&b.volume))
        .map(|l| l.price)
        .unwrap_or(Decimal::ZERO);

    // Calculate Value Area (70% of volume centered on POC)
    let (vah, val) = calculate_value_area(&levels, poc, total_volume);

    MarketProfile {
        tick_size,
        levels,
        poc,
        vah,
        val,
        total_volume,
    }
}

/// Calculate Value Area — the price range containing 70% of total volume,
/// expanding outward from the POC.
fn calculate_value_area(
    levels: &[ProfileLevel],
    poc: Decimal,
    total_volume: Decimal,
) -> (Decimal, Decimal) {
    if levels.is_empty() || total_volume.is_zero() {
        return (Decimal::ZERO, Decimal::ZERO);
    }

    let target = total_volume * Decimal::new(70, 2); // 70%
    let poc_idx = levels.iter().position(|l| l.price == poc).unwrap_or(0);

    let mut included_vol = levels[poc_idx].volume;
    let mut upper = poc_idx;
    let mut lower = poc_idx;

    while included_vol < target {
        let can_go_up = upper + 1 < levels.len();
        let can_go_down = lower > 0;

        if !can_go_up && !can_go_down {
            break;
        }

        let up_vol = if can_go_up {
            levels[upper + 1].volume
        } else {
            Decimal::ZERO
        };

        let down_vol = if can_go_down {
            levels[lower - 1].volume
        } else {
            Decimal::ZERO
        };

        if up_vol >= down_vol && can_go_up {
            upper += 1;
            included_vol += up_vol;
        } else if can_go_down {
            lower -= 1;
            included_vol += down_vol;
        } else {
            upper += 1;
            included_vol += up_vol;
        }
    }

    (levels[upper].price, levels[lower].price)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn candle_range(low: &str, high: &str, vol: &str) -> Candle {
        let l = Decimal::from_str(low).unwrap();
        let h = Decimal::from_str(high).unwrap();
        let v = Decimal::from_str(vol).unwrap();
        Candle {
            open_time: 0,
            open: l,
            high: h,
            low: l,
            close: h,
            volume: v,
            close_time: 0,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: v / Decimal::from(2),
            taker_sell_volume: v / Decimal::from(2),
        }
    }

    #[test]
    fn basic_profile() {
        let candles = vec![
            candle_range("100", "110", "100"),
            candle_range("105", "115", "100"),
            candle_range("100", "110", "100"),
        ];
        let profile = build_profile(&candles, Decimal::from(5));

        assert!(!profile.levels.is_empty());
        assert!(profile.total_volume > Decimal::ZERO);
        assert!(profile.poc >= Decimal::from(100));
        assert!(profile.vah >= profile.val);
    }

    #[test]
    fn poc_at_high_volume() {
        // All candles at same range — POC should be within that range
        let candles: Vec<_> = (0..10).map(|_| candle_range("100", "105", "100")).collect();
        let profile = build_profile(&candles, Decimal::from(1));
        assert!(profile.poc >= Decimal::from(100) && profile.poc <= Decimal::from(105));
    }

    #[test]
    fn empty_candles() {
        let profile = build_profile(&[], Decimal::from(1));
        assert!(profile.levels.is_empty());
        assert_eq!(profile.poc, Decimal::ZERO);
    }

    #[test]
    fn value_area_contains_poc() {
        let candles: Vec<_> = (0..20)
            .map(|i| {
                candle_range(
                    &format!("{}", 100 + i % 5),
                    &format!("{}", 105 + i % 5),
                    "50",
                )
            })
            .collect();
        let profile = build_profile(&candles, Decimal::from(1));
        assert!(profile.poc >= profile.val);
        assert!(profile.poc <= profile.vah);
    }

    #[test]
    fn zero_tick_size_returns_empty_profile() {
        let candles = vec![candle_range("100", "110", "100")];
        let profile = build_profile(&candles, Decimal::ZERO);
        assert!(profile.levels.is_empty());
        assert_eq!(profile.total_volume, Decimal::ZERO);
    }

    #[test]
    fn single_candle_profile() {
        let candles = vec![candle_range("100", "105", "50")];
        let profile = build_profile(&candles, Decimal::from(1));
        assert!(!profile.levels.is_empty());
        // Volume may have tiny rounding due to distribution across levels
        let diff = (profile.total_volume - Decimal::from(50)).abs();
        assert!(diff < Decimal::new(1, 10), "total_volume should be ~50, got {}", profile.total_volume);
        // POC must be within the candle's range
        assert!(profile.poc >= Decimal::from(100));
        assert!(profile.poc <= Decimal::from(105));
    }

    #[test]
    fn vah_gte_val() {
        // Value Area High should always be >= Value Area Low
        let candles = vec![
            candle_range("90", "110", "100"),
            candle_range("95", "115", "200"),
            candle_range("100", "120", "150"),
        ];
        let profile = build_profile(&candles, Decimal::from(5));
        assert!(profile.vah >= profile.val, "VAH ({}) must be >= VAL ({})", profile.vah, profile.val);
    }

    #[test]
    fn single_tick_candle_all_volume_at_one_level() {
        // Candle where high == low — all volume at one level
        let candles = vec![candle_range("100", "100", "75")];
        let profile = build_profile(&candles, Decimal::from(1));
        assert_eq!(profile.levels.len(), 1);
        assert_eq!(profile.poc, Decimal::from(100));
        assert_eq!(profile.total_volume, Decimal::from(75));
    }

    #[test]
    fn large_tick_size_fewer_levels() {
        // With a large tick size, multiple prices collapse into fewer levels
        let candles = vec![candle_range("100", "110", "100")];
        let profile_fine = build_profile(&candles, Decimal::from(1));
        let profile_coarse = build_profile(&candles, Decimal::from(5));
        assert!(
            profile_coarse.levels.len() <= profile_fine.levels.len(),
            "coarser tick should produce equal or fewer levels"
        );
    }

    #[test]
    fn total_volume_preserved_across_levels() {
        // Total volume across all levels should equal sum of input candle volumes
        let candles = vec![
            candle_range("100", "110", "60"),
            candle_range("105", "115", "40"),
        ];
        let profile = build_profile(&candles, Decimal::from(5));
        assert_eq!(profile.total_volume, Decimal::from(100));
    }

    #[test]
    fn poc_is_level_with_max_volume() {
        // Concentrate volume at one spot and verify POC
        let candles = vec![
            candle_range("100", "100", "200"), // all at level 100
            candle_range("200", "200", "10"),  // small at 200
        ];
        let profile = build_profile(&candles, Decimal::from(1));
        assert_eq!(profile.poc, Decimal::from(100));
    }
}
