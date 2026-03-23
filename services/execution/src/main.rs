use anyhow::{Context, Result};
use rust_decimal::Decimal;
use ssm_core::{AIAction, ExecutionMode, Signal};
use ssm_execution::engine::ExecutionEngine;
use ssm_nats::{Publisher, Subscriber};
use tokio::sync::mpsc;

const DEFAULT_SYMBOL: &str = "BTCUSDT";
const DEFAULT_QUANTITY: &str = "0.001";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let symbol = env_or("SYMBOL", DEFAULT_SYMBOL);
    let mode = match env_or("EXECUTION_MODE", "paper").as_str() {
        "live" => ExecutionMode::Live,
        _ => ExecutionMode::Paper,
    };
    let quantity: Decimal = env_or("TRADE_QUANTITY", DEFAULT_QUANTITY)
        .parse()
        .context("parsing TRADE_QUANTITY")?;

    tracing::info!(%symbol, ?mode, %quantity, "execution service starting");

    let nats_client = ssm_nats::connect().await?;
    let publisher = Publisher::new(nats_client.clone());
    let subscriber = Subscriber::new(nats_client);

    let mut engine = ExecutionEngine::new(mode);

    // Subscribe to signals
    let signal_topic = ssm_nats::topics::signals(&symbol);
    let (tx, mut rx) = mpsc::channel::<Signal>(1_000);

    tokio::spawn(async move {
        if let Err(e) = subscriber.subscribe_typed(&signal_topic, tx).await {
            tracing::error!(error = %e, "signal subscription failed");
        }
    });

    let order_topic = ssm_nats::topics::orders(&symbol);
    let position_topic = ssm_nats::topics::positions(&symbol);

    tracing::info!("waiting for signals");

    while let Some(signal) = rx.recv().await {
        if signal.action == AIAction::Neutral {
            continue;
        }

        // Use a placeholder price — in production, fetch from market data
        let current_price = Decimal::from(50_000);

        match engine.submit_signal(&signal, quantity, current_price) {
            Ok(order) => {
                tracing::info!(
                    order_id = %order.id,
                    side = %order.side,
                    status = ?order.status,
                    "order executed"
                );
                if let Err(e) = publisher.publish(&order_topic, &order).await {
                    tracing::warn!(error = %e, "failed to publish order");
                }

                // Publish position update
                if let Some(pos) = engine.positions().get(&symbol) {
                    if let Err(e) = publisher.publish(&position_topic, pos).await {
                        tracing::warn!(error = %e, "failed to publish position");
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, action = ?signal.action, "order submission failed");
            }
        }
    }

    Ok(())
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
