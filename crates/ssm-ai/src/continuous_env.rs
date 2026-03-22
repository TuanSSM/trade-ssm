use rust_decimal::prelude::ToPrimitive;
use ssm_core::Candle;

use crate::config::{EnvConfig, RewardConfig};
use crate::metrics::{CompletedTrade, EpisodeMetrics, MetricsAccumulator};
use ssm_core::Side;

/// Continuous action for the trading environment.
///
/// Instead of discrete Base5Action, the agent outputs a continuous position target.
#[derive(Debug, Clone, Copy)]
pub struct ContinuousAction {
    /// Target position: -1.0 (full short) to +1.0 (full long), 0.0 = flat.
    pub position_target: f64,
}

/// Observation returned to the agent in the continuous environment.
#[derive(Debug, Clone)]
pub struct ContinuousObservation {
    pub step: usize,
    pub current_price: f64,
    /// Current position as a fraction: -1.0 to +1.0.
    pub current_position: f64,
    pub unrealized_pnl: f64,
    pub balance: f64,
    pub equity: f64,
    pub done: bool,
    /// Optional feature vector for the current candle.
    pub features: Vec<f64>,
}

/// Continuous-action RL trading environment.
///
/// The agent outputs a position target (-1 to +1) and the environment
/// adjusts the position accordingly, supporting smooth transitions.
pub struct ContinuousTradingEnv {
    candles: Vec<Candle>,
    step: usize,
    /// Current position as fraction: -1.0 (full short) to +1.0 (full long).
    position: f64,
    entry_price: f64,
    config: EnvConfig,
    _reward_config: RewardConfig,
    balance: f64,
    metrics: MetricsAccumulator,
}

impl ContinuousTradingEnv {
    pub fn new(candles: Vec<Candle>, config: EnvConfig, reward_config: RewardConfig) -> Self {
        let first_price = candles
            .first()
            .and_then(|c| c.close.to_f64())
            .unwrap_or(0.0);
        let balance = config.initial_balance;
        Self {
            candles,
            step: 0,
            position: 0.0,
            entry_price: first_price,
            config,
            balance,
            _reward_config: reward_config,
            metrics: MetricsAccumulator::new(balance, first_price),
        }
    }

    pub fn reset(&mut self) -> ContinuousObservation {
        self.step = 0;
        self.position = 0.0;
        let first_price = self
            .candles
            .first()
            .and_then(|c| c.close.to_f64())
            .unwrap_or(0.0);
        self.entry_price = first_price;
        self.balance = self.config.initial_balance;
        self.metrics = MetricsAccumulator::new(self.balance, first_price);
        self.observe()
    }

    pub fn balance(&self) -> f64 {
        self.balance
    }

    pub fn episode_metrics(&self, steps_per_year: f64) -> EpisodeMetrics {
        let mut acc = self.metrics.clone();
        acc.set_balance(self.balance);
        acc.finalize(steps_per_year)
    }

    /// Take a continuous action, advance one candle, return (observation, reward).
    pub fn step(&mut self, action: ContinuousAction) -> (ContinuousObservation, f64) {
        let prev_price = self.current_price();
        let target = action.position_target.clamp(-1.0, 1.0);

        // Calculate PnL from position change
        let mut reward = 0.0;

        // Realize PnL for the portion being closed
        let position_change = target - self.position;
        if position_change.abs() > 1e-10 {
            let closing_portion = if position_change.signum() != self.position.signum()
                && self.position.abs() > 1e-10
            {
                // We're reducing or flipping — realize PnL on the reduced amount
                let close_amount = self.position.abs().min(position_change.abs());
                let pnl_pct = if self.entry_price > 0.0 {
                    (prev_price - self.entry_price) / self.entry_price * self.position.signum()
                } else {
                    0.0
                };
                let notional = self.balance * self.config.position_size_pct * close_amount;
                let fees = self.config.fee_rate * notional;
                let pnl = pnl_pct * notional - fees;
                self.balance += pnl;

                let side = if self.position > 0.0 {
                    Side::Buy
                } else {
                    Side::Sell
                };
                self.metrics.record_trade(CompletedTrade {
                    side,
                    entry_price: self.entry_price,
                    exit_price: prev_price,
                    duration: 1,
                    pnl,
                    fees,
                });
                self.metrics.set_balance(self.balance);
                pnl_pct - self.config.fee_rate
            } else {
                0.0
            };

            reward += closing_portion;

            // Update entry price for new position
            if target.abs() > 1e-10 {
                // Slippage on new entry
                let slipped = if target > 0.0 {
                    prev_price * (1.0 + self.config.slippage_rate)
                } else {
                    prev_price * (1.0 - self.config.slippage_rate)
                };
                self.entry_price = slipped;
            }
        }

        self.position = target;
        self.step += 1;

        let equity = self.compute_equity();
        let obs_price = self.current_price();
        self.metrics.record_step(equity, obs_price);

        // Unrealized PnL reward component
        let unrealized_pnl = self.unrealized_pnl_pct();
        reward += unrealized_pnl * 0.01; // Small reward for paper gains

        (self.observe(), reward)
    }

