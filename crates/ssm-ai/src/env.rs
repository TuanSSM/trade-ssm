use rust_decimal::prelude::ToPrimitive;
use ssm_core::{AIAction, Candle, Side};

use crate::config::{EnvConfig, RewardConfig};
use crate::metrics::{CompletedTrade, EpisodeMetrics, MetricsAccumulator};

/// Reinforcement learning environment (FreqAI Base5Action inspired).
///
/// The agent steps through candles one at a time, takes actions, and receives rewards.
/// This environment is deterministic given the same candle sequence and configuration.
pub struct TradingEnv {
    candles: Vec<Candle>,
    step: usize,
    position: Option<EnvPosition>,
    total_reward: f64,
    trade_count: u32,
    config: EnvConfig,
    reward_config: RewardConfig,
    balance: f64,
    metrics: MetricsAccumulator,
}

struct EnvPosition {
    side: Side,
    entry_price: f64,
    entry_step: usize,
}

/// Observation returned to the agent after each step.
#[derive(Debug, Clone)]
pub struct Observation {
    pub step: usize,
    pub current_price: f64,
    pub position_side: Option<Side>,
    pub unrealized_pnl: f64,
    pub hold_duration: usize,
    pub done: bool,
    pub balance: f64,
    pub equity: f64,
}

impl TradingEnv {
    /// Create environment with default configuration (backward compatible).
    pub fn new(candles: Vec<Candle>) -> Self {
        Self::with_config(candles, EnvConfig::default(), RewardConfig::default())
    }

    /// Create environment with custom configuration.
    pub fn with_config(
        candles: Vec<Candle>,
        config: EnvConfig,
        reward_config: RewardConfig,
    ) -> Self {
        let first_price = candles
            .first()
            .and_then(|c| c.close.to_f64())
            .unwrap_or(0.0);
        let balance = config.initial_balance;
        Self {
            candles,
            step: 0,
            position: None,
            total_reward: 0.0,
            trade_count: 0,
            metrics: MetricsAccumulator::new(balance, first_price),
            balance,
            config,
            reward_config,
        }
    }

    pub fn reset(&mut self) -> Observation {
        self.step = 0;
        self.position = None;
        self.total_reward = 0.0;
        self.trade_count = 0;
        self.balance = self.config.initial_balance;
        let first_price = self
            .candles
            .first()
            .and_then(|c| c.close.to_f64())
            .unwrap_or(0.0);
        self.metrics = MetricsAccumulator::new(self.balance, first_price);
        self.observe()
    }

    pub fn total_reward(&self) -> f64 {
        self.total_reward
    }

    pub fn trade_count(&self) -> u32 {
        self.trade_count
    }

    pub fn balance(&self) -> f64 {
        self.balance
    }

    /// Compute final episode metrics. `steps_per_year` is used for Sharpe annualization.
    pub fn episode_metrics(&self, steps_per_year: f64) -> EpisodeMetrics {
        let mut acc = self.metrics.clone();
        acc.set_balance(self.balance);
        acc.finalize(steps_per_year)
    }

    /// Take an action, advance one candle, return (observation, reward).
    pub fn step(&mut self, action: AIAction) -> (Observation, f64) {
        let price = self.current_price();
        let mut reward = 0.0;

        match action {
            AIAction::EnterLong if self.position.is_none() => {
                let slipped = price * (1.0 + self.config.slippage_rate);
                self.position = Some(EnvPosition {
                    side: Side::Buy,
                    entry_price: slipped,
                    entry_step: self.step,
                });
            }
            AIAction::EnterShort if self.position.is_none() => {
                let slipped = price * (1.0 - self.config.slippage_rate);
                self.position = Some(EnvPosition {
                    side: Side::Sell,
                    entry_price: slipped,
                    entry_step: self.step,
                });
            }
            AIAction::ExitLong if matches!(&self.position, Some(p) if p.side == Side::Buy) => {
                reward = self.close_position(price);
            }
            AIAction::ExitShort if matches!(&self.position, Some(p) if p.side == Side::Sell) => {
                reward = self.close_position(price);
            }
            AIAction::Neutral => {
                if let Some(pos) = &self.position {
                    let duration = self.step - pos.entry_step;
                    if duration > self.reward_config.hold_penalty_threshold {
                        reward = -self.reward_config.hold_penalty_rate * duration as f64;
                    }
                }
            }
            _ => {
                reward = -self.reward_config.invalid_action_penalty;
            }
        }

        self.total_reward += reward;
        self.step += 1;

        let equity = self.compute_equity();
        let obs_price = self.current_price();
        self.metrics.record_step(equity, obs_price);

        (self.observe(), reward)
    }

