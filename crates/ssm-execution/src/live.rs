use anyhow::{Context, Result};
use rust_decimal::Decimal;
use ssm_core::{Order, OrderStatus};

/// Account balance information from Binance.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BalanceInfo {
    pub asset: String,
    pub balance: Decimal,
    pub available: Decimal,
}

/// Open position information from Binance.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PositionInfo {
    pub symbol: String,
    pub amount: Decimal,
    pub entry_price: Decimal,
    pub unrealized_pnl: Decimal,
    pub leverage: u32,
}

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

    /// Query order status from Binance.
    pub async fn query_order(&self, symbol: &str, order_id: &str) -> Result<OrderStatus> {
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
            .get(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .query(&params)
            .query(&[("signature", &signature)])
            .send()
            .await
            .context("querying order on Binance")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Binance order query failed: {body}");
        }

        let body: serde_json::Value = resp.json().await.context("parsing order response")?;
        let status_str = body["status"].as_str().unwrap_or("UNKNOWN");
        let status = match status_str {
            "NEW" => OrderStatus::Open,
            "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
            "FILLED" => OrderStatus::Filled,
            "CANCELED" => OrderStatus::Cancelled,
            "REJECTED" => OrderStatus::Rejected,
            "EXPIRED" => OrderStatus::Expired,
            _ => OrderStatus::Pending,
        };

        tracing::debug!(order_id, symbol, ?status, "order status queried");
        Ok(status)
    }

    /// Fetch account balance from Binance Futures.
    pub async fn fetch_balance(&self) -> Result<Vec<BalanceInfo>> {
        let timestamp = chrono::Utc::now().timestamp_millis();
        let query_string = format!("timestamp={timestamp}");
        let signature = self.sign(&query_string);
        let url = format!("{}/fapi/v2/balance", self.base_url);

        let resp = self
            .client
            .get(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .query(&[("timestamp", &timestamp.to_string())])
            .query(&[("signature", &signature)])
            .send()
            .await
            .context("fetching balance from Binance")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Binance balance query failed: {body}");
        }

        let balances: Vec<serde_json::Value> =
            resp.json().await.context("parsing balance response")?;

        let result: Vec<BalanceInfo> = balances
            .iter()
            .filter_map(|b| {
                let asset = b["asset"].as_str()?;
                let balance: Decimal = b["balance"].as_str()?.parse().ok()?;
                let available: Decimal = b["availableBalance"].as_str()?.parse().ok()?;
                if balance > Decimal::ZERO {
                    Some(BalanceInfo {
                        asset: asset.to_string(),
                        balance,
                        available,
                    })
                } else {
                    None
                }
            })
            .collect();

        tracing::info!(assets = result.len(), "balance fetched from Binance");
        Ok(result)
    }

    /// Fetch open positions from Binance Futures.
    pub async fn fetch_positions(&self) -> Result<Vec<PositionInfo>> {
        let timestamp = chrono::Utc::now().timestamp_millis();
        let query_string = format!("timestamp={timestamp}");
        let signature = self.sign(&query_string);
        let url = format!("{}/fapi/v2/positionRisk", self.base_url);

        let resp = self
            .client
            .get(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .query(&[("timestamp", &timestamp.to_string())])
            .query(&[("signature", &signature)])
            .send()
            .await
            .context("fetching positions from Binance")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Binance position query failed: {body}");
        }

        let positions: Vec<serde_json::Value> =
            resp.json().await.context("parsing position response")?;

        let result: Vec<PositionInfo> = positions
            .iter()
            .filter_map(|p| {
                let symbol = p["symbol"].as_str()?;
                let amount: Decimal = p["positionAmt"].as_str()?.parse().ok()?;
                if amount == Decimal::ZERO {
                    return None;
                }
                let entry_price: Decimal = p["entryPrice"].as_str()?.parse().ok()?;
                let unrealized_pnl: Decimal = p["unRealizedProfit"].as_str()?.parse().ok()?;
                let leverage: u32 = p["leverage"].as_str()?.parse().ok()?;
                Some(PositionInfo {
                    symbol: symbol.to_string(),
                    amount,
                    entry_price,
                    unrealized_pnl,
                    leverage,
                })
            })
            .collect();

        tracing::info!(positions = result.len(), "positions fetched from Binance");
        Ok(result)
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

    #[test]
    fn test_sign_deterministic() {
        let engine = LiveEngine::new("api_key".into(), "secret_key".into());
        let sig1 = engine.sign("same_message");
        let sig2 = engine.sign("same_message");
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_sign_different_keys() {
        let engine1 = LiveEngine::new("api_key".into(), "secret_one".into());
        let engine2 = LiveEngine::new("api_key".into(), "secret_two".into());
        let sig1 = engine1.sign("test_message");
        let sig2 = engine2.sign("test_message");
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn test_sign_different_messages() {
        let engine = LiveEngine::new("api_key".into(), "secret_key".into());
        let sig1 = engine.sign("message_one");
        let sig2 = engine.sign("message_two");
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn test_sign_empty_message() {
        let engine = LiveEngine::new("api_key".into(), "secret_key".into());
        let sig = engine.sign("");
        assert_eq!(sig.len(), 64);
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_sign_long_message() {
        let engine = LiveEngine::new("api_key".into(), "secret_key".into());
        let long_msg = "a".repeat(10_000);
        let sig = engine.sign(&long_msg);
        assert_eq!(sig.len(), 64);
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_sign_special_characters() {
        let engine = LiveEngine::new("api_key".into(), "secret_key".into());
        let sig =
            engine.sign("symbol=BTCUSDT&side=BUY&type=MARKET&quantity=1&timestamp=1234567890");
        assert_eq!(sig.len(), 64);
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_live_engine_new_stores_credentials() {
        let engine = LiveEngine::new("my_api_key".into(), "my_secret".into());
        // Verify the engine was created (we can only test sign since fields are private)
        let sig = engine.sign("test");
        assert_eq!(sig.len(), 64);
    }
}
