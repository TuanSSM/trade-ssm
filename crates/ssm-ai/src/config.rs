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
}

impl Default for EnvConfig {
    fn default() -> Self {
        Self {
            fee_rate: 0.0,
            slippage_rate: 0.0,
            initial_balance: 10_000.0,
            position_size_pct: 1.0,
            max_steps: None,
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
        }
    }
}

/// Top-level RL configuration combining environment, reward, and timeframes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlConfig {
    pub env: EnvConfig,
    pub reward: RewardConfig,
    /// Timeframe strings to evaluate (e.g., ["3m", "15m", "1h", "4h"]).
    pub timeframes: Vec<String>,
}

impl Default for RlConfig {
    fn default() -> Self {
        Self {
            env: EnvConfig::default(),
            reward: RewardConfig::default(),
            timeframes: vec!["15m".to_string()],
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
}
