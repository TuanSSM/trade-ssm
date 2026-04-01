use rust_decimal::Decimal;
use ssm_core::Side;

/// Slippage model for realistic fill price simulation.
#[derive(Debug, Clone, Default)]
pub enum SlippageModel {
    /// No slippage (ideal fills)
    #[default]
    None,
    /// Fixed basis points slippage (e.g. 5 = 0.05%)
    FixedBps(Decimal),
    /// Volume-dependent slippage: base_bps + (order_volume / candle_volume) * impact_bps
    VolumeImpact {
        base_bps: Decimal,
        impact_bps: Decimal,
    },
}

impl SlippageModel {
    /// Returns the slipped fill price.
    ///
    /// For `Buy` orders the price goes UP (adverse fill); for `Sell` orders
    /// the price goes DOWN.
    ///
    /// * `price` – the reference (mid) price
    /// * `side` – order direction
    /// * `order_volume` – notional value of the order, used for `VolumeImpact`
    /// * `candle_volume` – total candle volume (quote), used for `VolumeImpact`
    pub fn apply(
        &self,
        price: Decimal,
        side: Side,
        order_volume: Option<Decimal>,
        candle_volume: Option<Decimal>,
    ) -> Decimal {
        let bps = self.bps(order_volume, candle_volume);
        if bps == Decimal::ZERO {
            return price;
        }
        // bps is in basis points (1 bps = 0.0001)
        let slip_fraction = bps / Decimal::from(10_000);
        match side {
            Side::Buy => price * (Decimal::ONE + slip_fraction),
            Side::Sell => price * (Decimal::ONE - slip_fraction),
        }
    }

