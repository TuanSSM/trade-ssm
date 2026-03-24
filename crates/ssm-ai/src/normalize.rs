use serde::{Deserialize, Serialize};
use ssm_core::FeatureRow;

/// Z-score feature normalizer for ML/RL training and inference.
///
/// Fitted on training data, transforms features to zero mean and unit variance.
/// Serializable so the same normalization can be applied during inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureNormalizer {
    mean: Vec<f64>,
    std: Vec<f64>,
    /// Floor for std to avoid division by zero.
    epsilon: f64,
}

impl FeatureNormalizer {
    /// Fit from training data. Computes per-feature mean and population std.
    pub fn fit(data: &[FeatureRow]) -> Self {
        if data.is_empty() {
            return Self {
                mean: Vec::new(),
                std: Vec::new(),
                epsilon: 1e-8,
            };
        }

        let num_features = data[0].features.len();
        let n = data.len() as f64;

        let mut mean = vec![0.0; num_features];
        for row in data {
            for (i, &val) in row.features.iter().enumerate() {
                if i < num_features {
                    mean[i] += val;
                }
            }
        }
        for m in &mut mean {
            *m /= n;
        }

        let mut variance = vec![0.0; num_features];
        for row in data {
            for (i, &val) in row.features.iter().enumerate() {
                if i < num_features {
                    let diff = val - mean[i];
                    variance[i] += diff * diff;
                }
            }
        }
        let std: Vec<f64> = variance.iter().map(|v| (v / n).sqrt()).collect();

        Self {
            mean,
            std,
            epsilon: 1e-8,
        }
    }

    /// Z-score normalize a feature vector in-place: (x - mean) / max(std, epsilon).
    pub fn transform(&self, features: &mut [f64]) {
        for (i, val) in features.iter_mut().enumerate() {
            if i < self.mean.len() {
                *val = (*val - self.mean[i]) / self.std[i].max(self.epsilon);
            }
        }
    }

    /// Transform a FeatureRow, returning a new one with normalized features.
    pub fn transform_row(&self, row: &FeatureRow) -> FeatureRow {
        let mut features = row.features.clone();
        self.transform(&mut features);
        FeatureRow {
            timestamp: row.timestamp,
            features,
            label: row.label,
        }
    }

    /// Batch transform all rows.
    pub fn transform_batch(&self, rows: &[FeatureRow]) -> Vec<FeatureRow> {
        rows.iter().map(|r| self.transform_row(r)).collect()
    }

    /// Inverse transform: recover original scale from normalized values.
    pub fn inverse_transform(&self, features: &mut [f64]) {
        for (i, val) in features.iter_mut().enumerate() {
            if i < self.mean.len() {
                *val = *val * self.std[i].max(self.epsilon) + self.mean[i];
            }
        }
    }

    pub fn num_features(&self) -> usize {
        self.mean.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rows(data: &[Vec<f64>]) -> Vec<FeatureRow> {
        data.iter()
            .enumerate()
            .map(|(i, features)| FeatureRow {
                timestamp: i as i64,
                features: features.clone(),
                label: None,
            })
            .collect()
    }

    #[test]
    fn fit_empty_data() {
        let norm = FeatureNormalizer::fit(&[]);
        assert_eq!(norm.num_features(), 0);
    }

    #[test]
    fn fit_single_row() {
        let rows = make_rows(&[vec![10.0, 20.0]]);
        let norm = FeatureNormalizer::fit(&rows);
        assert_eq!(norm.num_features(), 2);
        assert!((norm.mean[0] - 10.0).abs() < 1e-10);
        assert!((norm.mean[1] - 20.0).abs() < 1e-10);
        // Std is 0 for single sample, epsilon floor prevents div-by-zero
        assert!((norm.std[0] - 0.0).abs() < 1e-10);
    }

    #[test]
    fn transform_zero_mean_unit_variance() {
        let rows = make_rows(&[vec![10.0, 20.0], vec![20.0, 40.0], vec![30.0, 60.0]]);
        let norm = FeatureNormalizer::fit(&rows);

        // Mean: [20, 40], Std: [8.165, 16.33]
        let mut features = vec![20.0, 40.0]; // should become ~0.0
        norm.transform(&mut features);
        assert!(features[0].abs() < 1e-10, "mean should normalize to 0");
        assert!(features[1].abs() < 1e-10, "mean should normalize to 0");
    }

    #[test]
    fn inverse_transform_roundtrip() {
        let rows = make_rows(&[vec![5.0, 15.0], vec![10.0, 25.0], vec![15.0, 35.0]]);
        let norm = FeatureNormalizer::fit(&rows);

        let original = vec![10.0, 25.0];
        let mut features = original.clone();
        norm.transform(&mut features);
        norm.inverse_transform(&mut features);

        for (o, r) in original.iter().zip(features.iter()) {
            assert!((o - r).abs() < 1e-9, "roundtrip failed: {o} vs {r}");
        }
    }

    #[test]
    fn transform_row_preserves_metadata() {
        let rows = make_rows(&[vec![1.0, 2.0], vec![3.0, 4.0]]);
        let norm = FeatureNormalizer::fit(&rows);

        let row = FeatureRow {
            timestamp: 42,
            features: vec![2.0, 3.0],
            label: Some(1.0),
        };
        let transformed = norm.transform_row(&row);
        assert_eq!(transformed.timestamp, 42);
        assert_eq!(transformed.label, Some(1.0));
        assert_eq!(transformed.features.len(), 2);
    }

    #[test]
    fn transform_batch_correct_count() {
        let rows = make_rows(&[vec![1.0], vec![2.0], vec![3.0]]);
        let norm = FeatureNormalizer::fit(&rows);
        let batch = norm.transform_batch(&rows);
        assert_eq!(batch.len(), 3);
    }

    #[test]
    fn serde_roundtrip() {
        let rows = make_rows(&[vec![10.0, 20.0], vec![30.0, 40.0]]);
        let norm = FeatureNormalizer::fit(&rows);
        let json = serde_json::to_string(&norm).unwrap();
        let parsed: FeatureNormalizer = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.num_features(), norm.num_features());
        for (a, b) in parsed.mean.iter().zip(norm.mean.iter()) {
            assert!((a - b).abs() < 1e-15);
        }
    }

    #[test]
    fn constant_feature_uses_epsilon() {
        let rows = make_rows(&[vec![5.0], vec![5.0], vec![5.0]]);
        let norm = FeatureNormalizer::fit(&rows);
        // Std is 0, epsilon prevents NaN
        let mut features = vec![5.0];
        norm.transform(&mut features);
        assert!(features[0].is_finite());
        assert!(features[0].abs() < 1e-2); // (5-5)/epsilon ≈ 0
    }
}
