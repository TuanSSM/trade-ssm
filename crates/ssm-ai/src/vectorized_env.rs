use ssm_core::{AIAction, Candle};

use crate::config::{EnvConfig, RewardConfig};
use crate::env::{Observation, TradingEnv};

/// Vectorized environment — runs N environments in parallel.
///
/// Collects batches of (observation, reward) tuples for efficient training.
pub struct VectorizedEnv {
    envs: Vec<TradingEnv>,
}

impl VectorizedEnv {
    /// Create N environments, each with its own candle data.
    pub fn new(
        candle_sets: Vec<Vec<Candle>>,
        env_config: EnvConfig,
        reward_config: RewardConfig,
    ) -> Self {
        let envs = candle_sets
            .into_iter()
            .map(|candles| {
                TradingEnv::with_config(candles, env_config.clone(), reward_config.clone())
            })
            .collect();
        Self { envs }
    }

    /// Number of parallel environments.
    pub fn num_envs(&self) -> usize {
        self.envs.len()
    }

    /// Reset all environments and return initial observations.
    pub fn reset_all(&mut self) -> Vec<Observation> {
        self.envs.iter_mut().map(|env| env.reset()).collect()
    }

    /// Step all environments with the given actions.
    ///
    /// Returns (observations, rewards). If an env is done, its observation
    /// reflects the terminal state.
    pub fn step_all(&mut self, actions: &[AIAction]) -> (Vec<Observation>, Vec<f64>) {
        assert_eq!(
            actions.len(),
            self.envs.len(),
            "action count must match env count"
        );

        let mut observations = Vec::with_capacity(self.envs.len());
        let mut rewards = Vec::with_capacity(self.envs.len());

        for (env, &action) in self.envs.iter_mut().zip(actions.iter()) {
            let (obs, reward) = env.step(action);
            observations.push(obs);
            rewards.push(reward);
        }

        (observations, rewards)
    }

    /// Get the number of environments.
    pub fn len(&self) -> usize {
        self.envs.len()
    }

    /// Check if there are no environments.
    pub fn is_empty(&self) -> bool {
        self.envs.is_empty()
    }

    /// Get total rewards for each environment.
    pub fn total_rewards(&self) -> Vec<f64> {
        self.envs.iter().map(|env| env.total_reward()).collect()
    }

    /// Get trade counts for each environment.
    pub fn trade_counts(&self) -> Vec<u32> {
        self.envs.iter().map(|env| env.trade_count()).collect()
    }

    /// Collect a full rollout from all environments.
    ///
    /// Steps all envs with the policy function until all are done.
    /// Returns per-env rollouts: `Vec<(observations, actions, rewards)>`.
    pub fn collect_rollouts(
        &mut self,
        policy: &dyn Fn(&Observation, usize) -> AIAction,
        max_steps: usize,
    ) -> Vec<Rollout> {
        let mut rollouts: Vec<Rollout> = (0..self.envs.len())
            .map(|_| Rollout {
                observations: Vec::new(),
                actions: Vec::new(),
                rewards: Vec::new(),
            })
            .collect();

        let mut observations = self.reset_all();

        for _ in 0..max_steps {
            // Collect actions from policy
            let actions: Vec<AIAction> = observations
                .iter()
                .enumerate()
                .map(|(i, obs)| policy(obs, i))
                .collect();

            // Store pre-step data
            for (i, (obs, &action)) in observations.iter().zip(actions.iter()).enumerate() {
                rollouts[i].observations.push(obs.clone());
                rollouts[i].actions.push(action);
            }

            // Step all envs
            let (new_obs, rewards) = self.step_all(&actions);

            for (i, reward) in rewards.iter().enumerate() {
                rollouts[i].rewards.push(*reward);
            }

            // Check if all done
            let all_done = new_obs.iter().all(|o| o.done);
            observations = new_obs;

            if all_done {
                break;
            }
        }

        rollouts
    }
}

/// A rollout collected from a single environment.
#[derive(Debug, Clone)]
pub struct Rollout {
    pub observations: Vec<Observation>,
    pub actions: Vec<AIAction>,
    pub rewards: Vec<f64>,
}

impl Rollout {
    /// Compute discounted returns for each step.
    pub fn compute_returns(&self, gamma: f64) -> Vec<f64> {
        let n = self.rewards.len();
        let mut returns = vec![0.0; n];
        if n == 0 {
            return returns;
        }

        returns[n - 1] = self.rewards[n - 1];
        for i in (0..n - 1).rev() {
            returns[i] = self.rewards[i] + gamma * returns[i + 1];
        }
        returns
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn candle_price(price: &str) -> Candle {
        let p = Decimal::from_str(price).unwrap();
        Candle {
            open_time: 0,
            open: p,
            high: p,
            low: p,
            close: p,
            volume: Decimal::from(100),
            close_time: 0,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: Decimal::from(50),
            taker_sell_volume: Decimal::from(50),
        }
    }

    fn make_candles(n: usize) -> Vec<Candle> {
        (0..n)
            .map(|i| candle_price(&format!("{}", 100 + i % 10)))
            .collect()
    }

    #[test]
    fn vectorized_basic() {
        let candle_sets = vec![make_candles(20), make_candles(20)];
        let mut venv =
            VectorizedEnv::new(candle_sets, EnvConfig::default(), RewardConfig::default());
        assert_eq!(venv.num_envs(), 2);

        let obs = venv.reset_all();
        assert_eq!(obs.len(), 2);
    }

    #[test]
    fn step_all_returns_correct_count() {
        let candle_sets = vec![make_candles(20), make_candles(20), make_candles(20)];
        let mut venv =
            VectorizedEnv::new(candle_sets, EnvConfig::default(), RewardConfig::default());
        venv.reset_all();

        let actions = vec![AIAction::Neutral, AIAction::EnterLong, AIAction::Neutral];
        let (obs, rewards) = venv.step_all(&actions);
        assert_eq!(obs.len(), 3);
        assert_eq!(rewards.len(), 3);
    }

    #[test]
    fn collect_rollouts_basic() {
        let candle_sets = vec![make_candles(10), make_candles(10)];
        let mut venv =
            VectorizedEnv::new(candle_sets, EnvConfig::default(), RewardConfig::default());

        let rollouts = venv.collect_rollouts(&|_, _| AIAction::Neutral, 100);
        assert_eq!(rollouts.len(), 2);
        assert!(!rollouts[0].observations.is_empty());
        assert_eq!(rollouts[0].observations.len(), rollouts[0].actions.len());
        assert_eq!(rollouts[0].observations.len(), rollouts[0].rewards.len());
    }

    #[test]
    fn discounted_returns() {
        let rollout = Rollout {
            observations: Vec::new(),
            actions: Vec::new(),
            rewards: vec![1.0, 1.0, 1.0],
        };
        let returns = rollout.compute_returns(0.99);
        assert_eq!(returns.len(), 3);
        // Last = 1.0, second = 1.0 + 0.99*1.0 = 1.99, first = 1.0 + 0.99*1.99
        assert!((returns[2] - 1.0).abs() < 1e-10);
        assert!((returns[1] - 1.99).abs() < 1e-10);
        assert!((returns[0] - (1.0 + 0.99 * 1.99)).abs() < 1e-10);
    }
}
