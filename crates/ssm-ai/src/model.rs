use anyhow::{bail, Context, Result};
use ssm_core::{AIAction, FeatureRow};
use std::path::Path;

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
    fn save(&self, path: &Path) -> Result<()>;
    fn load(&mut self, path: &Path) -> Result<()>;
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrainMetrics {
    pub model_name: String,
    pub samples: usize,
    pub accuracy: f64,
    pub loss: f64,
}

/// JSON-serializable model checkpoint for persistence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelCheckpoint {
    pub model_name: String,
    pub version: u32,
    pub num_features: usize,
    pub num_actions: usize,
    /// Flattened weight matrix: [num_actions * num_features].
    pub weights: Vec<f64>,
    pub learning_rate: f64,
    pub train_epochs: u64,
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

    fn save(&self, _path: &Path) -> Result<()> {
        Ok(())
    }

    fn load(&mut self, _path: &Path) -> Result<()> {
        Ok(())
    }
}

/// Number of discrete actions in the Base5Action space.
const NUM_ACTIONS: usize = 5;

/// Linear weight model for RL trading.
///
/// Maps feature vectors to actions via `action = argmax(weights * features)`.
/// Each action has a weight vector of size `num_features`.
/// Training uses simple policy gradient updates from labeled episodes.
pub struct TableModel {
    /// Weight matrix: weights\[action_idx\]\[feature_idx\].
    weights: Vec<Vec<f64>>,
    num_features: usize,
    learning_rate: f64,
    train_epochs: u64,
}

impl TableModel {
    /// Create a new model with zero-initialized weights.
    pub fn new(num_features: usize, learning_rate: f64) -> Self {
        let weights = vec![vec![0.0; num_features]; NUM_ACTIONS];
        Self {
            weights,
            num_features,
            learning_rate,
            train_epochs: 0,
        }
    }

    /// Load a model from a checkpoint file.
    pub fn from_checkpoint(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading checkpoint: {}", path.display()))?;
        let checkpoint: ModelCheckpoint =
            serde_json::from_str(&content).context("parsing checkpoint JSON")?;

        if checkpoint.weights.len() != checkpoint.num_actions * checkpoint.num_features {
            bail!(
                "weight size mismatch: expected {}x{}={}, got {}",
                checkpoint.num_actions,
                checkpoint.num_features,
                checkpoint.num_actions * checkpoint.num_features,
                checkpoint.weights.len()
            );
        }

        let mut weights = Vec::with_capacity(checkpoint.num_actions);
        for a in 0..checkpoint.num_actions {
            let start = a * checkpoint.num_features;
            let end = start + checkpoint.num_features;
            weights.push(checkpoint.weights[start..end].to_vec());
        }

        Ok(Self {
            weights,
            num_features: checkpoint.num_features,
            learning_rate: checkpoint.learning_rate,
            train_epochs: checkpoint.train_epochs,
        })
    }

    /// Compute raw scores for each action given features.
    fn scores(&self, features: &[f64]) -> [f64; NUM_ACTIONS] {
        let mut scores = [0.0; NUM_ACTIONS];
        for (a, w) in self.weights.iter().enumerate() {
            let mut s = 0.0;
            for (j, &feat) in features.iter().enumerate() {
                if j < w.len() {
                    s += w[j] * feat;
                }
            }
            scores[a] = s;
        }
        scores
    }

    /// Compute softmax probabilities from scores.
    fn softmax(scores: &[f64; NUM_ACTIONS]) -> [f64; NUM_ACTIONS] {
        let max_score = scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let mut probs = [0.0; NUM_ACTIONS];
        let mut sum = 0.0;
        for (i, &s) in scores.iter().enumerate() {
            probs[i] = (s - max_score).exp();
            sum += probs[i];
        }
        if sum > 0.0 {
            for p in probs.iter_mut() {
                *p /= sum;
            }
        }
        probs
    }
}

impl AIModel for TableModel {
    fn name(&self) -> &str {
        "table_model"
    }

    fn predict(&self, features: &FeatureRow) -> Result<AIAction> {
        let scores = self.scores(&features.features);
        // Find action with highest score; on ties, prefer lower index (Neutral=0)
        let mut best_idx = 0;
        let mut best_score = scores[0];
        for (i, &s) in scores.iter().enumerate().skip(1) {
            if s > best_score {
                best_score = s;
                best_idx = i;
            }
        }
        Ok(AIAction::from_index(best_idx as u8))
    }

