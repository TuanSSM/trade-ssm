use axum::{
    extract::{
        ws::{Message, WebSocket},
        FromRequest, Request, State, WebSocketUpgrade,
    },
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use governor::clock::DefaultClock;
use governor::state::keyed::DefaultKeyedStateStore;
use governor::{Quota, RateLimiter};
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use ssm_store::TradeStore;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tower_http::cors::{Any, CorsLayer};
use validator::Validate;

type KeyedRateLimiter = RateLimiter<String, DefaultKeyedStateStore<String>, DefaultClock>;

// ---------------------------------------------------------------------------
// Validated JSON extractor
// ---------------------------------------------------------------------------

struct ValidatedJson<T>(pub T);

impl<S, T> FromRequest<S> for ValidatedJson<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state).await.map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        })?;
        value.validate().map_err(|e| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"errors": e.to_string()})),
            )
        })?;
        Ok(ValidatedJson(value))
    }
}

// ---------------------------------------------------------------------------
// Response / request types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LiveResponse {
    live: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReadyResponse {
    ready: bool,
    checks: ReadyChecks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReadyChecks {
    store: String,
    uptime_seconds: u64,
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

#[derive(Debug, Deserialize, Validate)]
struct ForceExitRequest {
    #[validate(length(min = 1, max = 20))]
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
    store: Option<Arc<TradeStore>>,
    event_tx: broadcast::Sender<String>,
    app_config: ssm_core::AppConfig,
    rate_limiter: Arc<KeyedRateLimiter>,
    live_balance: Option<(Decimal, Decimal)>,
}

impl AppState {
    fn new() -> Self {
        let symbol = std::env::var("SYMBOL").unwrap_or_else(|_| "BTCUSDT".to_string());
        let mode = std::env::var("EXECUTION_MODE").unwrap_or_else(|_| "paper".to_string());
        let balance_str = std::env::var("INITIAL_BALANCE").unwrap_or_else(|_| "10000".to_string());
        let balance = balance_str
            .parse::<Decimal>()
            .unwrap_or_else(|_| Decimal::new(10000, 0));

        // Open persistent store
        let store_path =
            std::env::var("STORE_PATH").unwrap_or_else(|_| "data/trade-ssm.db".to_string());
        let store = match TradeStore::open(&store_path) {
            Ok(s) => {
                tracing::info!(path = %store_path, "persistent store opened");
                Some(Arc::new(s))
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to open store, running without persistence");
                None
            }
        };

        // Load realized PnL from store
        let realized_pnl = store
            .as_ref()
            .and_then(|s| s.total_realized_pnl().ok())
            .unwrap_or_default();

        let (event_tx, _) = broadcast::channel(1000);

        let app_config = ssm_core::AppConfig::from_env_or_default();

        let rate_limiter = Arc::new(RateLimiter::keyed(Quota::per_minute(
            NonZeroU32::new(100).unwrap(),
        )));

        Self {
            start_time: std::time::Instant::now(),
            running: true,
            symbol,
            mode,
            positions: HashMap::new(),
            realized_pnl,
            balance,
            store,
            event_tx,
            app_config,
            rate_limiter,
            live_balance: None,
        }
    }
}

type SharedState = Arc<RwLock<AppState>>;

// ---------------------------------------------------------------------------
// Metrics middleware
// ---------------------------------------------------------------------------

async fn track_metrics(request: Request, next: Next) -> impl IntoResponse {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let response = next.run(request).await;
    metrics::counter!("ssm_api_requests_total", "method" => method, "path" => path).increment(1);
    response
}

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
// Per-key rate limit middleware
// ---------------------------------------------------------------------------

async fn rate_limit_middleware(
    State(state): State<SharedState>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Result<axum::response::Response, StatusCode> {
    let key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("anonymous")
        .to_string();

    let limiter = state.read().await.rate_limiter.clone();

    match limiter.check_key(&key) {
        Ok(()) => Ok(next.run(request).await),
        Err(_) => Err(StatusCode::TOO_MANY_REQUESTS),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health_live() -> Json<LiveResponse> {
    Json(LiveResponse { live: true })
}

async fn health_ready(State(state): State<SharedState>) -> (StatusCode, Json<ReadyResponse>) {
    let s = state.read().await;
    let uptime_seconds = s.start_time.elapsed().as_secs();

    let store_status = match &s.store {
        Some(store) => match store.ping() {
            Ok(()) => "ok".to_string(),
            Err(e) => format!("{e}"),
        },
        None => "unavailable".to_string(),
    };

    let ready = store_status == "ok";
    let status_code = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status_code,
        Json(ReadyResponse {
            ready,
            checks: ReadyChecks {
                store: store_status,
                uptime_seconds,
            },
        }),
    )
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
    let (balance, available) = match s.live_balance {
        Some((b, a)) => (b, a),
        None => (s.balance, s.balance),
    };
    Json(BalanceResponse {
        balance: balance.to_string(),
        available: available.to_string(),
    })
}

fn extract_client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

async fn force_exit(
    State(state): State<SharedState>,
    headers: HeaderMap,
    ValidatedJson(req): ValidatedJson<ForceExitRequest>,
) -> Result<Json<MessageResponse>, (StatusCode, Json<serde_json::Value>)> {
    let mut s = state.write().await;
    let ip = extract_client_ip(&headers);
    if s.positions.remove(&req.symbol).is_some() {
        tracing::info!(symbol = %req.symbol, "force-exited position");
        let _ = s.event_tx.send(
            serde_json::json!({
                "type": "force_exited",
                "symbol": req.symbol,
                "timestamp": chrono::Utc::now().timestamp_millis()
            })
            .to_string(),
        );
        if let Some(ref store) = s.store {
            let _ = store.log_audit("force_exit", "api", Some(&req.symbol), Some(&ip));
        }
        Ok(Json(MessageResponse {
            message: format!("force-exited {}", req.symbol),
        }))
    } else {
        Ok(Json(MessageResponse {
            message: format!("no open position for {}", req.symbol),
        }))
    }
}

async fn start_bot(State(state): State<SharedState>, headers: HeaderMap) -> Json<MessageResponse> {
    let mut s = state.write().await;
    let ip = extract_client_ip(&headers);
    s.running = true;
    tracing::info!("bot started via API");
    let _ = s.event_tx.send(
        serde_json::json!({
            "type": "bot_started",
            "timestamp": chrono::Utc::now().timestamp_millis()
        })
        .to_string(),
    );
    if let Some(ref store) = s.store {
        let _ = store.log_audit("start_bot", "api", None, Some(&ip));
    }
    Json(MessageResponse {
        message: "bot started".to_string(),
    })
}

async fn stop_bot(State(state): State<SharedState>, headers: HeaderMap) -> Json<MessageResponse> {
    let mut s = state.write().await;
    let ip = extract_client_ip(&headers);
    s.running = false;
    tracing::info!("bot stopped via API");
    let _ = s.event_tx.send(
        serde_json::json!({
            "type": "bot_stopped",
            "timestamp": chrono::Utc::now().timestamp_millis()
        })
        .to_string(),
    );
    if let Some(ref store) = s.store {
        let _ = store.log_audit("stop_bot", "api", None, Some(&ip));
    }
    Json(MessageResponse {
        message: "bot stopped".to_string(),
    })
}

async fn reload_config(State(state): State<SharedState>) -> impl IntoResponse {
    let config_path =
        std::env::var("CONFIG_FILE").unwrap_or_else(|_| "config/default.toml".to_string());
    match ssm_core::AppConfig::reload(std::path::Path::new(&config_path)) {
        Ok(new_config) => {
            let mut s = state.write().await;
            s.app_config = new_config.clone();
            tracing::info!("config reloaded successfully");
            let _ = s.event_tx.send(
                serde_json::json!({
                    "type": "config_reloaded",
                    "timestamp": chrono::Utc::now().timestamp_millis()
                })
                .to_string(),
            );
            Json(serde_json::json!({ "status": "reloaded", "config": new_config }))
        }
        Err(e) => {
            tracing::warn!(error = %e, "config reload failed");
            Json(serde_json::json!({ "status": "error", "error": e.to_string() }))
        }
    }
}

// ---------------------------------------------------------------------------
// Trade history & performance endpoints (powered by ssm-store)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TradeHistoryQuery {
    from: Option<i64>,
    to: Option<i64>,
    symbol: Option<String>,
}

#[derive(Debug, Serialize)]
struct TradeHistoryResponse {
    trades: Vec<TradeHistoryItem>,
    total: usize,
}

#[derive(Debug, Serialize)]
struct TradeHistoryItem {
    id: String,
    symbol: String,
    side: String,
    entry_price: String,
    exit_price: String,
    quantity: String,
    profit: String,
    profit_pct: String,
    entry_time: i64,
    exit_time: i64,
    duration_candles: u64,
    exit_reason: String,
}

async fn get_trade_history(
    State(state): State<SharedState>,
    axum::extract::Query(q): axum::extract::Query<TradeHistoryQuery>,
) -> Json<TradeHistoryResponse> {
    let s = state.read().await;
    let Some(store) = &s.store else {
        return Json(TradeHistoryResponse {
            trades: vec![],
            total: 0,
        });
    };

    let trades = store
        .load_trades(q.from, q.to, q.symbol.as_deref())
        .unwrap_or_default();
    let total = trades.len();
    let items: Vec<TradeHistoryItem> = trades
        .into_iter()
        .map(|t| TradeHistoryItem {
            id: t.id,
            symbol: t.symbol,
            side: format!("{}", t.side),
            entry_price: t.entry_price.to_string(),
            exit_price: t.exit_price.to_string(),
            quantity: t.quantity.to_string(),
            profit: t.profit.to_string(),
            profit_pct: t.profit_pct.to_string(),
            entry_time: t.entry_time,
            exit_time: t.exit_time,
            duration_candles: t.duration_candles,
            exit_reason: format!("{:?}", t.exit_reason),
        })
        .collect();

    Json(TradeHistoryResponse {
        trades: items,
        total,
    })
}

async fn get_performance(State(state): State<SharedState>) -> Json<ssm_store::PerformanceSummary> {
    let s = state.read().await;
    let Some(store) = &s.store else {
        return Json(ssm_store::summarize(&[]));
    };

    let trades = store.load_trades(None, None, None).unwrap_or_default();
    Json(ssm_store::summarize(&trades))
}

#[derive(Debug, Serialize)]
struct DailyPerfResponse {
    days: Vec<ssm_store::analytics::DailyPerformance>,
}

async fn get_daily_performance(State(state): State<SharedState>) -> Json<DailyPerfResponse> {
    let s = state.read().await;
    let Some(store) = &s.store else {
        return Json(DailyPerfResponse { days: vec![] });
    };

    let trades = store.load_trades(None, None, None).unwrap_or_default();
    Json(DailyPerfResponse {
        days: ssm_store::daily_performance(&trades),
    })
}

async fn get_audit_log(State(state): State<SharedState>) -> axum::response::Response {
    let s = state.read().await;
    if let Some(ref store) = s.store {
        match store.load_audit_log(100) {
            Ok(entries) => Json(serde_json::json!({ "entries": entries })).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        }
    } else {
        Json(serde_json::json!({ "entries": [] })).into_response()
    }
}

// ---------------------------------------------------------------------------
// Signal injection + order listing
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Validate)]
struct InjectSignalRequest {
    #[validate(length(min = 1, max = 20))]
    symbol: String,
    action: String,
    confidence: Option<f64>,
}

async fn inject_signal(
    State(state): State<SharedState>,
    headers: HeaderMap,
    ValidatedJson(req): ValidatedJson<InjectSignalRequest>,
) -> impl IntoResponse {
    let action = match req.action.as_str() {
        "enter_long" => "EnterLong",
        "exit_long" => "ExitLong",
        "enter_short" => "EnterShort",
        "exit_short" => "ExitShort",
        _ => "Neutral",
    };

    let event = serde_json::json!({
        "type": "signal_injected",
        "symbol": req.symbol,
        "action": action,
        "confidence": req.confidence.unwrap_or(1.0),
        "timestamp": chrono::Utc::now().timestamp_millis()
    });

    let s = state.read().await;
    let _ = s.event_tx.send(event.to_string());

    if let Some(ref store) = s.store {
        let ip = extract_client_ip(&headers);
        let _ = store.log_audit(
            "inject_signal",
            "api",
            Some(&format!("{} {}", req.symbol, action)),
            Some(&ip),
        );
    }

    Json(serde_json::json!({ "status": "signal_injected", "action": action }))
}

async fn list_orders(State(state): State<SharedState>) -> axum::response::Response {
    let s = state.read().await;
    if let Some(ref store) = s.store {
        match store.load_orders_by_status(ssm_core::OrderStatus::Open) {
            Ok(orders) => Json(serde_json::json!({ "orders": orders })).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response(),
        }
    } else {
        Json(serde_json::json!({ "orders": [] })).into_response()
    }
}

// ---------------------------------------------------------------------------
// Internal balance update endpoint
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct BalanceUpdateRequest {
    balance: String,
    available: String,
}

async fn update_balance(
    State(state): State<SharedState>,
    Json(req): Json<BalanceUpdateRequest>,
) -> impl IntoResponse {
    match (
        req.balance.parse::<Decimal>(),
        req.available.parse::<Decimal>(),
    ) {
        (Ok(b), Ok(a)) => {
            let mut s = state.write().await;
            s.live_balance = Some((b, a));
            Json(serde_json::json!({ "status": "ok" }))
        }
        _ => Json(serde_json::json!({ "status": "error", "error": "invalid decimal" })),
    }
}

// ---------------------------------------------------------------------------
// WebSocket streaming
// ---------------------------------------------------------------------------

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<SharedState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_ws_connection(socket, state))
}

