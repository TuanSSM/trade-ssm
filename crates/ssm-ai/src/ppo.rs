use anyhow::Result;
use serde::{Deserialize, Serialize};
use ssm_core::{AIAction, FeatureRow};
use std::path::Path;

use crate::model::{AIModel, TrainMetrics};

/// PPO agent configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PpoConfig {
    pub num_features: usize,
    pub num_actions: usize,
    pub learning_rate: f64,
    pub gamma: f64,
    pub epsilon_clip: f64,
    pub value_coeff: f64,
    pub entropy_coeff: f64,
    pub max_grad_norm: f64,
    pub batch_size: usize,
    pub epochs_per_update: usize,
}

impl Default for PpoConfig {
    fn default() -> Self {
        Self {
            num_features: 22,
            num_actions: 5,
            learning_rate: 0.0003,
            gamma: 0.99,
            epsilon_clip: 0.2,
            value_coeff: 0.5,
            entropy_coeff: 0.01,
            max_grad_norm: 0.5,
            batch_size: 64,
            epochs_per_update: 4,
        }
    }
}

/// Experience sample for PPO training.
#[derive(Debug, Clone)]
pub struct Experience {
    pub state: Vec<f64>,
    pub action: usize,
    pub reward: f64,
    pub next_state: Vec<f64>,
    pub done: bool,
    pub log_prob: f64,
    pub value: f64,
}

/// Experience replay buffer for PPO.
pub struct PpoReplayBuffer {
    buffer: Vec<Experience>,
    capacity: usize,
}

