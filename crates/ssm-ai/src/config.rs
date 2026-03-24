use serde::{Deserialize, Serialize};

/// Environment configuration for the RL trading environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvConfig {
    /// Trading fee rate per side (e.g., 0.0004 = 0.04% taker fee).
    pub fee_rate: f64,
    /// Slippage rate applied to entry/exit prices (e.g., 0.0001 = 0.01%).
    pub slippage_rate: f64,
    /// Starting balance in USD.
    pub initial_balance: f64,
    /// Fraction of balance to use per trade (1.0 = 100%).
    pub position_size_pct: f64,
    /// Optional maximum episode length in steps.
    pub max_steps: Option<usize>,
    /// Enable hedge mode (simultaneous long + short positions).
    pub hedge_mode: bool,
    /// Maximum gross exposure as fraction of balance (long + short combined).
    pub max_gross_exposure: f64,
    /// Append environment state (position, PnL, duration, exposure) to the
    /// observation feature vector. FreqAI's `add_state_info` equivalent.
    #[serde(default)]
    pub add_state_info: bool,
    /// Maximum candles a position can be held before forced exit.
    /// None = no limit. FreqAI's `max_trade_duration_candles`.
    #[serde(default)]
    pub max_trade_duration_candles: Option<usize>,
}

impl Default for EnvConfig {
    fn default() -> Self {
        Self {
            fee_rate: 0.0,
            slippage_rate: 0.0,
            initial_balance: 10_000.0,
            position_size_pct: 1.0,
            max_steps: None,
            hedge_mode: false,
            max_gross_exposure: 2.0,
            add_state_info: false,
            max_trade_duration_candles: None,
        }
    }
}

/// Reward shaping parameters — tunable hyperparameters for the RL environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardConfig {
    /// Steps holding a position before penalty kicks in during Neutral actions.
    pub hold_penalty_threshold: usize,
    /// Penalty rate per step beyond hold_penalty_threshold.
    pub hold_penalty_rate: f64,
    /// Penalty for invalid actions (e.g., ExitLong with no position).
    pub invalid_action_penalty: f64,
    /// Steps holding before duration penalty applies at position close.
    pub close_penalty_threshold: usize,
    /// Duration penalty rate at close per step beyond threshold.
    pub close_penalty_rate: f64,
    /// Whether to subtract trading fees from the reward signal.
    pub fee_penalty: bool,
    /// Bonus multiplier applied to profitable trades (0.0 = none).
    pub win_bonus: f64,
    /// Penalty proportional to current drawdown (0.0 = none).
    pub drawdown_penalty_rate: f64,
    /// Per-step penalty rate when gross exposure exceeds threshold (0.0 = off).
    pub exposure_penalty_rate: f64,
    /// Gross exposure threshold before penalty applies (1.5 = 150%).
    pub exposure_penalty_threshold: f64,
    /// Per-step bonus when both long and short are open (0.0 = off).
    pub hedge_bonus: f64,
}

impl Default for RewardConfig {
    fn default() -> Self {
        Self {
            hold_penalty_threshold: 20,
            hold_penalty_rate: 0.001,
            invalid_action_penalty: 0.01,
            close_penalty_threshold: 50,
            close_penalty_rate: 0.001,
            fee_penalty: false,
            win_bonus: 0.0,
            drawdown_penalty_rate: 0.0,
            exposure_penalty_rate: 0.0,
            exposure_penalty_threshold: 1.5,
            hedge_bonus: 0.0,
        }
    }
}

/// RL algorithm type (FreqAI `model_type` equivalent).
///
/// Only `PPO` is currently implemented. Other variants are configuration
/// placeholders for future algorithm backends.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum ModelType {
    #[default]
    PPO,
    A2C,
    DQN,
    TRPO,
    ARS,
    RecurrentPPO,
    MaskablePPO,
}

/// Network policy type (FreqAI `policy_type` equivalent).
///
/// Only `MlpPolicy` maps to the current linear PPO implementation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum PolicyType {
    #[default]
    MlpPolicy,
    CnnPolicy,
    MultiInputPolicy,
}

fn default_train_cycles() -> usize {
    10
}
fn default_max_training_drawdown_pct() -> f64 {
    0.8
}
fn default_cpu_count() -> usize {
    1
}
fn default_net_arch() -> Vec<usize> {
    vec![128, 128]
}
fn default_live_retrain_hours() -> f64 {
    24.0
}
fn default_expiration_hours() -> f64 {
    48.0
}