async fn handle_ws_connection(mut socket: WebSocket, state: SharedState) {
    let mut rx = state.read().await.event_tx.subscribe();
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(text) => {
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
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
        .route("/api/v1/trades/history", get(get_trade_history))
        .route("/api/v1/performance", get(get_performance))
        .route("/api/v1/performance/daily", get(get_daily_performance))
        .route("/api/v1/audit", get(get_audit_log))
        .route("/api/v1/signals/inject", post(inject_signal))
        .route("/api/v1/orders", get(list_orders))
        .route("/api/v1/internal/balance", post(update_balance))
        .with_state(state.clone())
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ))
        .route_layer(middleware::from_fn_with_state(api_key, api_key_auth));

    Router::new()
        .route("/health", get(health_live))
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/api/v1/ws", get(ws_handler))
        .with_state(state.clone())
        .merge(api)
        .layer(cors)
        .layer(middleware::from_fn(track_metrics))
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ssm_core::init_logging();

    let metrics_port: u16 = std::env::var("METRICS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9091);
    ssm_core::init_metrics(metrics_port);

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
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("api-service shut down");
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl+c");
    tracing::info!("shutdown signal received, exiting gracefully");
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
        let (event_tx, _) = broadcast::channel(16);
        Arc::new(RwLock::new(AppState {
            start_time: std::time::Instant::now(),
            running: true,
            symbol: "BTCUSDT".to_string(),
            mode: "paper".to_string(),
            positions: HashMap::new(),
            realized_pnl: Decimal::ZERO,
            balance: Decimal::new(10000, 0),
            store: None,
            event_tx,
            app_config: ssm_core::AppConfig::default(),
            rate_limiter: Arc::new(RateLimiter::keyed(Quota::per_minute(
                NonZeroU32::new(100).unwrap(),
            ))),
            live_balance: None,
        }))
    }

    fn test_router(state: SharedState) -> Router {
        build_router(state, None)
    }

    fn test_router_with_key(state: SharedState, key: &str) -> Router {
        build_router(state, Some(key.to_string()))
    }

    #[tokio::test]
    async fn test_health_live() {
        let app = test_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health/live")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let live: LiveResponse = serde_json::from_slice(&body).unwrap();
        assert!(live.live);
    }

    #[tokio::test]
    async fn test_health_backward_compat() {
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
        let live: LiveResponse = serde_json::from_slice(&body).unwrap();
        assert!(live.live);
    }

    #[tokio::test]
    async fn test_health_ready_no_store() {
        // Default test_state has store: None -> not ready
        let app = test_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let ready: ReadyResponse = serde_json::from_slice(&body).unwrap();
        assert!(!ready.ready);
        assert_eq!(ready.checks.store, "unavailable");
    }

    #[tokio::test]
    async fn test_health_ready_with_store() {
        let store = TradeStore::open_memory().unwrap();
        let (event_tx, _) = broadcast::channel(16);
        let state: SharedState = Arc::new(RwLock::new(AppState {
            start_time: std::time::Instant::now(),
            running: true,
            symbol: "BTCUSDT".to_string(),
            mode: "paper".to_string(),
            positions: HashMap::new(),
            realized_pnl: Decimal::ZERO,
            balance: Decimal::new(10000, 0),
            store: Some(Arc::new(store)),
            event_tx,
            app_config: ssm_core::AppConfig::default(),
            rate_limiter: Arc::new(RateLimiter::keyed(Quota::per_minute(
                NonZeroU32::new(100).unwrap(),
            ))),
            live_balance: None,
        }));
        let app = test_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/health/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let ready: ReadyResponse = serde_json::from_slice(&body).unwrap();
        assert!(ready.ready);
        assert_eq!(ready.checks.store, "ok");
        assert!(ready.checks.uptime_seconds < 5);
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

    #[tokio::test]
    async fn test_forceexit_empty_symbol_returns_422() {
        let app = test_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/forceexit")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"symbol":""}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn test_forceexit_long_symbol_returns_422() {
        let app = test_router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/forceexit")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"symbol":"AAAAABBBBBCCCCCDDDDDE"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn test_forceexit_valid_symbol_passes_validation() {
        let app = test_router(test_state());
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
    }
}
