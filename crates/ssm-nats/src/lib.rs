pub mod producer_consumer;
pub mod publisher;
pub mod subscriber;
pub mod topics;

use anyhow::{Context, Result};
use async_nats::Client;
use rand::Rng;
use std::time::Duration;

pub use publisher::Publisher;
pub use subscriber::Subscriber;

/// Base delay for exponential backoff (500ms).
const BASE_DELAY_MS: u64 = 500;

/// Maximum delay cap for exponential backoff (30s).
const MAX_DELAY_MS: u64 = 30_000;

/// Default number of retry attempts for `connect()`.
const DEFAULT_MAX_ATTEMPTS: u32 = 10;

/// Connect to a NATS server with retry logic (10 attempts by default).
///
/// Uses `NATS_URL` env var, defaulting to `nats://localhost:4222`.
/// If `NATS_USER` and `NATS_PASS` env vars are set, authenticates with
/// username/password credentials.
pub async fn connect() -> Result<Client> {
    let url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    connect_with_retry(&url, DEFAULT_MAX_ATTEMPTS).await
}

/// Connect to a NATS server with exponential backoff and jitter.
///
/// Retries up to `max_attempts` times with exponential backoff starting at
/// 500ms, capped at 30s, with random jitter. After exhausting all attempts,
/// returns the last connection error.
pub async fn connect_with_retry(url: &str, max_attempts: u32) -> Result<Client> {
    let mut last_err = None;

    for attempt in 1..=max_attempts {
        match connect_to(url).await {
            Ok(client) => return Ok(client),
            Err(e) => {
                last_err = Some(e);
                if attempt < max_attempts {
                    let base = BASE_DELAY_MS * 2u64.saturating_pow(attempt - 1);
                    let capped = base.min(MAX_DELAY_MS);
                    let jitter = rand::thread_rng().gen_range(0..=capped / 2);
                    let delay = Duration::from_millis(capped + jitter);

                    tracing::warn!(
                        attempt,
                        max_attempts,
                        delay_ms = delay.as_millis() as u64,
                        "NATS connection failed, retrying"
                    );
                    tokio::time::sleep(delay).await;
                } else {
                    tracing::error!(
                        attempt,
                        max_attempts,
                        "NATS connection failed, no more retries"
                    );
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("connect_with_retry called with 0 attempts")))
}

/// Connect to a specific NATS server URL with resilient connection options.
///
/// Configures the client with:
/// - Automatic retry on initial connect failure
/// - Increased client capacity (256) for message buffering during disconnects
/// - Event callback with tracing logs for connect/disconnect events
/// - Custom reconnect delay with exponential backoff and jitter
///
/// If `NATS_USER` and `NATS_PASS` env vars are set, authenticates with
/// username/password credentials. Otherwise connects without auth.
pub async fn connect_to(url: &str) -> Result<Client> {
    let user = std::env::var("NATS_USER").ok();
    let pass = std::env::var("NATS_PASS").ok();

    let opts = match (user, pass) {
        (Some(u), Some(p)) => async_nats::ConnectOptions::with_user_and_password(u, p),
        _ => async_nats::ConnectOptions::new(),
    };

    let url_owned = url.to_string();

    let client = opts
        .retry_on_initial_connect()
        .client_capacity(256)
        .event_callback(move |event| {
            let url = url_owned.clone();
            async move {
                match event {
                    async_nats::Event::Connected => {
                        tracing::info!(%url, "reconnected to NATS");
                    }
                    async_nats::Event::Disconnected => {
                        tracing::warn!(%url, "disconnected from NATS");
                    }
                    other => {
                        tracing::debug!(%url, event = %other, "NATS event");
                    }
                }
            }
        })
        .reconnect_delay_callback(|attempts| {
            let base = BASE_DELAY_MS * 2u64.saturating_pow(attempts as u32);
            let capped = base.min(MAX_DELAY_MS);
            let jitter = rand::thread_rng().gen_range(0..=capped / 2);
            Duration::from_millis(capped + jitter)
        })
        .connect(url)
        .await
        .with_context(|| format!("connecting to NATS at {url}"))?;

    tracing::info!(%url, "connected to NATS");
    Ok(client)
}

/// Create a publisher and subscriber pair from a shared connection.
///
/// Uses retry-enabled connect with 10 attempts by default.
pub async fn create_pair() -> Result<(Publisher, Subscriber)> {
    let client = connect().await?;
    Ok((Publisher::new(client.clone()), Subscriber::new(client)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_format_correctly() {
        assert_eq!(topics::trades("BTCUSDT"), "ssm.trades.btcusdt");
        assert_eq!(topics::candles("BTCUSDT", "15m"), "ssm.candles.btcusdt.15m");
    }

    #[test]
    fn all_topic_functions_accessible_from_lib() {
        // Verify all topic functions are reachable through the public module
        let _ = topics::trades("BTCUSDT");
        let _ = topics::candles("BTCUSDT", "1h");
        let _ = topics::liquidations("BTCUSDT");
        let _ = topics::signals("BTCUSDT");
        let _ = topics::orders("BTCUSDT");
        let _ = topics::positions("BTCUSDT");
        let _ = topics::metrics("analyzer");
    }

    #[test]
    fn topics_return_consistent_prefix() {
        // All topics should start with "ssm."
        assert!(topics::trades("X").starts_with("ssm."));
        assert!(topics::candles("X", "1m").starts_with("ssm."));
        assert!(topics::liquidations("X").starts_with("ssm."));
        assert!(topics::signals("X").starts_with("ssm."));
        assert!(topics::orders("X").starts_with("ssm."));
        assert!(topics::positions("X").starts_with("ssm."));
        assert!(topics::metrics("x").starts_with("ssm."));
    }

    #[test]
    fn publisher_and_subscriber_types_exported() {
        // Verify that Publisher and Subscriber are re-exported from lib
        fn _assert_pub_export(_: Publisher) {}
        fn _assert_sub_export(_: Subscriber) {}
    }
}
