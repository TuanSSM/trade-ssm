use anyhow::Result;
use ssm_core::{AIAction, Candle, Signal};
use ssm_indicators::cvd::{analyze_cvd, CvdTrend};

use crate::traits::Strategy;

/// Simple CVD momentum strategy — enters when CVD trend is clear.
/// This is the default bot strategy (no AI required).
pub struct CvdMomentumStrategy {
    window: usize,
    min_confidence: f64,
}

impl CvdMomentumStrategy {
    pub fn new(window: usize) -> Self {
        Self {
            window,
            min_confidence: 0.6,
        }
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
        let magnitude = cvd
            .total_cvd
            .abs()
            .to_string()
            .parse::<f64>()
            .unwrap_or(0.0);
        let confidence = (magnitude / self.window as f64).min(1.0);

        if confidence < self.min_confidence {
            return Ok(None);
        }

        let symbol = "BTCUSDT"; // Default; in real use, would come from context
        let last = candles.last().unwrap();

        Ok(Some(Signal {
            timestamp: last.close_time,
            symbol: symbol.into(),
            action,
            confidence,
            source: self.name().into(),
            metadata: std::collections::HashMap::new(),
        }))
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
}