    fn current_price(&self) -> f64 {
        self.candles
            .get(self.step)
            .and_then(|c| c.close.to_f64())
            .unwrap_or(0.0)
    }

    fn compute_equity(&self) -> f64 {
        let price = self.current_price();
        if self.position.abs() < 1e-10 || self.entry_price <= 0.0 {
            return self.balance;
        }

        let notional = self.balance * self.config.position_size_pct * self.position.abs();
        let pnl_pct = (price - self.entry_price) / self.entry_price * self.position.signum();
        self.balance + pnl_pct * notional
    }

    fn unrealized_pnl_pct(&self) -> f64 {
        if self.position.abs() < 1e-10 || self.entry_price <= 0.0 {
            return 0.0;
        }
        let price = self.current_price();
        (price - self.entry_price) / self.entry_price * self.position.signum()
    }

    fn observe(&self) -> ContinuousObservation {
        ContinuousObservation {
            step: self.step,
            current_price: self.current_price(),
            current_position: self.position,
            unrealized_pnl: self.unrealized_pnl_pct(),
            balance: self.balance,
            equity: self.compute_equity(),
            done: self.step >= self.candles.len().saturating_sub(1),
            features: Vec::new(), // Populated externally
        }
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
    fn continuous_long_profit() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"),
            candle_price("120"),
        ];
        let mut env =
            ContinuousTradingEnv::new(candles, EnvConfig::default(), RewardConfig::default());
        env.reset();

        // Go full long
        let (obs, _) = env.step(ContinuousAction {
            position_target: 1.0,
        });
        assert!((obs.current_position - 1.0).abs() < 1e-10);

        // Exit
        let (_, _) = env.step(ContinuousAction {
            position_target: 0.0,
        });
        assert!(env.balance() >= 10_000.0); // Should have some profit
    }

    #[test]
    fn continuous_short_profit() {
        let candles = vec![candle_price("100"), candle_price("90"), candle_price("80")];
        let mut env =
            ContinuousTradingEnv::new(candles, EnvConfig::default(), RewardConfig::default());
        env.reset();

        env.step(ContinuousAction {
            position_target: -1.0,
        });
        let (_, _) = env.step(ContinuousAction {
            position_target: 0.0,
        });
        assert!(env.balance() >= 10_000.0);
    }

    #[test]
    fn partial_position() {
        let candles = vec![
            candle_price("100"),
            candle_price("110"),
            candle_price("120"),
        ];
        let mut env =
            ContinuousTradingEnv::new(candles, EnvConfig::default(), RewardConfig::default());
        env.reset();

        // Half long
        let (obs, _) = env.step(ContinuousAction {
            position_target: 0.5,
        });
        assert!((obs.current_position - 0.5).abs() < 1e-10);
    }

    #[test]
    fn position_clamping() {
        let candles = vec![candle_price("100"), candle_price("100")];
        let mut env =
            ContinuousTradingEnv::new(candles, EnvConfig::default(), RewardConfig::default());
        env.reset();

        let (obs, _) = env.step(ContinuousAction {
            position_target: 5.0,
        });
        assert!((obs.current_position - 1.0).abs() < 1e-10); // Clamped to 1.0
    }

    #[test]
    fn done_at_end() {
        let candles = vec![candle_price("100"), candle_price("100")];
        let mut env =
            ContinuousTradingEnv::new(candles, EnvConfig::default(), RewardConfig::default());
        env.reset();
        let (obs, _) = env.step(ContinuousAction {
            position_target: 0.0,
        });
        assert!(obs.done);
    }
}
