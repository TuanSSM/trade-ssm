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

        let last = candles.last().unwrap();
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
}
