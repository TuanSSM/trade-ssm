use anyhow::{Context, Result};
use async_nats::Client;
use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use tokio::sync::mpsc;

/// Typed subscriber that deserializes NATS messages into domain types.
pub struct Subscriber {
    client: Client,
}

impl Subscriber {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Subscribe to a subject and forward deserialized messages to an mpsc channel.
    ///
    /// Runs until the channel receiver is dropped or the subscription ends.
    pub async fn subscribe_typed<T: DeserializeOwned + Send + 'static>(
        &self,
        subject: &str,
        tx: mpsc::Sender<T>,
    ) -> Result<()> {
        let mut sub = self
            .client
            .subscribe(subject.to_string())
            .await
            .context("subscribing to NATS subject")?;

        while let Some(msg) = sub.next().await {
            let traceparent = msg
                .headers
                .as_ref()
                .and_then(|h| h.get("traceparent"))
                .map(|v| v.to_string());

            match serde_json::from_slice::<T>(&msg.payload) {
                Ok(value) => {
                    if let Some(tp) = &traceparent {
                        tracing::debug!(
                            traceparent = %tp,
                            subject = %msg.subject,
                            "received NATS message with trace context"
                        );
                    }
                    if tx.send(value).await.is_err() {
                        tracing::debug!("receiver dropped, ending subscription");
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        subject = %msg.subject,
                        "failed to deserialize NATS message"
                    );
                }
            }
        }

        Ok(())
    }

    /// Subscribe and return a raw async_nats subscriber for manual processing.
    pub async fn subscribe_raw(&self, subject: &str) -> Result<async_nats::Subscriber> {
        self.client
            .subscribe(subject.to_string())
            .await
            .context("subscribing to NATS subject")
    }
}
