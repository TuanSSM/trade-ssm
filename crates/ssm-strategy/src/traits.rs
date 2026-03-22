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
}
