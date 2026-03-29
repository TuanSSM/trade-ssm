use anyhow::{Context, Result};
use ssm_core::{env_or, Candle, DEFAULT_CVD_WINDOW, DEFAULT_INTERVAL,
    DEFAULT_MAX_CANDLES, DEFAULT_SYMBOL};
use ssm_nats::{Publisher, Subscriber};
use ssm_strategy::traits::Strategy;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let symbol = env_or("SYMBOL", DEFAULT_SYMBOL);
    let interval = env_or("INTERVAL", DEFAULT_INTERVAL);
    let strategy_mode = env_or("STRATEGY_MODE", "cvd");

    tracing::info!(%symbol, %interval, %strategy_mode, "signal service starting");

    let strategy: Box<dyn Strategy> = match strategy_mode.as_str() {
        "ai" => {
            let model_path = std::env::var("MODEL_PATH")
                .context("MODEL_PATH env var required when STRATEGY_MODE=ai")?;
            tracing::info!(%model_path, "loading AI model for signal generation");
            let model =
                ssm_ai::model::TableModel::from_checkpoint(&std::path::PathBuf::from(&model_path))?;
            Box::new(ssm_strategy::ai_strategy::AiStrategy::new(
                Box::new(model),
                DEFAULT_CVD_WINDOW,
            ))
        }
        _ => {
            tracing::info!("using CVD momentum strategy");
            Box::new(ssm_strategy::cvd_momentum::CvdMomentumStrategy::new(
                DEFAULT_CVD_WINDOW,
            ))
        }
    };

    let nats_client = ssm_nats::connect().await?;
    let publisher = Publisher::new(nats_client.clone());
    let subscriber = Subscriber::new(nats_client);

    let signal_topic = ssm_nats::topics::signals(&symbol);

    // Subscribe to candle feed
    let candle_topic = ssm_nats::topics::candles(&symbol, &interval);
    let (tx, mut rx) = mpsc::channel::<Candle>(1_000);

    tokio::spawn(async move {
        if let Err(e) = subscriber.subscribe_typed(&candle_topic, tx).await {
            tracing::error!(error = %e, "candle subscription failed");
        }
    });

    let mut candle_buffer: Vec<Candle> = Vec::with_capacity(DEFAULT_MAX_CANDLES);

    tracing::info!("waiting for candle data");

    while let Some(candle) = rx.recv().await {
        candle_buffer.push(candle);

        // Keep buffer bounded
        if candle_buffer.len() > DEFAULT_MAX_CANDLES {
            candle_buffer.drain(0..candle_buffer.len() - DEFAULT_MAX_CANDLES);
        }

        // Anti-repainting: only analyze closed candles (all candles from NATS should be closed)
        if candle_buffer.len() >= DEFAULT_CVD_WINDOW {
            match strategy.analyze(&candle_buffer) {
                Ok(Some(signal)) => {
                    tracing::info!(
                        action = ?signal.action,
                        confidence = signal.confidence,
                        source = %signal.source,
                        "signal generated"
                    );
                    if let Err(e) = publisher.publish(&signal_topic, &signal).await {
                        tracing::warn!(error = %e, "failed to publish signal");
                    }
                }
                Ok(None) => {
                    tracing::debug!("no signal this candle");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "strategy analysis failed");
                }
            }
        }
    }

    Ok(())
}