    fn close_position(&mut self, exit_price: f64) -> f64 {
        if let Some(pos) = self.position.take() {
            self.trade_count += 1;
            let duration = self.step - pos.entry_step;

            // Apply slippage to exit
            let effective_exit = match pos.side {
                Side::Buy => exit_price * (1.0 - self.config.slippage_rate),
                Side::Sell => exit_price * (1.0 + self.config.slippage_rate),
            };

            // Compute raw PnL percentage
            let pnl_pct = match pos.side {
                Side::Buy => (effective_exit - pos.entry_price) / pos.entry_price,
                Side::Sell => (pos.entry_price - effective_exit) / pos.entry_price,
            };

            // Compute fees on both entry and exit
            let notional = self.balance * self.config.position_size_pct;
            let fees = 2.0 * self.config.fee_rate * notional;
            let pnl_dollar = pnl_pct * notional - fees;

            // Update balance
            self.balance += pnl_dollar;
            self.metrics.set_balance(self.balance);

            // Record trade
            self.metrics.record_trade(CompletedTrade {
                side: pos.side,
                entry_price: pos.entry_price,
                exit_price: effective_exit,
                duration,
                pnl: pnl_dollar,
                fees,
            });

            // Compute reward
            let duration_penalty = if duration > self.reward_config.close_penalty_threshold {
                self.reward_config.close_penalty_rate * duration as f64
            } else {
                0.0
            };

            let fee_penalty = if self.reward_config.fee_penalty {
                2.0 * self.config.fee_rate
            } else {
                0.0
            };

            let mut reward = pnl_pct - duration_penalty - fee_penalty;

            // Win bonus
            if pnl_pct > 0.0 && self.reward_config.win_bonus > 0.0 {
                reward += pnl_pct * self.reward_config.win_bonus;
            }

            reward
        } else {
            0.0
        }
    }

    fn current_price(&self) -> f64 {
        self.candles
            .get(self.step)
            .and_then(|c| c.close.to_f64())
            .unwrap_or(0.0)
    }

    fn compute_equity(&self) -> f64 {
        let price = self.current_price();
        if let Some(pos) = &self.position {
            let notional = self.balance * self.config.position_size_pct;
            let unrealized_pnl = match pos.side {
                Side::Buy => (price - pos.entry_price) / pos.entry_price * notional,
                Side::Sell => (pos.entry_price - price) / pos.entry_price * notional,
            };
            self.balance + unrealized_pnl
        } else {
            self.balance
        }
    }

    fn observe(&self) -> Observation {
        let price = self.current_price();
        let (side, pnl, duration) = if let Some(pos) = &self.position {
            let pnl = match pos.side {
                Side::Buy => (price - pos.entry_price) / pos.entry_price,
                Side::Sell => (pos.entry_price - price) / pos.entry_price,
            };
            (Some(pos.side), pnl, self.step - pos.entry_step)
        } else {
            (None, 0.0, 0)
        };

        let equity = self.compute_equity();

        Observation {
            step: self.step,
            current_price: price,
            position_side: side,
            unrealized_pnl: pnl,
            hold_duration: duration,
            done: self.is_done(),
            balance: self.balance,
            equity,
        }
    }

    fn is_done(&self) -> bool {
        let at_end = self.step >= self.candles.len().saturating_sub(1);
        let at_max = self.config.max_steps.is_some_and(|max| self.step >= max);
        at_end || at_max
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

    #[test]
    fn long_profit() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"),
            candle_price("120"),
        ];
        let mut env = TradingEnv::new(candles);
        env.reset();

        // Enter long at 100
        let (obs, _) = env.step(AIAction::EnterLong);
        assert!(obs.position_side.is_some());

