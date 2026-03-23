pub mod publisher;
pub mod subscriber;
pub mod topics;

use anyhow::{Context, Result};
use async_nats::Client;

pub use publisher::Publisher;
pub use subscriber::Subscriber;

/// Connect to a NATS server and return a client.
///
/// Uses `NATS_URL` env var, defaulting to `nats://localhost:4222`.
pub async fn connect() -> Result<Client> {
    let url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    connect_to(&url).await
}

/// Connect to a specific NATS server URL.
pub async fn connect_to(url: &str) -> Result<Client> {
    let client = async_nats::connect(url)
        .await
        .with_context(|| format!("connecting to NATS at {url}"))?;
    tracing::info!(%url, "connected to NATS");
    Ok(client)
}

/// Create a publisher and subscriber pair from a shared connection.
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
}
