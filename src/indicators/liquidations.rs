use rust_decimal::Decimal;

use crate::exchange::types::{Liquidation, LiquidationTier};

/// Summary of liquidations over a time period
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
    /// More longs liquidated — bearish pressure
    LongsLiquidated,
    /// More shorts liquidated — bullish pressure
    ShortsLiquidated,
    /// Balanced
    Balanced,
}

impl std::fmt::Display for LiquidationBias {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LongsLiquidated => write!(f, "LONGS REKT (bearish)"),
            Self::ShortsLiquidated => write!(f, "SHORTS REKT (bullish)"),
            Self::Balanced => write!(f, "BALANCED"),
        }
    }
}

/// Analyze liquidation events and produce a summary
pub fn analyze_liquidations(liquidations: &[Liquidation]) -> LiquidationSummary {
    let mut total_long_count = 0u32;
    let mut total_short_count = 0u32;
    let mut total_long_value = Decimal::ZERO;
    let mut total_short_value = Decimal::ZERO;

    // Tier accumulators
    let tiers = [
        LiquidationTier::Small,
        LiquidationTier::Medium,
        LiquidationTier::Large,
        LiquidationTier::Massive,
    ];
    let mut tier_long_counts = [0u32; 4];
    let mut tier_short_counts = [0u32; 4];
    let mut tier_long_values = [Decimal::ZERO; 4];
    let mut tier_short_values = [Decimal::ZERO; 4];

    for liq in liquidations {
        let usd_value = liq.price * liq.quantity;
        let is_long = liq.side.eq_ignore_ascii_case("SELL"); // liquidated long = sell order

        if is_long {
            total_long_count += 1;
            total_long_value += usd_value;
        } else {
            total_short_count += 1;
            total_short_value += usd_value;
        }

        if let Some(tier) = LiquidationTier::classify(usd_value) {
            let idx = match tier {
                LiquidationTier::Small => 0,
                LiquidationTier::Medium => 1,
                LiquidationTier::Large => 2,
                LiquidationTier::Massive => 3,
            };
            if is_long {
                tier_long_counts[idx] += 1;
                tier_long_values[idx] += usd_value;
            } else {
                tier_short_counts[idx] += 1;
                tier_short_values[idx] += usd_value;
            }
        }
    }

    let by_tier = tiers
        .iter()
        .enumerate()
        .map(|(i, &tier)| TierSummary {
            tier,
            long_count: tier_long_counts[i],
            short_count: tier_short_counts[i],
            long_value: tier_long_values[i],
            short_value: tier_short_values[i],
        })
        .collect();

    let bias = if total_long_value > total_short_value * Decimal::from(2) {
        LiquidationBias::LongsLiquidated
    } else if total_short_value > total_long_value * Decimal::from(2) {
        LiquidationBias::ShortsLiquidated
    } else {
        LiquidationBias::Balanced
    };

    LiquidationSummary {
        total_long_liquidations: total_long_count,
        total_short_liquidations: total_short_count,
        total_long_value,
        total_short_value,
        by_tier,
        bias,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn make_liq(side: &str, price: &str, qty: &str) -> Liquidation {
        Liquidation {
            symbol: "BTCUSDT".to_string(),
            side: side.to_string(),
            price: Decimal::from_str(price).unwrap(),
            quantity: Decimal::from_str(qty).unwrap(),
            time: 0,
        }
    }

    #[test]
    fn test_long_liquidation_bias() {
        let liqs = vec![
            make_liq("SELL", "50000", "1.0"),   // $50k long liquidated
            make_liq("SELL", "50000", "0.5"),   // $25k long liquidated
            make_liq("BUY", "50000", "0.1"),    // $5k short liquidated
        ];

        let summary = analyze_liquidations(&liqs);
        assert_eq!(summary.total_long_liquidations, 2);
        assert_eq!(summary.total_short_liquidations, 1);
        assert_eq!(summary.bias, LiquidationBias::LongsLiquidated);
    }

    #[test]
    fn test_tier_classification() {
        let liqs = vec![
            make_liq("SELL", "50000", "3.0"),   // $150k — Massive
            make_liq("BUY", "50000", "0.7"),    // $35k — Large
            make_liq("SELL", "50000", "0.02"),  // $1k — Small
        ];

        let summary = analyze_liquidations(&liqs);
        let massive = summary.by_tier.iter().find(|t| t.tier == LiquidationTier::Massive).unwrap();
        assert_eq!(massive.long_count, 1);
    }
}
