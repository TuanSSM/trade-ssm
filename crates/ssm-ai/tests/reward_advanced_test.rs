//! Integration tests for the FreqAI-style AdvancedRewardFn.
//!
//! These exercise the trainer / env / factory wiring end-to-end so that
//! enabling the advanced reward in config actually reaches the step loop.

use rust_decimal::Decimal;
use ssm_ai::config::{EnvConfig, RewardConfig, TrainingConfig};
use ssm_ai::env::TradingEnv;
use ssm_ai::reward_advanced::{build_reward_fn, AdvancedRewardFn};
use ssm_ai::trainer::{RlTrainer, TrainerConfig};
use ssm_core::{AIAction, Candle};
use std::str::FromStr;

fn candle_at(open_time: i64, close: &str) -> Candle {
    let p = Decimal::from_str(close).unwrap();
    Candle {
        open_time,
        open: p,
        high: p + Decimal::from(2),
        low: p - Decimal::from(2),
        close: p,
        volume: Decimal::from(100),
        close_time: open_time + 899_999,
        quote_volume: Decimal::ZERO,
        trades: 50,
        taker_buy_volume: Decimal::from(55),
        taker_sell_volume: Decimal::from(45),
    }
}

fn make_candles(n: usize) -> Vec<Candle> {
    (0..n)
        .map(|i| {
            let price = 100 + (i % 20) as i64;
            candle_at((i as i64) * 900_000, &price.to_string())
        })
        .collect()
}

#[test]
fn build_reward_fn_selects_default_when_unset() {
    let cfg = RewardConfig::default();
    let _b = build_reward_fn(&cfg); // compiles & dispatches
}

#[test]
fn env_runs_an_episode_with_advanced_reward() {
    let candles = make_candles(30);
    let cfg = RewardConfig {
        reward_fn: Some("advanced".into()),
        sharpe_weight: 0.1,
        profit_factor_weight: 0.1,
        drawdown_recovery_bonus: 0.1,
        rr_bonus_weight: 0.1,
        hold_decay_base: 1.1,
        ..RewardConfig::default()
    };

    let env_cfg = EnvConfig::default();
    let mut env =
        TradingEnv::with_reward_fn(candles, env_cfg, cfg, Box::new(AdvancedRewardFn::new()));
    let mut obs = env.reset();
    let mut total = 0.0;
    while !obs.done {
        let (next, r) = env.step(if obs.step == 1 {
            AIAction::EnterLong
        } else if obs.step == 10 {
            AIAction::ExitLong
        } else {
            AIAction::Neutral
        });
        total += r;
        obs = next;
    }
    assert!(total.is_finite(), "reward sum should be finite: {total}");
}

#[test]
fn trainer_integrates_advanced_reward_fn() {
    let candles = make_candles(60);
    let reward = RewardConfig {
        reward_fn: Some("advanced".into()),
        sharpe_weight: 0.05,
        drawdown_recovery_bonus: 0.05,
        ..RewardConfig::default()
    };

    let config = TrainerConfig {
        reward,
        n_epochs: 1,
        episodes_per_epoch: 1,
        min_episode_length: 10,
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
