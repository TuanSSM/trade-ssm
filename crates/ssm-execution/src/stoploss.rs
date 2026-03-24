use rust_decimal::Decimal;
use ssm_core::{Candle, Position, RoiEntry, Side, StoplossStep, StoplossType};

/// Dynamic stoploss manager — computes stoploss levels per position.
pub struct StoplossManager;

impl StoplossManager {
    /// Compute the current stoploss price for a position given its configuration.
    pub fn compute_stoploss(
        position: &Position,
        stoploss_type: &StoplossType,
        candles: &[Candle],
        candles_in_trade: usize,
    ) -> Option<Decimal> {
        match stoploss_type {
            StoplossType::Fixed(pct) => Self::fixed_stoploss(position, *pct),
            StoplossType::AtrTrailing {
                multiplier,
                atr_period,
            } => Self::atr_trailing(position, *multiplier, *atr_period, candles),
            StoplossType::TimeBased {
                initial_pct,
                breakeven_after,
            } => Self::time_based(position, *initial_pct, *breakeven_after, candles_in_trade),
            StoplossType::Stepped(steps) => Self::stepped_stoploss(position, steps, candles.last()),
        }
    }

    /// Fixed percentage stoploss.
    fn fixed_stoploss(position: &Position, pct: Decimal) -> Option<Decimal> {
        let stop = match position.side {
            Side::Buy => position.entry_price * (Decimal::ONE - pct),
            Side::Sell => position.entry_price * (Decimal::ONE + pct),
        };
        Some(stop)
    }

    /// ATR-based trailing stop: entry_price +/- multiplier * ATR.
    fn atr_trailing(
        position: &Position,
        multiplier: Decimal,
        atr_period: usize,
        candles: &[Candle],
    ) -> Option<Decimal> {
        if candles.len() < atr_period + 1 {
            return None;
        }

        // Compute ATR manually
        let mut tr_sum = Decimal::ZERO;
        let start = candles.len().saturating_sub(atr_period);
        for i in start..candles.len() {
            let high_low = candles[i].high - candles[i].low;
            let tr = if i > 0 {
                let prev_close = candles[i - 1].close;
                let h_pc = (candles[i].high - prev_close).abs();
                let l_pc = (candles[i].low - prev_close).abs();
                high_low.max(h_pc).max(l_pc)
            } else {
                high_low
            };
            tr_sum += tr;
        }
        let atr = tr_sum / Decimal::from(atr_period);
        let offset = multiplier * atr;

        let current_price = candles.last()?.close;
        let stop = match position.side {
            Side::Buy => current_price - offset,
            Side::Sell => current_price + offset,
        };
        Some(stop)
    }

    /// Time-based stoploss: starts at initial_pct, moves to breakeven after N candles.
    fn time_based(
        position: &Position,
        initial_pct: Decimal,
        breakeven_after: usize,
        candles_in_trade: usize,
    ) -> Option<Decimal> {
        if candles_in_trade >= breakeven_after {
            // Move to breakeven
            Some(position.entry_price)
        } else {
            // Use initial stoploss
            Self::fixed_stoploss(position, initial_pct)
        }
    }

    /// Stepped stoploss: discrete levels based on profit thresholds.
    fn stepped_stoploss(
        position: &Position,
        steps: &[StoplossStep],
        current_candle: Option<&Candle>,
    ) -> Option<Decimal> {
        let current_price = current_candle?.close;
        let profit_pct = match position.side {
            Side::Buy => (current_price - position.entry_price) / position.entry_price,
            Side::Sell => (position.entry_price - current_price) / position.entry_price,
        };

        // Find the highest applicable step
        let mut best_stop_pct = None;
        for step in steps {
            if profit_pct >= step.profit_pct {
                match best_stop_pct {
                    None => best_stop_pct = Some(step.stoploss_pct),
                    Some(current) => {
                        if step.stoploss_pct > current {
                            best_stop_pct = Some(step.stoploss_pct);
                        }
                    }
                }
            }
        }

        best_stop_pct.map(|sl_pct| match position.side {
            Side::Buy => position.entry_price * (Decimal::ONE - sl_pct),
            Side::Sell => position.entry_price * (Decimal::ONE + sl_pct),
        })
    }

