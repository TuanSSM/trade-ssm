use anyhow::Result;
use ssm_ai::config::RlConfig;
use ssm_ai::multi_timeframe::Timeframe;
use ssm_ai::optimizer::run_trial;
use ssm_core::{AIAction, Candle};
use ssm_nats::{Publisher, Subscriber};
use tokio::sync::mpsc;

const DEFAULT_SYMBOL: &str = "BTCUSDT";
const DEFAULT_INTERVAL: &str = "15m";
const MIN_TRAINING_CANDLES: usize = 100;
const RETRAIN_INTERVAL_CANDLES: usize = 50;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let symbol = env_or("SYMBOL", DEFAULT_SYMBOL);
    let interval = env_or("INTERVAL", DEFAULT_INTERVAL);
    let tf = Timeframe::parse(&interval).unwrap_or(Timeframe::M15);

    tracing::info!(%symbol, %interval, "rl-trainer service starting");

    let nats_client = ssm_nats::connect().await?;
    let publisher = Publisher::new(nats_client.clone());
    let subscriber = Subscriber::new(nats_client);

    let config = RlConfig::default();

    // Subscribe to candle feed
    let candle_topic = ssm_nats::topics::candles(&symbol, &interval);
    let (tx, mut rx) = mpsc::channel::<Candle>(1_000);

    tokio::spawn(async move {
        if let Err(e) = subscriber.subscribe_typed(&candle_topic, tx).await {
            tracing::error!(error = %e, "candle subscription failed");
        }
    });

    let mut candle_buffer: Vec<Candle> = Vec::new();
    let mut candles_since_train = 0usize;

    tracing::info!("waiting for candle data to accumulate for training");

    while let Some(candle) = rx.recv().await {
        candle_buffer.push(candle);
        candles_since_train += 1;

        // Train when we have enough data and enough new candles
        if candle_buffer.len() >= MIN_TRAINING_CANDLES
            && candles_since_train >= RETRAIN_INTERVAL_CANDLES
        {
            tracing::info!(candles = candle_buffer.len(), "starting training episode");

            let steps_per_year = tf.steps_per_year();
            let metrics = run_trial(&candle_buffer, &config, steps_per_year, &momentum_policy);

            tracing::info!(
                return_pct = format!("{:.2}", metrics.total_return_pct),
                sharpe = format!("{:.4}", metrics.sharpe_ratio),
                trades = metrics.total_trades,
                win_rate = format!("{:.1}%", metrics.win_rate * 100.0),
                "training episode complete"
            );

            // Publish metrics
            let metrics_topic = ssm_nats::topics::metrics("rl-trainer");
            if let Err(e) = publisher.publish(&metrics_topic, &metrics).await {
                tracing::warn!(error = %e, "failed to publish training metrics");
            }

            candles_since_train = 0;
        }
    }

    Ok(())
}

/// Simple momentum policy used for baseline evaluation.
/// Will be replaced by trained RL model in future phases.
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
