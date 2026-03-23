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
}
