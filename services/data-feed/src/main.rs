use anyhow::{Context, Result};
use ssm_core::{env_or, interval_to_ms, DEFAULT_INTERVAL, DEFAULT_SYMBOL};
use ssm_exchange::aggregator::TradeAggregator;
use ssm_exchange::websocket::{BinanceWsClient, WsConfig, WsEvent};
use ssm_nats::Publisher;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    ssm_core::init_logging();

    let symbol = env_or("SYMBOL", DEFAULT_SYMBOL);
    let interval = env_or("INTERVAL", DEFAULT_INTERVAL);

    tracing::info!(%symbol, %interval, "data-feed service starting");

    // Connect to NATS
    let nats_client = ssm_nats::connect().await?;
    let publisher = Publisher::new(nats_client);

    // Create trade aggregator for building candles from raw trades
    let interval_ms = interval_to_ms(&interval);
    let mut aggregator = TradeAggregator::new(&symbol, interval_ms);

    // Start WebSocket feed
    let ws_config = WsConfig {
        symbol: symbol.to_lowercase(),
        kline_interval: interval.clone(),
        ..Default::default()
    };

    let (tx, mut rx) = mpsc::channel::<WsEvent>(10_000);

    let ws_client = BinanceWsClient::new(ws_config);

    // Spawn WebSocket reader
    let ws_handle = tokio::spawn(async move {
        if let Err(e) = ws_client.run(tx).await {
            tracing::error!(error = %e, "WebSocket client failed");
        }
    });

    // Process events
    let trade_topic = ssm_nats::topics::trades(&symbol);
    let candle_topic = ssm_nats::topics::candles(&symbol, &interval);
    let liq_topic = ssm_nats::topics::liquidations(&symbol);

    tracing::info!("processing WebSocket events");

    tokio::select! {
        _ = async {
            while let Some(event) = rx.recv().await {
                match event {
                    WsEvent::Trade(trade) => {
                        // Publish raw trade to NATS
                        if let Err(e) = publisher.publish(&trade_topic, &trade).await {
                            tracing::warn!(error = %e, "failed to publish trade");
                        }

                        // Feed into aggregator — may produce closed candle
                        if let Some(candle) = aggregator.ingest(&trade) {
                            tracing::info!(
                                open_time = candle.open_time,
                                trades = candle.trades,
                                "candle closed from aggregator"
                            );
                            if let Err(e) = publisher.publish(&candle_topic, &candle).await {
                                tracing::warn!(error = %e, "failed to publish aggregated candle");
                            }
                        }
                    }
                    WsEvent::Liquidation(liq) => {
                        tracing::debug!(
                            symbol = %liq.symbol,
                            side = %liq.side,
                            price = %liq.price,
                            "liquidation event"
                        );
                        if let Err(e) = publisher.publish(&liq_topic, &liq).await {
                            tracing::warn!(error = %e, "failed to publish liquidation");
                        }
                    }
                    WsEvent::Kline(candle) => {
                        // Kline events are closed candles from Binance directly
                        tracing::info!(
                            open_time = candle.open_time,
                            close = %candle.close,
                            "kline candle received"
                        );
                        let kline_topic = ssm_nats::topics::candles(&symbol, &format!("{interval}.kline"));
                        if let Err(e) = publisher.publish(&kline_topic, &candle).await {
                            tracing::warn!(error = %e, "failed to publish kline candle");
                        }
                    }
                }
            }
        } => {
            ws_handle.await.context("WebSocket task panicked")?;
        },
        _ = shutdown_signal() => {},
    }

    tracing::info!("data-feed shut down");
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl+c");
    tracing::info!("shutdown signal received, exiting gracefully");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_parsing() {
        assert_eq!(interval_to_ms("1m"), 60_000);
        assert_eq!(interval_to_ms("15m"), 900_000);
        assert_eq!(interval_to_ms("1h"), 3_600_000);
        assert_eq!(interval_to_ms("4h"), 14_400_000);
    }
}
