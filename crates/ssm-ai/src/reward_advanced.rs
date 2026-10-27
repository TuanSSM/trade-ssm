//! Advanced (FreqAI-style, 2026) reward shaping.
//!
//! `AdvancedRewardFn` layers additional terms on top of the realized-PnL /
//! fee / exposure / hedge baseline shared with `DefaultRewardFn`:
//!
//! 1. Differential Sharpe (Moody & Saffell): incremental Sharpe over a rolling
//!    returns window — rewards reduced return variance without needing to
//!    recompute the full stat each step.
//! 2. Profit-factor shaping: `tanh(pf − 1)` bonus on each closed trade.
//! 3. Risk-reward ratio: `pnl / |risk|` bonus for each closed trade.
//! 4. Exponential time-decay hold penalty.
//! 5. Inaction penalty when the Neutral streak overruns the configured budget.
//! 6. Drawdown recovery bonus when equity makes a new peak.
//! 7. Trade-frequency anti-churn penalty beyond a configured cap.
//!
//! All weights default to zero so an instance of this reward fn, with no
//! config tweaks, behaves like the legacy reward function.

use std::collections::VecDeque;
use std::sync::Mutex;

use ssm_core::AIAction;

use crate::config::RewardConfig;
use crate::reward::{RewardContext, RewardFn};

#[derive(Debug, Default)]
struct AdvancedState {
    /// Rolling buffer of equity-based step returns.
    returns: VecDeque<f64>,
    /// Exponential moving mean (A) for differential Sharpe.
    sharpe_a: f64,
    /// Exponential moving variance proxy (B) for differential Sharpe.
    sharpe_b: f64,
    /// Running sums of closed-trade PnL% magnitudes.
    gross_profit: f64,
    gross_loss: f64,
    /// Last observed equity for step-return computation.
    last_equity: Option<f64>,
    /// Last observed equity peak for drawdown-recovery detection.
    last_peak_equity: f64,
    /// Consecutive Neutral action streak.
    neutral_streak: usize,
    /// Total number of trades closed this episode (anti-churn).
    trade_count: u32,
}

/// FreqAI-style composite reward function.
pub struct AdvancedRewardFn {
    state: Mutex<AdvancedState>,
}

impl AdvancedRewardFn {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(AdvancedState::default()),
        }
    }

    /// Reset accumulated state. Call this between episodes if the same
    /// instance is reused; the trainer creates a fresh instance per episode
    /// via the reward-fn factory, so this is only useful for tests.
    pub fn reset(&self) {
        *self.state.lock().unwrap() = AdvancedState::default();
    }
}

impl Default for AdvancedRewardFn {
    fn default() -> Self {
        Self::new()
    }
}

