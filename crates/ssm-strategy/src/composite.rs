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

        let last = match candles.last() {
            Some(c) => c,
            None => return Ok(None),
        };
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

    #[test]
    fn test_composite_name() {
        let strategy = CompositeStrategy::new("my_composite");
        assert_eq!(strategy.name(), "my_composite");
    }

    #[test]
    fn test_single_strategy() {
        let strategy = CompositeStrategy::new("single")
            .with_min_confidence(0.0)
            .add(Box::new(AlwaysLong), 1.0);

        let candles = vec![dummy_candle()];
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.action, AIAction::EnterLong);
    }

    #[test]
    fn test_empty_candles_returns_none() {
        let strategy = CompositeStrategy::new("test")
            .with_min_confidence(0.0)
            .add(Box::new(AlwaysLong), 1.0);

        let candles: Vec<Candle> = vec![];
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    struct WeakLong;
    impl Strategy for WeakLong {
        fn name(&self) -> &str {
            "weak_long"
        }
        fn analyze(&self, candles: &[Candle]) -> Result<Option<Signal>> {
            if candles.is_empty() {
                return Ok(None);
            }
            Ok(Some(Signal {
                timestamp: 0,
                symbol: "TEST".into(),
                action: AIAction::EnterLong,
                confidence: 0.1,
                source: "weak_long".into(),
                metadata: Default::default(),
            }))
        }
    }

    #[test]
    fn test_confidence_threshold() {
        // WeakLong + AlwaysShort: long vote is small relative to total,
        // high min_confidence filters out the winner
        let strategy = CompositeStrategy::new("test")
            .with_min_confidence(0.99)
            .add(Box::new(WeakLong), 1.0)
            .add(Box::new(AlwaysShort), 1.0);

        let candles = vec![dummy_candle()];
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    struct AlwaysNone;
    impl Strategy for AlwaysNone {
        fn name(&self) -> &str {
            "always_none"
        }
        fn analyze(&self, _candles: &[Candle]) -> Result<Option<Signal>> {
            Ok(None)
        }
    }

    struct AlwaysExitLong;
    impl Strategy for AlwaysExitLong {
        fn name(&self) -> &str {
            "always_exit_long"
        }
        fn analyze(&self, candles: &[Candle]) -> Result<Option<Signal>> {
            if candles.is_empty() {
                return Ok(None);
            }
            Ok(Some(Signal {
                timestamp: 0,
                symbol: "TEST".into(),
                action: AIAction::ExitLong,
                confidence: 0.9,
                source: "always_exit_long".into(),
                metadata: Default::default(),
            }))
        }
    }

    #[test]
    fn zero_strategies_returns_none() {
        let strategy = CompositeStrategy::new("empty_composite");
        assert!(strategy.analyze(&[]).unwrap().is_none());
    }

    #[test]
    fn zero_strategies_with_candles_returns_none() {
        let strategy = CompositeStrategy::new("empty_composite");
        let candles = vec![dummy_candle(), dummy_candle()];
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn single_strategy_passthrough_long() {
        let strategy = CompositeStrategy::new("single_pass")
            .with_min_confidence(0.0)
            .add(Box::new(AlwaysLong), 1.0);
        let candles = vec![dummy_candle()];
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.action, AIAction::EnterLong);
        assert_eq!(signal.source, "single_pass");
    }

    #[test]
    fn single_strategy_passthrough_short() {
        let strategy = CompositeStrategy::new("single_pass")
            .with_min_confidence(0.0)
            .add(Box::new(AlwaysShort), 1.0);
        let candles = vec![dummy_candle()];
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.action, AIAction::EnterShort);
    }

    #[test]
    fn conflicting_equal_weight_strategies() {
        // Long and Short with equal weight and equal confidence
        // One should win based on voting mechanism
        let strategy = CompositeStrategy::new("conflict")
            .with_min_confidence(0.0)
            .add(Box::new(AlwaysLong), 1.0)
            .add(Box::new(AlwaysShort), 1.0);
        let candles = vec![dummy_candle()];
        let result = strategy.analyze(&candles).unwrap();
        // Both have 0.9 confidence * 1.0 weight = 0.9 vote each
        // Either action could win (implementation picks max); result should be Some
        assert!(result.is_some());
    }

    #[test]
    fn three_way_conflict_majority_long() {
        let strategy = CompositeStrategy::new("three_way")
            .with_min_confidence(0.0)
            .add(Box::new(AlwaysLong), 1.0)
            .add(Box::new(AlwaysLong), 1.0)
            .add(Box::new(AlwaysShort), 1.0);
        let candles = vec![dummy_candle()];
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.action, AIAction::EnterLong);
    }

    #[test]
    fn three_way_conflict_majority_short() {
        let strategy = CompositeStrategy::new("three_way")
            .with_min_confidence(0.0)
            .add(Box::new(AlwaysShort), 1.0)
            .add(Box::new(AlwaysShort), 1.0)
            .add(Box::new(AlwaysLong), 1.0);
        let candles = vec![dummy_candle()];
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.action, AIAction::EnterShort);
    }

    #[test]
    fn all_none_strategies_returns_none() {
        // All strategies return None, which votes Neutral with 0.5 weight
        // Neutral action is filtered out in analyze
        let strategy = CompositeStrategy::new("all_none")
            .with_min_confidence(0.0)
            .add(Box::new(AlwaysNone), 1.0)
            .add(Box::new(AlwaysNone), 1.0);
        let candles = vec![dummy_candle()];
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn name_returns_constructor_name() {
        let strategy = CompositeStrategy::new("custom_name");
        assert_eq!(strategy.name(), "custom_name");
    }

    #[test]
    fn high_weight_overrides_count() {
        // One short with weight 100 vs three longs with weight 1
        let strategy = CompositeStrategy::new("weight_test")
            .with_min_confidence(0.0)
            .add(Box::new(AlwaysShort), 100.0)
            .add(Box::new(AlwaysLong), 1.0)
            .add(Box::new(AlwaysLong), 1.0)
            .add(Box::new(AlwaysLong), 1.0);
        let candles = vec![dummy_candle()];
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.action, AIAction::EnterShort);
    }

    #[test]
    fn exit_long_action_passthrough() {
        let strategy = CompositeStrategy::new("exit_test")
            .with_min_confidence(0.0)
            .add(Box::new(AlwaysExitLong), 1.0);
        let candles = vec![dummy_candle()];
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert_eq!(signal.action, AIAction::ExitLong);
    }

    #[test]
    fn signal_confidence_is_bounded() {
        let strategy = CompositeStrategy::new("bounded")
            .with_min_confidence(0.0)
            .add(Box::new(AlwaysLong), 1.0);
        let candles = vec![dummy_candle()];
        let signal = strategy.analyze(&candles).unwrap().unwrap();
        assert!(signal.confidence >= 0.0 && signal.confidence <= 1.0);
    }

    #[test]
    fn min_confidence_1_0_filters_everything() {
        let strategy = CompositeStrategy::new("strict")
            .with_min_confidence(1.0)
            .add(Box::new(AlwaysLong), 1.0)
            .add(Box::new(AlwaysShort), 1.0);
        let candles = vec![dummy_candle()];
        // With two strategies voting differently, no single action can have 100% confidence
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }
}