/// Training lifecycle configuration (FreqAI training parameters).
///
/// Groups parameters that control the training loop, model selection,
/// feature engineering, and live deployment scheduling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingConfig {
    /// Training steps = train_cycles × data_points. When 0, falls back to n_epochs.
    #[serde(default = "default_train_cycles")]
    pub train_cycles: usize,

    /// RL algorithm to use.
    #[serde(default)]
    pub model_type: ModelType,

    /// Network policy type.
    #[serde(default)]
    pub policy_type: PolicyType,

    /// Early-stop training if max drawdown exceeds this fraction (0.8 = 80%).
    #[serde(default = "default_max_training_drawdown_pct")]
    pub max_training_drawdown_pct: f64,

    /// Thread count for training (future: parallel env vectorization).
    #[serde(default = "default_cpu_count")]
    pub cpu_count: usize,

    /// Hidden layer sizes for policy and value networks.
    #[serde(default = "default_net_arch")]
    pub net_arch: Vec<usize>,

    /// Vary episode start points for diversity.
    #[serde(default)]
    pub randomize_starting_position: bool,

    /// Remove OHLC features (indices 0-3) from agent input.
    #[serde(default)]
    pub drop_ohlc_from_features: bool,

    /// Display training progress bar (for CLI, future use).
    #[serde(default)]
    pub progress_bar: bool,

    /// Hours between live retraining cycles.
    #[serde(default = "default_live_retrain_hours")]
    pub live_retrain_hours: f64,

    /// Hours until a trained model is considered stale.
    #[serde(default = "default_expiration_hours")]
    pub expiration_hours: f64,

    /// Persist training metrics (loss curves, rewards) to disk.
    #[serde(default)]
    pub write_metrics_to_disk: bool,
}

impl Default for TrainingConfig {
    fn default() -> Self {
        Self {
            train_cycles: default_train_cycles(),
            model_type: ModelType::default(),
            policy_type: PolicyType::default(),
            max_training_drawdown_pct: default_max_training_drawdown_pct(),
            cpu_count: default_cpu_count(),
            net_arch: default_net_arch(),
            randomize_starting_position: false,
            drop_ohlc_from_features: false,
            progress_bar: false,
            live_retrain_hours: default_live_retrain_hours(),
            expiration_hours: default_expiration_hours(),
            write_metrics_to_disk: false,
        }
    }
}

/// Top-level RL configuration combining environment, reward, timeframes, and training.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlConfig {
    pub env: EnvConfig,
    pub reward: RewardConfig,
    /// Timeframe strings to evaluate (e.g., ["3m", "15m", "1h", "4h"]).
    pub timeframes: Vec<String>,
    /// Training lifecycle parameters (FreqAI-style).
    #[serde(default)]
    pub training: TrainingConfig,
}

impl Default for RlConfig {
    fn default() -> Self {
        Self {
            env: EnvConfig::default(),
            reward: RewardConfig::default(),
            timeframes: vec!["15m".to_string()],
            training: TrainingConfig::default(),
        }
    }
}

/// Optimizer configuration for hyperparameter search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizeConfig {
    pub enabled: bool,
    pub objective: String,
    pub method: String,
    pub n_trials: usize,
    pub seed: u64,
}

