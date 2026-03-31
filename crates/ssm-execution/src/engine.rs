use anyhow::Result;
use rust_decimal::Decimal;
use ssm_core::{ExecutionMode, Order, OrderStatus, OrderType, Side, Signal};
use ssm_store::TradeStore;
use std::sync::Arc;

use crate::error::ExecutionError;
use crate::live::LiveEngine;
use crate::paper::PaperEngine;
use crate::position_tracker::PositionTracker;

/// Unified execution engine — routes to paper or live based on mode.
pub struct ExecutionEngine {
    mode: ExecutionMode,
    paper: PaperEngine,
    positions: PositionTracker,
    live: Option<LiveEngine>,
    store: Option<Arc<TradeStore>>,
}

impl ExecutionEngine {
    pub fn new(mode: ExecutionMode) -> Self {
        tracing::info!(?mode, "execution engine initialized");
        Self {
            mode,
            paper: PaperEngine::new(),
            positions: PositionTracker::new(),
            live: None,
            store: None,
        }
    }

    /// Create an engine with a LiveEngine attached for live execution.
    pub fn with_live(live: LiveEngine) -> Self {
        tracing::info!(mode = ?ExecutionMode::Live, "execution engine initialized with live backend");
        Self {
            mode: ExecutionMode::Live,
            paper: PaperEngine::new(),
            positions: PositionTracker::new(),
            live: Some(live),
            store: None,
        }
    }

