use ssm_core::{AIAction, Side};

use crate::config::RewardConfig;

/// Information about an open position, passed to reward functions.
#[derive(Debug, Clone)]
pub struct PositionInfo {
    pub side: Side,
    pub entry_price: f64,
    pub entry_step: usize,
    pub unrealized_pnl_pct: f64,
    pub hold_duration: usize,
}

/// Result of closing a position this step.
#[derive(Debug, Clone)]
pub struct TradeResult {
    pub side: Side,
    pub pnl_pct: f64,
    pub duration: usize,
    pub fees: f64,
}

/// All context needed by a reward function to compute the reward for one step.
#[derive(Debug, Clone)]
pub struct RewardContext {
    pub action: AIAction,
    pub price: f64,
    pub balance: f64,
    pub equity: f64,
    pub equity_peak: f64,
    pub long_position: Option<PositionInfo>,
    pub short_position: Option<PositionInfo>,
    /// Set when a position was closed this step (may have multiple in hedge mode).
    pub trade_results: Vec<TradeResult>,
    pub gross_exposure: f64,
    pub hedge_mode: bool,
    pub step: usize,
    /// Whether the action was invalid (e.g. ExitLong with no long position).
    pub action_was_invalid: bool,
}

/// Trait for custom reward calculation.
///
/// Implement this to define custom reward shaping beyond the default
/// hold/invalid/duration/fee/win/drawdown/exposure/hedge penalties.
///
/// ```rust,ignore
/// use ssm_ai::reward::{RewardFn, RewardContext};
/// use ssm_ai::config::RewardConfig;
///
/// struct MyReward;
/// impl RewardFn for MyReward {
///     fn calculate(&self, ctx: &RewardContext, config: &RewardConfig) -> f64 {
///         // Custom reward: only care about PnL from closed trades
///         ctx.trade_results.iter().map(|t| t.pnl_pct).sum()
///     }
/// }
/// ```
pub trait RewardFn: Send + Sync {
    fn calculate(&self, ctx: &RewardContext, config: &RewardConfig) -> f64;
}

/// Default reward function reproducing the existing hardcoded behavior.
pub struct DefaultRewardFn;

