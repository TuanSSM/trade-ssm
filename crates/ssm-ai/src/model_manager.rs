use anyhow::Result;
use ssm_core::{AIAction, FeatureRow};

use crate::model::AIModel;

/// Manages model lifecycle: sliding-window retraining and expiration.
pub struct ModelManager {
    model: Box<dyn AIModel>,
    last_trained: Option<i64>,
    train_window_size: usize,
    expiration_hours: f64,
    live_retrain_hours: f64,
}

impl ModelManager {
    pub fn new(
        model: Box<dyn AIModel>,
        train_window: usize,
        expiration_hours: f64,
        live_retrain_hours: f64,
    ) -> Self {
        Self {
            model,
            last_trained: None,
            train_window_size: train_window,
            expiration_hours,
            live_retrain_hours,
        }
    }

    /// Check if the model should be retrained based on live_retrain_hours.
    pub fn needs_retrain(&self, now: i64) -> bool {
        match self.last_trained {
            None => true,
            Some(trained_at) => {
                let retrain_ms = (self.live_retrain_hours * 3600.0 * 1000.0) as i64;
                now - trained_at >= retrain_ms
            }
        }
    }

    /// Check if model is expired based on current timestamp.
    pub fn is_expired(&self, now: i64) -> bool {
        match self.last_trained {
            None => true,
            Some(trained_at) => {
                let expiration_ms = (self.expiration_hours * 3600.0 * 1000.0) as i64;
                now - trained_at >= expiration_ms
            }
        }
    }

    /// Retrain model on the latest window of data.
    ///
    /// Uses only the last `train_window_size` rows from the provided data.
    /// Updates `last_trained` to the timestamp of the most recent row.
    pub fn retrain(&mut self, data: &[FeatureRow]) -> Result<()> {
        let window_start = data.len().saturating_sub(self.train_window_size);
        let window = &data[window_start..];

        self.model.train(window)?;

        // Update last_trained to the latest timestamp in the window
        self.last_trained = window.iter().map(|r| r.timestamp).max();

        Ok(())
    }

    /// Predict using the model, but reject if model is expired.
    ///
    /// Returns `Ok(None)` if the model is expired (needs retraining).
    /// Returns `Ok(Some(action))` if the model is fresh and prediction succeeds.
    pub fn predict(&self, features: &FeatureRow, now: i64) -> Result<Option<AIAction>> {
        if self.is_expired(now) {
            return Ok(None);
        }
        let action = self.model.predict(features)?;
        Ok(Some(action))
    }

    /// Get the timestamp of the last training, if any.
    pub fn last_trained(&self) -> Option<i64> {
        self.last_trained
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StubModel;

    fn make_data(n: usize, base_ts: i64) -> Vec<FeatureRow> {
        (0..n)
            .map(|i| FeatureRow {
                timestamp: base_ts + (i as i64) * 1000,
                features: vec![1.0, 2.0, 3.0],
                label: Some(1.0),
            })
            .collect()
    }

    #[test]
    fn is_expired_returns_true_when_never_trained() {
        let mgr = ModelManager::new(Box::new(StubModel), 100, 1.0, 24.0);
        assert!(mgr.is_expired(0));
        assert!(mgr.is_expired(999999));
    }

    #[test]
    fn is_expired_returns_true_after_expiration() {
        let mut mgr = ModelManager::new(Box::new(StubModel), 100, 1.0, 24.0); // 1 hour
        let data = make_data(10, 1_000_000);
        mgr.retrain(&data).unwrap();

        let trained_at = mgr.last_trained().unwrap();
        // Right at expiration boundary (1 hour = 3_600_000 ms)
        assert!(mgr.is_expired(trained_at + 3_600_000));
        // Well past expiration
        assert!(mgr.is_expired(trained_at + 10_000_000));
    }

    #[test]
    fn is_expired_returns_false_before_expiration() {
        let mut mgr = ModelManager::new(Box::new(StubModel), 100, 1.0, 24.0);
        let data = make_data(10, 1_000_000);
        mgr.retrain(&data).unwrap();

        let trained_at = mgr.last_trained().unwrap();
        // 30 minutes later — not expired
        assert!(!mgr.is_expired(trained_at + 1_800_000));
        // Immediately after training
        assert!(!mgr.is_expired(trained_at));
    }

    #[test]
    fn predict_returns_none_when_expired() {
        let mgr = ModelManager::new(Box::new(StubModel), 100, 1.0, 24.0);
        let row = FeatureRow {
            timestamp: 0,
            features: vec![1.0],
            label: None,
        };
        let result = mgr.predict(&row, 0).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn predict_returns_action_when_fresh() {
        let mut mgr = ModelManager::new(Box::new(StubModel), 100, 1.0, 24.0);
        let data = make_data(10, 1_000_000);
        mgr.retrain(&data).unwrap();

        let row = FeatureRow {
            timestamp: 0,
            features: vec![1.0],
            label: None,
        };
        let trained_at = mgr.last_trained().unwrap();
        let result = mgr.predict(&row, trained_at + 1000).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), AIAction::Neutral); // StubModel always returns Neutral
    }

    #[test]
    fn retrain_updates_last_trained() {
        let mut mgr = ModelManager::new(Box::new(StubModel), 100, 1.0, 24.0);
        assert!(mgr.last_trained().is_none());

        let data = make_data(10, 5_000_000);
        mgr.retrain(&data).unwrap();

        let last = mgr.last_trained().unwrap();
        // Should be the timestamp of the last row
        assert_eq!(last, 5_000_000 + 9 * 1000);
    }

    #[test]
    fn retrain_uses_window_size() {
        let mut mgr = ModelManager::new(Box::new(StubModel), 5, 1.0, 24.0);
        let data = make_data(20, 0);
        mgr.retrain(&data).unwrap();

        // Last trained should be the timestamp of the last row in the window
        // Window = last 5 of 20 rows => rows 15..20, timestamps 15000..19000
        let last = mgr.last_trained().unwrap();
        assert_eq!(last, 19_000);
    }

    #[test]
    fn needs_retrain_true_when_never_trained() {
        let mgr = ModelManager::new(Box::new(StubModel), 100, 1.0, 24.0);
        assert!(mgr.needs_retrain(0));
        assert!(mgr.needs_retrain(999999));
    }

    #[test]
    fn needs_retrain_true_after_interval() {
        let mut mgr = ModelManager::new(Box::new(StubModel), 100, 48.0, 24.0);
        let data = make_data(10, 1_000_000);
        mgr.retrain(&data).unwrap();

        let trained_at = mgr.last_trained().unwrap();
        // 24 hours = 86_400_000 ms
        assert!(mgr.needs_retrain(trained_at + 86_400_000));
        assert!(mgr.needs_retrain(trained_at + 100_000_000));
    }

    #[test]
    fn needs_retrain_false_before_interval() {
        let mut mgr = ModelManager::new(Box::new(StubModel), 100, 48.0, 24.0);
        let data = make_data(10, 1_000_000);
        mgr.retrain(&data).unwrap();

        let trained_at = mgr.last_trained().unwrap();
        // 12 hours = 43_200_000 ms
        assert!(!mgr.needs_retrain(trained_at + 43_200_000));
        assert!(!mgr.needs_retrain(trained_at));
    }

    #[test]
    fn constructor_with_live_retrain_hours() {
        let mgr = ModelManager::new(Box::new(StubModel), 100, 48.0, 12.0);
        // 12 hours = 43_200_000 ms — should not need retrain at 11h but need at 12h
        // First, needs retrain since never trained
        assert!(mgr.needs_retrain(0));
    }
}
