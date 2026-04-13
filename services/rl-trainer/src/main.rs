use anyhow::Result;
use ssm_ai::config::RlConfig;
use ssm_ai::correlated_features::CorrelatedPairFeatures;
use ssm_ai::correlated_features::CROSS_PAIR_FEATURE_COUNT;
use ssm_ai::env::TradingEnv;
use ssm_ai::features::{extract_features, label_features, FEATURE_COUNT};
use ssm_ai::metrics::EpisodeMetrics;
use ssm_ai::model::{AIModel, TableModel};
use ssm_ai::multi_timeframe::Timeframe;
use ssm_ai::optimizer::run_trial;
use ssm_core::{AIAction, Candle};
use ssm_nats::{Publisher, Subscriber};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

const DEFAULT_SYMBOL: &str = "BTCUSDT";
const DEFAULT_INTERVAL: &str = "15m";
const MIN_TRAINING_CANDLES: usize = 100;
const RETRAIN_INTERVAL_CANDLES: usize = 50;
const CVD_WINDOW: usize = 15;
const TRAINING_EPOCHS: usize = 5;
const LEARNING_RATE: f64 = 0.01;
const LABEL_HORIZON: usize = 3;

#[tokio::main]
async fn main() -> Result<()> {
    ssm_core::init_logging();

    let symbol = env_or("SYMBOL", DEFAULT_SYMBOL);
    let interval = env_or("INTERVAL", DEFAULT_INTERVAL);
    let tf = Timeframe::parse(&interval).unwrap_or(Timeframe::M15);
    let model_dir = env_or("MODEL_DIR", "models");
    let learning_rate: f64 = env_or("LEARNING_RATE", &LEARNING_RATE.to_string())
        .parse()
        .unwrap_or(LEARNING_RATE);

    let correlation_pairs: Vec<String> = env_or("CORRELATION_PAIRS", "")
        .split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();

    // Validate correlation pairs
    if let Err(e) = ssm_ai::config::validate_correlation_pairs(&symbol, &correlation_pairs) {
        tracing::error!(error = %e, "invalid correlation pairs configuration");
        return Err(e.into());
    }

    tracing::info!(
        %symbol, %interval, %model_dir,
        correlation_pairs = ?correlation_pairs,
        "rl-trainer service starting"
    );

    // Ensure model directory exists
    std::fs::create_dir_all(&model_dir)?;

    let nats_client = ssm_nats::connect().await?;
    let publisher = Publisher::new(nats_client.clone());

    let config = RlConfig {
        correlation_pairs: correlation_pairs.clone(),
        ..RlConfig::default()
    };

    // Compute feature count: 22 base + (22 raw + 5 derived) per correlated pair
    let total_features =
        FEATURE_COUNT + (FEATURE_COUNT + CROSS_PAIR_FEATURE_COUNT) * correlation_pairs.len();

    // Initialize or load model
    let model_path = PathBuf::from(&model_dir).join("table_model_latest.json");
    let mut model = if model_path.exists() {
        tracing::info!(path = %model_path.display(), "loading existing model checkpoint");
        TableModel::from_checkpoint(&model_path)?
    } else {
        tracing::info!(
            "initializing new TableModel with {} features ({} base + {} correlated)",
            total_features,
            FEATURE_COUNT,
            FEATURE_COUNT * correlation_pairs.len(),
        );
        TableModel::new(total_features, learning_rate)
    };

    let mut best_sharpe = f64::NEG_INFINITY;

    // Subscribe to primary candle feed
    let candle_topic = ssm_nats::topics::candles(&symbol, &interval);
    let (tx, mut rx) = mpsc::channel::<Candle>(1_000);

    let primary_sub = Subscriber::new(nats_client.clone());
    tokio::spawn(async move {
        if let Err(e) = primary_sub.subscribe_typed(&candle_topic, tx).await {
            tracing::error!(error = %e, "candle subscription failed");
        }
    });

    // Subscribe to correlated pair candle feeds with shared buffer
    let corr_buffers: Arc<Mutex<HashMap<String, Vec<Candle>>>> = {
        let mut map = HashMap::new();
        for pair in &correlation_pairs {
            map.insert(pair.clone(), Vec::new());
        }
        Arc::new(Mutex::new(map))
    };

    for pair in &correlation_pairs {
        let corr_topic = ssm_nats::topics::candles(pair, &interval);
        let (corr_tx, mut corr_rx) = mpsc::channel::<Candle>(1_000);
        let corr_sub = Subscriber::new(nats_client.clone());
        let pair_clone = pair.clone();
        tokio::spawn(async move {
            if let Err(e) = corr_sub.subscribe_typed(&corr_topic, corr_tx).await {
                tracing::error!(symbol = %pair_clone, error = %e, "correlated pair subscription failed");
            }
        });
        let buffers = Arc::clone(&corr_buffers);
        let pair_for_drain = pair.clone();
        tokio::spawn(async move {
            while let Some(candle) = corr_rx.recv().await {
                let mut map = buffers.lock().await;
                if let Some(buf) = map.get_mut(&pair_for_drain) {
                    buf.push(candle);
                }
            }
        });
    }

    let mut candle_buffer: Vec<Candle> = Vec::new();
    let mut candles_since_train = 0usize;

    tracing::info!("waiting for candle data to accumulate for training");

    tokio::select! {
        result = async {
    while let Some(candle) = rx.recv().await {
        candle_buffer.push(candle);
        candles_since_train += 1;

        // Train when we have enough data and enough new candles
        if candle_buffer.len() >= MIN_TRAINING_CANDLES
            && candles_since_train >= RETRAIN_INTERVAL_CANDLES
        {
            // Snapshot correlated buffers and trim to bound memory
            let corr_snapshot: HashMap<String, Vec<Candle>> = {
                let mut map = corr_buffers.lock().await;
                let max_buffer = candle_buffer.len() * 2;
                for buf in map.values_mut() {
                    if buf.len() > max_buffer {
                        buf.drain(0..buf.len() - max_buffer);
                    }
                }
                map.clone()
            };

            tracing::info!(
                candles = candle_buffer.len(),
                correlated_pairs = corr_snapshot.len(),
                "starting training cycle"
            );

            // Phase 1: Supervised pre-training on labeled features
            let mut features = extract_features(&candle_buffer, CVD_WINDOW);

            // Merge correlated pair features (raw + derived)
            if !correlation_pairs.is_empty() && !corr_snapshot.is_empty() {
                let cpf = CorrelatedPairFeatures::new(
                    String::new(),
                    correlation_pairs.clone(),
                );
                features = cpf.merge_features_with_derived(
                    &features,
                    &candle_buffer,
                    &corr_snapshot,
                    CVD_WINDOW,
                );
            }

            label_features(&mut features, &candle_buffer, LABEL_HORIZON);

            let labeled: Vec<_> = features.into_iter().filter(|f| f.label.is_some()).collect();

            if !labeled.is_empty() {
                for epoch in 0..TRAINING_EPOCHS {
                    let metrics = model.train(&labeled)?;
                    tracing::debug!(
                        epoch,
                        accuracy = format!("{:.2}%", metrics.accuracy * 100.0),
                        loss = format!("{:.4}", metrics.loss),
                        "supervised training epoch"
                    );
                }
                tracing::info!(
                    samples = labeled.len(),
                    epochs = TRAINING_EPOCHS,
                    "supervised training complete"
                );
            }

            // Phase 2: Evaluate via RL environment rollout
            let steps_per_year = tf.steps_per_year();
            let eval_metrics = evaluate_model(
                &model,
                &candle_buffer,
                &config,
                steps_per_year,
                &corr_snapshot,
            );

            tracing::info!(
                return_pct = format!("{:.2}", eval_metrics.total_return_pct),
                sharpe = format!("{:.4}", eval_metrics.sharpe_ratio),
                trades = eval_metrics.total_trades,
                win_rate = format!("{:.1}%", eval_metrics.win_rate * 100.0),
                alpha = format!("{:.2}", eval_metrics.alpha),
                "evaluation episode complete"
            );

            // Phase 3: Compare against momentum baseline
            let baseline_metrics =
                run_trial(&candle_buffer, &config, steps_per_year, &momentum_policy);

            tracing::info!(
                model_return = format!("{:.2}%", eval_metrics.total_return_pct),
                baseline_return = format!("{:.2}%", baseline_metrics.total_return_pct),
                model_sharpe = format!("{:.4}", eval_metrics.sharpe_ratio),
                baseline_sharpe = format!("{:.4}", baseline_metrics.sharpe_ratio),
                "model vs baseline comparison"
            );

            // Phase 4: Save checkpoint (best model by Sharpe)
            model.save(&model_path)?;
            tracing::info!(path = %model_path.display(), "saved latest model checkpoint");

            if eval_metrics.sharpe_ratio > best_sharpe {
                best_sharpe = eval_metrics.sharpe_ratio;
                let best_path = PathBuf::from(&model_dir).join("table_model_best.json");
                model.save(&best_path)?;
                tracing::info!(
                    sharpe = format!("{:.4}", best_sharpe),
                    path = %best_path.display(),
                    "new best model saved"
                );
            }

            // Phase 5: Publish metrics to NATS
            let metrics_topic = ssm_nats::topics::metrics("rl-trainer");
            if let Err(e) = publisher.publish(&metrics_topic, &eval_metrics).await {
                tracing::warn!(error = %e, "failed to publish training metrics");
            }

            candles_since_train = 0;
        }
    }
    Ok::<(), anyhow::Error>(())
        } => { result?; },
        _ = shutdown_signal() => {},
    }

    tracing::info!("rl-trainer shut down");
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl+c");
    tracing::info!("shutdown signal received, exiting gracefully");
}

/// Evaluate a trained model by running it through the RL environment.
fn evaluate_model(
    model: &TableModel,
    candles: &[Candle],
    config: &RlConfig,
    steps_per_year: f64,
    correlated_candles: &HashMap<String, Vec<Candle>>,
) -> EpisodeMetrics {
    let features = extract_features(candles, CVD_WINDOW);
    let features = if !config.correlation_pairs.is_empty() && !correlated_candles.is_empty() {
        let cpf = CorrelatedPairFeatures::new(String::new(), config.correlation_pairs.clone());
        cpf.merge_features_with_derived(&features, candles, correlated_candles, CVD_WINDOW)
    } else {
        features
    };
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

    env.episode_metrics(steps_per_year)
}

/// Simple momentum policy used for baseline comparison.
fn momentum_policy(obs: &ssm_ai::env::Observation) -> AIAction {
    match obs.position_side {
        None => {
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

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
