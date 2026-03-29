use anyhow::{Context, Result};
use rust_decimal::Decimal;
use ssm_core::{Order, OrderStatus};
use std::time::Duration;

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

/// Binance Futures base URLs.
const BINANCE_MAINNET: &str = "https://fapi.binance.com";
const BINANCE_TESTNET: &str = "https://testnet.binancefuture.com";

/// Default retry configuration.
const DEFAULT_MAX_RETRIES: u32 = 3;
const DEFAULT_BASE_DELAY_MS: u64 = 500;

/// Live execution engine for Binance Futures.
///
/// Uses signed API (HMAC-SHA256) for order placement.
/// Requires `BINANCE_API_KEY` and `BINANCE_SECRET_KEY` environment variables.
pub struct LiveEngine {
    api_key: String,
    secret_key: String,
    base_url: String,
    client: reqwest::Client,
    max_retries: u32,
    base_delay: Duration,
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
            base_url: BINANCE_MAINNET.to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            max_retries: DEFAULT_MAX_RETRIES,
            base_delay: Duration::from_millis(DEFAULT_BASE_DELAY_MS),
        })
    }

    /// Create with explicit credentials (for testing).
    pub fn new(api_key: String, secret_key: String) -> Self {
        Self {
            api_key,
            secret_key,
            base_url: BINANCE_MAINNET.to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            max_retries: DEFAULT_MAX_RETRIES,
            base_delay: Duration::from_millis(DEFAULT_BASE_DELAY_MS),
        }
    }

    /// Create with testnet base URL.
    pub fn with_testnet(api_key: String, secret_key: String) -> Self {
        Self {
            api_key,
            secret_key,
            base_url: BINANCE_TESTNET.to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            max_retries: DEFAULT_MAX_RETRIES,
            base_delay: Duration::from_millis(DEFAULT_BASE_DELAY_MS),
        }
    }

    /// Create from environment variables using the testnet endpoint.
    pub fn from_env_testnet() -> Result<Self> {
        let api_key =
            std::env::var("BINANCE_API_KEY").context("BINANCE_API_KEY env var required")?;
        let secret_key =
            std::env::var("BINANCE_SECRET_KEY").context("BINANCE_SECRET_KEY env var required")?;

        Ok(Self {
            api_key,
            secret_key,
            base_url: BINANCE_TESTNET.to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            max_retries: DEFAULT_MAX_RETRIES,
            base_delay: Duration::from_millis(DEFAULT_BASE_DELAY_MS),
        })
    }

    /// Get the configured base URL (useful for testing).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Submit an order to Binance Futures with retry logic.
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

        // Add client order ID for tracking
        params.push(("newClientOrderId", order.id.clone()));

        let query_string: String = params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");

        let signature = self.sign(&query_string)?;
        let url = format!("{}/fapi/v1/order", self.base_url);

        let resp = self
            .retry(|| async {
                self.client
                    .post(&url)
                    .header("X-MBX-APIKEY", &self.api_key)
                    .query(&params)
                    .query(&[("signature", &signature)])
                    .send()
                    .await
                    .context("submitting order to Binance")
            })
            .await?;

        let status = resp.status();
        if status.is_success() {
            let body: serde_json::Value = resp.json().await.context("parsing order response")?;

            // Handle partial fills from the response
            let filled_qty = body["executedQty"]
                .as_str()
                .and_then(|s| s.parse::<Decimal>().ok())
                .unwrap_or(Decimal::ZERO);

            let order_status = body["status"].as_str().unwrap_or("NEW");
            order.status = match order_status {
                "FILLED" => OrderStatus::Filled,
                "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
                "NEW" => OrderStatus::Open,
                "CANCELED" => OrderStatus::Cancelled,
                "REJECTED" => OrderStatus::Rejected,
                "EXPIRED" => OrderStatus::Expired,
                _ => OrderStatus::Open,
            };

            // Update the fill price from avg_price if available
            if let Some(avg_price) = body["avgPrice"]
                .as_str()
                .and_then(|s| s.parse::<Decimal>().ok())
            {
                if avg_price > Decimal::ZERO {
                    order.price = Some(avg_price);
                }
            }

            order.updated_at = timestamp;

            tracing::info!(
                order_id = %order.id,
                exchange_status = order_status,
                filled_qty = %filled_qty,
                total_qty = %order.quantity,
                "order submitted to Binance"
            );
        } else {
            let body = resp.text().await.unwrap_or_default();
            order.status = OrderStatus::Rejected;
            order.updated_at = timestamp;
            anyhow::bail!("Binance order rejected: {status} {body}");
        }

        Ok(())
    }

    /// Cancel an order on Binance with retry logic.
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

        let signature = self.sign(&query_string)?;
        let url = format!("{}/fapi/v1/order", self.base_url);

        let resp = self
            .retry(|| async {
                self.client
                    .delete(&url)
                    .header("X-MBX-APIKEY", &self.api_key)
                    .query(&params)
                    .query(&[("signature", &signature)])
                    .send()
                    .await
                    .context("cancelling order on Binance")
            })
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Binance cancel failed: {body}");
        }

        tracing::info!(order_id, symbol, "order cancelled on Binance");
        Ok(())
    }

    /// Query order status from Binance with retry logic.
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

        let signature = self.sign(&query_string)?;
        let url = format!("{}/fapi/v1/order", self.base_url);

        let resp = self
            .retry(|| async {
                self.client
                    .get(&url)
                    .header("X-MBX-APIKEY", &self.api_key)
                    .query(&params)
                    .query(&[("signature", &signature)])
                    .send()
                    .await
                    .context("querying order on Binance")
            })
            .await?;

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

    /// Query order details including filled quantity from Binance.
    pub async fn query_order_detail(
        &self,
        symbol: &str,
        order_id: &str,
    ) -> Result<(OrderStatus, Decimal)> {
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

        let signature = self.sign(&query_string)?;
        let url = format!("{}/fapi/v1/order", self.base_url);

        let resp = self
            .retry(|| async {
                self.client
                    .get(&url)
                    .header("X-MBX-APIKEY", &self.api_key)
                    .query(&params)
                    .query(&[("signature", &signature)])
                    .send()
                    .await
                    .context("querying order detail on Binance")
            })
            .await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Binance order detail query failed: {body}");
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

        let filled_qty = body["executedQty"]
            .as_str()
            .and_then(|s| s.parse::<Decimal>().ok())
            .unwrap_or(Decimal::ZERO);

        tracing::debug!(
            order_id,
            symbol,
            ?status,
            filled_qty = %filled_qty,
            "order detail queried"
        );
        Ok((status, filled_qty))
    }

    /// Fetch account balance from Binance Futures with retry logic.
    pub async fn fetch_balance(&self) -> Result<Vec<BalanceInfo>> {
        let timestamp = chrono::Utc::now().timestamp_millis();
        let query_string = format!("timestamp={timestamp}");
        let signature = self.sign(&query_string)?;
        let url = format!("{}/fapi/v2/balance", self.base_url);

        let resp = self
            .retry(|| async {
                self.client
                    .get(&url)
                    .header("X-MBX-APIKEY", &self.api_key)
                    .query(&[("timestamp", &timestamp.to_string())])
                    .query(&[("signature", &signature)])
                    .send()
                    .await
                    .context("fetching balance from Binance")
            })
            .await?;

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

    /// Fetch open positions from Binance Futures with retry logic.
    pub async fn fetch_positions(&self) -> Result<Vec<PositionInfo>> {
        let timestamp = chrono::Utc::now().timestamp_millis();
        let query_string = format!("timestamp={timestamp}");
        let signature = self.sign(&query_string)?;
        let url = format!("{}/fapi/v2/positionRisk", self.base_url);

        let resp = self
            .retry(|| async {
                self.client
                    .get(&url)
                    .header("X-MBX-APIKEY", &self.api_key)
                    .query(&[("timestamp", &timestamp.to_string())])
                    .query(&[("signature", &signature)])
                    .send()
                    .await
                    .context("fetching positions from Binance")
            })
            .await?;

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

    /// Retry an async operation with exponential backoff.
    ///
    /// Retries on transient network errors. Does not retry on successful HTTP responses
    /// (even if the status code indicates an error — that is handled by the caller).
    async fn retry<F, Fut, T>(&self, op: F) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut last_err = None;
        for attempt in 0..=self.max_retries {
            match op().await {
                Ok(val) => return Ok(val),
                Err(e) => {
                    last_err = Some(e);
                    if attempt < self.max_retries {
                        let base = self.base_delay * 2u32.pow(attempt);
                        // Add jitter: 75%-125% of base delay
                        let jitter_pct = ((attempt as u64 * 37 + 13) % 51) + 75; // deterministic pseudo-jitter
                        let delay = base * jitter_pct as u32 / 100;
                        tracing::warn!(
                            attempt = attempt + 1,
                            max = self.max_retries,
                            delay_ms = delay.as_millis(),
                            "exchange API call failed, retrying"
                        );
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }
        Err(last_err.unwrap())
    }

    /// HMAC-SHA256 signature for Binance API.
    fn sign(&self, message: &str) -> Result<String> {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        type HmacSha256 = Hmac<Sha256>;

        let mut mac = HmacSha256::new_from_slice(self.secret_key.as_bytes())
            .map_err(|e| crate::error::ExecutionError::SigningError(e.to_string()))?;
        mac.update(message.as_bytes());
        let result = mac.finalize();
        Ok(hex::encode(result.into_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_produces_hex() {
        let engine = LiveEngine::new("api_key".into(), "secret_key".into());
        let sig = engine.sign("test_message").unwrap();
        // Should be 64-char hex string (SHA256 = 32 bytes)
        assert_eq!(sig.len(), 64);
        assert!(sig.chars().all(|c: char| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_sign_deterministic() {
        let engine = LiveEngine::new("api_key".into(), "secret_key".into());
        let sig1 = engine.sign("same_message").unwrap();
        let sig2 = engine.sign("same_message").unwrap();
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_sign_different_keys() {
        let engine1 = LiveEngine::new("api_key".into(), "secret_one".into());
        let engine2 = LiveEngine::new("api_key".into(), "secret_two".into());
        let sig1 = engine1.sign("test_message").unwrap();
        let sig2 = engine2.sign("test_message").unwrap();
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn test_sign_different_messages() {
        let engine = LiveEngine::new("api_key".into(), "secret_key".into());
        let sig1 = engine.sign("message_one").unwrap();
        let sig2 = engine.sign("message_two").unwrap();
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn test_sign_empty_message() {
        let engine = LiveEngine::new("api_key".into(), "secret_key".into());
        let sig = engine.sign("").unwrap();
        assert_eq!(sig.len(), 64);
        assert!(sig.chars().all(|c: char| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_sign_long_message() {
        let engine = LiveEngine::new("api_key".into(), "secret_key".into());
        let long_msg = "a".repeat(10_000);
        let sig = engine.sign(&long_msg).unwrap();
        assert_eq!(sig.len(), 64);
        assert!(sig.chars().all(|c: char| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_sign_special_characters() {
        let engine = LiveEngine::new("api_key".into(), "secret_key".into());
        let sig = engine
            .sign("symbol=BTCUSDT&side=BUY&type=MARKET&quantity=1&timestamp=1234567890")
            .unwrap();
        assert_eq!(sig.len(), 64);
        assert!(sig.chars().all(|c: char| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_live_engine_new_stores_credentials() {
        let engine = LiveEngine::new("my_api_key".into(), "my_secret".into());
        // Verify the engine was created (we can only test sign since fields are private)
        let sig = engine.sign("test").unwrap();
        assert_eq!(sig.len(), 64);
    }

    #[test]
    fn test_with_testnet_uses_testnet_url() {
        let engine = LiveEngine::with_testnet("key".into(), "secret".into());
        assert_eq!(engine.base_url(), BINANCE_TESTNET);
    }

    #[test]
    fn test_new_uses_mainnet_url() {
        let engine = LiveEngine::new("key".into(), "secret".into());
        assert_eq!(engine.base_url(), BINANCE_MAINNET);
    }

    #[test]
    fn test_testnet_and_mainnet_urls_differ() {
        let mainnet = LiveEngine::new("key".into(), "secret".into());
        let testnet = LiveEngine::with_testnet("key".into(), "secret".into());
        assert_ne!(mainnet.base_url(), testnet.base_url());
    }

    #[test]
    fn test_retry_config_defaults() {
        let engine = LiveEngine::new("key".into(), "secret".into());
        assert_eq!(engine.max_retries, DEFAULT_MAX_RETRIES);
        assert_eq!(
            engine.base_delay,
            Duration::from_millis(DEFAULT_BASE_DELAY_MS)
        );
    }

    #[tokio::test]
    async fn test_retry_succeeds_first_attempt() {
        let engine = LiveEngine::new("key".into(), "secret".into());
        let result = engine
            .retry(|| async { Ok::<i32, anyhow::Error>(42) })
            .await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_retry_fails_after_max_retries() {
        let engine = LiveEngine {
            api_key: "key".into(),
            secret_key: "secret".into(),
            base_url: BINANCE_MAINNET.into(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            max_retries: 1,
            base_delay: Duration::from_millis(1), // fast for testing
        };

        let attempt_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count = attempt_count.clone();

        let result: Result<i32> = engine
            .retry(|| {
                let c = count.clone();
                async move {
                    c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    anyhow::bail!("always fails")
                }
            })
            .await;

        assert!(result.is_err());
        // 1 initial + 1 retry = 2 attempts
        assert_eq!(attempt_count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_retry_succeeds_on_second_attempt() {
        let engine = LiveEngine {
            api_key: "key".into(),
            secret_key: "secret".into(),
            base_url: BINANCE_MAINNET.into(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            max_retries: 3,
            base_delay: Duration::from_millis(1),
        };

        let attempt_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count = attempt_count.clone();

        let result: Result<i32> = engine
            .retry(|| {
                let c = count.clone();
                async move {
                    let n = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if n == 0 {
                        anyhow::bail!("first attempt fails")
                    }
                    Ok(42)
                }
            })
            .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempt_count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
