use anyhow::Result;
use ssm_core::Candle;
use ssm_nats::{Publisher, Subscriber};
use ssm_strategy::cvd_momentum::CvdMomentumStrategy;
use ssm_strategy::traits::Strategy;
use tokio::sync::mpsc;

const DEFAULT_SYMBOL: &str = "BTCUSDT";
const DEFAULT_INTERVAL: &str = "15m";
const CVD_WINDOW: usize = 15;
const MAX_CANDLE_BUFFER: usize = 200;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let symbol = env_or("SYMBOL", DEFAULT_SYMBOL);
    let interval = env_or("INTERVAL", DEFAULT_INTERVAL);

    tracing::info!(%symbol, %interval, "signal service starting");

    let nats_client = ssm_nats::connect().await?;
    let publisher = Publisher::new(nats_client.clone());
    let subscriber = Subscriber::new(nats_client);

    let strategy = CvdMomentumStrategy::new(CVD_WINDOW);
    let signal_topic = ssm_nats::topics::signals(&symbol);

    // Subscribe to candle feed
    let candle_topic = ssm_nats::topics::candles(&symbol, &interval);
    let (tx, mut rx) = mpsc::channel::<Candle>(1_000);

    tokio::spawn(async move {
        if let Err(e) = subscriber.subscribe_typed(&candle_topic, tx).await {
            tracing::error!(error = %e, "candle subscription failed");
        }
    });

    let mut candle_buffer: Vec<Candle> = Vec::with_capacity(MAX_CANDLE_BUFFER);

    tracing::info!("waiting for candle data");

    while let Some(candle) = rx.recv().await {
        candle_buffer.push(candle);

        // Keep buffer bounded
        if candle_buffer.len() > MAX_CANDLE_BUFFER {
            candle_buffer.drain(0..candle_buffer.len() - MAX_CANDLE_BUFFER);
        }

        // Anti-repainting: only analyze closed candles (all candles from NATS should be closed)
        if candle_buffer.len() >= CVD_WINDOW {
            match strategy.analyze(&candle_buffer) {
                Ok(Some(signal)) => {
                    tracing::info!(
                        action = ?signal.action,
                        confidence = signal.confidence,
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

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
