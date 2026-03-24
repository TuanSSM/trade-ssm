use serde::{Deserialize, Serialize};
use ssm_core::{AIAction, Candle, FeatureRow};

use crate::config::{EnvConfig, RewardConfig, TrainingConfig};
use crate::env::{TradingEnv, STATE_INFO_COUNT};
use crate::episode_sampler::EpisodeSampler;
use crate::features::{drop_ohlc_batch, extract_features, FEATURE_COUNT, FEATURE_COUNT_NO_OHLC};
use crate::metrics::EpisodeMetrics;
use crate::normalize::FeatureNormalizer;
use crate::ppo::{Experience, PpoAgent, PpoConfig};
use crate::reward::{DefaultRewardFn, RewardFn};

/// Configuration for the RL training loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainerConfig {
    pub env: EnvConfig,
    pub reward: RewardConfig,
    pub ppo: PpoConfig,
    /// Number of training epochs (full passes through episode batches).
    pub n_epochs: usize,
    /// Episodes per epoch.
    pub episodes_per_epoch: usize,
    /// CVD window for feature extraction.
    pub cvd_window: usize,
    /// Whether to z-score normalize features before training.
    pub normalize_features: bool,
    /// Steps per year for Sharpe annualization (default: 35040 for 15m candles).
    pub steps_per_year: f64,
    /// Minimum episode length in candles.
    pub min_episode_length: usize,
    /// Training lifecycle parameters (FreqAI-style).
    #[serde(default)]
    pub training: TrainingConfig,
}

impl Default for TrainerConfig {
    fn default() -> Self {
        Self {
            env: EnvConfig::default(),
            reward: RewardConfig::default(),
            ppo: PpoConfig::default(),
            n_epochs: 10,
            episodes_per_epoch: 5,
            cvd_window: 15,
            normalize_features: true,
            steps_per_year: 35040.0,
            min_episode_length: 50,
            training: TrainingConfig::default(),
        }
    }
}

/// Result of a training run.
pub struct TrainResult {
    pub agent: PpoAgent,
    pub normalizer: Option<FeatureNormalizer>,
    pub epoch_metrics: Vec<EpisodeMetrics>,
    pub total_episodes: usize,
}

/// Orchestrates RL training: feature extraction -> normalization -> env episodes -> PPO updates.
///
/// Implements the FreqAI `IFreqaiModel.fit()` pattern as a single `train()` call.
pub struct RlTrainer {
    config: TrainerConfig,
    _reward_fn: Box<dyn RewardFn>,
}

impl RlTrainer {
    pub fn new(config: TrainerConfig) -> Self {
        Self {
            config,
            _reward_fn: Box::new(DefaultRewardFn),
        }
    }

    pub fn with_reward_fn(config: TrainerConfig, reward_fn: Box<dyn RewardFn>) -> Self {
        Self {
            config,
            _reward_fn: reward_fn,
        }
    }

    /// Run the full training loop on the provided candles.
    pub fn train(&self, candles: &[Candle]) -> TrainResult {
        // 1. Extract features
        let features = extract_features(candles, self.config.cvd_window);

        // 1b. Optionally drop OHLC features (indices 0-3)
        let features = if self.config.training.drop_ohlc_from_features {
            drop_ohlc_batch(&features)
        } else {
            features
        };

        // 2. Optionally fit normalizer and transform
        let normalizer = if self.config.normalize_features {
            Some(FeatureNormalizer::fit(&features))
        } else {
            None
        };
        let features = match &normalizer {
            Some(n) => n.transform_batch(&features),
            None => features,
        };

        // 3. Determine input dimension
        let base_features = if self.config.training.drop_ohlc_from_features {
            FEATURE_COUNT_NO_OHLC
        } else {
            FEATURE_COUNT
        };
        let input_dim = if self.config.env.add_state_info {
            base_features + STATE_INFO_COUNT
        } else {
            base_features
        };

        // 4. Create agent with correct dimensions
        let mut ppo_config = self.config.ppo.clone();
        ppo_config.num_features = input_dim;
        let mut agent = PpoAgent::new(ppo_config);

        // 5. Create episode sampler
        let sampler = if self.config.training.randomize_starting_position {
            EpisodeSampler::new(self.config.min_episode_length, candles.len())
        } else {
            // Fixed window: always use full candle range
            EpisodeSampler::new(candles.len(), candles.len())
        };

        // 6. Determine effective epoch count
        let effective_epochs = if self.config.training.train_cycles > 0 {
            let data_points = features.len();
            let total_steps = self.config.training.train_cycles * data_points;
            let steps_per_epoch = self.config.episodes_per_epoch * self.config.min_episode_length;
            if steps_per_epoch > 0 {
                (total_steps / steps_per_epoch).max(1)
            } else {
                self.config.n_epochs
            }
        } else {
            self.config.n_epochs
        };

        // 7. Training loop
        let mut epoch_metrics = Vec::new();
        let mut total_episodes = 0;

        for epoch in 0..effective_epochs {
            let windows =
                sampler.sample_batch(candles, self.config.episodes_per_epoch, epoch as u64);
            for window in windows {
                let window_features = extract_features(window, self.config.cvd_window);
                let window_features = if self.config.training.drop_ohlc_from_features {
                    drop_ohlc_batch(&window_features)
                } else {
                    window_features
                };
                let window_features = match &normalizer {
                    Some(n) => n.transform_batch(&window_features),
                    None => window_features,
                };
                self.run_episode(&mut agent, window, &window_features);
                total_episodes += 1;
            }
            agent.update();

            // Evaluate on full data
            let eval = self.evaluate(&agent, candles, &features);

            // Early stopping on training drawdown
            let max_dd = self.config.training.max_training_drawdown_pct;
            let exceeded = eval.max_drawdown_pct / 100.0 > max_dd;

            epoch_metrics.push(eval);

            if exceeded {
                break;
            }
        }

        // Optionally write metrics to disk
        if self.config.training.write_metrics_to_disk {
            if let Ok(json) = serde_json::to_string_pretty(&epoch_metrics) {
                let _ = std::fs::write("train_metrics.json", json);
            }
        }

        TrainResult {
            agent,
            normalizer,
            epoch_metrics,
            total_episodes,
        }
    }