impl Default for OptimizeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            objective: "SharpeRatio".to_string(),
            method: "random".to_string(),
            n_trials: 100,
            seed: 42,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_env_config_backward_compat() {
        let cfg = EnvConfig::default();
        assert!((cfg.fee_rate - 0.0).abs() < f64::EPSILON);
        assert!((cfg.slippage_rate - 0.0).abs() < f64::EPSILON);
        assert!((cfg.initial_balance - 10_000.0).abs() < f64::EPSILON);
        assert!((cfg.position_size_pct - 1.0).abs() < f64::EPSILON);
        assert!(cfg.max_steps.is_none());
        assert!(!cfg.hedge_mode);
        assert!((cfg.max_gross_exposure - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn default_reward_config_matches_legacy() {
        let cfg = RewardConfig::default();
        assert_eq!(cfg.hold_penalty_threshold, 20);
        assert!((cfg.hold_penalty_rate - 0.001).abs() < f64::EPSILON);
        assert!((cfg.invalid_action_penalty - 0.01).abs() < f64::EPSILON);
        assert_eq!(cfg.close_penalty_threshold, 50);
        assert!((cfg.close_penalty_rate - 0.001).abs() < f64::EPSILON);
        assert!(!cfg.fee_penalty);
        assert!((cfg.win_bonus - 0.0).abs() < f64::EPSILON);
        assert!((cfg.drawdown_penalty_rate - 0.0).abs() < f64::EPSILON);
        assert!((cfg.exposure_penalty_rate - 0.0).abs() < f64::EPSILON);
        assert!((cfg.exposure_penalty_threshold - 1.5).abs() < f64::EPSILON);
        assert!((cfg.hedge_bonus - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn config_serde_roundtrip() {
        let cfg = RlConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: RlConfig = serde_json::from_str(&json).unwrap();
        assert!((parsed.env.initial_balance - cfg.env.initial_balance).abs() < f64::EPSILON);
        assert_eq!(
            parsed.reward.hold_penalty_threshold,
            cfg.reward.hold_penalty_threshold
        );
        assert_eq!(parsed.timeframes, cfg.timeframes);
    }

    #[test]
    fn env_config_custom_values() {
        let cfg = EnvConfig {
            fee_rate: 0.001,
            slippage_rate: 0.0005,
            initial_balance: 50_000.0,
            position_size_pct: 0.5,
            max_steps: Some(200),
            hedge_mode: false,
            max_gross_exposure: 2.0,
            add_state_info: false,
            max_trade_duration_candles: None,
        };
        assert!((cfg.fee_rate - 0.001).abs() < f64::EPSILON);
        assert_eq!(cfg.max_steps, Some(200));
        assert!((cfg.position_size_pct - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn env_config_serde_roundtrip() {
        let cfg = EnvConfig {
            fee_rate: 0.0004,
            slippage_rate: 0.0001,
            initial_balance: 25_000.0,
            position_size_pct: 0.75,
            max_steps: Some(500),
            hedge_mode: false,
            max_gross_exposure: 2.0,
            add_state_info: false,
            max_trade_duration_candles: None,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: EnvConfig = serde_json::from_str(&json).unwrap();
        assert!((parsed.fee_rate - 0.0004).abs() < f64::EPSILON);
        assert!((parsed.slippage_rate - 0.0001).abs() < f64::EPSILON);
        assert_eq!(parsed.max_steps, Some(500));
    }

    #[test]
    fn optimize_config_default() {
        let cfg = OptimizeConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.objective, "SharpeRatio");
        assert_eq!(cfg.method, "random");
        assert_eq!(cfg.n_trials, 100);
        assert_eq!(cfg.seed, 42);
    }

    #[test]
    fn optimize_config_serde_roundtrip() {
        let cfg = OptimizeConfig {
            enabled: true,
            objective: "TotalReturn".to_string(),
            method: "grid".to_string(),
            n_trials: 50,
            seed: 123,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: OptimizeConfig = serde_json::from_str(&json).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.objective, "TotalReturn");
        assert_eq!(parsed.n_trials, 50);
        assert_eq!(parsed.seed, 123);
    }

    #[test]
    fn training_config_default_values() {
        let cfg = TrainingConfig::default();
        assert_eq!(cfg.train_cycles, 10);
        assert!((cfg.max_training_drawdown_pct - 0.8).abs() < f64::EPSILON);
        assert_eq!(cfg.cpu_count, 1);
        assert_eq!(cfg.net_arch, vec![128, 128]);
        assert!(!cfg.randomize_starting_position);
        assert!(!cfg.drop_ohlc_from_features);
        assert!(!cfg.progress_bar);
        assert!((cfg.live_retrain_hours - 24.0).abs() < f64::EPSILON);
        assert!((cfg.expiration_hours - 48.0).abs() < f64::EPSILON);
        assert!(!cfg.write_metrics_to_disk);
        assert_eq!(cfg.model_type, ModelType::PPO);
        assert_eq!(cfg.policy_type, PolicyType::MlpPolicy);
    }

    #[test]
    fn training_config_serde_roundtrip() {
        let cfg = TrainingConfig {
            train_cycles: 20,
            model_type: ModelType::DQN,
            policy_type: PolicyType::CnnPolicy,
            max_training_drawdown_pct: 0.5,
            cpu_count: 4,
            net_arch: vec![256, 256, 128],
            randomize_starting_position: true,
            drop_ohlc_from_features: true,
            progress_bar: true,
            live_retrain_hours: 12.0,
            expiration_hours: 72.0,
            write_metrics_to_disk: true,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: TrainingConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.train_cycles, 20);
        assert_eq!(parsed.model_type, ModelType::DQN);
        assert_eq!(parsed.policy_type, PolicyType::CnnPolicy);
        assert!((parsed.max_training_drawdown_pct - 0.5).abs() < f64::EPSILON);
        assert_eq!(parsed.cpu_count, 4);
        assert_eq!(parsed.net_arch, vec![256, 256, 128]);
        assert!(parsed.randomize_starting_position);
        assert!(parsed.drop_ohlc_from_features);
        assert!(parsed.progress_bar);
        assert!((parsed.live_retrain_hours - 12.0).abs() < f64::EPSILON);
        assert!((parsed.expiration_hours - 72.0).abs() < f64::EPSILON);
        assert!(parsed.write_metrics_to_disk);
    }

    #[test]
    fn rl_config_backwards_compat_no_training_key() {
        let json = r#"{
            "env": {"fee_rate": 0.0, "slippage_rate": 0.0, "initial_balance": 10000.0, "position_size_pct": 1.0, "max_steps": null, "hedge_mode": false, "max_gross_exposure": 2.0},
            "reward": {"hold_penalty_threshold": 20, "hold_penalty_rate": 0.001, "invalid_action_penalty": 0.01, "close_penalty_threshold": 50, "close_penalty_rate": 0.001, "fee_penalty": false, "win_bonus": 0.0, "drawdown_penalty_rate": 0.0, "exposure_penalty_rate": 0.0, "exposure_penalty_threshold": 1.5, "hedge_bonus": 0.0},
            "timeframes": ["15m"]
        }"#;
        let parsed: RlConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.training.train_cycles, 10);
        assert_eq!(parsed.training.model_type, ModelType::PPO);
    }

    #[test]
    fn model_type_serde_all_variants() {
        let variants = vec![
            ModelType::PPO,
            ModelType::A2C,
            ModelType::DQN,
            ModelType::TRPO,
            ModelType::ARS,
            ModelType::RecurrentPPO,
            ModelType::MaskablePPO,
        ];
        for v in variants {
            let json = serde_json::to_string(&v).unwrap();
            let parsed: ModelType = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, v);
        }
    }

    #[test]
    fn policy_type_serde_all_variants() {
        let variants = vec![
            PolicyType::MlpPolicy,
            PolicyType::CnnPolicy,
            PolicyType::MultiInputPolicy,
        ];
        for v in variants {
            let json = serde_json::to_string(&v).unwrap();
            let parsed: PolicyType = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, v);
        }
    }

    #[test]
    fn training_config_partial_json_uses_defaults() {
        let json = r#"{"train_cycles": 5, "drop_ohlc_from_features": true}"#;
        let parsed: TrainingConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.train_cycles, 5);
        assert!(parsed.drop_ohlc_from_features);
        // Rest should be defaults
        assert_eq!(parsed.model_type, ModelType::PPO);
        assert_eq!(parsed.cpu_count, 1);
        assert!((parsed.max_training_drawdown_pct - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn rl_config_default_timeframes() {
        let cfg = RlConfig::default();
        assert_eq!(cfg.timeframes.len(), 1);
        assert_eq!(cfg.timeframes[0], "15m");
    }

    #[test]
    fn reward_config_serde_roundtrip() {
        let cfg = RewardConfig {
            hold_penalty_threshold: 5,
            hold_penalty_rate: 0.05,
            invalid_action_penalty: 0.1,
            close_penalty_threshold: 10,
            close_penalty_rate: 0.02,
            fee_penalty: true,
            win_bonus: 0.5,
            drawdown_penalty_rate: 0.03,
            exposure_penalty_rate: 0.0,
            exposure_penalty_threshold: 1.5,
            hedge_bonus: 0.0,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: RewardConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.hold_penalty_threshold, 5);
        assert!((parsed.hold_penalty_rate - 0.05).abs() < f64::EPSILON);
        assert!(parsed.fee_penalty);
        assert!((parsed.win_bonus - 0.5).abs() < f64::EPSILON);
        assert!((parsed.drawdown_penalty_rate - 0.03).abs() < f64::EPSILON);
    }

    #[test]
    fn env_config_max_steps_none_serde() {
        let cfg = EnvConfig {
            max_steps: None,
            ..EnvConfig::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: EnvConfig = serde_json::from_str(&json).unwrap();
        assert!(parsed.max_steps.is_none());
    }

    #[test]
    fn rl_config_multiple_timeframes_serde() {
        let cfg = RlConfig {
            timeframes: vec!["3m".into(), "15m".into(), "1h".into(), "4h".into()],
            ..RlConfig::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: RlConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.timeframes.len(), 4);
        assert_eq!(parsed.timeframes[0], "3m");
        assert_eq!(parsed.timeframes[3], "4h");
    }

    #[test]
    fn hedge_config_serde_roundtrip() {
        let cfg = EnvConfig {
            hedge_mode: true,
            max_gross_exposure: 1.5,
            ..EnvConfig::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: EnvConfig = serde_json::from_str(&json).unwrap();
        assert!(parsed.hedge_mode);
        assert!((parsed.max_gross_exposure - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn exposure_reward_config_serde_roundtrip() {
        let cfg = RewardConfig {
            exposure_penalty_rate: 0.05,
            exposure_penalty_threshold: 1.2,
            hedge_bonus: 0.01,
            ..RewardConfig::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: RewardConfig = serde_json::from_str(&json).unwrap();
        assert!((parsed.exposure_penalty_rate - 0.05).abs() < f64::EPSILON);
        assert!((parsed.exposure_penalty_threshold - 1.2).abs() < f64::EPSILON);
        assert!((parsed.hedge_bonus - 0.01).abs() < f64::EPSILON);
    }
}
