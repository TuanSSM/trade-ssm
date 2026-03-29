use anyhow::Result;
use ssm_ai::features::extract_features;
use ssm_ai::model::AIModel;
use ssm_core::{AIAction, Candle, Signal};

use crate::traits::Strategy;

/// Strategy that wraps an AI model to produce trading signals.
///
/// Bridges the `AIModel` trait to the `Strategy` trait.
pub struct AiStrategy {
    model: Box<dyn AIModel>,
    cvd_window: usize,
    min_confidence: f64,
}

impl AiStrategy {
    pub fn new(model: Box<dyn AIModel>, cvd_window: usize) -> Self {
        Self {
            model,
            cvd_window,
            min_confidence: 0.0,
        }
    }

    pub fn with_min_confidence(mut self, c: f64) -> Self {
        self.min_confidence = c;
        self
    }
}

impl Strategy for AiStrategy {
    fn name(&self) -> &str {
        self.model.name()
    }

    fn analyze(&self, candles: &[Candle]) -> Result<Option<Signal>> {
        if candles.len() < self.cvd_window {
            return Ok(None);
        }

        let features = extract_features(candles, self.cvd_window);
        let last_feature = match features.last() {
            Some(f) => f,
            None => return Ok(None),
        };

        let action = self.model.predict(last_feature)?;
        if action == AIAction::Neutral {
            return Ok(None);
        }

        let last = match candles.last() {
            Some(c) => c,
            None => return Ok(None),
        };
        Ok(Some(Signal {
            timestamp: last.close_time,
            symbol: String::new(),
            action,
            confidence: 1.0,
            source: format!("ai:{}", self.model.name()),
            metadata: std::collections::HashMap::new(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use ssm_ai::model::StubModel;

    fn dummy_candle() -> Candle {
        Candle {
            open_time: 0,
            open: Decimal::from(100),
            high: Decimal::from(105),
            low: Decimal::from(95),
            close: Decimal::from(102),
            volume: Decimal::from(100),
            close_time: 1000,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: Decimal::from(60),
            taker_sell_volume: Decimal::from(40),
        }
    }

    #[test]
    fn stub_model_returns_none() {
        // StubModel always predicts Neutral → strategy returns None
        let strategy = AiStrategy::new(Box::new(StubModel), 5);
        let candles: Vec<_> = (0..10).map(|_| dummy_candle()).collect();
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn name_from_model() {
        let strategy = AiStrategy::new(Box::new(StubModel), 5);
        assert_eq!(strategy.name(), "stub");
    }

    #[test]
    fn test_insufficient_candles() {
        let strategy = AiStrategy::new(Box::new(StubModel), 20);
        let candles: Vec<_> = (0..5).map(|_| dummy_candle()).collect();
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn test_min_confidence_setter() {
        let strategy = AiStrategy::new(Box::new(StubModel), 5).with_min_confidence(0.8);
        assert!((strategy.min_confidence - 0.8).abs() < f64::EPSILON);
    }

    /// A model that always predicts EnterLong.
    struct AlwaysLongModel;
    impl AIModel for AlwaysLongModel {
        fn name(&self) -> &str {
            "always_long"
        }
        fn predict(&self, _features: &ssm_core::FeatureRow) -> anyhow::Result<AIAction> {
            Ok(AIAction::EnterLong)
        }
        fn train(
            &mut self,
            data: &[ssm_core::FeatureRow],
        ) -> anyhow::Result<ssm_ai::model::TrainMetrics> {
            Ok(ssm_ai::model::TrainMetrics {
                model_name: "always_long".into(),
                samples: data.len(),
                accuracy: 1.0,
                loss: 0.0,
            })
        }
        fn save(&self, _path: &std::path::Path) -> anyhow::Result<()> {
            Ok(())
        }
        fn load(&mut self, _path: &std::path::Path) -> anyhow::Result<()> {
            Ok(())
        }
    }

    /// A model that always predicts EnterShort.
    struct AlwaysShortModel;
    impl AIModel for AlwaysShortModel {
        fn name(&self) -> &str {
            "always_short"
        }
        fn predict(&self, _features: &ssm_core::FeatureRow) -> anyhow::Result<AIAction> {
            Ok(AIAction::EnterShort)
        }
        fn train(
            &mut self,
            data: &[ssm_core::FeatureRow],
        ) -> anyhow::Result<ssm_ai::model::TrainMetrics> {
            Ok(ssm_ai::model::TrainMetrics {
                model_name: "always_short".into(),
                samples: data.len(),
                accuracy: 1.0,
                loss: 0.0,
            })
        }
        fn save(&self, _path: &std::path::Path) -> anyhow::Result<()> {
            Ok(())
        }
        fn load(&mut self, _path: &std::path::Path) -> anyhow::Result<()> {
            Ok(())
        }
    }

    /// A model that always predicts ExitLong.
    struct ExitLongModel;
    impl AIModel for ExitLongModel {
        fn name(&self) -> &str {
            "exit_long_model"
        }
        fn predict(&self, _features: &ssm_core::FeatureRow) -> anyhow::Result<AIAction> {
            Ok(AIAction::ExitLong)
        }
        fn train(
            &mut self,
            data: &[ssm_core::FeatureRow],
        ) -> anyhow::Result<ssm_ai::model::TrainMetrics> {
            Ok(ssm_ai::model::TrainMetrics {
                model_name: "exit_long_model".into(),
                samples: data.len(),
                accuracy: 1.0,
                loss: 0.0,
            })
        }
        fn save(&self, _path: &std::path::Path) -> anyhow::Result<()> {
            Ok(())
        }
        fn load(&mut self, _path: &std::path::Path) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn enter_long_model_produces_enter_long_signal() {
        let strategy = AiStrategy::new(Box::new(AlwaysLongModel), 5);
        let candles: Vec<_> = (0..10).map(|_| dummy_candle()).collect();
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.action, AIAction::EnterLong);
    }

    #[test]
    fn enter_short_model_produces_enter_short_signal() {
        let strategy = AiStrategy::new(Box::new(AlwaysShortModel), 5);
        let candles: Vec<_> = (0..10).map(|_| dummy_candle()).collect();
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.action, AIAction::EnterShort);
    }

    #[test]
    fn exit_long_model_produces_exit_long_signal() {
        let strategy = AiStrategy::new(Box::new(ExitLongModel), 5);
        let candles: Vec<_> = (0..10).map(|_| dummy_candle()).collect();
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.action, AIAction::ExitLong);
    }

    #[test]
    fn name_returns_model_name() {
        let strategy = AiStrategy::new(Box::new(AlwaysLongModel), 5);
        assert_eq!(strategy.name(), "always_long");
    }

    #[test]
    fn signal_source_includes_ai_prefix() {
        let strategy = AiStrategy::new(Box::new(AlwaysLongModel), 5);
        let candles: Vec<_> = (0..10).map(|_| dummy_candle()).collect();
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.source, "ai:always_long");
    }

    #[test]
    fn signal_confidence_is_one() {
        let strategy = AiStrategy::new(Box::new(AlwaysLongModel), 5);
        let candles: Vec<_> = (0..10).map(|_| dummy_candle()).collect();
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert!((signal.confidence - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn insufficient_candles_for_large_window() {
        let strategy = AiStrategy::new(Box::new(AlwaysLongModel), 50);
        let candles: Vec<_> = (0..10).map(|_| dummy_candle()).collect();
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn exact_minimum_candles_produces_signal() {
        let strategy = AiStrategy::new(Box::new(AlwaysLongModel), 5);
        let candles: Vec<_> = (0..5).map(|_| dummy_candle()).collect();
        let result = strategy.analyze(&candles).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn one_fewer_than_minimum_returns_none() {
        let strategy = AiStrategy::new(Box::new(AlwaysLongModel), 5);
        let candles: Vec<_> = (0..4).map(|_| dummy_candle()).collect();
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn empty_candles_returns_none() {
        let strategy = AiStrategy::new(Box::new(AlwaysLongModel), 5);
        assert!(strategy.analyze(&[]).unwrap().is_none());
    }

    #[test]
    fn signal_timestamp_from_last_candle() {
        let strategy = AiStrategy::new(Box::new(AlwaysLongModel), 5);
        let candles: Vec<_> = (0..10).map(|_| dummy_candle()).collect();
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.timestamp, 1000); // close_time from dummy_candle
    }

    #[test]
    fn neutral_model_returns_none_with_sufficient_candles() {
        // StubModel always returns Neutral, so even with enough candles we get None
        let strategy = AiStrategy::new(Box::new(StubModel), 5);
        let candles: Vec<_> = (0..50).map(|_| dummy_candle()).collect();
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }
}
