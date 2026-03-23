use rust_decimal::Decimal;
use ssm_core::{Liquidation, LiquidationTier};

#[derive(Debug, Clone)]
pub struct LiquidationSummary {
    pub total_long_liquidations: u32,
    pub total_short_liquidations: u32,
    pub total_long_value: Decimal,
    pub total_short_value: Decimal,
    pub by_tier: Vec<TierSummary>,
    pub bias: LiquidationBias,
}

#[derive(Debug, Clone)]
pub struct TierSummary {
    pub tier: LiquidationTier,
    pub long_count: u32,
    pub short_count: u32,
    pub long_value: Decimal,
    pub short_value: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiquidationBias {
    LongsLiquidated,
    ShortsLiquidated,
    Balanced,
}

impl std::fmt::Display for LiquidationBias {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::LongsLiquidated => "LONGS REKT (bearish)",
            Self::ShortsLiquidated => "SHORTS REKT (bullish)",
            Self::Balanced => "BALANCED",
        })
    }
}

pub fn analyze_liquidations(liquidations: &[Liquidation]) -> LiquidationSummary {
    const TIERS: [LiquidationTier; 4] = [
        LiquidationTier::Small,
        LiquidationTier::Medium,
        LiquidationTier::Large,
        LiquidationTier::Massive,
    ];

    let mut long_count = 0u32;
    let mut short_count = 0u32;
    let mut long_val = Decimal::ZERO;
    let mut short_val = Decimal::ZERO;
    let mut t_lc = [0u32; 4];
    let mut t_sc = [0u32; 4];
    let mut t_lv = [Decimal::ZERO; 4];
    let mut t_sv = [Decimal::ZERO; 4];

    for liq in liquidations {
        let usd = liq.price * liq.quantity;
        let is_long = liq.side.eq_ignore_ascii_case("SELL");

        if is_long {
            long_count += 1;
            long_val += usd;
        } else {
            short_count += 1;
            short_val += usd;
        }

        if let Some(tier) = LiquidationTier::classify(usd) {
            let idx = match tier {
                LiquidationTier::Small => 0,
                LiquidationTier::Medium => 1,
                LiquidationTier::Large => 2,
                LiquidationTier::Massive => 3,
            };
            if is_long {
                t_lc[idx] += 1;
                t_lv[idx] += usd;
            } else {
                t_sc[idx] += 1;
                t_sv[idx] += usd;
            }
        }
    }

    let by_tier = TIERS
        .iter()
        .enumerate()
        .map(|(i, &tier)| TierSummary {
            tier,
            long_count: t_lc[i],
            short_count: t_sc[i],
            long_value: t_lv[i],
            short_value: t_sv[i],
        })
        .collect();

    let bias = if long_val > short_val * Decimal::from(2) {
        LiquidationBias::LongsLiquidated
    } else if short_val > long_val * Decimal::from(2) {
        LiquidationBias::ShortsLiquidated
    } else {
        LiquidationBias::Balanced
    };

    LiquidationSummary {
        total_long_liquidations: long_count,
        total_short_liquidations: short_count,
        total_long_value: long_val,
        total_short_value: short_val,
        by_tier,
        bias,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn liq(side: &str, price: &str, qty: &str) -> Liquidation {
        Liquidation {
            symbol: "BTCUSDT".into(),
            side: side.into(),
            price: Decimal::from_str(price).unwrap(),
            quantity: Decimal::from_str(qty).unwrap(),
            time: 0,
        }
    }

    #[test]
    fn long_bias() {
        let liqs = vec![
            liq("SELL", "50000", "1.0"), // $50k long liq
            liq("SELL", "50000", "0.5"), // $25k long liq
            liq("BUY", "50000", "0.1"),  // $5k short liq
        ];
        let s = analyze_liquidations(&liqs);
        assert_eq!(s.total_long_liquidations, 2);
        assert_eq!(s.total_short_liquidations, 1);
        assert_eq!(s.bias, LiquidationBias::LongsLiquidated);
    }

    #[test]
    fn tier_classification() {
        let liqs = vec![
            liq("SELL", "50000", "3.0"),  // $150k Massive
            liq("BUY", "50000", "0.7"),   // $35k Large
            liq("SELL", "50000", "0.02"), // $1k Small
        ];
        let s = analyze_liquidations(&liqs);
        let massive = s
            .by_tier
            .iter()
            .find(|t| t.tier == LiquidationTier::Massive)
            .unwrap();
        assert_eq!(massive.long_count, 1);
    }

    #[test]
    fn empty_liquidations() {
        let s = analyze_liquidations(&[]);
        assert_eq!(s.total_long_liquidations, 0);
        assert_eq!(s.total_short_liquidations, 0);
        assert_eq!(s.bias, LiquidationBias::Balanced);
    }

    #[test]
    fn test_short_bias() {
        let liqs = vec![
            liq("BUY", "50000", "1.0"),  // $50k short liq
            liq("BUY", "50000", "0.5"),  // $25k short liq
            liq("SELL", "50000", "0.1"), // $5k long liq
        ];
        let s = analyze_liquidations(&liqs);
        assert_eq!(s.total_short_liquidations, 2);
        assert_eq!(s.total_long_liquidations, 1);
        assert_eq!(s.bias, LiquidationBias::ShortsLiquidated);
    }

    #[test]
    fn test_balanced_bias() {
        let liqs = vec![
            liq("SELL", "50000", "1.0"), // $50k long liq
            liq("BUY", "50000", "1.0"),  // $50k short liq
        ];
        let s = analyze_liquidations(&liqs);
        assert_eq!(s.bias, LiquidationBias::Balanced);
    }

    #[test]
    fn test_liquidation_bias_display() {
        assert_eq!(
            LiquidationBias::LongsLiquidated.to_string(),
            "LONGS REKT (bearish)"
        );
        assert_eq!(
            LiquidationBias::ShortsLiquidated.to_string(),
            "SHORTS REKT (bullish)"
        );
        assert_eq!(LiquidationBias::Balanced.to_string(), "BALANCED");
    }

    #[test]
    fn test_sub_threshold_liquidations() {
        // All below $1K → no tier counts (classify returns None)
        let liqs = vec![
            liq("SELL", "100", "0.5"), // $50
            liq("BUY", "200", "1.0"),  // $200
        ];
        let s = analyze_liquidations(&liqs);
        for tier in &s.by_tier {
            assert_eq!(tier.long_count, 0, "No long tier counts for sub-threshold");
            assert_eq!(
                tier.short_count, 0,
                "No short tier counts for sub-threshold"
            );
        }
    }

    #[test]
    fn test_all_tiers_populated() {
        // by_tier should always have 4 entries regardless of data
        let s = analyze_liquidations(&[]);
        assert_eq!(s.by_tier.len(), 4);
        assert_eq!(s.by_tier[0].tier, LiquidationTier::Small);
        assert_eq!(s.by_tier[1].tier, LiquidationTier::Medium);
        assert_eq!(s.by_tier[2].tier, LiquidationTier::Large);
        assert_eq!(s.by_tier[3].tier, LiquidationTier::Massive);
    }

    #[test]
    fn test_single_long_liquidation() {
        let liqs = vec![liq("SELL", "50000", "1.0")]; // $50k long liq
        let s = analyze_liquidations(&liqs);
        assert_eq!(s.total_long_liquidations, 1);
        assert_eq!(s.total_short_liquidations, 0);
        assert_eq!(s.total_long_value, Decimal::from_str("50000").unwrap());
        assert_eq!(s.total_short_value, Decimal::ZERO);
        // One long with 0 shorts => long_val > short_val * 2 => LongsLiquidated
        assert_eq!(s.bias, LiquidationBias::LongsLiquidated);
    }

    #[test]
    fn test_single_short_liquidation() {
        let liqs = vec![liq("BUY", "40000", "2.0")]; // $80k short liq
        let s = analyze_liquidations(&liqs);
        assert_eq!(s.total_long_liquidations, 0);
        assert_eq!(s.total_short_liquidations, 1);
        assert_eq!(s.total_short_value, Decimal::from_str("80000").unwrap());
        assert_eq!(s.bias, LiquidationBias::ShortsLiquidated);
    }

    #[test]
    fn test_extreme_leverage_large_quantity() {
        // Very large quantity simulating high leverage liquidation
        let liqs = vec![liq("SELL", "60000", "100.0")]; // $6M long liq
        let s = analyze_liquidations(&liqs);
        assert_eq!(s.total_long_liquidations, 1);
        let expected = Decimal::from_str("6000000").unwrap();
        assert_eq!(s.total_long_value, expected);
        // Should be classified as Massive tier
        let massive = s
            .by_tier
            .iter()
            .find(|t| t.tier == LiquidationTier::Massive)
            .unwrap();
        assert_eq!(massive.long_count, 1);
        assert_eq!(massive.long_value, expected);
    }

    #[test]
    fn test_extreme_leverage_tiny_quantity() {
        // Very small quantity (e.g., someone liquidated with tiny position)
        let liqs = vec![liq("SELL", "50000", "0.001")]; // $50 long liq
        let s = analyze_liquidations(&liqs);
        assert_eq!(s.total_long_liquidations, 1);
        assert_eq!(s.total_long_value, Decimal::from_str("50").unwrap());
        // Below $1K, should not appear in any tier
        for tier in &s.by_tier {
            assert_eq!(tier.long_count, 0);
        }
    }

    #[test]
    fn test_empty_liquidations_totals_zero() {
        let s = analyze_liquidations(&[]);
        assert_eq!(s.total_long_liquidations, 0);
        assert_eq!(s.total_short_liquidations, 0);
        assert_eq!(s.total_long_value, Decimal::ZERO);
        assert_eq!(s.total_short_value, Decimal::ZERO);
        assert_eq!(s.bias, LiquidationBias::Balanced);
        assert_eq!(s.by_tier.len(), 4);
        for tier in &s.by_tier {
            assert_eq!(tier.long_count, 0);
            assert_eq!(tier.short_count, 0);
            assert_eq!(tier.long_value, Decimal::ZERO);
            assert_eq!(tier.short_value, Decimal::ZERO);
        }
    }

    #[test]
    fn test_many_small_liquidations_add_up() {
        // Many small liquidations that individually are sub-threshold
        let liqs: Vec<_> = (0..100)
            .map(|_| liq("SELL", "100", "0.5")) // $50 each
            .collect();
        let s = analyze_liquidations(&liqs);
        assert_eq!(s.total_long_liquidations, 100);
        assert_eq!(s.total_long_value, Decimal::from_str("5000").unwrap());
        // Each is $50, below Small tier threshold
        for tier in &s.by_tier {
            assert_eq!(tier.long_count, 0);
        }
    }
}
