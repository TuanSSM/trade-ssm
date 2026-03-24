use rust_decimal::prelude::ToPrimitive;
use ssm_core::{AIAction, Candle, Side};

use crate::config::{EnvConfig, RewardConfig};
use crate::metrics::{CompletedTrade, EpisodeMetrics, MetricsAccumulator};
use crate::reward::{DefaultRewardFn, PositionInfo, RewardContext, RewardFn, TradeResult};

/// Reinforcement learning environment (FreqAI Base5Action inspired).
///
/// The agent steps through candles one at a time, takes actions, and receives rewards.
/// This environment is deterministic given the same candle sequence and configuration.
pub struct TradingEnv {
    candles: Vec<Candle>,
    step: usize,
    long_position: Option<EnvPosition>,
    short_position: Option<EnvPosition>,
    total_reward: f64,
    trade_count: u32,
    config: EnvConfig,
    reward_config: RewardConfig,
    balance: f64,
    metrics: MetricsAccumulator,
    equity_peak: f64,
    reward_fn: Box<dyn RewardFn>,
}

struct EnvPosition {
    side: Side,
    entry_price: f64,
    entry_step: usize,
}

/// Number of state features appended when `add_state_info` is enabled.
pub const STATE_INFO_COUNT: usize = 8;

/// Observation returned to the agent after each step.
#[derive(Debug, Clone)]
pub struct Observation {
    pub step: usize,
    pub current_price: f64,
    /// Legacy field: long side if long open, else short side if short open, else None.
    pub position_side: Option<Side>,
    /// Legacy field: unrealized PnL from primary position (long preferred).
    pub unrealized_pnl: f64,
    /// Legacy field: hold duration from primary position (long preferred).
    pub hold_duration: usize,
    pub done: bool,
    pub balance: f64,
    pub equity: f64,
    // Hedge mode observation fields
    pub long_position_active: bool,
    pub long_unrealized_pnl: f64,
    pub long_hold_duration: usize,
    pub short_position_active: bool,
    pub short_unrealized_pnl: f64,
    pub short_hold_duration: usize,
    /// Long fraction minus short fraction of balance.
    pub net_exposure: f64,
    /// Long fraction plus short fraction of balance.
    pub gross_exposure: f64,
}

