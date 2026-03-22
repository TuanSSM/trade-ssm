use anyhow::Result;
use ssm_core::{AIAction, FeatureRow};

/// Trait for AI/ML models (XGBoost, RL agents, etc.).
///
/// Inspired by FreqAI's model interface:
/// - `predict` maps feature rows to actions
/// - `train` updates the model from labeled data
/// - `save`/`load` for model persistence
pub trait AIModel: Send + Sync {
    fn name(&self) -> &str;
    fn predict(&self, features: &FeatureRow) -> Result<AIAction>;
    fn predict_batch(&self, features: &[FeatureRow]) -> Result<Vec<AIAction>> {
        features.iter().map(|f| self.predict(f)).collect()
    }
    fn train(&mut self, data: &[FeatureRow]) -> Result<TrainMetrics>;
    fn save(&self, path: &std::path::Path) -> Result<()>;
    fn load(&mut self, path: &std::path::Path) -> Result<()>;
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TrainMetrics {
    pub model_name: String,
    pub samples: usize,
    pub accuracy: f64,
    pub loss: f64,
}

/// Stub model that always predicts Neutral — used for testing pipelines.
pub struct StubModel;

impl AIModel for StubModel {
    fn name(&self) -> &str {
        "stub"
    }

    fn predict(&self, _features: &FeatureRow) -> Result<AIAction> {
        Ok(AIAction::Neutral)
    }

    fn train(&mut self, data: &[FeatureRow]) -> Result<TrainMetrics> {
        Ok(TrainMetrics {
            model_name: "stub".into(),
            samples: data.len(),
            accuracy: 0.0,
            loss: 0.0,
        })
    }

    fn save(&self, _path: &std::path::Path) -> Result<()> {
        Ok(())
    }

    fn load(&mut self, _path: &std::path::Path) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_model_predicts_neutral() {
        let model = StubModel;
        let row = FeatureRow {
            timestamp: 1000,
            features: vec![1.0, 2.0, 3.0],
            label: None,
        };
        assert_eq!(model.predict(&row).unwrap(), AIAction::Neutral);
    }

    #[test]
    fn stub_model_trains() {
        let mut model = StubModel;
        let data = vec![FeatureRow {
            timestamp: 0,
            features: vec![],
            label: Some(1.0),
        }];
        let m = model.train(&data).unwrap();
        assert_eq!(m.samples, 1);
    }

    #[test]
    fn model_trait_is_object_safe() {
        let _m: Box<dyn AIModel> = Box::new(StubModel);
    }
}