    fn run_episode(&self, agent: &mut PpoAgent, candles: &[Candle], features: &[FeatureRow]) {
        let mut env = TradingEnv::with_config(
            candles.to_vec(),
            self.config.env.clone(),
            self.config.reward.clone(),
        );
        let mut obs = env.reset();

        while !obs.done {
            let candle_feats = features
                .get(obs.step)
                .map(|r| r.features.as_slice())
                .unwrap_or(&[]);
            let state = env.build_agent_input(candle_feats);
            let (action_idx, log_prob) = agent.select_action(&state);
            let value = agent.value(&state);
            let action = AIAction::from_index(action_idx as u8);

            let (next_obs, reward) = env.step(action);
            let next_candle_feats = features
                .get(next_obs.step)
                .map(|r| r.features.as_slice())
                .unwrap_or(&[]);
            let next_state = env.build_agent_input(next_candle_feats);

            agent.store_experience(Experience {
                state,
                action: action_idx,
                reward,
                next_state,
                done: next_obs.done,
                log_prob,
                value,
            });
            obs = next_obs;
        }
    }

    fn evaluate(
        &self,
        agent: &PpoAgent,
        candles: &[Candle],
        features: &[FeatureRow],
    ) -> EpisodeMetrics {
        let mut env = TradingEnv::with_config(
            candles.to_vec(),
            self.config.env.clone(),
            self.config.reward.clone(),
        );
        let mut obs = env.reset();

        while !obs.done {
            let candle_feats = features
                .get(obs.step)
                .map(|r| r.features.as_slice())
                .unwrap_or(&[]);
            let state = env.build_agent_input(candle_feats);
            let (action_idx, _) = agent.select_action(&state);
            let action = AIAction::from_index(action_idx as u8);
            let (next_obs, _) = env.step(action);
            obs = next_obs;
        }

        env.episode_metrics(self.config.steps_per_year)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn candle_at(close: &str) -> Candle {
        let p = Decimal::from_str(close).unwrap();
        Candle {
            open_time: 0,
            open: p,
            high: p + Decimal::from(5),
            low: p - Decimal::from(5),
            close: p,
            volume: Decimal::from(100),
            close_time: 1000,
            quote_volume: Decimal::ZERO,
            trades: 100,
            taker_buy_volume: Decimal::from(60),
            taker_sell_volume: Decimal::from(40),
        }
    }

    fn make_candles(n: usize) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let price = format!("{}", 100 + (i % 10));
                candle_at(&price)
            })
            .collect()
    }

    #[test]
    fn train_on_small_dataset() {
        let candles = make_candles(60);
        let config = TrainerConfig {
            n_epochs: 2,
            episodes_per_epoch: 2,
            min_episode_length: 10,
            normalize_features: true,
            training: TrainingConfig {
                train_cycles: 0,
                ..TrainingConfig::default()
            },
            ..TrainerConfig::default()
        };
        let trainer = RlTrainer::new(config);
        let result = trainer.train(&candles);

        assert_eq!(result.total_episodes, 4);
        assert_eq!(result.epoch_metrics.len(), 2);
        assert!(result.normalizer.is_some());
    }

    #[test]
    fn train_without_normalization() {
        let candles = make_candles(60);
        let config = TrainerConfig {
            n_epochs: 1,
            episodes_per_epoch: 1,
            min_episode_length: 10,
            normalize_features: false,
            training: TrainingConfig {
                train_cycles: 0,
                ..TrainingConfig::default()
            },
            ..TrainerConfig::default()
        };
        let trainer = RlTrainer::new(config);
        let result = trainer.train(&candles);
        assert!(result.normalizer.is_none());
    }

    #[test]
    fn train_with_state_info() {
        let candles = make_candles(60);
        let config = TrainerConfig {
            n_epochs: 1,
            episodes_per_epoch: 1,
            min_episode_length: 10,
            env: EnvConfig {
                add_state_info: true,
                ..EnvConfig::default()
            },
            training: TrainingConfig {
                train_cycles: 0,
                ..TrainingConfig::default()
            },
            ..TrainerConfig::default()
        };
        let trainer = RlTrainer::new(config);
        let result = trainer.train(&candles);
        assert_eq!(result.epoch_metrics.len(), 1);
    }

    #[test]
    fn trainer_config_serde_roundtrip() {
        let config = TrainerConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: TrainerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.n_epochs, config.n_epochs);
        assert_eq!(parsed.episodes_per_epoch, config.episodes_per_epoch);
    }

    #[test]
    fn train_with_drop_ohlc() {
        let candles = make_candles(60);
        let config = TrainerConfig {
            n_epochs: 1,
            episodes_per_epoch: 1,
            min_episode_length: 10,
            training: TrainingConfig {
                train_cycles: 0,
                drop_ohlc_from_features: true,
                ..TrainingConfig::default()
            },
            ..TrainerConfig::default()
        };
        let trainer = RlTrainer::new(config);
        let result = trainer.train(&candles);
        assert_eq!(result.epoch_metrics.len(), 1);
    }

    #[test]
    fn train_early_stop_on_drawdown() {
        let candles = make_candles(60);
        let config = TrainerConfig {
            n_epochs: 100,
            episodes_per_epoch: 2,
            min_episode_length: 10,
            training: TrainingConfig {
                train_cycles: 0,
                max_training_drawdown_pct: 0.0, // Any drawdown stops training
                ..TrainingConfig::default()
            },
            ..TrainerConfig::default()
        };
        let trainer = RlTrainer::new(config);
        let result = trainer.train(&candles);
        // Should stop early (much fewer than 100 epochs)
        assert!(
            result.epoch_metrics.len() < 100,
            "expected early stop, got {} epochs",
            result.epoch_metrics.len()
        );
    }

    #[test]
    fn train_with_randomize_starting_position() {
        let candles = make_candles(60);
        let config = TrainerConfig {
            n_epochs: 2,
            episodes_per_epoch: 2,
            min_episode_length: 10,
            training: TrainingConfig {
                train_cycles: 0,
                randomize_starting_position: true,
                ..TrainingConfig::default()
            },
            ..TrainerConfig::default()
        };
        let trainer = RlTrainer::new(config);
        let result = trainer.train(&candles);
        assert_eq!(result.epoch_metrics.len(), 2);
    }

    #[test]
    fn train_cycles_controls_epochs() {
        let candles = make_candles(60);
        let config = TrainerConfig {
            n_epochs: 100, // Should be overridden by train_cycles
            episodes_per_epoch: 1,
            min_episode_length: 10,
            training: TrainingConfig {
                train_cycles: 1, // 1 * ~15 features / (1 * 10) = ~1-2 epochs
                ..TrainingConfig::default()
            },
            ..TrainerConfig::default()
        };
        let trainer = RlTrainer::new(config);
        let result = trainer.train(&candles);
        // train_cycles=1 with ~15 data points and 10 steps_per_epoch => ~1-2 epochs
        assert!(
            result.epoch_metrics.len() < 100,
            "train_cycles should limit epochs, got {}",
            result.epoch_metrics.len()
        );
    }

    #[test]
    fn trainer_config_with_training_serde_roundtrip() {
        let config = TrainerConfig {
            training: TrainingConfig {
                train_cycles: 5,
                drop_ohlc_from_features: true,
                ..TrainingConfig::default()
            },
            ..TrainerConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: TrainerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.training.train_cycles, 5);
        assert!(parsed.training.drop_ohlc_from_features);
    }

    #[test]
    fn train_with_max_trade_duration() {
        let candles = make_candles(60);
        let config = TrainerConfig {
            n_epochs: 1,
            episodes_per_epoch: 1,
            min_episode_length: 10,
            env: EnvConfig {
                max_trade_duration_candles: Some(5),
                ..EnvConfig::default()
            },
            training: TrainingConfig {
                train_cycles: 0,
                ..TrainingConfig::default()
            },
            ..TrainerConfig::default()
        };
        let trainer = RlTrainer::new(config);
        let result = trainer.train(&candles);
        assert_eq!(result.epoch_metrics.len(), 1);
    }
}