impl RewardFn for AdvancedRewardFn {
    fn calculate(&self, ctx: &RewardContext, config: &RewardConfig) -> f64 {
        // 1. Invalid action short-circuit — matches DefaultRewardFn.
        if ctx.action_was_invalid {
            return -config.invalid_action_penalty;
        }

        let mut state = self.state.lock().unwrap();
        let mut reward = 0.0_f64;

        // 2. Realized PnL from closed trades (base term).
        for trade in &ctx.trade_results {
            let mut trade_reward = trade.pnl_pct;
            if trade.duration > config.close_penalty_threshold {
                trade_reward -= config.close_penalty_rate * trade.duration as f64;
            }
            if config.fee_penalty {
                trade_reward -= trade.fees;
            }
            if trade.pnl_pct > 0.0 && config.win_bonus > 0.0 {
                trade_reward += trade.pnl_pct * config.win_bonus;
            }

            // Profit-factor accumulator.
            if trade.pnl_pct >= 0.0 {
                state.gross_profit += trade.pnl_pct;
            } else {
                state.gross_loss += -trade.pnl_pct;
            }

            // Profit-factor shaping bonus.
            if config.profit_factor_weight > 0.0 && state.gross_loss > 0.0 {
                let pf = state.gross_profit / state.gross_loss;
                trade_reward += config.profit_factor_weight * (pf - 1.0).tanh();
            }

            // Risk-reward bonus: approximate risk as 2% of entry when the
            // caller does not supply an explicit stop.
            if config.rr_bonus_weight > 0.0 {
                let risk = 0.02_f64;
                trade_reward += config.rr_bonus_weight * (trade.pnl_pct / risk);
            }

            reward += trade_reward;
            state.trade_count = state.trade_count.saturating_add(1);
        }

        // 3. Exponential (or linear) time-decay hold penalty while Neutral.
        if ctx.action == AIAction::Neutral {
            for pos in [&ctx.long_position, &ctx.short_position]
                .into_iter()
                .flatten()
            {
                if pos.hold_duration > config.hold_penalty_threshold {
                    let over = (pos.hold_duration - config.hold_penalty_threshold) as f64;
                    // factor = over * decay^over so:
                    //   decay == 1.0 → linear (over)
                    //   decay  > 1.0 → superlinear time-decay
                    // Cap the exponent to avoid overflow for pathological configs.
                    let decay = config.hold_decay_base.max(1.0);
                    let factor = over * decay.powf(over.min(64.0));
                    reward -= config.hold_penalty_rate * factor;
                }
            }
        }

        // 4. Exposure penalty (hedge mode) — same shape as DefaultRewardFn.
        if config.exposure_penalty_rate > 0.0
            && ctx.gross_exposure > config.exposure_penalty_threshold
        {
            reward -= config.exposure_penalty_rate
                * (ctx.gross_exposure - config.exposure_penalty_threshold);
        }

        // 5. Hedge bonus (hedge mode).
        if ctx.long_position.is_some() && ctx.short_position.is_some() && config.hedge_bonus > 0.0 {
            reward += config.hedge_bonus * (ctx.gross_exposure / 2.0);
        }

        // 6. Drawdown penalty (same as default) + recovery bonus.
        if ctx.equity_peak > 0.0 {
            let drawdown = (ctx.equity_peak - ctx.equity) / ctx.equity_peak;
            if drawdown > 0.0 && config.drawdown_penalty_rate > 0.0 {
                reward -= drawdown * config.drawdown_penalty_rate;
            }
        }
        if config.drawdown_recovery_bonus > 0.0
            && state.last_peak_equity > 0.0
            && ctx.equity > state.last_peak_equity
        {
            let gain = (ctx.equity - state.last_peak_equity) / state.last_peak_equity;
            reward += config.drawdown_recovery_bonus * gain;
        }
        if ctx.equity_peak > state.last_peak_equity {
            state.last_peak_equity = ctx.equity_peak;
        }

        // 7. Differential Sharpe over rolling returns.
        if config.sharpe_weight > 0.0 && config.sharpe_window > 1 {
            if let Some(prev) = state.last_equity {
                if prev > 0.0 {
                    let r = (ctx.equity - prev) / prev;
                    state.returns.push_back(r);
                    while state.returns.len() > config.sharpe_window {
                        state.returns.pop_front();
                    }

                    // Moody & Saffell differential Sharpe update.
                    let eta = 1.0 / config.sharpe_window as f64;
                    let delta_a = r - state.sharpe_a;
                    let delta_b = r * r - state.sharpe_b;
                    let denom = (state.sharpe_b - state.sharpe_a * state.sharpe_a).max(1e-9);
                    let numer = state.sharpe_b * delta_a - 0.5 * state.sharpe_a * delta_b;
                    let incremental = numer / denom.powf(1.5);
                    reward += config.sharpe_weight * incremental;
                    state.sharpe_a += eta * delta_a;
                    state.sharpe_b += eta * delta_b;
                }
            }
        }
        state.last_equity = Some(ctx.equity);

        // 8. Inaction penalty when the agent sits idle too long.
        if ctx.action == AIAction::Neutral {
            state.neutral_streak = state.neutral_streak.saturating_add(1);
        } else {
            state.neutral_streak = 0;
        }
        if config.inaction_penalty_rate > 0.0
            && state.neutral_streak > config.inaction_streak_threshold
        {
            let over = (state.neutral_streak - config.inaction_streak_threshold) as f64;
            reward -= config.inaction_penalty_rate * over;
        }

        // 9. Anti-churn: penalize once per trade beyond the configured cap.
        if config.trade_frequency_cap > 0 && state.trade_count > config.trade_frequency_cap {
            // Each trade above the cap imposes a small flat penalty.
            let over = (state.trade_count - config.trade_frequency_cap) as f64;
            reward -= 0.01 * over;
        }

        reward
    }
}