        // Exit at 110 → 10% profit
        let (_, reward) = env.step(AIAction::ExitLong);
        assert!(reward > 0.0, "expected profit, got {reward}");
        assert_eq!(env.trade_count(), 1);
    }

    #[test]
    fn short_profit() {
        let candles = vec![candle_price("100"), candle_price("90"), candle_price("80")];
        let mut env = TradingEnv::new(candles);
        env.reset();

        env.step(AIAction::EnterShort);
        let (_, reward) = env.step(AIAction::ExitShort);
        assert!(reward > 0.0);
    }

    #[test]
    fn invalid_action_penalized() {
        let candles = vec![candle_price("100"), candle_price("100")];
        let mut env = TradingEnv::new(candles);
        env.reset();

        // Exit long with no position
        let (_, reward) = env.step(AIAction::ExitLong);
        assert!(reward < 0.0);
    }

    #[test]
    fn neutral_no_penalty_short_hold() {
        let candles = vec![candle_price("100"), candle_price("100")];
        let mut env = TradingEnv::new(candles);
        env.reset();

        let (_, reward) = env.step(AIAction::Neutral);
        assert!((reward - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn env_done_at_last_candle() {
        let candles = vec![candle_price("100"), candle_price("110")];
        let mut env = TradingEnv::new(candles);
        env.reset();
        let (obs, _) = env.step(AIAction::Neutral);
        assert!(obs.done);
    }

    #[test]
    fn fee_deduction_on_trade() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"),
            candle_price("120"),
        ];
        let env_cfg = EnvConfig {
            fee_rate: 0.001,
            ..EnvConfig::default()
        };
        let reward_cfg = RewardConfig {
            fee_penalty: true,
            ..RewardConfig::default()
        };
        let mut env = TradingEnv::with_config(candles, env_cfg, reward_cfg);
        env.reset();

        env.step(AIAction::EnterLong);
        let (_, reward) = env.step(AIAction::ExitLong);
        // 10% profit minus fee penalty (2 * 0.001 = 0.002)
        assert!(reward > 0.0);
        assert!(
            reward < 0.1,
            "reward should be reduced by fees, got {reward}"
        );
    }

    #[test]
    fn slippage_applied() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"),
            candle_price("120"),
        ];
        let env_cfg = EnvConfig {
            slippage_rate: 0.01,
            ..EnvConfig::default()
        };
        let mut env = TradingEnv::with_config(candles, env_cfg, RewardConfig::default());
        env.reset();

        env.step(AIAction::EnterLong);
        let (_, reward_with_slippage) = env.step(AIAction::ExitLong);

        // Without slippage: 10%. With slippage: entry at 101, exit at ~108.9
        // Should be less than 10%
        assert!(reward_with_slippage < 0.1);
        assert!(reward_with_slippage > 0.0);
    }

    #[test]
    fn balance_tracking() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"),
            candle_price("120"),
        ];
        let mut env = TradingEnv::new(candles);
        env.reset();

        let initial = env.balance();
        assert!((initial - 10_000.0).abs() < f64::EPSILON);

        env.step(AIAction::EnterLong);
        env.step(AIAction::ExitLong);
        // Balance should increase with profit
        assert!(env.balance() > initial);
    }

    #[test]
    fn max_steps_terminates_episode() {
        let candles: Vec<_> = (0..100).map(|_| candle_price("100")).collect();
        let env_cfg = EnvConfig {
            max_steps: Some(10),
            ..EnvConfig::default()
        };
        let mut env = TradingEnv::with_config(candles, env_cfg, RewardConfig::default());
        env.reset();

        for _ in 0..9 {
            let (obs, _) = env.step(AIAction::Neutral);
            assert!(!obs.done);
        }
        let (obs, _) = env.step(AIAction::Neutral);
        assert!(obs.done);
    }

    #[test]
    fn observation_has_equity() {
        let candles = vec![candle_price("100"), candle_price("110")];
        let mut env = TradingEnv::new(candles);
        let obs = env.reset();
        assert!((obs.equity - 10_000.0).abs() < f64::EPSILON);
        assert!((obs.balance - 10_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn reset_clears_state() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"),
            candle_price("120"),
        ];
        let mut env = TradingEnv::new(candles);
        env.reset();
        env.step(AIAction::EnterLong);
        env.step(AIAction::ExitLong);
        assert!(env.trade_count() > 0);
        assert!(env.total_reward() != 0.0 || env.balance() != 10_000.0);

        // After reset, everything should be clean
        let obs = env.reset();
        assert_eq!(obs.step, 0);
        assert!(obs.position_side.is_none());
        assert_eq!(env.trade_count(), 0);
        assert!((env.total_reward() - 0.0).abs() < f64::EPSILON);
        assert!((env.balance() - 10_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn double_enter_is_invalid() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"),
            candle_price("120"),
        ];
        let mut env = TradingEnv::new(candles);
        env.reset();
        env.step(AIAction::EnterLong);
        // Second enter while already in position should be penalized
        let (_, reward) = env.step(AIAction::EnterLong);
        assert!(reward < 0.0, "duplicate entry should be penalized");
    }

    #[test]
    fn exit_wrong_side_is_invalid() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"),
            candle_price("120"),
        ];
        let mut env = TradingEnv::new(candles);
        env.reset();
        env.step(AIAction::EnterLong);
        // ExitShort while holding long should be penalized
        let (_, reward) = env.step(AIAction::ExitShort);
        assert!(reward < 0.0, "exit wrong side should be penalized");
    }

    #[test]
    fn hold_penalty_after_threshold() {
        let candles: Vec<_> = (0..30).map(|_| candle_price("100")).collect();
        let reward_cfg = RewardConfig {
            hold_penalty_threshold: 5,
            hold_penalty_rate: 0.01,
            ..RewardConfig::default()
        };
        let mut env = TradingEnv::with_config(candles, EnvConfig::default(), reward_cfg);
        env.reset();
        env.step(AIAction::EnterLong);

        // Step with Neutral until past threshold
        let mut penalty_observed = false;
        for _ in 0..10 {
            let (_, reward) = env.step(AIAction::Neutral);
            if reward < 0.0 {
                penalty_observed = true;
            }
        }
        assert!(penalty_observed, "hold penalty should apply after threshold");
    }

    #[test]
    fn equity_reflects_unrealized_pnl() {
        let candles = vec![
            candle_price("100"),
            candle_price("120"),
            candle_price("130"),
        ];
        let mut env = TradingEnv::new(candles);
        env.reset();
        // Enter long at 100
        let (obs, _) = env.step(AIAction::EnterLong);
        // Price is now 120, we have unrealized gain
        assert!(obs.equity > obs.balance, "equity should exceed balance with unrealized gain");
        assert!(obs.unrealized_pnl > 0.0);
    }

    #[test]
    fn short_loss_decreases_balance() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"), // price goes up = short loses
            candle_price("120"),
        ];
        let mut env = TradingEnv::new(candles);
        env.reset();
        env.step(AIAction::EnterShort);
        let (_, reward) = env.step(AIAction::ExitShort);
        assert!(reward < 0.0, "short should lose when price rises");
        assert!(env.balance() < 10_000.0);
    }

    #[test]
    fn neutral_with_no_position_zero_reward() {
        let candles: Vec<_> = (0..5).map(|_| candle_price("100")).collect();
        let mut env = TradingEnv::new(candles);
        env.reset();
        // No position, neutral should always give zero reward
        for _ in 0..4 {
            let (_, reward) = env.step(AIAction::Neutral);
            assert!((reward - 0.0).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn episode_metrics_after_trading() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"),
            candle_price("120"),
        ];
        let mut env = TradingEnv::new(candles);
        env.reset();
        env.step(AIAction::EnterLong);
        env.step(AIAction::ExitLong);
        let metrics = env.episode_metrics(35040.0);
        assert_eq!(metrics.total_trades, 1);
        assert!(metrics.total_return_pct > 0.0);
        assert_eq!(metrics.winning_trades, 1);
    }

    #[test]
    fn win_bonus_increases_reward() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"),
            candle_price("120"),
        ];
        // Without win bonus
        let mut env_no_bonus = TradingEnv::with_config(
            candles.clone(),
            EnvConfig::default(),
            RewardConfig::default(),
        );
        env_no_bonus.reset();
        env_no_bonus.step(AIAction::EnterLong);
        let (_, reward_no) = env_no_bonus.step(AIAction::ExitLong);

        // With win bonus
        let reward_cfg = RewardConfig {
            win_bonus: 1.0,
            ..RewardConfig::default()
        };
        let mut env_bonus =
            TradingEnv::with_config(candles, EnvConfig::default(), reward_cfg);
        env_bonus.reset();
        env_bonus.step(AIAction::EnterLong);
        let (_, reward_yes) = env_bonus.step(AIAction::ExitLong);

        assert!(reward_yes > reward_no, "win bonus should increase reward for profitable trade");
    }
}