impl Observation {
    /// Convert environment state into feature values for model input.
    ///
    /// Returns 8 features (STATE_INFO_COUNT):
    ///   0: long_active (0.0 or 1.0)
    ///   1: short_active (0.0 or 1.0)
    ///   2: long_unrealized_pnl
    ///   3: short_unrealized_pnl
    ///   4: long_hold_duration (normalized by 100)
    ///   5: short_hold_duration (normalized by 100)
    ///   6: net_exposure
    ///   7: gross_exposure
    pub fn to_state_features(&self) -> Vec<f64> {
        vec![
            if self.long_position_active { 1.0 } else { 0.0 },
            if self.short_position_active { 1.0 } else { 0.0 },
            self.long_unrealized_pnl,
            self.short_unrealized_pnl,
            self.long_hold_duration as f64 / 100.0,
            self.short_hold_duration as f64 / 100.0,
            self.net_exposure,
            self.gross_exposure,
        ]
    }
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
        Self::with_reward_fn(candles, config, reward_config, Box::new(DefaultRewardFn))
    }

    /// Create environment with custom configuration and reward function.
    pub fn with_reward_fn(
        candles: Vec<Candle>,
        config: EnvConfig,
        reward_config: RewardConfig,
        reward_fn: Box<dyn RewardFn>,
    ) -> Self {
        let first_price = candles
            .first()
            .and_then(|c| c.close.to_f64())
            .unwrap_or(0.0);
        let balance = config.initial_balance;
        Self {
            candles,
            step: 0,
            long_position: None,
            short_position: None,
            total_reward: 0.0,
            trade_count: 0,
            metrics: MetricsAccumulator::new(balance, first_price),
            balance,
            equity_peak: balance,
            config,
            reward_config,
            reward_fn,
        }
    }

    pub fn reset(&mut self) -> Observation {
        self.step = 0;
        self.long_position = None;
        self.short_position = None;
        self.total_reward = 0.0;
        self.trade_count = 0;
        self.balance = self.config.initial_balance;
        self.equity_peak = self.balance;
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

    /// Helper: check if no position is open on either side.
    fn is_flat(&self) -> bool {
        self.long_position.is_none() && self.short_position.is_none()
    }

    /// Current gross exposure as fraction of balance.
    fn gross_exposure(&self) -> f64 {
        let long_frac = if self.long_position.is_some() {
            self.config.position_size_pct
        } else {
            0.0
        };
        let short_frac = if self.short_position.is_some() {
            self.config.position_size_pct
        } else {
            0.0
        };
        long_frac + short_frac
    }

    /// Build a complete feature vector for the agent, optionally appending state info.
    ///
    /// `candle_features` is the raw feature vector from `extract_features()`.
    /// When `add_state_info` is enabled, appends 8 state features (position, PnL, etc.).
    pub fn build_agent_input(&self, candle_features: &[f64]) -> Vec<f64> {
        if self.config.add_state_info {
            let obs = self.observe();
            let mut v = candle_features.to_vec();
            v.extend(obs.to_state_features());
            v
        } else {
            candle_features.to_vec()
        }
    }

    /// Take an action, advance one candle, return (observation, reward).
    pub fn step(&mut self, action: AIAction) -> (Observation, f64) {
        let price = self.current_price();
        let mut trade_results = Vec::new();
        let mut action_was_invalid = false;

        // --- Process agent action: position management side effects ---
        if self.config.hedge_mode {
            match action {
                AIAction::EnterLong if self.long_position.is_none() => {
                    let new_gross = self.gross_exposure() + self.config.position_size_pct;
                    if new_gross <= self.config.max_gross_exposure {
                        let slipped = price * (1.0 + self.config.slippage_rate);
                        self.long_position = Some(EnvPosition {
                            side: Side::Buy,
                            entry_price: slipped,
                            entry_step: self.step,
                        });
                    } else {
                        action_was_invalid = true;
                    }
                }
                AIAction::EnterShort if self.short_position.is_none() => {
                    let new_gross = self.gross_exposure() + self.config.position_size_pct;
                    if new_gross <= self.config.max_gross_exposure {
                        let slipped = price * (1.0 - self.config.slippage_rate);
                        self.short_position = Some(EnvPosition {
                            side: Side::Sell,
                            entry_price: slipped,
                            entry_step: self.step,
                        });
                    } else {
                        action_was_invalid = true;
                    }
                }
                AIAction::ExitLong if self.long_position.is_some() => {
                    if let Some(pos) = self.long_position.take() {
                        trade_results.push(self.close_position_record(pos, price));
                    }
                }
                AIAction::ExitShort if self.short_position.is_some() => {
                    if let Some(pos) = self.short_position.take() {
                        trade_results.push(self.close_position_record(pos, price));
                    }
                }
                AIAction::Neutral => {}
                _ => {
                    action_was_invalid = true;
                }
            }
        } else {
            match action {
                AIAction::EnterLong if self.is_flat() => {
                    let slipped = price * (1.0 + self.config.slippage_rate);
                    self.long_position = Some(EnvPosition {
                        side: Side::Buy,
                        entry_price: slipped,
                        entry_step: self.step,
                    });
                }
                AIAction::EnterShort if self.is_flat() => {
                    let slipped = price * (1.0 - self.config.slippage_rate);
                    self.short_position = Some(EnvPosition {
                        side: Side::Sell,
                        entry_price: slipped,
                        entry_step: self.step,
                    });
                }
                AIAction::ExitLong if self.long_position.is_some() => {
                    if let Some(pos) = self.long_position.take() {
                        trade_results.push(self.close_position_record(pos, price));
                    }
                }
                AIAction::ExitShort if self.short_position.is_some() => {
                    if let Some(pos) = self.short_position.take() {
                        trade_results.push(self.close_position_record(pos, price));
                    }
                }
                AIAction::Neutral => {}
                _ => {
                    action_was_invalid = true;
                }
            }
        }

        // --- Auto-exit positions exceeding max_trade_duration_candles ---
        if let Some(max_dur) = self.config.max_trade_duration_candles {
            if self
                .long_position
                .as_ref()
                .is_some_and(|p| self.step - p.entry_step >= max_dur)
            {
                let pos = self.long_position.take().unwrap();
                trade_results.push(self.close_position_record(pos, price));
            }
            if self
                .short_position
                .as_ref()
                .is_some_and(|p| self.step - p.entry_step >= max_dur)
            {
                let pos = self.short_position.take().unwrap();
                trade_results.push(self.close_position_record(pos, price));
            }
        }

        // --- Compute equity for drawdown tracking ---
        let equity = self.compute_equity();
        if equity > self.equity_peak {
            self.equity_peak = equity;
        }

        // --- Build reward context and delegate to reward function ---
        let ctx = RewardContext {
            action,
            price,
            balance: self.balance,
            equity,
            equity_peak: self.equity_peak,
            long_position: self.long_position.as_ref().map(|p| PositionInfo {
                side: p.side,
                entry_price: p.entry_price,
                entry_step: p.entry_step,
                unrealized_pnl_pct: Self::pnl_pct_for(p, price),
                hold_duration: self.step - p.entry_step,
            }),
            short_position: self.short_position.as_ref().map(|p| PositionInfo {
                side: p.side,
                entry_price: p.entry_price,
                entry_step: p.entry_step,
                unrealized_pnl_pct: Self::pnl_pct_for(p, price),
                hold_duration: self.step - p.entry_step,
            }),
            trade_results,
            gross_exposure: self.gross_exposure(),
            hedge_mode: self.config.hedge_mode,
            step: self.step,
            action_was_invalid,
        };

        let reward = self.reward_fn.calculate(&ctx, &self.reward_config);

        self.total_reward += reward;
        self.step += 1;

        let obs_price = self.current_price();
        self.metrics.record_step(equity, obs_price);

        (self.observe(), reward)
    }

    /// Close a position: update balance/metrics/trade_count, return TradeResult for reward.
    fn close_position_record(&mut self, pos: EnvPosition, exit_price: f64) -> TradeResult {
        self.trade_count += 1;
        let duration = self.step - pos.entry_step;

        let effective_exit = match pos.side {
            Side::Buy => exit_price * (1.0 - self.config.slippage_rate),
            Side::Sell => exit_price * (1.0 + self.config.slippage_rate),
        };

        let pnl_pct = match pos.side {
            Side::Buy => (effective_exit - pos.entry_price) / pos.entry_price,
            Side::Sell => (pos.entry_price - effective_exit) / pos.entry_price,
        };

        let notional = self.balance * self.config.position_size_pct;
        let fees = 2.0 * self.config.fee_rate * notional;
        let pnl_dollar = pnl_pct * notional - fees;

        self.balance += pnl_dollar;
        self.metrics.set_balance(self.balance);

        self.metrics.record_trade(CompletedTrade {
            side: pos.side,
            entry_price: pos.entry_price,
            exit_price: effective_exit,
            duration,
            pnl: pnl_dollar,
            fees,
        });

        TradeResult {
            side: pos.side,
            pnl_pct,
            duration,
            fees: 2.0 * self.config.fee_rate,
        }
    }

    fn current_price(&self) -> f64 {
        self.candles
            .get(self.step)
            .and_then(|c| c.close.to_f64())
            .unwrap_or(0.0)
    }

    fn unrealized_pnl_for(pos: &EnvPosition, price: f64, notional: f64) -> f64 {
        let pnl_pct = match pos.side {
            Side::Buy => (price - pos.entry_price) / pos.entry_price,
            Side::Sell => (pos.entry_price - price) / pos.entry_price,
        };
        pnl_pct * notional
    }

    fn compute_equity(&self) -> f64 {
        let price = self.current_price();
        let notional = self.balance * self.config.position_size_pct;
        let mut equity = self.balance;
        if let Some(pos) = &self.long_position {
            equity += Self::unrealized_pnl_for(pos, price, notional);
        }
        if let Some(pos) = &self.short_position {
            equity += Self::unrealized_pnl_for(pos, price, notional);
        }
        equity
    }

    fn pnl_pct_for(pos: &EnvPosition, price: f64) -> f64 {
        match pos.side {
            Side::Buy => (price - pos.entry_price) / pos.entry_price,
            Side::Sell => (pos.entry_price - price) / pos.entry_price,
        }
    }

    fn observe(&self) -> Observation {
        let price = self.current_price();

        // Per-side fields
        let (long_active, long_pnl, long_dur) = if let Some(pos) = &self.long_position {
            (
                true,
                Self::pnl_pct_for(pos, price),
                self.step - pos.entry_step,
            )
        } else {
            (false, 0.0, 0)
        };
        let (short_active, short_pnl, short_dur) = if let Some(pos) = &self.short_position {
            (
                true,
                Self::pnl_pct_for(pos, price),
                self.step - pos.entry_step,
            )
        } else {
            (false, 0.0, 0)
        };

        // Legacy compat: prefer long if open, else short
        let (side, pnl, duration) = if long_active {
            (Some(Side::Buy), long_pnl, long_dur)
        } else if short_active {
            (Some(Side::Sell), short_pnl, short_dur)
        } else {
            (None, 0.0, 0)
        };

        let long_frac = if long_active {
            self.config.position_size_pct
        } else {
            0.0
        };
        let short_frac = if short_active {
            self.config.position_size_pct
        } else {
            0.0
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
            long_position_active: long_active,
            long_unrealized_pnl: long_pnl,
            long_hold_duration: long_dur,
            short_position_active: short_active,
            short_unrealized_pnl: short_pnl,
            short_hold_duration: short_dur,
            net_exposure: long_frac - short_frac,
            gross_exposure: long_frac + short_frac,
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

        let (obs, _) = env.step(AIAction::EnterLong);
        assert!(obs.position_side.is_some());

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

    // --- Tests from branch (ours) ---

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

        let mut penalty_observed = false;
        for _ in 0..10 {
            let (_, reward) = env.step(AIAction::Neutral);
            if reward < 0.0 {
                penalty_observed = true;
            }
        }
        assert!(
            penalty_observed,
            "hold penalty should apply after threshold"
        );
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
        let (obs, _) = env.step(AIAction::EnterLong);
        assert!(
            obs.equity > obs.balance,
            "equity should exceed balance with unrealized gain"
        );
        assert!(obs.unrealized_pnl > 0.0);
    }

    #[test]
    fn short_loss_decreases_balance() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"),
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
        let mut env_no_bonus = TradingEnv::with_config(
            candles.clone(),
            EnvConfig::default(),
            RewardConfig::default(),
        );
        env_no_bonus.reset();
        env_no_bonus.step(AIAction::EnterLong);
        let (_, reward_no) = env_no_bonus.step(AIAction::ExitLong);

        let reward_cfg = RewardConfig {
            win_bonus: 1.0,
            ..RewardConfig::default()
        };
        let mut env_bonus = TradingEnv::with_config(candles, EnvConfig::default(), reward_cfg);
        env_bonus.reset();
        env_bonus.step(AIAction::EnterLong);
        let (_, reward_yes) = env_bonus.step(AIAction::ExitLong);

        assert!(
            reward_yes > reward_no,
            "win bonus should increase reward for profitable trade"
        );
    }

    // --- Tests from main (theirs) ---

    #[test]
    fn drawdown_penalty_applied_when_equity_drops() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"),
            candle_price("90"),
            candle_price("85"),
        ];
        let env_cfg = EnvConfig::default();
        let reward_cfg = RewardConfig {
            drawdown_penalty_rate: 1.0,
            ..RewardConfig::default()
        };
        let mut env = TradingEnv::with_config(candles, env_cfg, reward_cfg);
        env.reset();

        env.step(AIAction::EnterLong);
        let (_, reward_at_peak) = env.step(AIAction::Neutral);
        let (_, reward_in_drawdown) = env.step(AIAction::Neutral);
        assert!(
            reward_in_drawdown < reward_at_peak,
            "drawdown penalty should reduce reward: peak_reward={reward_at_peak}, drawdown_reward={reward_in_drawdown}"
        );
    }

    #[test]
    fn no_drawdown_penalty_at_new_highs() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"),
            candle_price("120"),
        ];
        let reward_cfg = RewardConfig {
            drawdown_penalty_rate: 1.0,
            hold_penalty_threshold: 1000,
            ..RewardConfig::default()
        };
        let mut env = TradingEnv::with_config(candles, EnvConfig::default(), reward_cfg);
        env.reset();

        env.step(AIAction::EnterLong);
        let (_, reward) = env.step(AIAction::Neutral);
        assert!(
            (reward - 0.0).abs() < f64::EPSILON,
            "no drawdown penalty at new highs, got {reward}"
        );
    }

    #[test]
    fn drawdown_penalty_zero_rate_has_no_effect() {
        let candles = vec![candle_price("100"), candle_price("110"), candle_price("80")];
        let reward_cfg = RewardConfig {
            drawdown_penalty_rate: 0.0,
            hold_penalty_threshold: 1000,
            ..RewardConfig::default()
        };
        let mut env = TradingEnv::with_config(candles, EnvConfig::default(), reward_cfg);
        env.reset();

        env.step(AIAction::EnterLong);
        let (_, reward) = env.step(AIAction::Neutral);
        assert!(
            (reward - 0.0).abs() < f64::EPSILON,
            "zero drawdown_penalty_rate should have no effect, got {reward}"
        );
    }

    // --- Hedge mode tests ---

    fn hedge_env(candles: Vec<Candle>) -> TradingEnv {
        let env_cfg = EnvConfig {
            hedge_mode: true,
            ..EnvConfig::default()
        };
        TradingEnv::with_config(candles, env_cfg, RewardConfig::default())
    }

    #[test]
    fn hedge_enter_long_and_short() {
        let candles: Vec<_> = (0..10).map(|_| candle_price("100")).collect();
        let mut env = hedge_env(candles);
        env.reset();

        let (obs, _) = env.step(AIAction::EnterLong);
        assert!(obs.long_position_active);
        assert!(!obs.short_position_active);

        let (obs, _) = env.step(AIAction::EnterShort);
        assert!(obs.long_position_active);
        assert!(obs.short_position_active);
    }

    #[test]
    fn hedge_exit_long_keeps_short() {
        let candles: Vec<_> = (0..10).map(|_| candle_price("100")).collect();
        let mut env = hedge_env(candles);
        env.reset();

        env.step(AIAction::EnterLong);
        env.step(AIAction::EnterShort);
        let (obs, _) = env.step(AIAction::ExitLong);
        assert!(!obs.long_position_active);
        assert!(obs.short_position_active);
        assert_eq!(env.trade_count(), 1);
    }

    #[test]
    fn hedge_exit_short_keeps_long() {
        let candles: Vec<_> = (0..10).map(|_| candle_price("100")).collect();
        let mut env = hedge_env(candles);
        env.reset();

        env.step(AIAction::EnterLong);
        env.step(AIAction::EnterShort);
        let (obs, _) = env.step(AIAction::ExitShort);
        assert!(obs.long_position_active);
        assert!(!obs.short_position_active);
        assert_eq!(env.trade_count(), 1);
    }

    #[test]
    fn hedge_equity_both_positions() {
        // Price rises: long gains, short loses — net effect on equity
        let candles = vec![
            candle_price("100"),
            candle_price("100"),
            candle_price("110"),
            candle_price("120"),
        ];
        let mut env = hedge_env(candles);
        env.reset();

        env.step(AIAction::EnterLong);
        env.step(AIAction::EnterShort);
        let (obs, _) = env.step(AIAction::Neutral);

        // With equal sizes, price-up means long gains = short losses (approximately)
        // Equity should be close to balance since they offset
        assert!(obs.long_unrealized_pnl > 0.0);
        assert!(obs.short_unrealized_pnl < 0.0);
    }

    #[test]
    fn hedge_net_exposure_zero_when_hedged() {
        let candles: Vec<_> = (0..10).map(|_| candle_price("100")).collect();
        let mut env = hedge_env(candles);
        env.reset();

        env.step(AIAction::EnterLong);
        let (obs, _) = env.step(AIAction::EnterShort);
        assert!(
            obs.net_exposure.abs() < f64::EPSILON,
            "equal long+short should have net_exposure ~0, got {}",
            obs.net_exposure
        );
        assert!(
            (obs.gross_exposure - 2.0).abs() < f64::EPSILON,
            "gross_exposure should be 2.0 (100%+100%), got {}",
            obs.gross_exposure
        );
    }

    #[test]
    fn hedge_gross_exposure_gate() {
        let candles: Vec<_> = (0..10).map(|_| candle_price("100")).collect();
        let env_cfg = EnvConfig {
            hedge_mode: true,
            max_gross_exposure: 1.0, // Only allow 100% total
            ..EnvConfig::default()
        };
        let mut env = TradingEnv::with_config(candles, env_cfg, RewardConfig::default());
        env.reset();

        // First entry at 100% fills the limit
        let (obs, _) = env.step(AIAction::EnterLong);
        assert!(obs.long_position_active);

        // Second entry should be rejected (would be 200% > 100% limit)
        let (obs, reward) = env.step(AIAction::EnterShort);
        assert!(
            !obs.short_position_active,
            "should reject due to exposure limit"
        );
        assert!(reward < 0.0, "should be penalized as invalid");
    }

    #[test]
    fn hedge_exposure_penalty() {
        let candles: Vec<_> = (0..10).map(|_| candle_price("100")).collect();
        let env_cfg = EnvConfig {
            hedge_mode: true,
            max_gross_exposure: 3.0, // Allow entry
            ..EnvConfig::default()
        };
        let reward_cfg = RewardConfig {
            exposure_penalty_rate: 1.0,
            exposure_penalty_threshold: 1.5, // Penalty above 150%
            hold_penalty_threshold: 1000,    // Disable hold penalty
            ..RewardConfig::default()
        };
        let mut env = TradingEnv::with_config(candles, env_cfg, reward_cfg);
        env.reset();

        env.step(AIAction::EnterLong);
        // gross = 1.0, below threshold — no penalty on neutral
        let (_, reward_below) = env.step(AIAction::Neutral);
        assert!(
            reward_below.abs() < f64::EPSILON,
            "no penalty below threshold, got {reward_below}"
        );

        env.step(AIAction::EnterShort);
        // gross = 2.0, above 1.5 threshold — penalty = 1.0 * (2.0 - 1.5) = 0.5
        let (_, reward_above) = env.step(AIAction::Neutral);
        assert!(
            reward_above < 0.0,
            "exposure penalty should apply, got {reward_above}"
        );
    }

    #[test]
    fn hedge_hold_penalty_both() {
        let candles: Vec<_> = (0..30).map(|_| candle_price("100")).collect();
        let env_cfg = EnvConfig {
            hedge_mode: true,
            ..EnvConfig::default()
        };
        let reward_cfg = RewardConfig {
            hold_penalty_threshold: 2,
            hold_penalty_rate: 0.01,
            ..RewardConfig::default()
        };
        let mut env = TradingEnv::with_config(candles, env_cfg, reward_cfg);
        env.reset();

        env.step(AIAction::EnterLong); // step 0
        env.step(AIAction::EnterShort); // step 1

        // After a few neutrals, both should accumulate penalties
        let mut total_penalty = 0.0;
        for _ in 0..5 {
            let (_, reward) = env.step(AIAction::Neutral);
            total_penalty += reward;
        }
        assert!(
            total_penalty < 0.0,
            "hold penalty should accumulate from both sides, got {total_penalty}"
        );
    }

    #[test]
    fn hedge_invalid_double_enter() {
        let candles: Vec<_> = (0..10).map(|_| candle_price("100")).collect();
        let mut env = hedge_env(candles);
        env.reset();

        env.step(AIAction::EnterLong);
        let (_, reward) = env.step(AIAction::EnterLong);
        assert!(reward < 0.0, "double enter should be penalized");
    }

    #[test]
    fn hedge_invalid_exit_no_position() {
        let candles: Vec<_> = (0..10).map(|_| candle_price("100")).collect();
        let mut env = hedge_env(candles);
        env.reset();

        let (_, reward) = env.step(AIAction::ExitLong);
        assert!(reward < 0.0, "exit with no position should be penalized");
    }

    #[test]
    fn hedge_mode_false_rejects_simultaneous() {
        let candles: Vec<_> = (0..10).map(|_| candle_price("100")).collect();
        let mut env = TradingEnv::new(candles); // default: hedge_mode = false
        env.reset();

        env.step(AIAction::EnterLong);
        let (_, reward) = env.step(AIAction::EnterShort);
        assert!(
            reward < 0.0,
            "one-way mode should reject short while long is open"
        );
    }

    #[test]
    fn hedge_reset_clears_both() {
        let candles: Vec<_> = (0..10).map(|_| candle_price("100")).collect();
        let mut env = hedge_env(candles);
        env.reset();

        env.step(AIAction::EnterLong);
        env.step(AIAction::EnterShort);

        let obs = env.reset();
        assert!(!obs.long_position_active);
        assert!(!obs.short_position_active);
        assert!(obs.gross_exposure.abs() < f64::EPSILON);
    }

    #[test]
    fn hedge_bonus_when_both_open() {
        let candles: Vec<_> = (0..10).map(|_| candle_price("100")).collect();
        let env_cfg = EnvConfig {
            hedge_mode: true,
            ..EnvConfig::default()
        };
        let reward_cfg = RewardConfig {
            hedge_bonus: 0.1,
            hold_penalty_threshold: 1000, // Disable hold penalty
            ..RewardConfig::default()
        };
        let mut env = TradingEnv::with_config(candles, env_cfg, reward_cfg);
        env.reset();

        env.step(AIAction::EnterLong);
        env.step(AIAction::EnterShort);
        // Both open: bonus = 0.1 * min(1.0, 1.0) = 0.1
        let (_, reward) = env.step(AIAction::Neutral);
        assert!(
            reward > 0.0,
            "hedge bonus should produce positive reward, got {reward}"
        );
    }

    #[test]
    fn hedge_observation_fields() {
        let candles = vec![
            candle_price("100"),
            candle_price("100"),
            candle_price("110"),
            candle_price("110"),
        ];
        let mut env = hedge_env(candles);
        env.reset();

        env.step(AIAction::EnterLong); // step 0, long opens
        env.step(AIAction::EnterShort); // step 1, short opens
        let (obs, _) = env.step(AIAction::Neutral); // step 2, price at 110

        assert!(obs.long_position_active);
        assert!(obs.short_position_active);
        assert!(
            obs.long_unrealized_pnl > 0.0,
            "long should profit from 100→110"
        );
        assert!(
            obs.short_unrealized_pnl < 0.0,
            "short should lose from 100→110"
        );
        assert_eq!(obs.long_hold_duration, 3); // entered at step 0, now observing step 3
        assert_eq!(obs.short_hold_duration, 2); // entered at step 1, now observing step 3
                                                // Legacy compat: prefers long
        assert_eq!(obs.position_side, Some(Side::Buy));
    }

    #[test]
    fn hedge_metrics_track_both_sides() {
        let candles = vec![
            candle_price("100"),
            candle_price("100"),
            candle_price("110"),
            candle_price("110"),
            candle_price("110"),
        ];
        let mut env = hedge_env(candles);
        env.reset();

        env.step(AIAction::EnterLong);
        env.step(AIAction::EnterShort);
        env.step(AIAction::ExitLong);
        env.step(AIAction::ExitShort);

        assert_eq!(env.trade_count(), 2);
        let metrics = env.episode_metrics(35040.0);
        assert_eq!(metrics.total_trades, 2);
    }

    // --- State info and auto-exit tests ---

    #[test]
    fn state_features_correct_count() {
        let candles = vec![candle_price("100"), candle_price("110")];
        let mut env = TradingEnv::new(candles);
        let obs = env.reset();
        let sf = obs.to_state_features();
        assert_eq!(sf.len(), STATE_INFO_COUNT);
    }

    #[test]
    fn state_features_reflect_position() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"),
            candle_price("120"),
        ];
        let mut env = TradingEnv::new(candles);
        env.reset();

        let (obs, _) = env.step(AIAction::EnterLong);
        let sf = obs.to_state_features();
        assert!((sf[0] - 1.0).abs() < f64::EPSILON, "long should be active");
        assert!(
            (sf[1] - 0.0).abs() < f64::EPSILON,
            "short should be inactive"
        );
        assert!(sf[2] > 0.0, "long should have positive unrealized PnL");
    }

    #[test]
    fn build_agent_input_without_state_info() {
        let candles = vec![candle_price("100"), candle_price("110")];
        let mut env = TradingEnv::new(candles); // add_state_info defaults to false
        env.reset();

        let feats = vec![1.0, 2.0, 3.0];
        let input = env.build_agent_input(&feats);
        assert_eq!(input.len(), 3, "should not append state info");
    }

    #[test]
    fn build_agent_input_with_state_info() {
        let candles = vec![candle_price("100"), candle_price("110")];
        let env_cfg = EnvConfig {
            add_state_info: true,
            ..EnvConfig::default()
        };
        let mut env = TradingEnv::with_config(candles, env_cfg, RewardConfig::default());
        env.reset();

        let feats = vec![1.0, 2.0, 3.0];
        let input = env.build_agent_input(&feats);
        assert_eq!(
            input.len(),
            3 + STATE_INFO_COUNT,
            "should append state info"
        );
    }

    #[test]
    fn max_trade_duration_auto_exits() {
        let candles: Vec<_> = (0..20).map(|_| candle_price("100")).collect();
        let env_cfg = EnvConfig {
            max_trade_duration_candles: Some(3),
            ..EnvConfig::default()
        };
        let mut env = TradingEnv::with_config(candles, env_cfg, RewardConfig::default());
        env.reset();

        env.step(AIAction::EnterLong); // step 0

        // Steps 1 and 2 — position still open (duration < 3)
        let (obs, _) = env.step(AIAction::Neutral);
        assert!(obs.position_side.is_some());
        let (obs, _) = env.step(AIAction::Neutral);
        assert!(obs.position_side.is_some());

        // Step 3 — duration = 3, should auto-exit
        let (obs, _) = env.step(AIAction::Neutral);
        assert!(
            obs.position_side.is_none(),
            "position should be auto-closed after max duration"
        );
        assert_eq!(env.trade_count(), 1);
    }

    #[test]
    fn max_trade_duration_none_no_auto_exit() {
        let candles: Vec<_> = (0..20).map(|_| candle_price("100")).collect();
        let env_cfg = EnvConfig {
            max_trade_duration_candles: None,
            ..EnvConfig::default()
        };
        let mut env = TradingEnv::with_config(candles, env_cfg, RewardConfig::default());
        env.reset();

        env.step(AIAction::EnterLong);
        for _ in 0..15 {
            let (obs, _) = env.step(AIAction::Neutral);
            if !obs.done {
                assert!(obs.position_side.is_some(), "position should remain open");
            }
        }
    }

    #[test]
    fn max_trade_duration_hedge_mode_both_sides() {
        let candles: Vec<_> = (0..20).map(|_| candle_price("100")).collect();
        let env_cfg = EnvConfig {
            hedge_mode: true,
            max_trade_duration_candles: Some(3),
            ..EnvConfig::default()
        };
        let mut env = TradingEnv::with_config(candles, env_cfg, RewardConfig::default());
        env.reset();

        env.step(AIAction::EnterLong); // step 0
        env.step(AIAction::EnterShort); // step 1

        // Step 2: long_dur=2, short_dur=1
        env.step(AIAction::Neutral);
        // Step 3: long_dur=3 -> auto-exit long, short_dur=2
        let (obs, _) = env.step(AIAction::Neutral);
        assert!(!obs.long_position_active, "long should be auto-closed");
        assert!(obs.short_position_active, "short should still be open");
        assert_eq!(env.trade_count(), 1);
    }

    #[test]
    fn custom_reward_fn_in_env() {
        use crate::reward::RewardFn;

        struct AlwaysOne;
        impl RewardFn for AlwaysOne {
            fn calculate(
                &self,
                _ctx: &crate::reward::RewardContext,
                _config: &RewardConfig,
            ) -> f64 {
                1.0
            }
        }

        let candles = vec![candle_price("100"), candle_price("110")];
        let mut env = TradingEnv::with_reward_fn(
            candles,
            EnvConfig::default(),
            RewardConfig::default(),
            Box::new(AlwaysOne),
        );
        env.reset();
        let (_, reward) = env.step(AIAction::Neutral);
        assert!((reward - 1.0).abs() < f64::EPSILON);
    }
}
