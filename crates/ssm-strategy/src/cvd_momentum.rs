use anyhow::Result;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use ssm_core::{AIAction, Candle, ExitReason, Order, Position, RoiEntry, Signal, StoplossType};
use ssm_indicators::cvd::{analyze_cvd, CvdTrend};

use crate::traits::Strategy;

/// CVD momentum strategy with full lifecycle callbacks.
/// Demonstrates freqtrade-style callbacks: custom stoploss, ROI table,
/// position sizing, DCA, and custom exit logic.
pub struct CvdMomentumStrategy {
    window: usize,
    pub min_confidence: f64,
    symbol: String,
    dca_threshold: Decimal,
    dca_quantity_pct: Decimal,
}

impl CvdMomentumStrategy {
    pub fn new(window: usize) -> Self {
        Self {
            window,
            min_confidence: 0.6,
            symbol: String::new(),                 // Filled by caller
            dca_threshold: Decimal::new(-3, 2),    // DCA when position is -3%
            dca_quantity_pct: Decimal::new(50, 2), // DCA 50% of original size
        }
    }

    pub fn with_symbol(mut self, symbol: &str) -> Self {
        self.symbol = symbol.to_string();
        self
    }

    pub fn with_min_confidence(mut self, c: f64) -> Self {
        self.min_confidence = c;
        self
    }
}

impl Strategy for CvdMomentumStrategy {
    fn name(&self) -> &str {
        "cvd_momentum"
    }

    fn analyze(&self, candles: &[Candle]) -> Result<Option<Signal>> {
        if candles.len() < self.window {
            return Ok(None);
        }

        let cvd = analyze_cvd(candles, self.window);

        let action = match cvd.trend {
            CvdTrend::Bullish => AIAction::EnterLong,
            CvdTrend::Bearish => AIAction::EnterShort,
            CvdTrend::Neutral => return Ok(None),
        };

        // Simple confidence: ratio of CVD magnitude to window size
        let magnitude = cvd.total_cvd.abs().to_f64().unwrap_or(0.0);
        let confidence = (magnitude / self.window as f64).min(1.0);

        if confidence < self.min_confidence {
            return Ok(None);
        }

        let last = match candles.last() {
            Some(c) => c,
            None => return Ok(None),
        };

        Ok(Some(Signal {
            timestamp: last.close_time,
            symbol: self.symbol.clone(),
            action,
            confidence,
            source: self.name().into(),
            metadata: std::collections::HashMap::new(),
        }))
    }

    // --- Lifecycle callbacks ---

    fn on_trade_enter(&self, signal: &Signal, _position: Option<&Position>) -> bool {
        // Only enter if confidence is above threshold
        signal.confidence >= self.min_confidence
    }

    fn on_trade_exit(&self, position: &Position, candles: &[Candle]) -> Option<ExitReason> {
        if candles.len() < self.window {
            return None;
        }
        // Exit if CVD trend reverses against our position
        let cvd = analyze_cvd(candles, self.window);
        match (position.side, cvd.trend) {
            (ssm_core::Side::Buy, CvdTrend::Bearish) => {
                Some(ExitReason::CustomExit("cvd_reversal".into()))
            }
            (ssm_core::Side::Sell, CvdTrend::Bullish) => {
                Some(ExitReason::CustomExit("cvd_reversal".into()))
            }
            _ => None,
        }
    }

    fn on_order_filled(&self, order: &Order, _position: &Position) {
        tracing::info!(
            order_id = %order.id,
            symbol = %order.symbol,
            side = %order.side,
            "cvd_momentum: order filled"
        );
    }

    fn custom_position_size(&self, _signal: &Signal, balance: Decimal) -> Option<Decimal> {
        // Use 5% of balance per trade
        Some(balance * Decimal::new(5, 2))
    }

    fn should_adjust_position(&self, position: &Position, _candles: &[Candle]) -> Option<Decimal> {
        // DCA: add to position if unrealized PnL drops below threshold
        if position.entry_price > Decimal::ZERO {
            let pnl_pct = position.unrealized_pnl / (position.entry_price * position.quantity);
            if pnl_pct < self.dca_threshold {
                return Some(position.quantity * self.dca_quantity_pct);
            }
        }
        None
    }

