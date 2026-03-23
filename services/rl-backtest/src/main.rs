use anyhow::{bail, Context, Result};
use ssm_ai::config::{OptimizeConfig, RlConfig};
use ssm_ai::env::{Observation, TradingEnv};
use ssm_ai::features::extract_features;
use ssm_ai::metrics::EpisodeMetrics;
use ssm_ai::model::{AIModel, TableModel};
use ssm_ai::multi_timeframe::Timeframe;
use ssm_ai::optimizer::{
    grid_search, optimize, random_search, run_trial, Objective, ParamRange, SearchSpace,
};
use ssm_core::{AIAction, Candle};
use ssm_exchange::history;
use std::collections::HashMap;
use std::path::PathBuf;

const CVD_WINDOW: usize = 15;

/// Full config file structure including optimizer settings.
#[derive(Debug, Clone, serde::Deserialize)]
struct FileConfig {
    #[serde(default)]
    env: ssm_ai::config::EnvConfig,
    #[serde(default)]
    reward: ssm_ai::config::RewardConfig,
    #[serde(default = "default_timeframes")]
    timeframes: Vec<String>,
    #[serde(default)]
    optimize: OptimizeConfig,
}

fn default_timeframes() -> Vec<String> {
    vec!["15m".to_string()]
}

impl From<FileConfig> for RlConfig {
    fn from(fc: FileConfig) -> Self {
        RlConfig {
            env: fc.env,
            reward: fc.reward,
            timeframes: fc.timeframes,
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let datafile = std::env::var("DATAFILE").context("DATAFILE env var required")?;
    let mode = std::env::var("RL_MODE").unwrap_or_else(|_| "single".into());
    let tf_str = std::env::var("RL_TIMEFRAME").unwrap_or_else(|_| "15m".into());

    let (rl_config, opt_config) = load_config()?;

    let path = PathBuf::from(&datafile);
    let candles = history::load_candles(&path)?;

    if candles.len() < 10 {
        bail!("need at least 10 candles, got {}", candles.len());
    }

    tracing::info!(
        candles = candles.len(),
        mode = %mode,
        timeframe = %tf_str,
        file = %path.display(),
        "starting RL backtest"
    );

    match mode.as_str() {
        "single" => run_single(&candles, &rl_config, &tf_str, &path)?,
        "model" => run_model(&candles, &rl_config, &tf_str, &path)?,
        "optimize" => run_optimize(&candles, &rl_config, &opt_config, &tf_str, &path)?,
        "multi_tf" => run_multi_tf(&candles, &rl_config, &path)?,
        other => bail!("unknown RL_MODE: {other} (expected: single, model, optimize, multi_tf)"),
    }

    Ok(())
}

fn load_config() -> Result<(RlConfig, OptimizeConfig)> {
    if let Ok(config_path) = std::env::var("RL_CONFIG") {
        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("reading config: {config_path}"))?;
        let fc: FileConfig = toml::from_str(&content).context("parsing TOML config")?;
        let opt = fc.optimize.clone();
        Ok((fc.into(), opt))
    } else {
        Ok((RlConfig::default(), OptimizeConfig::default()))
    }
}

/// Simple momentum policy: enter long when price rises, exit when it drops.
fn momentum_policy(obs: &Observation) -> AIAction {
    match obs.position_side {
        None => {
            // Enter long if price is above a simple threshold
            if obs.step > 0 {
                AIAction::EnterLong
            } else {
                AIAction::Neutral
            }
        }
        Some(ssm_core::Side::Buy) => {
            if obs.unrealized_pnl < -0.02 || obs.hold_duration > 10 || obs.unrealized_pnl > 0.03 {
                AIAction::ExitLong
            } else {
                AIAction::Neutral
            }
        }
        Some(ssm_core::Side::Sell) => {
            if obs.unrealized_pnl < -0.02 || obs.hold_duration > 10 || obs.unrealized_pnl > 0.03 {
                AIAction::ExitShort
            } else {
                AIAction::Neutral
            }
        }
    }
}

fn run_single(
    candles: &[Candle],
    config: &RlConfig,
    tf_str: &str,
    path: &std::path::Path,
) -> Result<()> {
    let tf = Timeframe::parse(tf_str).unwrap_or(Timeframe::M15);
    let steps_per_year = tf.steps_per_year();

    let metrics = run_trial(candles, config, steps_per_year, &momentum_policy);
    print_metrics(&metrics, tf_str);

    // Write results
    let out_path = path.with_extension("rl-backtest.json");
    let file = std::fs::File::create(&out_path).context("creating output file")?;
    serde_json::to_writer_pretty(std::io::BufWriter::new(file), &metrics)
        .context("writing results")?;
    tracing::info!(file = %out_path.display(), "RL backtest results saved");

    Ok(())
}

fn run_model(
    candles: &[Candle],
    config: &RlConfig,
    tf_str: &str,
    path: &std::path::Path,
) -> Result<()> {
    let model_path =
        std::env::var("MODEL_PATH").context("MODEL_PATH env var required for RL_MODE=model")?;
    let tf = Timeframe::parse(tf_str).unwrap_or(Timeframe::M15);
    let steps_per_year = tf.steps_per_year();

    tracing::info!(model = %model_path, "loading trained model for backtest");
    let model = TableModel::from_checkpoint(&PathBuf::from(&model_path))?;

    // Run model-based backtest
    let features = extract_features(candles, CVD_WINDOW);
    let mut env =
        TradingEnv::with_config(candles.to_vec(), config.env.clone(), config.reward.clone());
    let mut obs = env.reset();

    while !obs.done {
        let action = if obs.step < features.len() {
            model
                .predict(&features[obs.step])
                .unwrap_or(AIAction::Neutral)
        } else {
            AIAction::Neutral
        };
        let (new_obs, _) = env.step(action);
        obs = new_obs;
    }

    let model_metrics = env.episode_metrics(steps_per_year);

    // Run momentum baseline for comparison
    let baseline_metrics = run_trial(candles, config, steps_per_year, &momentum_policy);

    println!("\n=== Model Backtest Results ({tf_str}) ===");
    print_metrics(&model_metrics, tf_str);

    println!("\n=== Momentum Baseline ({tf_str}) ===");
    print_metrics(&baseline_metrics, tf_str);

    println!("\n=== Comparison ===");
    println!("Model alpha vs B&H:      {:.2}%", model_metrics.alpha);
    println!("Baseline alpha vs B&H:   {:.2}%", baseline_metrics.alpha);
    println!(
        "Model vs Baseline:       {:.2}%",
        model_metrics.total_return_pct - baseline_metrics.total_return_pct
    );

    // Write results
    let out_path = path.with_extension("rl-model-backtest.json");
    let results = serde_json::json!({
        "model": model_metrics,
        "baseline": baseline_metrics,
        "model_path": model_path,
    });
    let file = std::fs::File::create(&out_path).context("creating output file")?;
    serde_json::to_writer_pretty(std::io::BufWriter::new(file), &results)
        .context("writing results")?;
    tracing::info!(file = %out_path.display(), "model backtest results saved");

    Ok(())
}

fn run_optimize(
    candles: &[Candle],
    config: &RlConfig,
    opt_config: &OptimizeConfig,
    tf_str: &str,
    path: &std::path::Path,
) -> Result<()> {
    let tf = Timeframe::parse(tf_str).unwrap_or(Timeframe::M15);
    let steps_per_year = tf.steps_per_year();
    let objective = Objective::parse(&opt_config.objective).unwrap_or(Objective::SharpeRatio);

    // Default search space if none configured
    let space = SearchSpace {
        params: vec![
            (
                "fee_rate".into(),
                ParamRange::Float {
                    min: 0.0001,
                    max: 0.001,
                    steps: 5,
                },
            ),
            (
                "hold_penalty_threshold".into(),
                ParamRange::Int {
                    min: 10,
                    max: 50,
                    step: 10,
                },
            ),
            (
                "hold_penalty_rate".into(),
                ParamRange::Float {
                    min: 0.0005,
                    max: 0.005,
                    steps: 5,
                },
            ),
            (
                "close_penalty_threshold".into(),
                ParamRange::Int {
                    min: 20,
                    max: 100,
                    step: 20,
                },
            ),
            (
                "win_bonus".into(),
                ParamRange::Float {
                    min: 0.0,
                    max: 0.5,
                    steps: 5,
                },
            ),
        ],
    };

    let param_sets = match opt_config.method.as_str() {
        "grid" => grid_search(&space),
        _ => random_search(&space, opt_config.n_trials, opt_config.seed),
    };

    tracing::info!(
        method = %opt_config.method,
        trials = param_sets.len(),
        objective = %opt_config.objective,
        "starting optimization"
    );

    let results = optimize(
        candles,
        config,
        &param_sets,
        objective,
        steps_per_year,
        &momentum_policy,
    );

    println!("\n=== Optimization Results (Top 5) ===");
    for (i, result) in results.iter().take(5).enumerate() {
        println!("\n--- Trial #{} (rank {}) ---", result.trial_id, i + 1);
        println!("Objective ({:?}): {:.4}", objective, result.objective);
        println!("Params: {:?}", result.params);
        print_metrics(&result.metrics, tf_str);
    }

    // Write all results
    let out_path = path.with_extension("rl-optimize.json");
    let file = std::fs::File::create(&out_path).context("creating output file")?;
    serde_json::to_writer_pretty(std::io::BufWriter::new(file), &results)
        .context("writing results")?;
    tracing::info!(file = %out_path.display(), trials = results.len(), "optimization results saved");

    Ok(())
}

fn run_multi_tf(candles: &[Candle], config: &RlConfig, path: &std::path::Path) -> Result<()> {
    let mut all_results: HashMap<String, EpisodeMetrics> = HashMap::new();

    println!("\n=== Multi-Timeframe RL Backtest ===\n");

    for tf_str in &config.timeframes {
        let tf = match Timeframe::parse(tf_str) {
            Some(t) => t,
            None => {
                tracing::warn!(timeframe = %tf_str, "unknown timeframe, skipping");
                continue;
            }
        };

        let resampled = ssm_ai::multi_timeframe::resample_candles(candles, tf);
        if resampled.len() < 10 {
            tracing::warn!(
                timeframe = %tf_str,
                candles = resampled.len(),
                "insufficient candles after resampling, skipping"
            );
            continue;
        }

        let steps_per_year = tf.steps_per_year();
        let metrics = run_trial(&resampled, config, steps_per_year, &momentum_policy);

        println!(
            "--- Timeframe: {} ({} candles) ---",
            tf_str,
            resampled.len()
        );
        print_metrics(&metrics, tf_str);
        println!();

        all_results.insert(tf_str.clone(), metrics);
    }

    // Print comparison table
    print_comparison_table(&all_results);

    // Write results
    let out_path = path.with_extension("rl-multi-tf.json");
    let file = std::fs::File::create(&out_path).context("creating output file")?;
    serde_json::to_writer_pretty(std::io::BufWriter::new(file), &all_results)
        .context("writing results")?;
    tracing::info!(file = %out_path.display(), "multi-TF results saved");

    Ok(())
}

fn print_metrics(m: &EpisodeMetrics, tf: &str) {
    println!("=== RL Backtest Results ({tf}) ===");
    println!("Initial balance:     ${:.2}", m.initial_balance);
    println!("Final balance:       ${:.2}", m.final_balance);
    println!("Total return:        {:.2}%", m.total_return_pct);
    println!("Buy & Hold return:   {:.2}%", m.buy_and_hold_return_pct);
    println!("Alpha (RL - B&H):    {:.2}%", m.alpha);
    println!("Max drawdown:        {:.2}%", m.max_drawdown_pct);
    println!("Sharpe ratio:        {:.4}", m.sharpe_ratio);
    println!("Sortino ratio:       {:.4}", m.sortino_ratio);
    println!("Total trades:        {}", m.total_trades);
    println!("Win rate:            {:.1}%", m.win_rate * 100.0);
    println!("Profit factor:       {:.2}", m.profit_factor);
    println!("Avg win:             ${:.2}", m.avg_win);
    println!("Avg loss:            ${:.2}", m.avg_loss);
    println!("Largest win:         ${:.2}", m.largest_win);
    println!("Largest loss:        ${:.2}", m.largest_loss);
    println!("Avg hold duration:   {:.1} candles", m.avg_hold_duration);
    println!("Total fees paid:     ${:.2}", m.total_fees_paid);
}

fn print_comparison_table(results: &HashMap<String, EpisodeMetrics>) {
    if results.is_empty() {
        return;
    }

    println!("=== Timeframe Comparison ===");
    println!(
        "{:<8} {:>10} {:>10} {:>10} {:>10} {:>10} {:>8}",
        "TF", "Return%", "B&H%", "Alpha%", "MaxDD%", "Sharpe", "Trades"
    );
    println!("{}", "-".repeat(68));

    let mut sorted: Vec<_> = results.iter().collect();
    sorted.sort_by_key(|(k, _)| match k.as_str() {
        "3m" => 0,
        "15m" => 1,
        "1h" => 2,
        "4h" => 3,
        _ => 4,
    });

    for (tf, m) in sorted {
        println!(
            "{:<8} {:>10.2} {:>10.2} {:>10.2} {:>10.2} {:>10.4} {:>8}",
            tf,
            m.total_return_pct,
            m.buy_and_hold_return_pct,
            m.alpha,
            m.max_drawdown_pct,
            m.sharpe_ratio,
            m.total_trades
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn make_candles(n: usize) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let p = Decimal::from_str(&format!("{}", 100 + (i % 20))).unwrap();
                Candle {
                    open_time: (i as i64) * 900_000,
                    open: p,
                    high: p + Decimal::from(2),
                    low: p - Decimal::from(2),
                    close: p,
                    volume: Decimal::from(100),
                    close_time: (i as i64) * 900_000 + 899_999,
                    quote_volume: Decimal::ZERO,
                    trades: 10,
                    taker_buy_volume: Decimal::from(50),
                    taker_sell_volume: Decimal::from(50),
                }
            })
            .collect()
    }

