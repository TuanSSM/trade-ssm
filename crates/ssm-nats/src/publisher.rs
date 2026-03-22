use anyhow::{Context, Result};
use async_nats::Client;
use serde::Serialize;

/// Typed publisher that serializes domain types to NATS subjects.
#[derive(Clone)]
pub struct Publisher {
    client: Client,
}

impl Publisher {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Publish a serializable message to a NATS subject.
    pub async fn publish<T: Serialize>(&self, subject: &str, payload: &T) -> Result<()> {
        let bytes = serde_json::to_vec(payload).context("serializing NATS payload")?;
        self.client
            .publish(subject.to_string(), bytes.into())
            .await
            .context("publishing to NATS")?;
        Ok(())
    }

    /// Flush pending messages.
    pub async fn flush(&self) -> Result<()> {
        self.client.flush().await.context("flushing NATS")?;
        Ok(())
    }
}