    fn train(&mut self, data: &[FeatureRow]) -> Result<TrainMetrics> {
        if data.is_empty() {
            return Ok(TrainMetrics {
                model_name: self.name().into(),
                samples: 0,
                accuracy: 0.0,
                loss: 0.0,
            });
        }

        // Simple policy gradient: for each labeled sample, increase probability
        // of the correct action class based on label direction.
        // label > 0 → reward EnterLong (1), penalize EnterShort (3)
        // label < 0 → reward EnterShort (3), penalize EnterLong (1)
        // label == 0 → reward Neutral (0)
        let mut correct = 0usize;
        let mut total_loss = 0.0;

        for row in data {
            let label = row.label.unwrap_or(0.0);
            let target_action = if label > 0.0 {
                1 // EnterLong
            } else if label < 0.0 {
                3 // EnterShort
            } else {
                0 // Neutral
            };

            let scores = self.scores(&row.features);
            let probs = Self::softmax(&scores);

            // Cross-entropy loss
            let prob_target = probs[target_action].max(1e-10);
            total_loss -= prob_target.ln();

            // Check if prediction matches target
            let predicted = scores
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| i)
                .unwrap_or(0);
            if predicted == target_action {
                correct += 1;
            }

            // Gradient update: increase target action score, decrease others
            for (a, &prob) in probs.iter().enumerate() {
                let grad = if a == target_action {
                    1.0 - prob
                } else {
                    -prob
                };
                for (j, &feat) in row.features.iter().enumerate() {
                    if j < self.num_features {
                        self.weights[a][j] += self.learning_rate * grad * feat;
                    }
                }
            }
        }

        self.train_epochs += 1;
        let accuracy = correct as f64 / data.len() as f64;
        let avg_loss = total_loss / data.len() as f64;

        Ok(TrainMetrics {
            model_name: self.name().into(),
            samples: data.len(),
            accuracy,
            loss: avg_loss,
        })
    }

    fn save(&self, path: &Path) -> Result<()> {
        let mut flat_weights = Vec::with_capacity(NUM_ACTIONS * self.num_features);
        for w in &self.weights {
            flat_weights.extend_from_slice(w);
        }

        let checkpoint = ModelCheckpoint {
            model_name: self.name().into(),
            version: 1,
            num_features: self.num_features,
            num_actions: NUM_ACTIONS,
            weights: flat_weights,
            learning_rate: self.learning_rate,
            train_epochs: self.train_epochs,
        };

        let json = serde_json::to_string_pretty(&checkpoint).context("serializing checkpoint")?;
        std::fs::write(path, json)
            .with_context(|| format!("writing checkpoint: {}", path.display()))?;
        Ok(())
    }

    fn load(&mut self, path: &Path) -> Result<()> {
        let loaded = Self::from_checkpoint(path)?;
        self.weights = loaded.weights;
        self.num_features = loaded.num_features;
        self.learning_rate = loaded.learning_rate;
        self.train_epochs = loaded.train_epochs;
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

    // --- Tests from branch (ours) ---

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

    // --- Tests from main (theirs) ---

    #[test]
    fn table_model_predicts_neutral_with_zero_weights() {
        let model = TableModel::new(10, 0.01);
        let row = FeatureRow {
            timestamp: 0,
            features: vec![1.0; 10],
            label: None,
        };
        let action = model.predict(&row).unwrap();
        assert_eq!(action, AIAction::Neutral);
    }

    #[test]
    fn table_model_trains_and_predicts() {
        let mut model = TableModel::new(3, 0.1);

        let data: Vec<FeatureRow> = (0..50)
            .map(|i| FeatureRow {
                timestamp: i,
                features: vec![1.0, 0.5, 0.8],
                label: Some(1.0),
            })
            .collect();

        let metrics = model.train(&data).unwrap();
        assert_eq!(metrics.samples, 50);
        assert!(metrics.loss >= 0.0);

        let row = FeatureRow {
            timestamp: 0,
            features: vec![1.0, 0.5, 0.8],
            label: None,
        };
        let action = model.predict(&row).unwrap();
        assert_eq!(action, AIAction::EnterLong);
    }

    #[test]
    fn table_model_save_load_roundtrip() {
        let mut model = TableModel::new(5, 0.01);

        let data: Vec<FeatureRow> = (0..20)
            .map(|i| FeatureRow {
                timestamp: i,
                features: vec![1.0, 2.0, 3.0, 4.0, 5.0],
                label: Some(if i % 2 == 0 { 1.0 } else { -1.0 }),
            })
            .collect();
        model.train(&data).unwrap();

        let tmp = std::env::temp_dir().join("test_table_model.json");
        model.save(&tmp).unwrap();

        let loaded = TableModel::from_checkpoint(&tmp).unwrap();

        for (a, (w_orig, w_loaded)) in model.weights.iter().zip(loaded.weights.iter()).enumerate() {
            for (j, (o, l)) in w_orig.iter().zip(w_loaded.iter()).enumerate() {
                assert!(
                    (o - l).abs() < 1e-15,
                    "weight mismatch at [{a}][{j}]: {o} vs {l}"
                );
            }
        }

        let test_row = FeatureRow {
            timestamp: 0,
            features: vec![1.0, 2.0, 3.0, 4.0, 5.0],
            label: None,
        };
        assert_eq!(
            model.predict(&test_row).unwrap(),
            loaded.predict(&test_row).unwrap()
        );

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn table_model_is_object_safe() {
        let _m: Box<dyn AIModel> = Box::new(TableModel::new(10, 0.01));
    }

    #[test]
    fn table_model_handles_mismatched_feature_lengths() {
        let model = TableModel::new(5, 0.01);
        let row = FeatureRow {
            timestamp: 0,
            features: vec![1.0, 2.0],
            label: None,
        };
        let action = model.predict(&row).unwrap();
        assert_eq!(action, AIAction::Neutral);
    }
}