    #[test]
    fn momentum_policy_enters_and_exits() {
        let obs_no_pos = Observation {
            step: 5,
            current_price: 100.0,
            position_side: None,
            unrealized_pnl: 0.0,
            hold_duration: 0,
            done: false,
            balance: 10_000.0,
            equity: 10_000.0,
            long_position_active: false,
            long_unrealized_pnl: 0.0,
            long_hold_duration: 0,
            short_position_active: false,
            short_unrealized_pnl: 0.0,
            short_hold_duration: 0,
            net_exposure: 0.0,
            gross_exposure: 0.0,
        };
        assert_eq!(momentum_policy(&obs_no_pos), AIAction::EnterLong);

        let obs_long_loss = Observation {
            step: 10,
            current_price: 95.0,
            position_side: Some(ssm_core::Side::Buy),
            unrealized_pnl: -0.05,
            hold_duration: 5,
            done: false,
            balance: 10_000.0,
            equity: 9_500.0,
            long_position_active: true,
            long_unrealized_pnl: -0.05,
            long_hold_duration: 5,
            short_position_active: false,
            short_unrealized_pnl: 0.0,
            short_hold_duration: 0,
            net_exposure: 1.0,
            gross_exposure: 1.0,
        };
        assert_eq!(momentum_policy(&obs_long_loss), AIAction::ExitLong);
    }

    #[test]
    fn single_run_produces_metrics() {
        let candles = make_candles(50);
        let config = RlConfig::default();
        let tf = Timeframe::M15;
        let metrics = run_trial(&candles, &config, tf.steps_per_year(), &momentum_policy);
        assert!(metrics.total_trades > 0);
    }
}