    /// Compute the effective slippage in basis points for the given order.
    fn bps(&self, order_volume: Option<Decimal>, candle_volume: Option<Decimal>) -> Decimal {
        match self {
            SlippageModel::None => Decimal::ZERO,
            SlippageModel::FixedBps(bps) => *bps,
            SlippageModel::VolumeImpact {
                base_bps,
                impact_bps,
            } => {
                let volume_ratio = match (order_volume, candle_volume) {
                    (Some(ov), Some(cv)) if cv > Decimal::ZERO => ov / cv,
                    _ => Decimal::ZERO,
                };
                base_bps + volume_ratio * impact_bps
            }
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    // -----------------------------------------------------------------------
    // None variant
    // -----------------------------------------------------------------------

    #[test]
    fn no_slippage_returns_same_price_buy() {
        let model = SlippageModel::None;
        let price = Decimal::from(50_000);
        assert_eq!(model.apply(price, Side::Buy, None, None), price);
    }

    #[test]
    fn no_slippage_returns_same_price_sell() {
        let model = SlippageModel::None;
        let price = Decimal::from(50_000);
        assert_eq!(model.apply(price, Side::Sell, None, None), price);
    }

    // -----------------------------------------------------------------------
    // FixedBps variant
    // -----------------------------------------------------------------------

    #[test]
    fn fixed_bps_buy_increases_price() {
        // 10 bps = 0.10% slippage
        let model = SlippageModel::FixedBps(Decimal::from(10));
        let price = Decimal::from(10_000);
        let filled = model.apply(price, Side::Buy, None, None);
        // expected = 10_000 * 1.001 = 10_010
        assert_eq!(filled, Decimal::from(10_010));
    }

    #[test]
    fn fixed_bps_sell_decreases_price() {
        // 10 bps = 0.10%
        let model = SlippageModel::FixedBps(Decimal::from(10));
        let price = Decimal::from(10_000);
        let filled = model.apply(price, Side::Sell, None, None);
        // expected = 10_000 * 0.999 = 9_990
        assert_eq!(filled, Decimal::from(9_990));
    }

    #[test]
    fn fixed_bps_zero_returns_same_price() {
        let model = SlippageModel::FixedBps(Decimal::ZERO);
        let price = Decimal::from(50_000);
        assert_eq!(model.apply(price, Side::Buy, None, None), price);
        assert_eq!(model.apply(price, Side::Sell, None, None), price);
    }

    #[test]
    fn fixed_bps_5_buy() {
        // 5 bps = 0.05%
        let model = SlippageModel::FixedBps(Decimal::new(5, 0));
        let price = Decimal::from(100_000);
        let filled = model.apply(price, Side::Buy, None, None);
        // 100_000 * (1 + 0.0005) = 100_050
        assert_eq!(filled, Decimal::from(100_050));
    }

    #[test]
    fn fixed_bps_5_sell() {
        let model = SlippageModel::FixedBps(Decimal::new(5, 0));
        let price = Decimal::from(100_000);
        let filled = model.apply(price, Side::Sell, None, None);
        // 100_000 * (1 - 0.0005) = 99_950
        assert_eq!(filled, Decimal::from(99_950));
    }

    // -----------------------------------------------------------------------
    // VolumeImpact variant
    // -----------------------------------------------------------------------

    #[test]
    fn volume_impact_increases_with_larger_order() {
        let model = SlippageModel::VolumeImpact {
            base_bps: Decimal::from(2),    // 0.02% base
            impact_bps: Decimal::from(10), // 0.10% per unit ratio
        };
        let price = Decimal::from(10_000);
        let candle_volume = Some(Decimal::from(1_000_000));

        let small_order = Some(Decimal::from(10_000)); // 1% of candle
        let large_order = Some(Decimal::from(100_000)); // 10% of candle

        let small_fill = model.apply(price, Side::Buy, small_order, candle_volume);
        let large_fill = model.apply(price, Side::Buy, large_order, candle_volume);

        // Larger order should result in a worse (higher) fill price for buys
        assert!(large_fill > small_fill);
    }

    #[test]
    fn volume_impact_base_only_when_zero_order_volume() {
        let model = SlippageModel::VolumeImpact {
            base_bps: Decimal::from(5),
            impact_bps: Decimal::from(50),
        };
        let price = Decimal::from(10_000);
        // order_volume = 0 means no market impact, only base_bps applies
        let filled = model.apply(
            price,
            Side::Buy,
            Some(Decimal::ZERO),
            Some(Decimal::from(1_000_000)),
        );
        // base 5 bps: 10_000 * 1.0005 = 10_005
        assert_eq!(filled, Decimal::from(10_005));
    }

    #[test]
    fn volume_impact_zero_candle_volume_falls_back_to_base() {
        let model = SlippageModel::VolumeImpact {
            base_bps: Decimal::from(3),
            impact_bps: Decimal::from(100),
        };
        let price = Decimal::from(10_000);
        // candle_volume = 0 → ratio is 0, only base applies
        let filled = model.apply(
            price,
            Side::Buy,
            Some(Decimal::from(5_000)),
            Some(Decimal::ZERO),
        );
        // 3 bps: 10_000 * 1.0003 = 10_003
        assert_eq!(filled, Decimal::from(10_003));
    }

    #[test]
    fn volume_impact_none_volumes_fall_back_to_base() {
        let model = SlippageModel::VolumeImpact {
            base_bps: Decimal::from(4),
            impact_bps: Decimal::from(50),
        };
        let price = Decimal::from(10_000);
        let filled = model.apply(price, Side::Buy, None, None);
        // 4 bps: 10_000 * 1.0004 = 10_004
        assert_eq!(filled, Decimal::from(10_004));
    }

    #[test]
    fn volume_impact_sell_decreases_price() {
        let model = SlippageModel::VolumeImpact {
            base_bps: Decimal::from(5),
            impact_bps: Decimal::from(10),
        };
        let price = Decimal::from(10_000);
        // order = 10% of candle → ratio = 0.1, impact = 1 bps, total = 6 bps
        let filled = model.apply(
            price,
            Side::Sell,
            Some(Decimal::from(100_000)),
            Some(Decimal::from(1_000_000)),
        );
        // 6 bps → 10_000 * (1 - 0.0006) = 9_994
        assert_eq!(filled, Decimal::from(9_994));
    }

    // -----------------------------------------------------------------------
    // Default impl
    // -----------------------------------------------------------------------

    #[test]
    fn default_is_none() {
        let model = SlippageModel::default();
        let price = Decimal::from(50_000);
        assert_eq!(model.apply(price, Side::Buy, None, None), price);
        assert_eq!(model.apply(price, Side::Sell, None, None), price);
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn zero_price_stays_zero() {
        let model = SlippageModel::FixedBps(Decimal::from(10));
        assert_eq!(
            model.apply(Decimal::ZERO, Side::Buy, None, None),
            Decimal::ZERO
        );
    }

    #[test]
    fn volume_impact_both_volumes_none_returns_base_only() {
        let model = SlippageModel::VolumeImpact {
            base_bps: Decimal::ZERO,
            impact_bps: Decimal::from(100),
        };
        let price = Decimal::from(1_000);
        // Zero base + no volumes → no slippage
        assert_eq!(model.apply(price, Side::Buy, None, None), price);
    }
}