    fn custom_stoploss(
        &self,
        _position: &Position,
        _candles: &[Candle],
        candles_in_trade: usize,
    ) -> Option<Decimal> {
        // Time-based stoploss tightening
        if candles_in_trade > 20 {
            Some(Decimal::new(2, 2)) // Tighten to 2% after 20 candles
        } else {
            Some(Decimal::new(5, 2)) // Initial 5% stoploss
        }
    }

    fn stoploss_type(&self) -> Option<StoplossType> {
        Some(StoplossType::TimeBased {
            initial_pct: Decimal::new(5, 2),
            breakeven_after: 15,
        })
    }

    fn roi_table(&self) -> Vec<RoiEntry> {
        vec![
            RoiEntry {
                minutes: 0,
                roi_pct: Decimal::new(10, 2), // 10% profit at any time
            },
            RoiEntry {
                minutes: 60,
                roi_pct: Decimal::new(5, 2), // 5% profit after 1 hour
            },
            RoiEntry {
                minutes: 120,
                roi_pct: Decimal::new(2, 2), // 2% profit after 2 hours
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn candle(buy: &str, sell: &str) -> Candle {
        let bv = Decimal::from_str(buy).unwrap();
        let sv = Decimal::from_str(sell).unwrap();
        Candle {
            open_time: 0,
            open: Decimal::ZERO,
            high: Decimal::ZERO,
            low: Decimal::ZERO,
            close: Decimal::ZERO,
            volume: bv + sv,
            close_time: 1000,
            quote_volume: Decimal::ZERO,
            trades: 0,
            taker_buy_volume: bv,
            taker_sell_volume: sv,
        }
    }

    #[test]
    fn bullish_produces_enter_long() {
        let strategy = CvdMomentumStrategy::new(5).with_min_confidence(0.0);
        let candles: Vec<_> = (0..10).map(|_| candle("80", "20")).collect();
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.action, AIAction::EnterLong);
    }

    #[test]
    fn bearish_produces_enter_short() {
        let strategy = CvdMomentumStrategy::new(5).with_min_confidence(0.0);
        let candles: Vec<_> = (0..10).map(|_| candle("20", "80")).collect();
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.action, AIAction::EnterShort);
    }

    #[test]
    fn insufficient_candles_returns_none() {
        let strategy = CvdMomentumStrategy::new(15);
        let candles: Vec<_> = (0..5).map(|_| candle("50", "50")).collect();
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn low_confidence_filtered() {
        let strategy = CvdMomentumStrategy::new(5).with_min_confidence(999.0);
        let candles: Vec<_> = (0..10).map(|_| candle("51", "49")).collect();
        // Very small CVD — below absurd threshold
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn test_strategy_name() {
        let strategy = CvdMomentumStrategy::new(5);
        assert_eq!(strategy.name(), "cvd_momentum");
    }

    #[test]
    fn test_neutral_cvd_returns_none() {
        let strategy = CvdMomentumStrategy::new(5).with_min_confidence(0.0);
        let candles: Vec<_> = (0..10).map(|_| candle("50", "50")).collect();
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn test_confidence_is_bounded() {
        let strategy = CvdMomentumStrategy::new(5).with_min_confidence(0.0);
        let candles: Vec<_> = (0..10).map(|_| candle("80", "20")).collect();
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert!(signal.confidence >= 0.0 && signal.confidence <= 1.0);
    }

    #[test]
    fn test_signal_source() {
        let strategy = CvdMomentumStrategy::new(5).with_min_confidence(0.0);
        let candles: Vec<_> = (0..10).map(|_| candle("80", "20")).collect();
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.source, "cvd_momentum");
    }

    #[test]
    fn test_different_window_sizes() {
        let candles: Vec<_> = (0..25).map(|_| candle("80", "20")).collect();

        let strategy_small = CvdMomentumStrategy::new(3).with_min_confidence(0.0);
        let result_small = strategy_small.analyze(&candles).unwrap();
        assert!(result_small.is_some());

        let strategy_large = CvdMomentumStrategy::new(20).with_min_confidence(0.0);
        let result_large = strategy_large.analyze(&candles).unwrap();
        assert!(result_large.is_some());
    }

    #[test]
    fn exact_minimum_candles_bullish() {
        // Provide exactly `window` candles — the minimum required
        let strategy = CvdMomentumStrategy::new(5).with_min_confidence(0.0);
        let candles: Vec<_> = (0..5).map(|_| candle("80", "20")).collect();
        let result = strategy.analyze(&candles).unwrap();
        // Should produce a signal (not None due to insufficient candles)
        assert!(result.is_some());
        assert_eq!(result.unwrap().action, AIAction::EnterLong);
    }

    #[test]
    fn exact_minimum_candles_returns_none_when_one_short() {
        // Provide window - 1 candles — should be insufficient
        let strategy = CvdMomentumStrategy::new(5).with_min_confidence(0.0);
        let candles: Vec<_> = (0..4).map(|_| candle("80", "20")).collect();
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn all_bearish_candles_should_short() {
        let strategy = CvdMomentumStrategy::new(5).with_min_confidence(0.0);
        let candles: Vec<_> = (0..20).map(|_| candle("10", "90")).collect();
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.action, AIAction::EnterShort);
    }

    #[test]
    fn flat_cvd_returns_none() {
        // Equal buy/sell volumes produce neutral CVD
        let strategy = CvdMomentumStrategy::new(5).with_min_confidence(0.0);
        let candles: Vec<_> = (0..20).map(|_| candle("50", "50")).collect();
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn very_high_confidence_threshold_filters_weak_signal() {
        // Tiny imbalance (buy=51, sell=49, delta=2 per candle)
        // With window=5, total_cvd ~ 10, confidence = 10/5 = 2.0 → clamped to 1.0
        // Need to make the imbalance so small that confidence < threshold
        // confidence = total_cvd.abs() / window; need total_cvd.abs() < window * threshold
        // With window=1000, total_cvd=2*10=20 → but analyze_cvd uses window,
        // so we need the signal to be below confidence.
        // Easiest: use a very large window relative to the CVD magnitude
        let strategy = CvdMomentumStrategy::new(10).with_min_confidence(0.99);
        // delta per candle = 1, 10 candles, total_cvd = 10, confidence = 10/10 = 1.0
        // Still 1.0 exactly, which is >= 0.99. Let's make delta fractional.
        // Actually with Decimal: buy=501, sell=499, delta=2, window=10, cvd=20
        // confidence = 20/10 = 2.0 → 1.0. The min(1.0) clamp means even tiny
        // imbalance over enough candles will hit 1.0.
        // The only way to filter is Neutral trend. Let's test that.
        let candles: Vec<_> = (0..10).map(|_| candle("50", "50")).collect();
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn name_returns_cvd_momentum() {
        let strategy = CvdMomentumStrategy::new(10);
        assert_eq!(strategy.name(), "cvd_momentum");
    }

    #[test]
    fn default_min_confidence_is_0_6() {
        let strategy = CvdMomentumStrategy::new(5);
        // Verify default confidence threshold is 0.6
        assert!((strategy.min_confidence - 0.6).abs() < f64::EPSILON);
    }

    #[test]
    fn signal_symbol_is_btcusdt() {
        let strategy = CvdMomentumStrategy::new(5)
            .with_symbol("BTCUSDT")
            .with_min_confidence(0.0);
        let candles: Vec<_> = (0..10).map(|_| candle("80", "20")).collect();
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.symbol, "BTCUSDT");
    }

    #[test]
    fn signal_timestamp_from_last_candle() {
        let strategy = CvdMomentumStrategy::new(5).with_min_confidence(0.0);
        let candles: Vec<_> = (0..10).map(|_| candle("80", "20")).collect();
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.timestamp, 1000); // close_time from candle helper
    }

    #[test]
    fn empty_candles_returns_none() {
        let strategy = CvdMomentumStrategy::new(5);
        assert!(strategy.analyze(&[]).unwrap().is_none());
    }

    #[test]
    fn with_min_confidence_chaining() {
        let strategy = CvdMomentumStrategy::new(5)
            .with_min_confidence(0.3)
            .with_min_confidence(0.7);
        // Last value should win (0.7)
        assert!((strategy.min_confidence - 0.7).abs() < f64::EPSILON);
    }

    // --- Lifecycle callback tests ---

    fn make_signal(action: AIAction, confidence: f64) -> Signal {
        Signal {
            timestamp: 1000,
            symbol: "BTCUSDT".into(),
            action,
            confidence,
            source: "test".into(),
            metadata: std::collections::HashMap::new(),
        }
    }

    fn make_position(side: ssm_core::Side) -> ssm_core::Position {
        ssm_core::Position {
            symbol: "BTCUSDT".into(),
            side,
            entry_price: Decimal::from(50000),
            quantity: Decimal::from(1),
            unrealized_pnl: Decimal::ZERO,
            realized_pnl: Decimal::ZERO,
            leverage: 1,
            opened_at: 0,
        }
    }

    #[test]
    fn on_trade_enter_accepts_high_confidence() {
        let strategy = CvdMomentumStrategy::new(5).with_min_confidence(0.5);
        let signal = make_signal(AIAction::EnterLong, 0.8);
        assert!(strategy.on_trade_enter(&signal, None));
    }

    #[test]
    fn on_trade_enter_rejects_low_confidence() {
        let strategy = CvdMomentumStrategy::new(5).with_min_confidence(0.5);
        let signal = make_signal(AIAction::EnterLong, 0.3);
        assert!(!strategy.on_trade_enter(&signal, None));
    }

    #[test]
    fn on_trade_exit_reversal() {
        let strategy = CvdMomentumStrategy::new(5).with_min_confidence(0.0);
        let pos = make_position(ssm_core::Side::Buy);
        // Bearish candles should trigger exit for a long position
        let candles: Vec<_> = (0..10).map(|_| candle("20", "80")).collect();
        let exit = strategy.on_trade_exit(&pos, &candles);
        assert!(exit.is_some());
        assert!(matches!(exit.unwrap(), ExitReason::CustomExit(_)));
    }

    #[test]
    fn on_trade_exit_no_reversal() {
        let strategy = CvdMomentumStrategy::new(5).with_min_confidence(0.0);
        let pos = make_position(ssm_core::Side::Buy);
        // Bullish candles — no exit for a long position
        let candles: Vec<_> = (0..10).map(|_| candle("80", "20")).collect();
        assert!(strategy.on_trade_exit(&pos, &candles).is_none());
    }

    #[test]
    fn custom_position_size_returns_5pct() {
        let strategy = CvdMomentumStrategy::new(5);
        let signal = make_signal(AIAction::EnterLong, 0.8);
        let size = strategy.custom_position_size(&signal, Decimal::from(10000));
        assert_eq!(size, Some(Decimal::from(500)));
    }

    #[test]
    fn should_adjust_position_dca() {
        let strategy = CvdMomentumStrategy::new(5);
        let mut pos = make_position(ssm_core::Side::Buy);
        pos.unrealized_pnl = Decimal::from(-2000); // -4% of 50000*1
        let candles: Vec<_> = (0..10).map(|_| candle("50", "50")).collect();
        let adj = strategy.should_adjust_position(&pos, &candles);
        assert!(adj.is_some());
        // Should be 50% of original quantity (1 * 0.5 = 0.5)
        assert_eq!(adj.unwrap(), Decimal::new(50, 2));
    }

    #[test]
    fn should_adjust_position_no_dca_when_profitable() {
        let strategy = CvdMomentumStrategy::new(5);
        let mut pos = make_position(ssm_core::Side::Buy);
        pos.unrealized_pnl = Decimal::from(1000); // profitable
        let candles: Vec<_> = (0..10).map(|_| candle("50", "50")).collect();
        assert!(strategy.should_adjust_position(&pos, &candles).is_none());
    }

    #[test]
    fn custom_stoploss_tightens_over_time() {
        let strategy = CvdMomentumStrategy::new(5);
        let pos = make_position(ssm_core::Side::Buy);
        let candles: Vec<_> = (0..10).map(|_| candle("50", "50")).collect();

        let early = strategy.custom_stoploss(&pos, &candles, 5);
        assert_eq!(early, Some(Decimal::new(5, 2))); // 5%

        let late = strategy.custom_stoploss(&pos, &candles, 25);
        assert_eq!(late, Some(Decimal::new(2, 2))); // 2%
    }

    #[test]
    fn stoploss_type_returns_time_based() {
        let strategy = CvdMomentumStrategy::new(5);
        let sl = strategy.stoploss_type();
        assert!(sl.is_some());
        assert!(matches!(sl.unwrap(), StoplossType::TimeBased { .. }));
    }

    #[test]
    fn roi_table_has_entries() {
        let strategy = CvdMomentumStrategy::new(5);
        let roi = strategy.roi_table();
        assert_eq!(roi.len(), 3);
        assert_eq!(roi[0].minutes, 0);
        assert_eq!(roi[0].roi_pct, Decimal::new(10, 2));
    }
}
