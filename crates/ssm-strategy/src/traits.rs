use anyhow::Result;
use ssm_core::{Candle, Signal};

/// Core strategy trait — implemented by both bot strategies and AI-driven strategies.
///
/// Inspired by freqtrade's `IStrategy` interface:
/// - `populate_indicators` → `analyze`
/// - `populate_entry_trend` + `populate_exit_trend` → signal output
pub trait Strategy: Send + Sync {
    /// Human-readable strategy name.
    fn name(&self) -> &str;

    /// Analyze closed candles and produce a signal (if any).
    /// Anti-repainting: callers must pass only closed candles.
    fn analyze(&self, candles: &[Candle]) -> Result<Option<Signal>>;
}

/// Strategy that can be trained on historical data (AI strategies).
pub trait Trainable: Strategy {
    /// Train the model on historical candles.
    fn train(&mut self, candles: &[Candle]) -> Result<TrainResult>;
}

#[derive(Debug, Clone)]
pub struct TrainResult {
    pub epochs: usize,
    pub final_metric: f64,
    pub metric_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyStrategy;

    impl Strategy for DummyStrategy {
        fn name(&self) -> &str {
            "dummy"
        }

        fn analyze(&self, _candles: &[Candle]) -> Result<Option<Signal>> {
            Ok(None)
        }
    }

    #[test]
    fn strategy_trait_is_object_safe() {
        let s: Box<dyn Strategy> = Box::new(DummyStrategy);
        assert_eq!(s.name(), "dummy");
        assert!(s.analyze(&[]).unwrap().is_none());
    }

    #[test]
    fn test_train_result_fields() {
        let result = TrainResult {
            epochs: 100,
            final_metric: 0.95,
            metric_name: "accuracy".to_string(),
        };
        assert_eq!(result.epochs, 100);
        assert!((result.final_metric - 0.95).abs() < f64::EPSILON);
        assert_eq!(result.metric_name, "accuracy");
    }

    #[test]
    fn test_dummy_strategy_name() {
        let s = DummyStrategy;
        assert_eq!(s.name(), "dummy");
    }

    #[test]
    fn strategy_trait_returns_none_for_empty_candles() {
        let s: Box<dyn Strategy> = Box::new(DummyStrategy);
        let candles: Vec<Candle> = vec![];
        assert!(s.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn strategy_trait_returns_none_for_real_candles() {
        use rust_decimal::Decimal;
        let s = DummyStrategy;
        let candle = Candle {
            open_time: 0,
            open: Decimal::from(100),
            high: Decimal::from(105),
            low: Decimal::from(95),
            close: Decimal::from(102),
            volume: Decimal::from(1000),
            close_time: 1000,
            quote_volume: Decimal::ZERO,
            trades: 50,
            taker_buy_volume: Decimal::from(600),
            taker_sell_volume: Decimal::from(400),
        };
        assert!(s.analyze(&[candle]).unwrap().is_none());
    }

    #[test]
    fn train_result_clone() {
        let result = TrainResult {
            epochs: 50,
            final_metric: 0.88,
            metric_name: "f1_score".to_string(),
        };
        let cloned = result.clone();
        assert_eq!(cloned.epochs, 50);
        assert!((cloned.final_metric - 0.88).abs() < f64::EPSILON);
        assert_eq!(cloned.metric_name, "f1_score");
    }

    #[test]
    fn train_result_debug() {
        let result = TrainResult {
            epochs: 10,
            final_metric: 0.5,
            metric_name: "accuracy".to_string(),
        };
        let debug = format!("{:?}", result);
        assert!(debug.contains("TrainResult"));
        assert!(debug.contains("10"));
        assert!(debug.contains("accuracy"));
    }

    #[test]
    fn multiple_strategies_as_trait_objects() {
        struct StratA;
        impl Strategy for StratA {
            fn name(&self) -> &str {
                "a"
            }
            fn analyze(&self, _candles: &[Candle]) -> Result<Option<Signal>> {
                Ok(None)
            }
        }
        struct StratB;
        impl Strategy for StratB {
            fn name(&self) -> &str {
                "b"
            }
            fn analyze(&self, _candles: &[Candle]) -> Result<Option<Signal>> {
                Ok(None)
            }
        }

        let strategies: Vec<Box<dyn Strategy>> =
            vec![Box::new(DummyStrategy), Box::new(StratA), Box::new(StratB)];
        assert_eq!(strategies[0].name(), "dummy");
        assert_eq!(strategies[1].name(), "a");
        assert_eq!(strategies[2].name(), "b");
        for s in &strategies {
            assert!(s.analyze(&[]).unwrap().is_none());
        }
    }

    #[test]
    fn train_result_zero_epochs() {
        let result = TrainResult {
            epochs: 0,
            final_metric: 0.0,
            metric_name: "loss".to_string(),
        };
        assert_eq!(result.epochs, 0);
        assert!((result.final_metric - 0.0).abs() < f64::EPSILON);
    }
}
