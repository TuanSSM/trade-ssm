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

    #[test]
    fn stub_model_predict_batch() {
        let model = StubModel;
        let rows: Vec<FeatureRow> = (0..5)
            .map(|i| FeatureRow {
                timestamp: i,
                features: vec![i as f64],
                label: None,
            })
            .collect();
        let predictions = model.predict_batch(&rows).unwrap();
        assert_eq!(predictions.len(), 5);
        for p in &predictions {
            assert_eq!(*p, AIAction::Neutral);
        }
    }

    #[test]
    fn stub_model_predict_batch_empty() {
        let model = StubModel;
        let predictions = model.predict_batch(&[]).unwrap();
        assert!(predictions.is_empty());
    }

    #[test]
    fn stub_model_train_empty_data() {
        let mut model = StubModel;
        let m = model.train(&[]).unwrap();
        assert_eq!(m.samples, 0);
        assert_eq!(m.model_name, "stub");
        assert!((m.accuracy - 0.0).abs() < f64::EPSILON);
        assert!((m.loss - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn stub_model_save_load_roundtrip() {
        let mut model = StubModel;
        let path = std::path::Path::new("/tmp/stub_model_test");
        assert!(model.save(path).is_ok());
        assert!(model.load(path).is_ok());
        // After load, predict should still return Neutral
        let row = FeatureRow {
            timestamp: 0,
            features: vec![],
            label: None,
        };
        assert_eq!(model.predict(&row).unwrap(), AIAction::Neutral);
    }

    #[test]
    fn train_metrics_serialize() {
        let m = TrainMetrics {
            model_name: "test".into(),
            samples: 42,
            accuracy: 0.95,
            loss: 0.05,
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"model_name\":\"test\""));
        assert!(json.contains("\"samples\":42"));
    }

    #[test]
    fn stub_model_name() {
        let model = StubModel;
        assert_eq!(model.name(), "stub");
    }

    #[test]
    fn stub_model_predict_with_large_feature_vector() {
        let model = StubModel;
        let row = FeatureRow {
            timestamp: 999999,
            features: vec![0.0; 1000],
            label: Some(42.0),
        };
        // Should always return Neutral regardless of input size or label
        assert_eq!(model.predict(&row).unwrap(), AIAction::Neutral);
    }

    #[test]
    fn stub_model_train_large_dataset() {
        let mut model = StubModel;
        let data: Vec<FeatureRow> = (0..1000)
            .map(|i| FeatureRow {
                timestamp: i,
                features: vec![i as f64; 10],
                label: Some(if i % 2 == 0 { 1.0 } else { -1.0 }),
            })
            .collect();
        let m = model.train(&data).unwrap();
        assert_eq!(m.samples, 1000);
        assert_eq!(m.model_name, "stub");
        // Stub always returns zero accuracy and loss
        assert!((m.accuracy - 0.0).abs() < f64::EPSILON);
        assert!((m.loss - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn train_metrics_clone() {
        let m = TrainMetrics {
            model_name: "clone_test".into(),
            samples: 7,
            accuracy: 0.85,
            loss: 0.15,
        };
        let cloned = m.clone();
        assert_eq!(cloned.model_name, "clone_test");
        assert_eq!(cloned.samples, 7);
        assert!((cloned.accuracy - 0.85).abs() < f64::EPSILON);
        assert!((cloned.loss - 0.15).abs() < f64::EPSILON);
    }
}
