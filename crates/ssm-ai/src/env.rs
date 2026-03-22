use rust_decimal::prelude::ToPrimitive;
use ssm_core::{AIAction, Candle, Side};

/// Reinforcement learning environment (FreqAI Base5Action inspired).
///
/// The agent steps through candles one at a time, takes actions, and receives rewards.
/// This environment is deterministic given the same candle sequence.
pub struct TradingEnv {
    candles: Vec<Candle>,
    step: usize,
    position: Option<EnvPosition>,
    total_reward: f64,
    trade_count: u32,
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
}

impl TradingEnv {
    pub fn new(candles: Vec<Candle>) -> Self {
        Self {
            candles,
            step: 0,
            position: None,
            total_reward: 0.0,
            trade_count: 0,
        }
    }

    pub fn reset(&mut self) -> Observation {
        self.step = 0;
        self.position = None;
        self.total_reward = 0.0;
        self.trade_count = 0;
        self.observe()
    }

    pub fn total_reward(&self) -> f64 {
        self.total_reward
    }

    pub fn trade_count(&self) -> u32 {
        self.trade_count
    }

    /// Take an action, advance one candle, return (observation, reward).
    pub fn step(&mut self, action: AIAction) -> (Observation, f64) {
        let price = self.current_price();
        let mut reward = 0.0;

        match action {
            AIAction::EnterLong if self.position.is_none() => {
                self.position = Some(EnvPosition {
                    side: Side::Buy,
                    entry_price: price,
                    entry_step: self.step,
                });
            }
            AIAction::EnterShort if self.position.is_none() => {
                self.position = Some(EnvPosition {
                    side: Side::Sell,
                    entry_price: price,
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
                // Small penalty for holding too long
                if let Some(pos) = &self.position {
                    let duration = self.step - pos.entry_step;
                    if duration > 20 {
                        reward = -0.001 * duration as f64;
                    }
                }
            }
            _ => {
                // Invalid action (e.g., ExitLong with no position) — small penalty
                reward = -0.01;
            }
        }

        self.total_reward += reward;
        self.step += 1;

        (self.observe(), reward)
    }

    fn close_position(&mut self, exit_price: f64) -> f64 {
        if let Some(pos) = self.position.take() {
            self.trade_count += 1;
            let pnl = match pos.side {
                Side::Buy => (exit_price - pos.entry_price) / pos.entry_price,
                Side::Sell => (pos.entry_price - exit_price) / pos.entry_price,
            };
            // Reward is % PnL minus duration penalty
            let duration = self.step - pos.entry_step;
            let duration_penalty = if duration > 50 {
                0.001 * duration as f64
            } else {
                0.0
            };
            pnl - duration_penalty
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

        Observation {
            step: self.step,
            current_price: price,
            position_side: side,
            unrealized_pnl: pnl,
            hold_duration: duration,
            done: self.step >= self.candles.len().saturating_sub(1),
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
}