    /// Attach a persistent store for position/order/trade durability.
    pub fn with_store(mut self, store: Arc<TradeStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Load persisted positions from the store (startup recovery).
    pub fn recover_positions(&mut self) -> Result<usize> {
        let store = match &self.store {
            Some(s) => s,
            None => return Ok(0),
        };
        let positions = store.load_positions()?;
        let count = positions.len();
        for pos in positions {
            tracing::info!(
                symbol = %pos.symbol,
                side = %pos.side,
                qty = %pos.quantity,
                entry = %pos.entry_price,
                "recovered position from store"
            );
            self.positions.restore_position(pos);
        }
        if count > 0 {
            tracing::info!(count, "positions recovered from persistent store");
        }
        Ok(count)
    }

    /// Access the store (if attached).
    pub fn store(&self) -> Option<&Arc<TradeStore>> {
        self.store.as_ref()
    }

    /// Create from environment: reads EXECUTION_MODE and, for live mode,
    /// BINANCE_API_KEY / BINANCE_SECRET_KEY.
    pub fn from_env() -> Result<Self> {
        let mode = match std::env::var("EXECUTION_MODE")
            .unwrap_or_else(|_| "paper".into())
            .as_str()
        {
            "live" => ExecutionMode::Live,
            _ => ExecutionMode::Paper,
        };

        match mode {
            ExecutionMode::Live => {
                let live = LiveEngine::from_env()?;
                Ok(Self::with_live(live))
            }
            ExecutionMode::Paper => Ok(Self::new(ExecutionMode::Paper)),
        }
    }

    pub fn mode(&self) -> ExecutionMode {
        self.mode
    }

    /// Returns a reference to the LiveEngine, if configured.
    pub fn live_engine(&self) -> Option<&LiveEngine> {
        self.live.as_ref()
    }

    pub fn positions(&self) -> &PositionTracker {
        &self.positions
    }

    pub fn positions_mut(&mut self) -> &mut PositionTracker {
        &mut self.positions
    }

    /// Submit an order derived from a Signal.
    pub fn submit_signal(
        &mut self,
        signal: &Signal,
        quantity: Decimal,
        current_price: Decimal,
    ) -> Result<Order> {
        let side = match signal.action {
            ssm_core::AIAction::EnterLong | ssm_core::AIAction::ExitShort => Side::Buy,
            ssm_core::AIAction::EnterShort | ssm_core::AIAction::ExitLong => Side::Sell,
            ssm_core::AIAction::Neutral => return Err(ExecutionError::NeutralAction.into()),
        };

        let order = self.create_market_order(&signal.symbol, side, quantity, current_price)?;

        tracing::info!(
            order_id = %order.id,
            symbol = %order.symbol,
            side = %order.side,
            qty = %order.quantity,
            price = %current_price,
            mode = ?self.mode,
            source = %signal.source,
            "order executed"
        );

        Ok(order)
    }

    /// Submit a raw order synchronously (paper mode only).
    ///
    /// For live mode without a LiveEngine attached, the order stays Pending.
    /// Use `submit_order_async` for live exchange execution.
    pub fn submit_order(&mut self, mut order: Order, current_price: Decimal) -> Result<Order> {
        match self.mode {
            ExecutionMode::Paper => {
                self.paper.fill_order(&mut order, current_price)?;
                if order.status == OrderStatus::Filled {
                    self.positions.apply_fill(&order, current_price);
                    self.persist_position_state(&order.symbol);
                }
            }
            ExecutionMode::Live => {
                // Sync path for live mode — order stays Pending.
                // Use submit_order_async() for actual exchange execution.
                tracing::warn!("live execution via sync path, order stays Pending — use submit_order_async for exchange execution");
            }
        }
        self.persist_order(&order);
        Ok(order)
    }

    /// Submit a raw order, routing to LiveEngine for live mode (async).
    ///
    /// - Paper mode: delegates to the synchronous paper engine.
    /// - Live mode: calls the Binance Futures REST API via LiveEngine.
    ///
    /// After a successful fill (full or partial), the position tracker is updated.
    pub async fn submit_order_async(
        &mut self,
        mut order: Order,
        current_price: Decimal,
    ) -> Result<Order> {
        match self.mode {
            ExecutionMode::Paper => {
                self.paper.fill_order(&mut order, current_price)?;
                if order.status == OrderStatus::Filled {
                    self.positions.apply_fill(&order, current_price);
                    self.persist_position_state(&order.symbol);
                }
            }
            ExecutionMode::Live => {
                let live = self.live.as_ref().ok_or(ExecutionError::NoLiveEngine)?;
                live.submit_order(&mut order, current_price).await?;

                // Update positions based on exchange response
                match order.status {
                    OrderStatus::Filled => {
                        let fill_price = order.price.unwrap_or(current_price);
                        self.positions.apply_fill(&order, fill_price);
                        self.persist_position_state(&order.symbol);
                    }
                    OrderStatus::PartiallyFilled => {
                        // For partial fills, query the actual filled quantity
                        let (status, filled_qty) = live
                            .query_order_detail(&order.symbol, &order.id)
                            .await
                            .unwrap_or((OrderStatus::PartiallyFilled, Decimal::ZERO));

                        if filled_qty > Decimal::ZERO {
                            let mut partial_order = order.clone();
                            partial_order.quantity = filled_qty;
                            let fill_price = order.price.unwrap_or(current_price);
                            self.positions.apply_fill(&partial_order, fill_price);
                            self.persist_position_state(&order.symbol);
                        }
                        order.status = status;
                    }
                    _ => {
                        // Open, Rejected, etc. — no position update
                    }
                }
            }
        }
        self.persist_order(&order);
        Ok(order)
    }

    /// Submit an order derived from a Signal (async version for live mode).
    pub async fn submit_signal_async(
        &mut self,
        signal: &Signal,
        quantity: Decimal,
        current_price: Decimal,
    ) -> Result<Order> {
        let side = match signal.action {
            ssm_core::AIAction::EnterLong | ssm_core::AIAction::ExitShort => Side::Buy,
            ssm_core::AIAction::EnterShort | ssm_core::AIAction::ExitLong => Side::Sell,
            ssm_core::AIAction::Neutral => return Err(ExecutionError::NeutralAction.into()),
        };

        let now = chrono::Utc::now().timestamp_millis();
        let order = Order {
            id: format!("ssm-{now}"),
            symbol: signal.symbol.clone(),
            side,
            order_type: OrderType::Market,
            quantity,
            price: None,
            stop_price: None,
            trailing_delta: None,
            time_in_force: None,
            reduce_only: false,
            status: OrderStatus::Pending,
            created_at: now,
            updated_at: now,
        };

        let result = self.submit_order_async(order, current_price).await?;

        tracing::info!(
            order_id = %result.id,
            symbol = %result.symbol,
            side = %result.side,
            qty = %result.quantity,
            price = %current_price,
            mode = ?self.mode,
            source = %signal.source,
            status = ?result.status,
            "order executed (async)"
        );

        Ok(result)
    }

    /// Cancel an order on the exchange (live mode only).
    ///
    /// In paper mode this is a no-op that returns Ok.
    pub async fn cancel_order(&self, symbol: &str, order_id: &str) -> Result<()> {
        match self.mode {
            ExecutionMode::Paper => {
                tracing::info!(order_id, symbol, "paper cancel (no-op)");
                Ok(())
            }
            ExecutionMode::Live => {
                let live = self.live.as_ref().ok_or(ExecutionError::NoLiveEngine)?;
                live.cancel_order(symbol, order_id).await
            }
        }
    }

    /// Query the current status of an order on the exchange.
    ///
    /// In paper mode always returns `OrderStatus::Pending`.
    pub async fn query_order_status(&self, symbol: &str, order_id: &str) -> Result<OrderStatus> {
        match self.mode {
            ExecutionMode::Paper => {
                tracing::debug!(order_id, symbol, "paper query — returning Pending");
                Ok(OrderStatus::Pending)
            }
            ExecutionMode::Live => {
                let live = self.live.as_ref().ok_or(ExecutionError::NoLiveEngine)?;
                live.query_order(symbol, order_id).await
            }
        }
    }

    /// Pre-flight check: verify exchange connectivity, balance, and positions.
    ///
    /// Should be called before placing the first order in a session.
    /// In paper mode this always succeeds.
    pub async fn preflight_check(&self) -> Result<()> {
        match self.mode {
            ExecutionMode::Paper => {
                tracing::info!("paper mode preflight — OK");
                Ok(())
            }
            ExecutionMode::Live => {
                let live = self.live.as_ref().ok_or(ExecutionError::NoLiveEngine)?;

                tracing::info!("running live preflight checks");

                // Verify we can reach the exchange and authenticate
                let balances = live
                    .fetch_balance()
                    .await
                    .map_err(|e| ExecutionError::PreflightFailed(format!("balance check: {e}")))?;

                let total_available: Decimal = balances.iter().map(|b| b.available).sum();

                tracing::info!(
                    assets = balances.len(),
                    total_available = %total_available,
                    "balance check passed"
                );

                if total_available <= Decimal::ZERO {
                    return Err(
                        ExecutionError::PreflightFailed("no available balance".into()).into(),
                    );
                }

                // Fetch open positions
                let positions = live
                    .fetch_positions()
                    .await
                    .map_err(|e| ExecutionError::PreflightFailed(format!("position check: {e}")))?;

                tracing::info!(open_positions = positions.len(), "position check passed");

                for pos in &positions {
                    tracing::info!(
                        symbol = %pos.symbol,
                        amount = %pos.amount,
                        entry_price = %pos.entry_price,
                        leverage = pos.leverage,
                        "existing position found"
                    );
                }

                tracing::info!("preflight checks passed");
                Ok(())
            }
        }
    }

    /// Persist position state after a fill. Saves open positions, removes closed ones.
    fn persist_position_state(&self, symbol: &str) {
        let Some(store) = &self.store else { return };
        // Save current position (if still open)
        if let Some(pos) = self.positions.get(symbol) {
            if let Err(e) = store.save_position(pos) {
                tracing::error!(error = %e, symbol, "failed to persist position");
            }
        }
        // Remove closed positions from store
        for closed in self.positions.closed_symbols() {
            if self.positions.get(closed).is_none() {
                if let Err(e) = store.remove_position(closed) {
                    tracing::error!(error = %e, symbol = closed, "failed to remove position from store");
                }
            }
        }
    }

    /// Persist an order to the store.
    fn persist_order(&self, order: &Order) {
        let Some(store) = &self.store else { return };
        if let Err(e) = store.save_order(order) {
            tracing::error!(error = %e, order_id = %order.id, "failed to persist order");
        }
    }

    fn create_market_order(
        &mut self,
        symbol: &str,
        side: Side,
        quantity: Decimal,
        current_price: Decimal,
    ) -> Result<Order> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut order = Order {
            id: format!("ssm-{now}"),
            symbol: symbol.to_string(),
            side,
            order_type: OrderType::Market,
            quantity,
            price: None,
            stop_price: None,
            trailing_delta: None,
            time_in_force: None,
            reduce_only: false,
            status: OrderStatus::Pending,
            created_at: now,
            updated_at: now,
        };
        self.submit_order(order.clone(), current_price)?;
        // Re-read status after submission
        if self.mode == ExecutionMode::Paper {
            order.status = OrderStatus::Filled;
            order.updated_at = chrono::Utc::now().timestamp_millis();
        }
        Ok(order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_signal(action: ssm_core::AIAction) -> Signal {
        Signal {
            timestamp: 1000,
            symbol: "BTCUSDT".into(),
            action,
            confidence: 0.8,
            source: "test".into(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn paper_market_order_fills_instantly() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::EnterLong);
        let order = engine
            .submit_signal(&signal, Decimal::from(1), Decimal::from(50000))
            .unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
        assert_eq!(order.side, Side::Buy);
    }

    #[test]
    fn neutral_action_rejected() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::Neutral);
        let result = engine.submit_signal(&signal, Decimal::from(1), Decimal::from(50000));
        assert!(result.is_err());
    }

    #[test]
    fn position_tracked_after_fill() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::EnterLong);
        engine
            .submit_signal(&signal, Decimal::from(1), Decimal::from(50000))
            .unwrap();
        let pos = engine.positions().get("BTCUSDT");
        assert!(pos.is_some());
        assert_eq!(pos.unwrap().side, Side::Buy);
    }

    #[test]
    fn test_enter_short_signal() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::EnterShort);
        let order = engine
            .submit_signal(&signal, Decimal::from(1), Decimal::from(50000))
            .unwrap();
        assert_eq!(order.side, Side::Sell);
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_exit_long_signal() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::ExitLong);
        let order = engine
            .submit_signal(&signal, Decimal::from(1), Decimal::from(50000))
            .unwrap();
        assert_eq!(order.side, Side::Sell);
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_exit_short_signal() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::ExitShort);
        let order = engine
            .submit_signal(&signal, Decimal::from(1), Decimal::from(50000))
            .unwrap();
        assert_eq!(order.side, Side::Buy);
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_engine_mode() {
        let engine = ExecutionEngine::new(ExecutionMode::Paper);
        assert_eq!(engine.mode(), ExecutionMode::Paper);
    }

    #[test]
    fn test_live_mode_stays_pending() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Live);
        let order = Order {
            id: "test-live".into(),
            symbol: "BTCUSDT".into(),
            side: Side::Buy,
            order_type: OrderType::Market,
            quantity: Decimal::from(1),
            price: None,
            stop_price: None,
            trailing_delta: None,
            time_in_force: None,
            reduce_only: false,
            status: OrderStatus::Pending,
            created_at: 0,
            updated_at: 0,
        };
        let result = engine.submit_order(order, Decimal::from(50000)).unwrap();
        assert_eq!(result.status, OrderStatus::Pending);
    }

    #[test]
    fn test_submit_raw_order() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let order = Order {
            id: "raw-1".into(),
            symbol: "BTCUSDT".into(),
            side: Side::Buy,
            order_type: OrderType::Market,
            quantity: Decimal::from(2),
            price: None,
            stop_price: None,
            trailing_delta: None,
            time_in_force: None,
            reduce_only: false,
            status: OrderStatus::Pending,
            created_at: 0,
            updated_at: 0,
        };
        let result = engine.submit_order(order, Decimal::from(45000)).unwrap();
        assert_eq!(result.status, OrderStatus::Filled);
        assert!(engine.positions().has_position("BTCUSDT"));
    }

    #[test]
    fn test_order_id_contains_timestamp_prefix() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::EnterLong);
        let order = engine
            .submit_signal(&signal, Decimal::from(1), Decimal::from(50000))
            .unwrap();
        assert!(order.id.starts_with("ssm-"));
    }

    #[test]
    fn test_order_ids_are_unique() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::EnterLong);
        let order1 = engine
            .submit_signal(&signal, Decimal::from(1), Decimal::from(50000))
            .unwrap();
        // Small delay to ensure different timestamp
        std::thread::sleep(std::time::Duration::from_millis(2));
        let order2 = engine
            .submit_signal(&signal, Decimal::from(1), Decimal::from(50000))
            .unwrap();
        assert_ne!(order1.id, order2.id);
    }

    #[test]
    fn test_create_market_order_buy_side() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::EnterLong);
        let order = engine
            .submit_signal(&signal, Decimal::from(3), Decimal::from(42000))
            .unwrap();
        assert_eq!(order.side, Side::Buy);
        assert_eq!(order.order_type, OrderType::Market);
        assert_eq!(order.quantity, Decimal::from(3));
        assert_eq!(order.symbol, "BTCUSDT");
    }

    #[test]
    fn test_create_market_order_sell_side() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::EnterShort);
        let order = engine
            .submit_signal(&signal, Decimal::from(5), Decimal::from(60000))
            .unwrap();
        assert_eq!(order.side, Side::Sell);
        assert_eq!(order.order_type, OrderType::Market);
        assert_eq!(order.quantity, Decimal::from(5));
    }

    #[test]
    fn test_live_mode_engine() {
        let engine = ExecutionEngine::new(ExecutionMode::Live);
        assert_eq!(engine.mode(), ExecutionMode::Live);
    }

    #[test]
    fn test_live_mode_signal_stays_pending_no_position() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Live);
        let signal = test_signal(ssm_core::AIAction::EnterLong);
        let order = engine
            .submit_signal(&signal, Decimal::from(1), Decimal::from(50000))
            .unwrap();
        // In live mode, order stays pending and position is not tracked
        assert_eq!(order.status, OrderStatus::Pending);
        assert!(!engine.positions().has_position("BTCUSDT"));
    }

    #[test]
    fn test_paper_mode_tracks_position_after_signal() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::EnterShort);
        engine
            .submit_signal(&signal, Decimal::from(2), Decimal::from(50000))
            .unwrap();
        let pos = engine.positions().get("BTCUSDT").unwrap();
        assert_eq!(pos.side, Side::Sell);
        assert_eq!(pos.quantity, Decimal::from(2));
    }

    #[test]
    fn test_multiple_signals_accumulate_position() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::EnterLong);
        engine
            .submit_signal(&signal, Decimal::from(1), Decimal::from(50000))
            .unwrap();
        engine
            .submit_signal(&signal, Decimal::from(2), Decimal::from(51000))
            .unwrap();
        let pos = engine.positions().get("BTCUSDT").unwrap();
        assert_eq!(pos.quantity, Decimal::from(3));
    }

    #[test]
    fn test_positions_mut_access() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::EnterLong);
        engine
            .submit_signal(&signal, Decimal::from(1), Decimal::from(50000))
            .unwrap();
        // Verify mutable access works
        let prices = {
            let mut map = HashMap::new();
            map.insert("BTCUSDT".to_string(), Decimal::from(55000));
            map
        };
        engine.positions_mut().mark_to_market(&prices);
        let pos = engine.positions().get("BTCUSDT").unwrap();
        assert_eq!(pos.unrealized_pnl, Decimal::from(5000));
    }

    #[test]
    fn test_submit_limit_order_paper_open() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let order = Order {
            id: "limit-1".into(),
            symbol: "BTCUSDT".into(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            quantity: Decimal::from(1),
            price: Some(Decimal::from(40000)),
            stop_price: None,
            trailing_delta: None,
            time_in_force: None,
            reduce_only: false,
            status: OrderStatus::Pending,
            created_at: 0,
            updated_at: 0,
        };
        // Current price 50000 is above limit buy at 40000 — should stay open
        let result = engine.submit_order(order, Decimal::from(50000)).unwrap();
        assert_eq!(result.status, OrderStatus::Open);
        // No position since order not filled
        assert!(!engine.positions().has_position("BTCUSDT"));
    }

    #[test]
    fn test_neutral_action_error_message() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::Neutral);
        let err = engine
            .submit_signal(&signal, Decimal::from(1), Decimal::from(50000))
            .unwrap_err();
        assert!(err.to_string().contains("Neutral"));
    }

    // --- Tests for TODO-001: Live Exchange Execution ---

    #[test]
    fn test_with_live_constructor() {
        let live = LiveEngine::with_testnet("key".into(), "secret".into());
        let engine = ExecutionEngine::with_live(live);
        assert_eq!(engine.mode(), ExecutionMode::Live);
        assert!(engine.live_engine().is_some());
    }

    #[test]
    fn test_with_live_uses_testnet_url() {
        let live = LiveEngine::with_testnet("key".into(), "secret".into());
        let engine = ExecutionEngine::with_live(live);
        assert_eq!(
            engine.live_engine().unwrap().base_url(),
            "https://testnet.binancefuture.com"
        );
    }

    #[test]
    fn test_new_paper_has_no_live_engine() {
        let engine = ExecutionEngine::new(ExecutionMode::Paper);
        assert!(engine.live_engine().is_none());
    }

    #[test]
    fn test_new_live_without_backend_has_no_live_engine() {
        let engine = ExecutionEngine::new(ExecutionMode::Live);
        assert!(engine.live_engine().is_none());
    }

    #[tokio::test]
    async fn test_submit_order_async_paper_fills() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let order = Order {
            id: "async-paper-1".into(),
            symbol: "BTCUSDT".into(),
            side: Side::Buy,
            order_type: OrderType::Market,
            quantity: Decimal::from(1),
            price: None,
            stop_price: None,
            trailing_delta: None,
            time_in_force: None,
            reduce_only: false,
            status: OrderStatus::Pending,
            created_at: 0,
            updated_at: 0,
        };
        let result = engine
            .submit_order_async(order, Decimal::from(50000))
            .await
            .unwrap();
        assert_eq!(result.status, OrderStatus::Filled);
        assert!(engine.positions().has_position("BTCUSDT"));
    }

    #[tokio::test]
    async fn test_submit_order_async_live_no_backend_errors() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Live);
        let order = Order {
            id: "async-live-nobackend".into(),
            symbol: "BTCUSDT".into(),
            side: Side::Buy,
            order_type: OrderType::Market,
            quantity: Decimal::from(1),
            price: None,
            stop_price: None,
            trailing_delta: None,
            time_in_force: None,
            reduce_only: false,
            status: OrderStatus::Pending,
            created_at: 0,
            updated_at: 0,
        };
        let result = engine.submit_order_async(order, Decimal::from(50000)).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("live engine not configured"));
    }

    #[tokio::test]
    async fn test_cancel_order_paper_is_noop() {
        let engine = ExecutionEngine::new(ExecutionMode::Paper);
        let result = engine.cancel_order("BTCUSDT", "order-1").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_cancel_order_live_no_backend_errors() {
        let engine = ExecutionEngine::new(ExecutionMode::Live);
        let result = engine.cancel_order("BTCUSDT", "order-1").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("live engine not configured"));
    }

    #[tokio::test]
    async fn test_query_order_status_paper_returns_pending() {
        let engine = ExecutionEngine::new(ExecutionMode::Paper);
        let status = engine
            .query_order_status("BTCUSDT", "order-1")
            .await
            .unwrap();
        assert_eq!(status, OrderStatus::Pending);
    }

    #[tokio::test]
    async fn test_query_order_status_live_no_backend_errors() {
        let engine = ExecutionEngine::new(ExecutionMode::Live);
        let result = engine.query_order_status("BTCUSDT", "order-1").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("live engine not configured"));
    }

    #[tokio::test]
    async fn test_preflight_check_paper_succeeds() {
        let engine = ExecutionEngine::new(ExecutionMode::Paper);
        let result = engine.preflight_check().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_preflight_check_live_no_backend_errors() {
        let engine = ExecutionEngine::new(ExecutionMode::Live);
        let result = engine.preflight_check().await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("live engine not configured"));
    }

    #[tokio::test]
    async fn test_submit_signal_async_paper_fills() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::EnterLong);
        let order = engine
            .submit_signal_async(&signal, Decimal::from(1), Decimal::from(50000))
            .await
            .unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
        assert!(engine.positions().has_position("BTCUSDT"));
    }

    #[tokio::test]
    async fn test_submit_signal_async_neutral_rejected() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::Neutral);
        let result = engine
            .submit_signal_async(&signal, Decimal::from(1), Decimal::from(50000))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_submit_order_async_paper_limit_stays_open() {
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let order = Order {
            id: "async-limit-1".into(),
            symbol: "BTCUSDT".into(),
            side: Side::Buy,
            order_type: OrderType::Limit,
            quantity: Decimal::from(1),
            price: Some(Decimal::from(40000)),
            stop_price: None,
            trailing_delta: None,
            time_in_force: None,
            reduce_only: false,
            status: OrderStatus::Pending,
            created_at: 0,
            updated_at: 0,
        };
        let result = engine
            .submit_order_async(order, Decimal::from(50000))
            .await
            .unwrap();
        assert_eq!(result.status, OrderStatus::Open);
        assert!(!engine.positions().has_position("BTCUSDT"));
    }

    #[test]
    fn test_paper_mode_sync_still_works_after_refactor() {
        // Ensure the sync path for paper mode is unchanged
        let mut engine = ExecutionEngine::new(ExecutionMode::Paper);
        let signal = test_signal(ssm_core::AIAction::EnterLong);
        let order = engine
            .submit_signal(&signal, Decimal::from(1), Decimal::from(50000))
            .unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
        assert_eq!(order.side, Side::Buy);
        assert!(engine.positions().has_position("BTCUSDT"));
    }

    #[test]
    fn test_live_mode_sync_backward_compat() {
        // The sync submit_order in live mode still returns Pending (backward compat)
        let mut engine = ExecutionEngine::new(ExecutionMode::Live);
        let signal = test_signal(ssm_core::AIAction::EnterShort);
        let order = engine
            .submit_signal(&signal, Decimal::from(1), Decimal::from(50000))
            .unwrap();
        assert_eq!(order.status, OrderStatus::Pending);
        assert!(!engine.positions().has_position("BTCUSDT"));
    }

    #[test]
    fn test_with_live_mainnet() {
        let live = LiveEngine::new("key".into(), "secret".into());
        let engine = ExecutionEngine::with_live(live);
        assert_eq!(
            engine.live_engine().unwrap().base_url(),
            "https://fapi.binance.com"
        );
    }
}