impl RewardFn for DefaultRewardFn {
    fn calculate(&self, ctx: &RewardContext, config: &RewardConfig) -> f64 {
        let mut reward = 0.0;

        // Invalid action penalty
        if ctx.action_was_invalid {
            return -config.invalid_action_penalty;
        }

        // PnL from closed trades
        for trade in &ctx.trade_results {
            let mut trade_reward = trade.pnl_pct;

            // Duration penalty at close
            if trade.duration > config.close_penalty_threshold {
                trade_reward -= config.close_penalty_rate * trade.duration as f64;
            }

            // Fee penalty
            if config.fee_penalty {
                trade_reward -= trade.fees;
            }

            // Win bonus
            if trade.pnl_pct > 0.0 && config.win_bonus > 0.0 {
                trade_reward += trade.pnl_pct * config.win_bonus;
            }

            reward += trade_reward;
        }

        // Hold penalty on Neutral with open positions
        if ctx.action == AIAction::Neutral {
            if ctx.hedge_mode {
                // Sum penalties from both sides
                for pos in [&ctx.long_position, &ctx.short_position]
                    .into_iter()
                    .flatten()
                {
                    if pos.hold_duration > config.hold_penalty_threshold {
                        reward -= config.hold_penalty_rate * pos.hold_duration as f64;
                    }
                }
            } else {
                // One-way: at most one position
                for pos in [&ctx.long_position, &ctx.short_position]
                    .into_iter()
                    .flatten()
                {
                    if pos.hold_duration > config.hold_penalty_threshold {
                        reward = -config.hold_penalty_rate * pos.hold_duration as f64;
                    }
                }
            }
        }

        // Exposure penalty (hedge mode)
        if config.exposure_penalty_rate > 0.0
            && ctx.gross_exposure > config.exposure_penalty_threshold
        {
            reward -= config.exposure_penalty_rate
                * (ctx.gross_exposure - config.exposure_penalty_threshold);
        }

        // Hedge bonus
        if ctx.long_position.is_some() && ctx.short_position.is_some() && config.hedge_bonus > 0.0 {
            // Use gross_exposure / 2 as proxy for min(long_frac, short_frac) when equal sizes
            reward += config.hedge_bonus * (ctx.gross_exposure / 2.0);
        }

        // Drawdown penalty
        if config.drawdown_penalty_rate > 0.0 && ctx.equity_peak > 0.0 {
            let drawdown = (ctx.equity_peak - ctx.equity) / ctx.equity_peak;
            if drawdown > 0.0 {
                reward -= drawdown * config.drawdown_penalty_rate;
            }
        }

        reward
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn neutral_ctx() -> RewardContext {
        RewardContext {
            action: AIAction::Neutral,
            price: 100.0,
            balance: 10_000.0,
            equity: 10_000.0,
            equity_peak: 10_000.0,
            long_position: None,
            short_position: None,
            trade_results: vec![],
            gross_exposure: 0.0,
            hedge_mode: false,
            step: 0,
            action_was_invalid: false,
        }
    }

    #[test]
    fn default_neutral_no_position_zero_reward() {
        let reward = DefaultRewardFn.calculate(&neutral_ctx(), &RewardConfig::default());
        assert!((reward - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn invalid_action_penalized() {
        let mut ctx = neutral_ctx();
        ctx.action_was_invalid = true;
        let reward = DefaultRewardFn.calculate(&ctx, &RewardConfig::default());
        assert!(reward < 0.0);
    }

    #[test]
    fn profitable_trade_positive_reward() {
        let mut ctx = neutral_ctx();
        ctx.action = AIAction::ExitLong;
        ctx.trade_results.push(TradeResult {
            side: Side::Buy,
            pnl_pct: 0.05,
            duration: 5,
            fees: 0.0,
        });
        let reward = DefaultRewardFn.calculate(&ctx, &RewardConfig::default());
        assert!(reward > 0.0);
    }

    #[test]
    fn hold_penalty_after_threshold() {
        let mut ctx = neutral_ctx();
        ctx.long_position = Some(PositionInfo {
            side: Side::Buy,
            entry_price: 100.0,
            entry_step: 0,
            unrealized_pnl_pct: 0.0,
            hold_duration: 30, // > default threshold of 20
        });
        let reward = DefaultRewardFn.calculate(&ctx, &RewardConfig::default());
        assert!(reward < 0.0);
    }

    #[test]
    fn drawdown_penalty() {
        let mut ctx = neutral_ctx();
        ctx.equity = 9_000.0;
        ctx.equity_peak = 10_000.0;
        let config = RewardConfig {
            drawdown_penalty_rate: 1.0,
            ..RewardConfig::default()
        };
        let reward = DefaultRewardFn.calculate(&ctx, &config);
        assert!(reward < 0.0);
    }

    #[test]
    fn win_bonus_increases_reward() {
        let mut ctx = neutral_ctx();
        ctx.trade_results.push(TradeResult {
            side: Side::Buy,
            pnl_pct: 0.05,
            duration: 5,
            fees: 0.0,
        });
        let config_no_bonus = RewardConfig::default();
        let config_bonus = RewardConfig {
            win_bonus: 1.0,
            ..RewardConfig::default()
        };
        let r1 = DefaultRewardFn.calculate(&ctx, &config_no_bonus);
        let r2 = DefaultRewardFn.calculate(&ctx, &config_bonus);
        assert!(r2 > r1);
    }

    #[test]
    fn custom_reward_fn() {
        struct PnlOnly;
        impl RewardFn for PnlOnly {
            fn calculate(&self, ctx: &RewardContext, _config: &RewardConfig) -> f64 {
                ctx.trade_results.iter().map(|t| t.pnl_pct).sum()
            }
        }

        let mut ctx = neutral_ctx();
        ctx.trade_results.push(TradeResult {
            side: Side::Buy,
            pnl_pct: 0.10,
            duration: 100,
            fees: 0.05,
        });
        let reward = PnlOnly.calculate(&ctx, &RewardConfig::default());
        assert!((reward - 0.10).abs() < 1e-10);
    }

    #[test]
    fn exposure_penalty() {
        let mut ctx = neutral_ctx();
        ctx.hedge_mode = true;
        ctx.gross_exposure = 2.0;
        ctx.long_position = Some(PositionInfo {
            side: Side::Buy,
            entry_price: 100.0,
            entry_step: 0,
            unrealized_pnl_pct: 0.0,
            hold_duration: 0,
        });
        ctx.short_position = Some(PositionInfo {
            side: Side::Sell,
            entry_price: 100.0,
            entry_step: 0,
            unrealized_pnl_pct: 0.0,
            hold_duration: 0,
        });
        let config = RewardConfig {
            exposure_penalty_rate: 1.0,
            exposure_penalty_threshold: 1.5,
            hold_penalty_threshold: 1000,
            ..RewardConfig::default()
        };
        let reward = DefaultRewardFn.calculate(&ctx, &config);
        assert!(reward < 0.0);
    }
}
