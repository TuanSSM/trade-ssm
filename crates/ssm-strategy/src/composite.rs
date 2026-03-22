use anyhow::Result;
use ssm_core::{AIAction, Candle, Signal};

use crate::traits::Strategy;

/// A weighted strategy entry for the composite.
struct WeightedStrategy {
    strategy: Box<dyn Strategy>,
    weight: f64,
}

/// Composite strategy — combines multiple strategies with weighted voting.
///
/// Each sub-strategy produces a signal (or None). The composite aggregates
/// by summing weights for each action and selecting the highest-weighted action.
pub struct CompositeStrategy {
    name: String,
    strategies: Vec<WeightedStrategy>,
    min_confidence: f64,
}

impl CompositeStrategy {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            strategies: Vec::new(),
            min_confidence: 0.5,
        }
    }

    pub fn with_min_confidence(mut self, c: f64) -> Self {
        self.min_confidence = c;
        self
    }

    /// Add a strategy with a weight.
    pub fn add(mut self, strategy: Box<dyn Strategy>, weight: f64) -> Self {
        self.strategies.push(WeightedStrategy { strategy, weight });
        self
    }
}

impl Strategy for CompositeStrategy {
    fn name(&self) -> &str {
        &self.name
    }

    fn analyze(&self, candles: &[Candle]) -> Result<Option<Signal>> {
        if self.strategies.is_empty() {
            return Ok(None);
        }

        let mut votes: [f64; 5] = [0.0; 5]; // One per AIAction
        let total_weight: f64 = self.strategies.iter().map(|s| s.weight).sum();

        for ws in &self.strategies {
            if let Ok(Some(signal)) = ws.strategy.analyze(candles) {
                let idx = signal.action.to_index() as usize;
                votes[idx] += ws.weight * signal.confidence;
            } else {
                // No signal = vote for Neutral
                votes[0] += ws.weight * 0.5;
            }
        }

        // Normalize votes
        let sum: f64 = votes.iter().sum();
        if sum <= 0.0 || total_weight <= 0.0 {
            return Ok(None);
        }

        // Find winning action
        let (best_idx, &best_score) = votes
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();

        let confidence = best_score / sum;
        if confidence < self.min_confidence {
            return Ok(None);
        }

        let action = AIAction::from_index(best_idx as u8);
        if action == AIAction::Neutral {
            return Ok(None);
        }

        let last = candles.last().unwrap();
        Ok(Some(Signal {
            timestamp: last.close_time,
            symbol: String::new(), // Filled by caller
            action,
            confidence,
            source: self.name.clone(),
            metadata: std::collections::HashMap::new(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    struct AlwaysLong;
    impl Strategy for AlwaysLong {
        fn name(&self) -> &str {
            "always_long"
        }
        fn analyze(&self, candles: &[Candle]) -> Result<Option<Signal>> {
            if candles.is_empty() {
                return Ok(None);
            }
            Ok(Some(Signal {
                timestamp: 0,
                symbol: "TEST".into(),
                action: AIAction::EnterLong,
                confidence: 0.9,
                source: "always_long".into(),
                metadata: Default::default(),
            }))
        }
    }

    struct AlwaysShort;
    impl Strategy for AlwaysShort {
        fn name(&self) -> &str {
            "always_short"
        }
        fn analyze(&self, candles: &[Candle]) -> Result<Option<Signal>> {
            if candles.is_empty() {
                return Ok(None);
            }
            Ok(Some(Signal {
                timestamp: 0,
                symbol: "TEST".into(),
                action: AIAction::EnterShort,
                confidence: 0.9,
                source: "always_short".into(),
                metadata: Default::default(),
            }))
        }
    }

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
            taker_buy_volume: Decimal::from(50),
            taker_sell_volume: Decimal::from(50),
        }
    }

    #[test]
    fn composite_majority_wins() {
        let strategy = CompositeStrategy::new("test")
            .with_min_confidence(0.0)
            .add(Box::new(AlwaysLong), 1.0)
            .add(Box::new(AlwaysLong), 1.0)
            .add(Box::new(AlwaysShort), 1.0);

        let candles = vec![dummy_candle()];
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.action, AIAction::EnterLong);
    }

    #[test]
    fn composite_weight_matters() {
        let strategy = CompositeStrategy::new("test")
            .with_min_confidence(0.0)
            .add(Box::new(AlwaysLong), 1.0)
            .add(Box::new(AlwaysShort), 10.0); // Much higher weight

        let candles = vec![dummy_candle()];
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.action, AIAction::EnterShort);
    }

    #[test]
    fn empty_composite_returns_none() {
        let strategy = CompositeStrategy::new("empty");
        let candles = vec![dummy_candle()];
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }
}
