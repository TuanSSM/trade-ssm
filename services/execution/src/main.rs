use anyhow::{Context, Result};
use rust_decimal::Decimal;
use ssm_core::{env_or, AIAction, ExecutionMode, Signal, Trade, DEFAULT_EXECUTION_MODE, DEFAULT_SYMBOL};
use ssm_execution::engine::ExecutionEngine;
use ssm_execution::risk::{RiskConfig, RiskManager};
use ssm_nats::{Publisher, Subscriber};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

const DEFAULT_QUANTITY: &str = "0.001";
const MARK_TO_MARKET_INTERVAL_SECS: u64 = 10;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let symbol = env_or("SYMBOL", DEFAULT_SYMBOL);
    let mode = match env_or("EXECUTION_MODE", DEFAULT_EXECUTION_MODE).as_str() {
        "live" => ExecutionMode::Live,
        _ => ExecutionMode::Paper,
    };
    let quantity: Decimal = env_or("TRADE_QUANTITY", DEFAULT_QUANTITY)
        .parse()
        .context("parsing TRADE_QUANTITY")?;

    tracing::info!(%symbol, ?mode, %quantity, "execution service starting");

    let nats_client = ssm_nats::connect().await?;
    let publisher = Publisher::new(nats_client.clone());
    let subscriber = Subscriber::new(nats_client.clone());
    let price_subscriber = Subscriber::new(nats_client);

    let engine = Arc::new(Mutex::new(ExecutionEngine::new(mode)));
    let risk = Arc::new(Mutex::new(RiskManager::new(
        RiskConfig::default(),
        Decimal::from(10_000),
    )));

    // Track latest price from trade feed
    let latest_price = Arc::new(Mutex::new(None::<Decimal>));

    // Subscribe to trade feed for real-time price
    let trade_topic = ssm_nats::topics::trades(&symbol);
    let (price_tx, mut price_rx) = mpsc::channel::<Trade>(10_000);
    let latest_price_writer = Arc::clone(&latest_price);

    tokio::spawn(async move {
        if let Err(e) = price_subscriber
            .subscribe_typed(&trade_topic, price_tx)
            .await
        {
            tracing::error!(error = %e, "trade price subscription failed");
        }
    });

    // Price update task
    tokio::spawn(async move {
        while let Some(trade) = price_rx.recv().await {
            *latest_price_writer.lock().await = Some(trade.price);
        }
    });

    // Subscribe to signals
    let signal_topic = ssm_nats::topics::signals(&symbol);
    let (sig_tx, mut sig_rx) = mpsc::channel::<Signal>(1_000);

    tokio::spawn(async move {
        if let Err(e) = subscriber.subscribe_typed(&signal_topic, sig_tx).await {
            tracing::error!(error = %e, "signal subscription failed");
        }
    });

    let order_topic = ssm_nats::topics::orders(&symbol);
    let position_topic = ssm_nats::topics::positions(&symbol);

    // Mark-to-market task
    let mtm_engine = Arc::clone(&engine);
    let mtm_risk = Arc::clone(&risk);
    let mtm_price = Arc::clone(&latest_price);
    let mtm_publisher = Publisher::new(ssm_nats::connect().await?);
    let mtm_position_topic = ssm_nats::topics::positions(&symbol);
    let mtm_symbol = symbol.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(
            MARK_TO_MARKET_INTERVAL_SECS,
        ));
        loop {
            interval.tick().await;
            let price = *mtm_price.lock().await;
            if let Some(p) = price {
                let mut eng = mtm_engine.lock().await;
                let mut prices = HashMap::new();
                prices.insert(mtm_symbol.clone(), p);
                eng.positions_mut().mark_to_market(&prices);

                // Update risk manager with current equity
                if let Some(pos) = eng.positions().get(&mtm_symbol) {
                    let equity = pos.unrealized_pnl + pos.realized_pnl;
                    mtm_risk.lock().await.update_equity(equity);
                }

                // Publish position updates
                if let Some(pos) = eng.positions().get(&mtm_symbol) {
                    if let Err(e) = mtm_publisher.publish(&mtm_position_topic, pos).await {
                        tracing::warn!(error = %e, "failed to publish position update");
                    }
                }
            }
        }
    });

    tracing::info!("waiting for signals");

    while let Some(signal) = sig_rx.recv().await {
        if signal.action == AIAction::Neutral {
            continue;
        }

        // Get current market price
        let current_price = match *latest_price.lock().await {
            Some(p) => p,
            None => {
                tracing::warn!("no market price available yet, skipping signal");
                continue;
            }
        };

        let mut eng = engine.lock().await;

        // Risk check before execution
        {
            let risk_mgr = risk.lock().await;
            if risk_mgr.is_circuit_breaker_active() {
                tracing::warn!("circuit breaker active, rejecting all orders");
                continue;
            }
        }

        match eng.submit_signal(&signal, quantity, current_price) {
            Ok(order) => {
                tracing::info!(
                    order_id = %order.id,
                    side = %order.side,
                    status = ?order.status,
                    price = %current_price,
                    "order executed"
                );
                if let Err(e) = publisher.publish(&order_topic, &order).await {
                    tracing::warn!(error = %e, "failed to publish order");
                }

                // Publish position update
                if let Some(pos) = eng.positions().get(&symbol) {
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