impl PpoReplayBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(capacity.min(1024)),
            capacity,
        }
    }

    pub fn push(&mut self, exp: Experience) {
        if self.buffer.len() < self.capacity {
            self.buffer.push(exp);
        } else {
            // Evict oldest: shift everything left by 1, put new at end
            self.buffer.rotate_left(1);
            let last = self.buffer.len() - 1;
            self.buffer[last] = exp;
        }
    }

    /// Sample a batch using a simple deterministic LCG PRNG seeded from buffer length.
    pub fn sample(&self, batch_size: usize) -> Vec<&Experience> {
        let len = self.buffer.len();
        if len == 0 || batch_size == 0 {
            return vec![];
        }
        let actual = batch_size.min(len);
        // Simple deterministic sampling using LCG
        let mut state: u64 = (len as u64)
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (0..actual)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let idx = ((state >> 11) as usize) % len;
                &self.buffer[idx]
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

/// Simple linear policy network (weights-based, no deep learning framework).
///
/// Policy: softmax(W_policy * features + b_policy)
/// Value:  W_value * features + b_value
pub struct PpoAgent {
    config: PpoConfig,
    policy_weights: Vec<Vec<f64>>, // [num_actions x num_features]
    policy_bias: Vec<f64>,         // [num_actions]
    value_weights: Vec<f64>,       // [num_features]
    value_bias: f64,
    replay_buffer: PpoReplayBuffer,
    total_updates: usize,
}

impl PpoAgent {
    pub fn new(config: PpoConfig) -> Self {
        let num_actions = config.num_actions;
        let num_features = config.num_features;
        Self {
            policy_weights: vec![vec![0.0; num_features]; num_actions],
            policy_bias: vec![0.0; num_actions],
            value_weights: vec![0.0; num_features],
            value_bias: 0.0,
            replay_buffer: PpoReplayBuffer::new(config.batch_size * 32),
            total_updates: 0,
            config,
        }
    }

    /// Compute action probabilities via softmax over linear policy.
    pub fn action_probs(&self, features: &[f64]) -> Vec<f64> {
        let mut logits = vec![0.0; self.config.num_actions];
        for (a, w) in self.policy_weights.iter().enumerate() {
            let mut s = self.policy_bias[a];
            for (j, &feat) in features.iter().enumerate() {
                if j < w.len() {
                    s += w[j] * feat;
                }
            }
            logits[a] = s;
        }
        softmax(&logits)
    }

    /// Compute state value via linear value function.
    pub fn value(&self, features: &[f64]) -> f64 {
        let mut v = self.value_bias;
        for (j, &feat) in features.iter().enumerate() {
            if j < self.value_weights.len() {
                v += self.value_weights[j] * feat;
            }
        }
        v
    }

    /// Select action by sampling from the probability distribution.
    /// Returns (action_index, log_probability).
    pub fn select_action(&self, features: &[f64]) -> (usize, f64) {
        let probs = self.action_probs(features);

        // Deterministic sampling using a hash of the features
        let mut hash: u64 = 0xcbf29ce484222325;
        for &f in features {
            let bits = f.to_bits();
            hash ^= bits;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let r = (hash >> 11) as f64 / (1u64 << 53) as f64;

        let mut cumulative = 0.0;
        let mut selected = self.config.num_actions - 1;
        for (i, &p) in probs.iter().enumerate() {
            cumulative += p;
            if r < cumulative {
                selected = i;
                break;
            }
        }

        let log_prob = probs[selected].max(1e-10).ln();
        (selected, log_prob)
    }

    /// Store an experience in the replay buffer.
    pub fn store_experience(&mut self, exp: Experience) {
        self.replay_buffer.push(exp);
    }

    /// Run PPO update on collected experiences. Returns average policy loss.
    pub fn update(&mut self) -> f64 {
        let n = self.replay_buffer.len();
        if n == 0 {
            return 0.0;
        }

        // Collect data from buffer
        let rewards: Vec<f64> = self.replay_buffer.buffer.iter().map(|e| e.reward).collect();
        let values: Vec<f64> = self.replay_buffer.buffer.iter().map(|e| e.value).collect();
        let dones: Vec<bool> = self.replay_buffer.buffer.iter().map(|e| e.done).collect();

        let advantages = Self::compute_gae(&rewards, &values, &dones, self.config.gamma, 0.95);

        // Compute returns = advantages + values
        let returns: Vec<f64> = advantages
            .iter()
            .zip(values.iter())
            .map(|(a, v)| a + v)
            .collect();

        let mut total_loss = 0.0;
        let mut update_count = 0;

        for _epoch in 0..self.config.epochs_per_update {
            for i in 0..n {
                let exp = &self.replay_buffer.buffer[i];
                let probs = self.action_probs(&exp.state);
                let new_log_prob = probs[exp.action].max(1e-10).ln();
                let old_log_prob = exp.log_prob;

                // PPO ratio
                let ratio = (new_log_prob - old_log_prob).exp();
                let adv = advantages[i];

                // Clipped objective
                let surr1 = ratio * adv;
                let surr2 = ratio.clamp(
                    1.0 - self.config.epsilon_clip,
                    1.0 + self.config.epsilon_clip,
                ) * adv;
                let policy_loss = -surr1.min(surr2);

                // Value loss
                let v = self.value(&exp.state);
                let value_loss = (returns[i] - v).powi(2);

                // Entropy bonus
                let entropy: f64 = probs
                    .iter()
                    .map(|&p| if p > 1e-10 { -p * p.ln() } else { 0.0 })
                    .sum();

                let loss = policy_loss + self.config.value_coeff * value_loss
                    - self.config.entropy_coeff * entropy;
                total_loss += loss;
                update_count += 1;

                // Gradient update for policy weights
                for (a, &prob) in probs.iter().enumerate() {
                    let grad = if a == exp.action {
                        (1.0 - prob) * adv
                    } else {
                        -prob * adv
                    };

                    // Clip gradient
                    let grad = grad.clamp(-self.config.max_grad_norm, self.config.max_grad_norm);

                    for (j, &feat) in exp.state.iter().enumerate() {
                        if j < self.config.num_features {
                            self.policy_weights[a][j] += self.config.learning_rate * grad * feat;
                        }
                    }
                    self.policy_bias[a] += self.config.learning_rate * grad;
                }

                // Gradient update for value weights
                let v_error = returns[i] - v;
                let v_grad = v_error.clamp(-self.config.max_grad_norm, self.config.max_grad_norm);
                for (j, &feat) in exp.state.iter().enumerate() {
                    if j < self.config.num_features {
                        self.value_weights[j] +=
                            self.config.learning_rate * self.config.value_coeff * v_grad * feat;
                    }
                }
                self.value_bias += self.config.learning_rate * self.config.value_coeff * v_grad;
            }
        }

        self.total_updates += 1;
        self.replay_buffer.clear();

        if update_count > 0 {
            total_loss / update_count as f64
        } else {
            0.0
        }
    }

    /// Compute Generalized Advantage Estimation (GAE).
    fn compute_gae(
        rewards: &[f64],
        values: &[f64],
        dones: &[bool],
        gamma: f64,
        lambda: f64,
    ) -> Vec<f64> {
        let n = rewards.len();
        if n == 0 {
            return vec![];
        }
        let mut advantages = vec![0.0; n];
        let mut gae = 0.0;

        for t in (0..n).rev() {
            let next_value = if t + 1 < n { values[t + 1] } else { 0.0 };
            let mask = if dones[t] { 0.0 } else { 1.0 };
            let delta = rewards[t] + gamma * next_value * mask - values[t];
            gae = delta + gamma * lambda * mask * gae;
            advantages[t] = gae;
        }

        advantages
    }
}

impl AIModel for PpoAgent {
    fn name(&self) -> &str {
        "ppo"
    }

    fn predict(&self, features: &FeatureRow) -> Result<AIAction> {
        let (action_idx, _) = self.select_action(&features.features);
        Ok(AIAction::from_index(action_idx as u8))
    }

    fn predict_batch(&self, features: &[FeatureRow]) -> Result<Vec<AIAction>> {
        features.iter().map(|f| self.predict(f)).collect()
    }

    fn train(&mut self, data: &[FeatureRow]) -> Result<TrainMetrics> {
        // For each consecutive pair, create an experience with reward from label
        for window in data.windows(2) {
            let state = &window[0];
            let next_state = &window[1];
            let reward = state.label.unwrap_or(0.0);
            let (action, log_prob) = self.select_action(&state.features);
            let value = self.value(&state.features);
            self.store_experience(Experience {
                state: state.features.clone(),
                action,
                reward,
                next_state: next_state.features.clone(),
                done: false,
                log_prob,
                value,
            });
        }

        let loss = self.update();

        Ok(TrainMetrics {
            model_name: self.name().into(),
            samples: data.len(),
            accuracy: 0.0,
            loss,
        })
    }

    fn save(&self, path: &Path) -> Result<()> {
        let checkpoint = PpoCheckpoint {
            config: self.config.clone(),
            policy_weights: self.policy_weights.clone(),
            policy_bias: self.policy_bias.clone(),
            value_weights: self.value_weights.clone(),
            value_bias: self.value_bias,
            total_updates: self.total_updates,
        };
        let json = serde_json::to_string_pretty(&checkpoint)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    fn load(&mut self, path: &Path) -> Result<()> {
        let json = std::fs::read_to_string(path)?;
        let checkpoint: PpoCheckpoint = serde_json::from_str(&json)?;
        self.config = checkpoint.config;
        self.policy_weights = checkpoint.policy_weights;
        self.policy_bias = checkpoint.policy_bias;
        self.value_weights = checkpoint.value_weights;
        self.value_bias = checkpoint.value_bias;
        self.total_updates = checkpoint.total_updates;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct PpoCheckpoint {
    config: PpoConfig,
    policy_weights: Vec<Vec<f64>>,
    policy_bias: Vec<f64>,
    value_weights: Vec<f64>,
    value_bias: f64,
    total_updates: usize,
}

/// Compute softmax probabilities from logits.
fn softmax(logits: &[f64]) -> Vec<f64> {
    if logits.is_empty() {
        return vec![];
    }
    let max = logits.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let exps: Vec<f64> = logits.iter().map(|&l| (l - max).exp()).collect();
    let sum: f64 = exps.iter().sum();
    if sum > 0.0 {
        exps.iter().map(|&e| e / sum).collect()
    } else {
        vec![1.0 / logits.len() as f64; logits.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_agent() -> PpoAgent {
        PpoAgent::new(PpoConfig::default())
    }

    fn make_feature_row(features: Vec<f64>) -> FeatureRow {
        FeatureRow {
            timestamp: 0,
            features,
            label: None,
        }
    }

    #[test]
    fn action_probs_sum_to_one() {
        let agent = default_agent();
        let features = vec![1.0; 22];
        let probs = agent.action_probs(&features);
        assert_eq!(probs.len(), 5);
        let sum: f64 = probs.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-9,
            "action probs should sum to ~1.0, got {sum}"
        );
    }

    #[test]
    fn action_probs_all_non_negative() {
        let agent = default_agent();
        let features = vec![
            -2.0, 3.5, 0.0, 1.0, -0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0,
        ];
        let probs = agent.action_probs(&features);
        for &p in &probs {
            assert!(p >= 0.0, "probability should be non-negative, got {p}");
        }
    }

    #[test]
    fn value_returns_scalar() {
        let agent = default_agent();
        let features = vec![1.0; 22];
        let v = agent.value(&features);
        // With zero weights, value should be 0.0
        assert!(v.is_finite(), "value should be finite, got {v}");
        assert!(
            (v - 0.0).abs() < 1e-9,
            "initial value should be 0.0 with zero weights"
        );
    }

    #[test]
    fn select_action_returns_valid_index() {
        let agent = default_agent();
        let features = vec![0.5; 22];
        let (action, _log_prob) = agent.select_action(&features);
        assert!(action < 5, "action index should be 0-4, got {action}");
    }

    #[test]
    fn select_action_log_prob_is_negative() {
        let agent = default_agent();
        let features = vec![0.5; 22];
        let (_action, log_prob) = agent.select_action(&features);
        assert!(
            log_prob < 0.0,
            "log_prob should be negative, got {log_prob}"
        );
    }

    #[test]
    fn replay_buffer_push_and_len() {
        let mut buf = PpoReplayBuffer::new(100);
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());

        buf.push(Experience {
            state: vec![1.0],
            action: 0,
            reward: 1.0,
            next_state: vec![2.0],
            done: false,
            log_prob: -1.0,
            value: 0.5,
        });

        assert_eq!(buf.len(), 1);
        assert!(!buf.is_empty());
    }

    #[test]
    fn replay_buffer_sample_returns_correct_batch_size() {
        let mut buf = PpoReplayBuffer::new(100);
        for i in 0..50 {
            buf.push(Experience {
                state: vec![i as f64],
                action: 0,
                reward: i as f64,
                next_state: vec![(i + 1) as f64],
                done: false,
                log_prob: -0.5,
                value: 0.0,
            });
        }
        let batch = buf.sample(10);
        assert_eq!(batch.len(), 10);
    }

    #[test]
    fn replay_buffer_sample_clamps_to_len() {
        let mut buf = PpoReplayBuffer::new(100);
        for i in 0..5 {
            buf.push(Experience {
                state: vec![i as f64],
                action: 0,
                reward: 0.0,
                next_state: vec![],
                done: false,
                log_prob: -1.0,
                value: 0.0,
            });
        }
        let batch = buf.sample(50);
        assert_eq!(batch.len(), 5);
    }

    #[test]
    fn replay_buffer_clear_empties_buffer() {
        let mut buf = PpoReplayBuffer::new(100);
        for _ in 0..10 {
            buf.push(Experience {
                state: vec![1.0],
                action: 0,
                reward: 0.0,
                next_state: vec![],
                done: false,
                log_prob: -1.0,
                value: 0.0,
            });
        }
        assert_eq!(buf.len(), 10);
        buf.clear();
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
    }

    #[test]
    fn replay_buffer_capacity_limit_evicts_oldest() {
        let mut buf = PpoReplayBuffer::new(3);
        buf.push(Experience {
            state: vec![1.0],
            action: 0,
            reward: 1.0,
            next_state: vec![],
            done: false,
            log_prob: -1.0,
            value: 0.0,
        });
        buf.push(Experience {
            state: vec![2.0],
            action: 0,
            reward: 2.0,
            next_state: vec![],
            done: false,
            log_prob: -1.0,
            value: 0.0,
        });
        buf.push(Experience {
            state: vec![3.0],
            action: 0,
            reward: 3.0,
            next_state: vec![],
            done: false,
            log_prob: -1.0,
            value: 0.0,
        });
        // Buffer full, push a 4th
        buf.push(Experience {
            state: vec![4.0],
            action: 0,
            reward: 4.0,
            next_state: vec![],
            done: false,
            log_prob: -1.0,
            value: 0.0,
        });
        assert_eq!(buf.len(), 3);
        // Oldest (reward=1.0) should be evicted; remaining should be 2.0, 3.0, 4.0
        let rewards: Vec<f64> = buf.buffer.iter().map(|e| e.reward).collect();
        assert!(
            !rewards.contains(&1.0),
            "oldest experience should be evicted"
        );
        assert!(
            rewards.contains(&4.0),
            "newest experience should be present"
        );
    }

    #[test]
    fn ppo_agent_implements_ai_model_predict() {
        let agent = default_agent();
        let row = make_feature_row(vec![1.0; 22]);
        let action = agent.predict(&row).unwrap();
        // Should be a valid AIAction
        let idx = action.to_index();
        assert!(idx <= 4, "predicted action index should be 0-4, got {idx}");
    }

    #[test]
    fn ppo_agent_save_load_roundtrip() {
        let mut agent = default_agent();
        // Modify some weights so they're non-zero
        agent.policy_weights[1][0] = 0.42;
        agent.policy_weights[3][5] = -1.5;
        agent.value_weights[10] = 3.14;
        agent.value_bias = 0.7;
        agent.total_updates = 99;

        let path = std::env::temp_dir().join("ppo_test_checkpoint.json");
        agent.save(&path).unwrap();

        let mut loaded = default_agent();
        loaded.load(&path).unwrap();

        assert!((loaded.policy_weights[1][0] - 0.42).abs() < 1e-15);
        assert!((loaded.policy_weights[3][5] - (-1.5)).abs() < 1e-15);
        assert!((loaded.value_weights[10] - 3.14).abs() < 1e-15);
        assert!((loaded.value_bias - 0.7).abs() < 1e-15);
        assert_eq!(loaded.total_updates, 99);

        // Predictions should match
        let row = make_feature_row(vec![1.0; 22]);
        let a1 = agent.predict(&row).unwrap();
        let a2 = loaded.predict(&row).unwrap();
        assert_eq!(a1, a2);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn compute_gae_simple_case() {
        // Single step: reward=1.0, value=0.0, done=true
        // advantage = reward + gamma * 0 * 0 - value = 1.0 - 0.0 = 1.0
        let advantages = PpoAgent::compute_gae(&[1.0], &[0.0], &[true], 0.99, 0.95);
        assert_eq!(advantages.len(), 1);
        assert!(
            (advantages[0] - 1.0).abs() < 1e-9,
            "GAE for single done step: expected 1.0, got {}",
            advantages[0]
        );
    }

    #[test]
    fn compute_gae_multi_step() {
        // Two steps, not done
        // t=1 (last): delta = r[1] + gamma * 0 - v[1] = 1.0 + 0 - 0.5 = 0.5
        //   gae[1] = 0.5
        // t=0: delta = r[0] + gamma * v[1] - v[0] = 1.0 + 0.99*0.5 - 0.0 = 1.495
        //   gae[0] = 1.495 + 0.99*0.95*0.5 = 1.495 + 0.47025 = 1.96525
        let advantages =
            PpoAgent::compute_gae(&[1.0, 1.0], &[0.0, 0.5], &[false, false], 0.99, 0.95);
        assert_eq!(advantages.len(), 2);
        assert!(
            (advantages[1] - 0.5).abs() < 1e-9,
            "GAE[1] expected 0.5, got {}",
            advantages[1]
        );
        let expected_0 = 1.495 + 0.99 * 0.95 * 0.5;
        assert!(
            (advantages[0] - expected_0).abs() < 1e-9,
            "GAE[0] expected {expected_0}, got {}",
            advantages[0]
        );
    }

    #[test]
    fn compute_gae_empty() {
        let advantages = PpoAgent::compute_gae(&[], &[], &[], 0.99, 0.95);
        assert!(advantages.is_empty());
    }

    #[test]
    fn ppo_config_default_values_sensible() {
        let config = PpoConfig::default();
        assert_eq!(config.num_features, 22);
        assert_eq!(config.num_actions, 5);
        assert!(config.learning_rate > 0.0 && config.learning_rate < 1.0);
        assert!(config.gamma > 0.0 && config.gamma <= 1.0);
        assert!(config.epsilon_clip > 0.0 && config.epsilon_clip < 1.0);
        assert!(config.value_coeff > 0.0);
        assert!(config.entropy_coeff > 0.0);
        assert!(config.max_grad_norm > 0.0);
        assert!(config.batch_size > 0);
        assert!(config.epochs_per_update > 0);
    }

    #[test]
    fn ppo_agent_is_object_safe() {
        let _m: Box<dyn AIModel> = Box::new(default_agent());
    }

    #[test]
    fn ppo_agent_predict_batch() {
        let agent = default_agent();
        let rows: Vec<FeatureRow> = (0..5)
            .map(|i| FeatureRow {
                timestamp: i,
                features: vec![i as f64; 22],
                label: None,
            })
            .collect();
        let predictions = agent.predict_batch(&rows).unwrap();
        assert_eq!(predictions.len(), 5);
        for p in &predictions {
            assert!(p.to_index() <= 4);
        }
    }

    #[test]
    fn ppo_agent_train_returns_metrics() {
        let mut agent = default_agent();
        let data: Vec<FeatureRow> = (0..10)
            .map(|i| FeatureRow {
                timestamp: i,
                features: vec![0.1 * i as f64; 22],
                label: Some(if i % 2 == 0 { 1.0 } else { -1.0 }),
            })
            .collect();
        let metrics = agent.train(&data).unwrap();
        assert_eq!(metrics.model_name, "ppo");
        assert_eq!(metrics.samples, 10);
        assert!(metrics.loss.is_finite());
    }

    #[test]
    fn ppo_agent_update_with_experiences() {
        let mut agent = default_agent();
        for i in 0..10 {
            agent.store_experience(Experience {
                state: vec![i as f64; 22],
                action: (i % 5) as usize,
                reward: if i % 2 == 0 { 1.0 } else { -0.5 },
                next_state: vec![(i + 1) as f64; 22],
                done: i == 9,
                log_prob: -1.6,
                value: 0.0,
            });
        }
        let loss = agent.update();
        assert!(loss.is_finite(), "update loss should be finite, got {loss}");
        // Buffer should be cleared after update
        assert_eq!(agent.replay_buffer.len(), 0);
    }

    #[test]
    fn ppo_agent_name() {
        let agent = default_agent();
        assert_eq!(agent.name(), "ppo");
    }
}
