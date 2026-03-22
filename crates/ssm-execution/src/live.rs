use anyhow::{Context, Result};
use rust_decimal::Decimal;
use ssm_core::{Order, OrderStatus};

/// Live execution engine for Binance Futures.
///
/// Uses signed API (HMAC-SHA256) for order placement.
/// Requires `BINANCE_API_KEY` and `BINANCE_SECRET_KEY` environment variables.
pub struct LiveEngine {
    api_key: String,
    secret_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl LiveEngine {
    /// Create from environment variables.
    pub fn from_env() -> Result<Self> {
        let api_key =
            std::env::var("BINANCE_API_KEY").context("BINANCE_API_KEY env var required")?;
        let secret_key =
            std::env::var("BINANCE_SECRET_KEY").context("BINANCE_SECRET_KEY env var required")?;

        Ok(Self {
            api_key,
            secret_key,
            base_url: "https://fapi.binance.com".to_string(),
            client: reqwest::Client::new(),
        })
    }

    /// Create with explicit credentials (for testing).
    pub fn new(api_key: String, secret_key: String) -> Self {
        Self {
            api_key,
            secret_key,
            base_url: "https://fapi.binance.com".to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Submit an order to Binance Futures.
    pub async fn submit_order(&self, order: &mut Order, _current_price: Decimal) -> Result<()> {
        let timestamp = chrono::Utc::now().timestamp_millis();

        let mut params = vec![
            ("symbol", order.symbol.clone()),
            ("side", order.side.to_string()),
            ("type", order.order_type.to_string()),
            ("quantity", order.quantity.to_string()),
            ("timestamp", timestamp.to_string()),
        ];

        if let Some(price) = &order.price {
            params.push(("price", price.to_string()));
            params.push(("timeInForce", "GTC".to_string()));
        }

        if let Some(stop_price) = &order.stop_price {
            params.push(("stopPrice", stop_price.to_string()));
        }

        if order.reduce_only {
            params.push(("reduceOnly", "true".to_string()));
        }

        // Create query string for signing
        let query_string: String = params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");

        let signature = self.sign(&query_string);
        let url = format!("{}/fapi/v1/order", self.base_url);

        let resp = self
            .client
            .post(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .query(&params)
            .query(&[("signature", &signature)])
            .send()
            .await
            .context("submitting order to Binance")?;

        let status = resp.status();
        if status.is_success() {
            order.status = OrderStatus::Open;
            order.updated_at = timestamp;
            tracing::info!(order_id = %order.id, "order submitted to Binance");
        } else {
            let body = resp.text().await.unwrap_or_default();
            order.status = OrderStatus::Rejected;
            order.updated_at = timestamp;
            anyhow::bail!("Binance order rejected: {status} {body}");
        }

        Ok(())
    }

    /// Cancel an order on Binance.
    pub async fn cancel_order(&self, symbol: &str, order_id: &str) -> Result<()> {
        let timestamp = chrono::Utc::now().timestamp_millis();
        let params = [
            ("symbol", symbol.to_string()),
            ("origClientOrderId", order_id.to_string()),
            ("timestamp", timestamp.to_string()),
        ];

        let query_string: String = params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");

        let signature = self.sign(&query_string);
        let url = format!("{}/fapi/v1/order", self.base_url);

        let resp = self
            .client
            .delete(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .query(&params)
            .query(&[("signature", &signature)])
            .send()
            .await
            .context("cancelling order on Binance")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Binance cancel failed: {body}");
        }

        tracing::info!(order_id, symbol, "order cancelled on Binance");
        Ok(())
    }

    /// HMAC-SHA256 signature for Binance API.
    fn sign(&self, message: &str) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        type HmacSha256 = Hmac<Sha256>;

        let mut mac =
            HmacSha256::new_from_slice(self.secret_key.as_bytes()).expect("HMAC key length");
        mac.update(message.as_bytes());
        let result = mac.finalize();
        hex::encode(result.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_produces_hex() {
        let engine = LiveEngine::new("api_key".into(), "secret_key".into());
        let sig = engine.sign("test_message");
        // Should be 64-char hex string (SHA256 = 32 bytes)
        assert_eq!(sig.len(), 64);
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
