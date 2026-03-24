use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    routing::{get, post},
    Json, Router,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};

// ---------------------------------------------------------------------------
// Response / request types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HealthResponse {
    healthy: bool,
    version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StatusResponse {
    status: String,
    uptime_seconds: u64,
    symbol: String,
    mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TradeResponse {
    trades: Vec<TradeInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TradeInfo {
    symbol: String,
    side: String,
    entry_price: String,
    quantity: String,
    unrealized_pnl: String,
    opened_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProfitResponse {
    total_realized_pnl: String,
    total_unrealized_pnl: String,
    trade_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BalanceResponse {
    balance: String,
    available: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MessageResponse {
    message: String,
}

#[derive(Debug, Deserialize)]
struct ForceExitRequest {
    symbol: String,
}

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

struct AppState {
    start_time: std::time::Instant,
    running: bool,
    symbol: String,
    mode: String,
    positions: HashMap<String, TradeInfo>,
    realized_pnl: Decimal,
    balance: Decimal,
}

impl AppState {
    fn new() -> Self {
        let symbol = std::env::var("SYMBOL").unwrap_or_else(|_| "BTCUSDT".to_string());
        let mode = std::env::var("EXECUTION_MODE").unwrap_or_else(|_| "paper".to_string());
        let balance_str = std::env::var("INITIAL_BALANCE").unwrap_or_else(|_| "10000".to_string());
        let balance = balance_str
            .parse::<Decimal>()
            .unwrap_or_else(|_| Decimal::new(10000, 0));

        Self {
            start_time: std::time::Instant::now(),
            running: true,
            symbol,
            mode,
            positions: HashMap::new(),
            realized_pnl: Decimal::ZERO,
            balance,
        }
    }
}

type SharedState = Arc<RwLock<AppState>>;

// ---------------------------------------------------------------------------
// Auth middleware
// ---------------------------------------------------------------------------

async fn api_key_auth(
    State(expected_key): State<Option<String>>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Result<axum::response::Response, StatusCode> {
    // If no API_KEY is configured, skip auth
    let Some(expected) = expected_key.as_deref() else {
        return Ok(next.run(request).await);
    };

    match headers.get("X-API-Key").and_then(|v| v.to_str().ok()) {
        Some(provided) if provided == expected => Ok(next.run(request).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        healthy: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

async fn get_status(State(state): State<SharedState>) -> Json<StatusResponse> {
    let s = state.read().await;
    Json(StatusResponse {
        status: if s.running {
            "running".to_string()
        } else {
            "stopped".to_string()
        },
        uptime_seconds: s.start_time.elapsed().as_secs(),
        symbol: s.symbol.clone(),
        mode: s.mode.clone(),
    })
}

async fn get_trades(State(state): State<SharedState>) -> Json<TradeResponse> {
    let s = state.read().await;
    Json(TradeResponse {
        trades: s.positions.values().cloned().collect(),
    })
}

async fn get_profit(State(state): State<SharedState>) -> Json<ProfitResponse> {
    let s = state.read().await;
    let unrealized: Decimal = s
        .positions
        .values()
        .filter_map(|t| t.unrealized_pnl.parse::<Decimal>().ok())
        .sum();
    Json(ProfitResponse {
        total_realized_pnl: s.realized_pnl.to_string(),
        total_unrealized_pnl: unrealized.to_string(),
        trade_count: s.positions.len(),
    })
}

async fn get_balance(State(state): State<SharedState>) -> Json<BalanceResponse> {
    let s = state.read().await;
    Json(BalanceResponse {
        balance: s.balance.to_string(),
        available: s.balance.to_string(),
    })
}

async fn force_exit(
    State(state): State<SharedState>,
    Json(req): Json<ForceExitRequest>,
) -> Result<Json<MessageResponse>, StatusCode> {
    let mut s = state.write().await;
    if s.positions.remove(&req.symbol).is_some() {
        tracing::info!(symbol = %req.symbol, "force-exited position");
        Ok(Json(MessageResponse {
            message: format!("force-exited {}", req.symbol),
        }))
    } else {
        Ok(Json(MessageResponse {
            message: format!("no open position for {}", req.symbol),
        }))
    }
}

async fn start_bot(State(state): State<SharedState>) -> Json<MessageResponse> {
    let mut s = state.write().await;
    s.running = true;
    tracing::info!("bot started via API");
    Json(MessageResponse {
        message: "bot started".to_string(),
    })
}

async fn stop_bot(State(state): State<SharedState>) -> Json<MessageResponse> {
    let mut s = state.write().await;
    s.running = false;
    tracing::info!("bot stopped via API");
    Json(MessageResponse {
        message: "bot stopped".to_string(),
    })
}

async fn reload_config() -> Json<MessageResponse> {
    tracing::info!("config reload requested (placeholder)");
    Json(MessageResponse {
        message: "config reload accepted".to_string(),
    })
}

// ---------------------------------------------------------------------------
// Router builder
// ---------------------------------------------------------------------------

fn build_router(state: SharedState, api_key: Option<String>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api = Router::new()
        .route("/api/v1/status", get(get_status))
        .route("/api/v1/trades", get(get_trades))
        .route("/api/v1/profit", get(get_profit))
        .route("/api/v1/balance", get(get_balance))
        .route("/api/v1/forceexit", post(force_exit))
        .route("/api/v1/start", post(start_bot))
        .route("/api/v1/stop", post(stop_bot))
        .route("/api/v1/reload_config", post(reload_config))
        .with_state(state.clone())
        .route_layer(middleware::from_fn_with_state(api_key, api_key_auth));

    Router::new()
        .route("/health", get(health))
        .merge(api)
        .layer(cors)
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let host = std::env::var("API_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = std::env::var("API_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);
    let api_key = std::env::var("API_KEY").ok();

    let state: SharedState = Arc::new(RwLock::new(AppState::new()));
    let app = build_router(state, api_key);

    let addr = format!("{host}:{port}");
    tracing::info!(%addr, "api-service starting");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for `oneshot`

    fn test_state() -> SharedState {
        Arc::new(RwLock::new(AppState {
            start_time: std::time::Instant::now(),
            running: true,
            symbol: "BTCUSDT".to_string(),
            mode: "paper".to_string(),
            positions: HashMap::new(),
            realized_pnl: Decimal::ZERO,
            balance: Decimal::new(10000, 0),
        }))
    }

    fn test_router(state: SharedState) -> Router {
        build_router(state, None)
    }

    fn test_router_with_key(state: SharedState, key: &str) -> Router {
        build_router(state, Some(key.to_string()))
    }

    #[tokio::test]
    async fn test_health() {
        let app = test_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let health: HealthResponse = serde_json::from_slice(&body).unwrap();
        assert!(health.healthy);
    }

    #[tokio::test]
    async fn test_status() {
        let app = test_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let status: StatusResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(status.status, "running");
        assert_eq!(status.symbol, "BTCUSDT");
        assert_eq!(status.mode, "paper");
    }

    #[tokio::test]
    async fn test_trades_empty() {
        let app = test_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/trades")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let trades: TradeResponse = serde_json::from_slice(&body).unwrap();
        assert!(trades.trades.is_empty());
    }

    #[tokio::test]
    async fn test_start_stop_toggle() {
        let state = test_state();

        // Stop
        let app = test_router(state.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/stop")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!state.read().await.running);

        // Start
        let app = test_router(state.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/start")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(state.read().await.running);
    }

    #[tokio::test]
    async fn test_forceexit() {
        let state = test_state();

        // Insert a position
        {
            let mut s = state.write().await;
            s.positions.insert(
                "BTCUSDT".to_string(),
                TradeInfo {
                    symbol: "BTCUSDT".to_string(),
                    side: "long".to_string(),
                    entry_price: "50000".to_string(),
                    quantity: "0.1".to_string(),
                    unrealized_pnl: "100".to_string(),
                    opened_at: 1700000000,
                },
            );
        }

        let app = test_router(state.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/forceexit")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"symbol":"BTCUSDT"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(state.read().await.positions.is_empty());
    }

    #[tokio::test]
    async fn test_api_key_required() {
        let state = test_state();
        let app = test_router_with_key(state, "secret123");

        // Request without key → 401
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_api_key_accepted() {
        let state = test_state();
        let app = test_router_with_key(state, "secret123");

        // Request with correct key → 200
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/status")
                    .header("X-API-Key", "secret123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_health_bypasses_api_key() {
        let state = test_state();
        let app = test_router_with_key(state, "secret123");

        // Health should be accessible without key
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_profit_response() {
        let app = test_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/profit")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let profit: ProfitResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(profit.total_realized_pnl, "0");
        assert_eq!(profit.trade_count, 0);
    }

    #[tokio::test]
    async fn test_balance_response() {
        let app = test_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/balance")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let balance: BalanceResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(balance.balance, "10000");
    }

    #[tokio::test]
    async fn test_reload_config() {
        let app = test_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/reload_config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