/// Build a reward-fn trait object from the config's selector field.
pub fn build_reward_fn(config: &RewardConfig) -> Box<dyn RewardFn> {
    match config.reward_fn.as_deref() {
        Some("advanced") => Box::new(AdvancedRewardFn::new()),
        _ => Box::new(crate::reward::DefaultRewardFn),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reward::{PositionInfo, TradeResult};
    use ssm_core::Side;

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
    fn zero_config_matches_default_shape_on_quiet_step() {
        let fn_adv = AdvancedRewardFn::new();
        let r = fn_adv.calculate(&neutral_ctx(), &RewardConfig::default());
        assert!(r.abs() < f64::EPSILON);
    }

    #[test]
    fn invalid_action_penalized() {
        let fn_adv = AdvancedRewardFn::new();
        let mut ctx = neutral_ctx();
        ctx.action_was_invalid = true;
        let r = fn_adv.calculate(&ctx, &RewardConfig::default());
        assert!(r < 0.0);
    }

    #[test]
    fn profit_factor_bonus_positive_for_winning_trades() {
        let fn_adv = AdvancedRewardFn::new();
        let cfg = RewardConfig {
            profit_factor_weight: 1.0,
            ..RewardConfig::default()
        };
        let mut ctx = neutral_ctx();
        ctx.action = AIAction::ExitLong;
        ctx.trade_results.push(TradeResult {
            side: Side::Buy,
            pnl_pct: 0.05,
            duration: 5,
            fees: 0.0,
        });
        // First trade: gross_profit=0.05, gross_loss=0 → pf term skipped (loss=0).
        let r1 = fn_adv.calculate(&ctx, &cfg);
        assert!(r1 > 0.0);

        // Second trade: a losing one — now pf exists and the term engages.
        ctx.trade_results.clear();
        ctx.trade_results.push(TradeResult {
            side: Side::Buy,
            pnl_pct: -0.01,
            duration: 5,
            fees: 0.0,
        });
        let r2 = fn_adv.calculate(&ctx, &cfg);
        // pf = 0.05 / 0.01 = 5.0 → tanh(4) > 0, bonus should push reward above
        // the raw -0.01 PnL.
        assert!(r2 > -0.01);
    }

    #[test]
    fn drawdown_recovery_bonus_fires_on_new_peak() {
        let fn_adv = AdvancedRewardFn::new();
        let cfg = RewardConfig {
            drawdown_recovery_bonus: 10.0,
            ..RewardConfig::default()
        };
        let mut ctx = neutral_ctx();
        ctx.equity = 10_000.0;
        ctx.equity_peak = 10_000.0;
        // Seed the peak.
        fn_adv.calculate(&ctx, &cfg);

        // Equity climbs past the previous peak.
        ctx.equity = 11_000.0;
        ctx.equity_peak = 11_000.0;
        let r = fn_adv.calculate(&ctx, &cfg);
        assert!(r > 0.0, "expected recovery bonus, got {r}");
    }

    #[test]
    fn inaction_penalty_after_streak() {
        let fn_adv = AdvancedRewardFn::new();
        let cfg = RewardConfig {
            inaction_penalty_rate: 1.0,
            inaction_streak_threshold: 2,
            ..RewardConfig::default()
        };
        let mut ctx = neutral_ctx();
        // No position but just measuring inaction streak logic.
        let mut last = 0.0_f64;
        for _ in 0..5 {
            last = fn_adv.calculate(&ctx, &cfg);
            ctx.step += 1;
        }
        assert!(last < 0.0, "expected inaction penalty, got {last}");
    }

    #[test]
    fn exponential_hold_penalty_exceeds_linear() {
        let mut cfg = RewardConfig {
            hold_penalty_threshold: 5,
            hold_penalty_rate: 0.1,
            ..RewardConfig::default()
        };
        let mut ctx = neutral_ctx();
        ctx.long_position = Some(PositionInfo {
            side: Side::Buy,
            entry_price: 100.0,
            entry_step: 0,
            unrealized_pnl_pct: 0.0,
            hold_duration: 15, // over threshold by 10
        });

        // Linear baseline (decay_base = 1.0).
        let fn_lin = AdvancedRewardFn::new();
        let lin = fn_lin.calculate(&ctx, &cfg);

        // Exponential (decay_base = 1.2) → factor 1.2^10 ≈ 6.19.
        cfg.hold_decay_base = 1.2;
        let fn_exp = AdvancedRewardFn::new();
        let expn = fn_exp.calculate(&ctx, &cfg);

        assert!(expn < lin, "exp hold penalty should exceed linear");
    }

    #[test]
    fn differential_sharpe_rewards_steady_gains() {
        let fn_adv = AdvancedRewardFn::new();
        let cfg = RewardConfig {
            sharpe_weight: 1.0,
            sharpe_window: 32,
            ..RewardConfig::default()
        };
        let mut ctx = neutral_ctx();
        ctx.equity = 10_000.0;
        fn_adv.calculate(&ctx, &cfg);
        // Smooth upward returns.
        let mut total = 0.0_f64;
        for i in 1..40 {
            ctx.equity = 10_000.0 * (1.0 + 0.001 * i as f64);
            total += fn_adv.calculate(&ctx, &cfg);
        }
        assert!(
            total > 0.0,
            "expected positive Sharpe contribution: {total}"
        );
    }

    #[test]
    fn build_reward_fn_dispatches() {
        let cfg = RewardConfig {
            reward_fn: Some("default".into()),
            ..RewardConfig::default()
        };
        let b = build_reward_fn(&cfg);
        // Shape check: both impls return 0 on a quiet neutral step.
        assert!(b.calculate(&neutral_ctx(), &cfg).abs() < f64::EPSILON);

        let cfg = RewardConfig {
            reward_fn: Some("advanced".into()),
            ..RewardConfig::default()
        };
        let b = build_reward_fn(&cfg);
        assert!(b.calculate(&neutral_ctx(), &cfg).abs() < f64::EPSILON);
    }
}
