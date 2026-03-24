use ssm_core::FeatureRow;

/// Dissimilarity Index — flags predictions on data too far from training distribution.
///
/// Computes a z-score-based distance metric: for each feature, measures how many
/// standard deviations the input is from the training mean, then averages across
/// all features. If the average exceeds the threshold, the input is flagged as an outlier.
pub struct DissimilarityIndex {
    training_mean: Vec<f64>,
    training_std: Vec<f64>,
    threshold: f64,
}

impl DissimilarityIndex {
    /// Fit from training data. Computes per-feature mean and standard deviation.
    ///
    /// Default threshold is 3.0 (average z-score across features).
    pub fn fit(data: &[FeatureRow]) -> Self {
        if data.is_empty() {
            return Self {
                training_mean: Vec::new(),
                training_std: Vec::new(),
                threshold: 3.0,
            };
        }

        let n = data.len() as f64;
        let num_features = data[0].features.len();

        // Compute mean per feature
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

        // Compute std per feature
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
            training_mean: mean,
            training_std: std,
            threshold: 3.0,
        }
    }

    /// Compute the DI score for a feature row.
    ///
    /// The score is the average absolute z-score across all features.
    /// Higher values indicate greater dissimilarity from training data.
    pub fn score(&self, features: &FeatureRow) -> f64 {
        if self.training_mean.is_empty() {
            return 0.0;
        }

        let num_features = self.training_mean.len().min(features.features.len());
        if num_features == 0 {
            return 0.0;
        }

        let mut total_z = 0.0;
        for i in 0..num_features {
            let std = self.training_std[i];
            if std > 1e-15 {
                let z = ((features.features[i] - self.training_mean[i]) / std).abs();
                total_z += z;
            }
            // If std is ~0, feature is constant — deviation is 0 for same value,
            // but could be huge for different value. We treat constant features
            // as non-contributing to avoid division by zero.
        }

        total_z / num_features as f64
    }

    /// Returns true if the feature row is an outlier (score exceeds threshold).
    pub fn is_outlier(&self, features: &FeatureRow) -> bool {
        self.score(features) > self.threshold
    }

    /// Set a custom threshold. Returns self for builder-style usage.
    pub fn with_threshold(mut self, threshold: f64) -> Self {
        self.threshold = threshold;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_row(features: Vec<f64>) -> FeatureRow {
        FeatureRow {
            timestamp: 0,
            features,
            label: None,
        }
    }

    #[test]
    fn fit_computes_mean_and_std() {
        let data = vec![
            make_row(vec![10.0, 20.0]),
            make_row(vec![20.0, 40.0]),
            make_row(vec![30.0, 60.0]),
        ];
        let di = DissimilarityIndex::fit(&data);

        // Mean: [20.0, 40.0]
        assert!((di.training_mean[0] - 20.0).abs() < 1e-10);
        assert!((di.training_mean[1] - 40.0).abs() < 1e-10);

        // Std: sqrt(((10-20)^2 + (20-20)^2 + (30-20)^2) / 3) = sqrt(200/3) ~ 8.165
        let expected_std_0 = (200.0_f64 / 3.0).sqrt();
        assert!((di.training_std[0] - expected_std_0).abs() < 1e-10);

        let expected_std_1 = (800.0_f64 / 3.0).sqrt();
        assert!((di.training_std[1] - expected_std_1).abs() < 1e-10);
    }

    #[test]
    fn normal_data_is_not_outlier() {
        let data: Vec<FeatureRow> = (0..100)
            .map(|i| make_row(vec![50.0 + (i as f64 % 10.0), 100.0 + (i as f64 % 20.0)]))
            .collect();
        let di = DissimilarityIndex::fit(&data);

        // A data point near the mean should not be an outlier
        let normal = make_row(vec![55.0, 110.0]);
        assert!(!di.is_outlier(&normal));
    }

    #[test]
    fn extreme_data_is_outlier() {
        let data: Vec<FeatureRow> = (0..100)
            .map(|i| make_row(vec![50.0 + (i as f64 % 10.0), 100.0 + (i as f64 % 20.0)]))
            .collect();
        let di = DissimilarityIndex::fit(&data);

        // A point very far from the mean should be an outlier
        let extreme = make_row(vec![5000.0, 10000.0]);
        assert!(di.is_outlier(&extreme));
    }

    #[test]
    fn score_increases_with_distance_from_mean() {
        let data: Vec<FeatureRow> = (0..100)
            .map(|i| make_row(vec![50.0 + (i as f64 % 10.0)]))
            .collect();
        let di = DissimilarityIndex::fit(&data);

        let close = make_row(vec![55.0]);
        let medium = make_row(vec![100.0]);
        let far = make_row(vec![500.0]);

        let score_close = di.score(&close);
        let score_medium = di.score(&medium);
        let score_far = di.score(&far);

        assert!(
            score_close < score_medium,
            "close={score_close} should be < medium={score_medium}"
        );
        assert!(
            score_medium < score_far,
            "medium={score_medium} should be < far={score_far}"
        );
    }

    #[test]
    fn with_threshold_adjusts_outlier_detection() {
        let data: Vec<FeatureRow> = (0..100)
            .map(|i| make_row(vec![50.0 + (i as f64 % 10.0)]))
            .collect();

        // With a very high threshold, nothing is an outlier
        let di = DissimilarityIndex::fit(&data).with_threshold(1000.0);
        let extreme = make_row(vec![500.0]);
        assert!(!di.is_outlier(&extreme));

        // With a very low threshold, even near-mean data is an outlier
        let di = DissimilarityIndex::fit(&data).with_threshold(0.01);
        let near_mean = make_row(vec![55.0]);
        assert!(di.is_outlier(&near_mean));
    }

    #[test]
    fn fit_empty_data() {
        let di = DissimilarityIndex::fit(&[]);
        assert!(di.training_mean.is_empty());
        assert!(di.training_std.is_empty());

        let row = make_row(vec![1.0, 2.0]);
        assert!(!di.is_outlier(&row));
        assert!((di.score(&row) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn fit_single_row() {
        let data = vec![make_row(vec![10.0, 20.0])];
        let di = DissimilarityIndex::fit(&data);

        // Mean should be the single point, std should be 0
        assert!((di.training_mean[0] - 10.0).abs() < 1e-10);
        assert!((di.training_mean[1] - 20.0).abs() < 1e-10);
        assert!((di.training_std[0] - 0.0).abs() < 1e-10);
        assert!((di.training_std[1] - 0.0).abs() < 1e-10);

        // With zero std, score should be 0 (constant features are non-contributing)
        let row = make_row(vec![100.0, 200.0]);
        assert!((di.score(&row) - 0.0).abs() < 1e-10);
    }
}
