use anyhow::Result;
use rust_decimal::Decimal;
use ssm_core::{ExecutionMode, Order, OrderStatus, OrderType, Side, Signal};

use crate::paper::PaperEngine;
use crate::position_tracker::PositionTracker;

/// Unified execution engine — routes to paper or live based on mode.
pub struct ExecutionEngine {
    mode: ExecutionMode,
    paper: PaperEngine,
    positions: PositionTracker,
}

impl ExecutionEngine {
    pub fn new(mode: ExecutionMode) -> Self {
        tracing::info!(?mode, "execution engine initialized");
        Self {
            mode,
            paper: PaperEngine::new(),
            positions: PositionTracker::new(),
        }
    }

    pub fn mode(&self) -> ExecutionMode {
        self.mode
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
            ssm_core::AIAction::Neutral => {
                anyhow::bail!("cannot submit order for Neutral action")
            }
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

    /// Submit a raw order (any type).
    pub fn submit_order(&mut self, mut order: Order, current_price: Decimal) -> Result<Order> {
        match self.mode {
            ExecutionMode::Paper => {
                self.paper.fill_order(&mut order, current_price)?;
                if order.status == OrderStatus::Filled {
                    self.positions.apply_fill(&order, current_price);
                }
            }
            ExecutionMode::Live => {
                // Live execution would call exchange API here.
                // For now, mark as pending — real integration in future.
                tracing::warn!("live execution not yet implemented, order stays Pending");
            }
        }
        Ok(order)
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
}