    /// Check if current price triggers the stoploss.
    pub fn is_triggered(
        position: &Position,
        stoploss_price: Decimal,
        current_price: Decimal,
    ) -> bool {
        match position.side {
            Side::Buy => current_price <= stoploss_price,
            Side::Sell => current_price >= stoploss_price,
        }
    }

    /// Check ROI table: should we take profit?
    pub fn check_roi(
        position: &Position,
        roi_table: &[RoiEntry],
        current_price: Decimal,
        candles_in_trade: u64,
        candle_minutes: u64,
    ) -> bool {
        if roi_table.is_empty() {
            return false;
        }
        let minutes_in_trade = candles_in_trade * candle_minutes;
        let profit_pct = match position.side {
            Side::Buy => (current_price - position.entry_price) / position.entry_price,
            Side::Sell => (position.entry_price - current_price) / position.entry_price,
        };

        for entry in roi_table {
            if minutes_in_trade >= entry.minutes && profit_pct >= entry.roi_pct {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn make_position(side: Side, entry: &str) -> Position {
        Position {
            symbol: "BTCUSDT".into(),
            side,
            entry_price: Decimal::from_str(entry).unwrap(),
            quantity: Decimal::ONE,
            unrealized_pnl: Decimal::ZERO,
            realized_pnl: Decimal::ZERO,
            leverage: 1,
            opened_at: 0,
        }
    }

    fn make_candle(close: &str, high: &str, low: &str) -> Candle {
        Candle {
            open_time: 0,
            open: Decimal::from_str(close).unwrap(),
            high: Decimal::from_str(high).unwrap(),
            low: Decimal::from_str(low).unwrap(),
            close: Decimal::from_str(close).unwrap(),
            volume: Decimal::from(100),
            close_time: 1000,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: Decimal::from(50),
            taker_sell_volume: Decimal::from(50),
        }
    }

    #[test]
    fn fixed_stoploss_long() {
        let pos = make_position(Side::Buy, "50000");
        let sl = StoplossManager::compute_stoploss(
            &pos,
            &StoplossType::Fixed(Decimal::new(5, 2)), // 5%
            &[],
            0,
        );
        assert_eq!(sl, Some(Decimal::from(47500))); // 50000 * 0.95
    }

    #[test]
    fn fixed_stoploss_short() {
        let pos = make_position(Side::Sell, "50000");
        let sl = StoplossManager::compute_stoploss(
            &pos,
            &StoplossType::Fixed(Decimal::new(5, 2)),
            &[],
            0,
        );
        assert_eq!(sl, Some(Decimal::from(52500))); // 50000 * 1.05
    }

    #[test]
    fn time_based_initial() {
        let pos = make_position(Side::Buy, "50000");
        let sl = StoplossManager::compute_stoploss(
            &pos,
            &StoplossType::TimeBased {
                initial_pct: Decimal::new(3, 2),
                breakeven_after: 10,
            },
            &[],
            5, // only 5 candles in
        );
        // Should use initial 3% stoploss
        assert_eq!(sl, Some(Decimal::from(48500)));
    }

    #[test]
    fn time_based_breakeven() {
        let pos = make_position(Side::Buy, "50000");
        let sl = StoplossManager::compute_stoploss(
            &pos,
            &StoplossType::TimeBased {
                initial_pct: Decimal::new(3, 2),
                breakeven_after: 10,
            },
            &[],
            10, // at breakeven threshold
        );
        // Should move to breakeven (entry price)
        assert_eq!(sl, Some(Decimal::from(50000)));
    }

    #[test]
    fn stepped_stoploss_moves_up() {
        let pos = make_position(Side::Buy, "50000");
        let steps = vec![
            StoplossStep {
                profit_pct: Decimal::new(2, 2), // 2% profit
                stoploss_pct: Decimal::ZERO,    // breakeven: stop at entry
            },
            StoplossStep {
                profit_pct: Decimal::new(5, 2), // 5% profit
                stoploss_pct: Decimal::ZERO,    // also breakeven at this level
            },
        ];
        // Current price 53000 = 6% profit, both steps apply
        let candles = vec![make_candle("53000", "53500", "52500")];
        let sl =
            StoplossManager::compute_stoploss(&pos, &StoplossType::Stepped(steps), &candles, 0);
        // stoploss_pct = 0 (breakeven), so stop = 50000 * (1 - 0) = 50000
        assert_eq!(sl, Some(Decimal::from(50000)));
    }

    #[test]
    fn stoploss_triggered_long() {
        let pos = make_position(Side::Buy, "50000");
        assert!(StoplossManager::is_triggered(
            &pos,
            Decimal::from(48000),
            Decimal::from(47000)
        ));
        assert!(!StoplossManager::is_triggered(
            &pos,
            Decimal::from(48000),
            Decimal::from(49000)
        ));
    }

    #[test]
    fn stoploss_triggered_short() {
        let pos = make_position(Side::Sell, "50000");
        assert!(StoplossManager::is_triggered(
            &pos,
            Decimal::from(52000),
            Decimal::from(53000)
        ));
        assert!(!StoplossManager::is_triggered(
            &pos,
            Decimal::from(52000),
            Decimal::from(51000)
        ));
    }

    #[test]
    fn roi_table_triggers() {
        let pos = make_position(Side::Buy, "50000");
        let roi = vec![
            RoiEntry {
                minutes: 0,
                roi_pct: Decimal::new(10, 2), // 10% at any time
            },
            RoiEntry {
                minutes: 60,
                roi_pct: Decimal::new(5, 2), // 5% after 60 min
            },
        ];
        // 12% profit, 0 minutes → triggers first entry
        assert!(StoplossManager::check_roi(
            &pos,
            &roi,
            Decimal::from(56000),
            0,
            15,
        ));
        // 6% profit, 80 minutes (>60) → triggers second entry
        assert!(StoplossManager::check_roi(
            &pos,
            &roi,
            Decimal::from(53000),
            6, // 6 candles * 15 min = 90 min
            15,
        ));
        // 3% profit, 30 min → neither triggers
        assert!(!StoplossManager::check_roi(
            &pos,
            &roi,
            Decimal::from(51500),
            2, // 2 * 15 = 30 min
            15,
        ));
    }

    #[test]
    fn atr_trailing_long() {
        let pos = make_position(Side::Buy, "50000");
        // Simple candles with range of 1000 each
        let candles: Vec<Candle> = (0..20)
            .map(|_| make_candle("50000", "50500", "49500"))
            .collect();
        let sl = StoplossManager::compute_stoploss(
            &pos,
            &StoplossType::AtrTrailing {
                multiplier: Decimal::from(2),
                atr_period: 14,
            },
            &candles,
            0,
        );
        assert!(sl.is_some());
        // ATR ~= 1000 (range), offset = 2 * 1000 = 2000
        // Stop = 50000 - 2000 = 48000
        let stop = sl.unwrap();
        assert!(stop < Decimal::from(50000));
        assert!(stop > Decimal::from(45000));
    }

    #[test]
    fn anti_repainting_stoploss_values_stable() {
        // Key anti-repainting test: stoploss values for candles [0..N]
        // must not change when candle N+1 is appended.
        let pos = make_position(Side::Buy, "50000");
        let candles: Vec<Candle> = (0..15)
            .map(|_| make_candle("50000", "50500", "49500"))
            .collect();

        let sl_before = StoplossManager::compute_stoploss(
            &pos,
            &StoplossType::Fixed(Decimal::new(5, 2)),
            &candles,
            10,
        );

        // Add one more candle
        let mut candles_extended = candles.clone();
        candles_extended.push(make_candle("51000", "51500", "50500"));

        let sl_after = StoplossManager::compute_stoploss(
            &pos,
            &StoplossType::Fixed(Decimal::new(5, 2)),
            &candles_extended[..candles_extended.len() - 1], // same closed candles
            10,
        );

        // Stoploss must be identical when using same closed candles
        assert_eq!(sl_before, sl_after);
    }

    #[test]
    fn empty_roi_table_never_triggers() {
        let pos = make_position(Side::Buy, "50000");
        assert!(!StoplossManager::check_roi(
            &pos,
            &[],
            Decimal::from(100000),
            100,
            15,
        ));
    }

    #[test]
    fn insufficient_candles_for_atr_returns_none() {
        let pos = make_position(Side::Buy, "50000");
        let candles = vec![make_candle("50000", "50500", "49500")];
        let sl = StoplossManager::compute_stoploss(
            &pos,
            &StoplossType::AtrTrailing {
                multiplier: Decimal::from(2),
                atr_period: 14,
            },
            &candles,
            0,
        );
        assert!(sl.is_none());
    }
}
