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
        assert!(
            diff < Decimal::new(1, 10),
            "total_volume should be ~50, got {}",
            profile.total_volume
        );
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
        assert!(
            profile.vah >= profile.val,
            "VAH ({}) must be >= VAL ({})",
            profile.vah,
            profile.val
        );
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

    #[test]
    fn single_candle_zero_range_profile() {
        // Candle with high == low: single level, all volume there
        let candles = vec![candle_range("500", "500", "120")];
        let profile = build_profile(&candles, Decimal::from(10));
        assert_eq!(profile.levels.len(), 1);
        assert_eq!(profile.poc, Decimal::from(500));
        assert_eq!(profile.total_volume, Decimal::from(120));
        assert_eq!(profile.vah, Decimal::from(500));
        assert_eq!(profile.val, Decimal::from(500));
    }

    #[test]
    fn value_area_covers_at_least_70_pct() {
        // Build a wider profile and verify VA covers >= 70% of volume
        let candles: Vec<_> = (0..50)
            .map(|i| {
                let base = 100 + (i % 20);
                candle_range(&format!("{}", base), &format!("{}", base + 5), "100")
            })
            .collect();
        let profile = build_profile(&candles, Decimal::from(1));
        // Sum volume in VA range
        let va_volume: Decimal = profile
            .levels
            .iter()
            .filter(|l| l.price >= profile.val && l.price <= profile.vah)
            .map(|l| l.volume)
            .sum();
        let target = profile.total_volume * Decimal::new(70, 2);
        assert!(
            va_volume >= target,
            "VA volume ({}) should be >= 70% of total ({})",
            va_volume,
            target
        );
    }

    #[test]
    fn two_candles_different_ranges_tpo_count() {
        // Two candles overlapping at level 100: TPO count should be 2 there
        let candles = vec![
            candle_range("100", "102", "30"),
            candle_range("99", "100", "20"),
        ];
        let profile = build_profile(&candles, Decimal::from(1));
        let level_100 = profile
            .levels
            .iter()
            .find(|l| l.price == Decimal::from(100));
        assert!(level_100.is_some(), "level 100 should exist");
        assert_eq!(
            level_100.unwrap().tpo_count,
            2,
            "two candles touch level 100"
        );
    }

    #[test]
    fn levels_sorted_by_price() {
        let candles = vec![
            candle_range("200", "210", "50"),
            candle_range("100", "110", "50"),
        ];
        let profile = build_profile(&candles, Decimal::from(5));
        for i in 1..profile.levels.len() {
            assert!(
                profile.levels[i].price > profile.levels[i - 1].price,
                "levels should be sorted ascending by price"
            );
        }
    }

    #[test]
    fn calculate_value_area_empty_levels() {
        let (vah, val) = calculate_value_area(&[], Decimal::ZERO, Decimal::ZERO);
        assert_eq!(vah, Decimal::ZERO);
        assert_eq!(val, Decimal::ZERO);
    }

    #[test]
    fn calculate_value_area_zero_total_volume() {
        let levels = vec![ProfileLevel {
            price: Decimal::from(100),
            tpo_count: 1,
            volume: Decimal::ZERO,
        }];
        let (vah, val) = calculate_value_area(&levels, Decimal::from(100), Decimal::ZERO);
        assert_eq!(vah, Decimal::ZERO);
        assert_eq!(val, Decimal::ZERO);
    }

    #[test]
    fn profile_with_concentrated_volume() {
        // Most volume at one price, small amounts elsewhere
        // POC should be at the high-volume level, VA should be narrow
        let candles = vec![
            candle_range("100", "100", "1000"), // huge volume at 100
            candle_range("90", "110", "10"),    // spread thinly
        ];
        let profile = build_profile(&candles, Decimal::from(1));
        assert_eq!(profile.poc, Decimal::from(100));
        // VA should be relatively narrow since 1000 out of 1010 is at level 100
        assert!(
            profile.vah - profile.val <= Decimal::from(5),
            "VA should be narrow with concentrated volume"
        );
    }

    #[test]
    fn profile_with_uniform_volume() {
        // Equal volume at all levels - VA should be wide
        let candles: Vec<_> = (0..10)
            .map(|i| candle_range(&format!("{}", 100 + i), &format!("{}", 100 + i), "100"))
            .collect();
        let profile = build_profile(&candles, Decimal::from(1));
        assert_eq!(profile.levels.len(), 10);
        assert_eq!(profile.total_volume, Decimal::from(1000));
        // VA needs to cover 70% = 700, so at least 7 levels
        let va_levels = profile
            .levels
            .iter()
            .filter(|l| l.price >= profile.val && l.price <= profile.vah)
            .count();
        assert!(
            va_levels >= 7,
            "VA should cover at least 7 of 10 uniform levels, got {}",
            va_levels
        );
    }

    #[test]
    fn fractional_tick_size_profile() {
        let candles = vec![candle_range("100.0", "100.5", "60")];
        let profile = build_profile(&candles, Decimal::from_str("0.1").unwrap());
        // Range from 100.0 to 100.5 with tick 0.1 = 6 levels
        assert_eq!(profile.levels.len(), 6);
        // Volume distributed equally: 60/6 = 10 per level
        for level in &profile.levels {
            assert_eq!(level.volume, Decimal::from(10));
        }
    }
}
