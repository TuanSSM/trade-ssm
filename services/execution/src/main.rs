use anyhow::{Context, Result};
use rust_decimal::Decimal;
use ssm_core::{
    env_or, AIAction, ExecutionMode, Signal, Trade, TradeRecord, DEFAULT_EXECUTION_MODE,
    DEFAULT_SYMBOL,
};
use ssm_execution::engine::ExecutionEngine;
use ssm_execution::risk::{RiskConfig, RiskManager, SizingMode};
use ssm_execution::TradeStore;
use ssm_nats::{Publisher, Subscriber};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

const DEFAULT_STORE_PATH: &str = "data/trade-ssm.db";

const DEFAULT_QUANTITY: &str = "0.001";
const DEFAULT_INITIAL_BALANCE: &str = "10000";
const MARK_TO_MARKET_INTERVAL_SECS: u64 = 10;

/// Parse `SizingMode` from environment variables.
fn sizing_mode_from_env() -> SizingMode {
    match env_or("SIZING_MODE", "fixed").as_str() {
        "kelly" => SizingMode::Kelly {
            fraction_multiplier: env_or("KELLY_FRACTION_MULTIPLIER", "0.5")
                .parse()
                .unwrap_or(Decimal::new(5, 1)),
            min_trades: env_or("KELLY_MIN_TRADES", "30").parse().unwrap_or(30),
            fallback_fraction: env_or("KELLY_FALLBACK_FRACTION", "0.02")
                .parse()
                .unwrap_or(Decimal::new(2, 2)),
            max_fraction: env_or("KELLY_MAX_FRACTION", "0.25")
                .parse()
                .unwrap_or(Decimal::new(25, 2)),
        },
        _ => SizingMode::Fixed {
            fraction: env_or("POSITION_SIZE_FRACTION", "0.02")
                .parse()
                .unwrap_or(Decimal::new(2, 2)),
        },
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    ssm_core::init_logging();

    let metrics_port: u16 = env_or("METRICS_PORT", "9090").parse().unwrap_or(9090);
    ssm_core::init_metrics(metrics_port);

    let symbol = env_or("SYMBOL", DEFAULT_SYMBOL);
    let mode = match env_or("EXECUTION_MODE", DEFAULT_EXECUTION_MODE).as_str() {
        "live" => ExecutionMode::Live,
        _ => ExecutionMode::Paper,
    };
    let default_quantity: Decimal = env_or("TRADE_QUANTITY", DEFAULT_QUANTITY)
        .parse()
        .context("parsing TRADE_QUANTITY")?;
    let initial_balance: Decimal = env_or("INITIAL_BALANCE", DEFAULT_INITIAL_BALANCE)
        .parse()
        .unwrap_or(Decimal::from(10_000));
    let sizing_mode = sizing_mode_from_env();

    tracing::info!(%symbol, ?mode, %default_quantity, ?sizing_mode, "execution service starting");

    let nats_client = ssm_nats::connect().await?;
    let publisher = Publisher::new(nats_client.clone());
    let subscriber = Subscriber::new(nats_client.clone());
    let price_subscriber = Subscriber::new(nats_client);

    // Open SQLite store for position persistence and trade recording
    let store_path = env_or("STORE_PATH", DEFAULT_STORE_PATH);
    let store = Arc::new(TradeStore::open(&store_path)?);
    tracing::info!(%store_path, "trade store opened");

    let mut engine_inner = ExecutionEngine::new(mode).with_store(Arc::clone(&store));

    // Recover positions from previous session
    if let Err(e) = engine_inner.recover_positions() {
        tracing::warn!(error = %e, "failed to recover positions from store");
    } else {
        let count = engine_inner.positions().all().len();
        if count > 0 {
            tracing::info!(count, "recovered open positions from store");
        }
    }

    let engine = Arc::new(Mutex::new(engine_inner));
    let risk_config = RiskConfig {
        sizing_mode: sizing_mode.clone(),
        ..Default::default()
    };
    let risk = Arc::new(Mutex::new(RiskManager::new(risk_config, initial_balance)));

    // Track completed trades for Kelly sizing and paper balance.
    let completed_trades: Arc<Mutex<Vec<TradeRecord>>> = Arc::new(Mutex::new(
        store.load_trades(None, None, None).unwrap_or_default(),
    ));
    let paper_balance = Arc::new(Mutex::new(initial_balance));

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
    let signal_topic_for_dlq = signal_topic.clone();
    let (sig_tx, mut sig_rx) = mpsc::channel::<Signal>(1_000);

    tokio::spawn(async move {
        if let Err(e) = subscriber.subscribe_typed(&signal_topic, sig_tx).await {
            tracing::error!(error = %e, "signal subscription failed");
        }
    });

    // Clone store for dead letter queue usage in signal processing loop
    let store_for_dlq = Arc::clone(&store);

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

    tokio::select! {
        _ = async {
    while let Some(signal) = sig_rx.recv().await {
        metrics::counter!("ssm_signals_received_total").increment(1);

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
        let quantity = {
            let risk_mgr = risk.lock().await;
            if risk_mgr.is_circuit_breaker_active() {
                tracing::warn!("circuit breaker active, rejecting all orders");
                continue;
            }

            // Compute position size via configured SizingMode
            let trades = completed_trades.lock().await;
            let balance = *paper_balance.lock().await;
            let notional = risk_mgr.position_size_for_mode(balance, &trades);
            if notional <= Decimal::ZERO || current_price <= Decimal::ZERO {
                tracing::info!("kelly sizing returned zero notional, skipping");
                continue;
            }
            notional / current_price
        };

        // Fall back to default_quantity if dynamic sizing yields too-small value
        let quantity = if quantity < Decimal::new(1, 8) {
            default_quantity
        } else {
            quantity
        };

        match eng.submit_signal(&signal, quantity, current_price) {
            Ok(order) => {
                let side_str = format!("{}", order.side);
                let status_str = format!("{:?}", order.status);
                metrics::counter!("ssm_orders_total", "side" => side_str, "status" => status_str).increment(1);

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

                // Publish position update and track open positions
                let open_count = eng.positions().all().len();
                metrics::gauge!("ssm_open_positions").set(open_count as f64);

                if let Some(pos) = eng.positions().get(&symbol) {
                    if let Err(e) = publisher.publish(&position_topic, pos).await {
                        tracing::warn!(error = %e, "failed to publish position");
                    }
                }
            }
            Err(e) => {
                metrics::counter!("ssm_orders_total", "side" => "unknown", "status" => "failed").increment(1);
                tracing::warn!(error = %e, action = ?signal.action, "order submission failed");
                let payload = serde_json::to_string(&signal).unwrap_or_default();
                if let Err(dlq_err) = store_for_dlq.save_dead_letter(
                    &signal_topic_for_dlq,
                    &payload,
                    &e.to_string(),
                    3,
                ) {
                    tracing::error!(error = %dlq_err, "failed to save to dead letter queue");
                }
            }
        }
    }
        } => {},
        _ = shutdown_signal() => {},
    }

    tracing::info!("execution service shut down");
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl+c");
    tracing::info!("shutdown signal received, exiting gracefully");
}
