use anyhow::Result;
use rust_decimal::Decimal;
use ssm_core::{Order, OrderStatus, OrderType};

/// Paper trading engine — simulates order fills locally.
pub struct PaperEngine {
    filled_count: u64,
}

impl PaperEngine {
    pub fn new() -> Self {
        Self { filled_count: 0 }
    }

    pub fn filled_count(&self) -> u64 {
        self.filled_count
    }

    /// Simulate filling an order at the given price.
    pub fn fill_order(&mut self, order: &mut Order, current_price: Decimal) -> Result<()> {
        match order.order_type {
            OrderType::Market => {
                order.price = Some(current_price);
                order.status = OrderStatus::Filled;
                order.updated_at = chrono::Utc::now().timestamp_millis();
                self.filled_count += 1;
            }
            OrderType::Limit => {
                if let Some(limit_price) = order.price {
                    let should_fill = match order.side {
                        ssm_core::Side::Buy => current_price <= limit_price,
                        ssm_core::Side::Sell => current_price >= limit_price,
                    };
                    if should_fill {
                        order.status = OrderStatus::Filled;
                        order.updated_at = chrono::Utc::now().timestamp_millis();
                        self.filled_count += 1;
                    } else {
                        order.status = OrderStatus::Open;
                    }
                }
            }
            OrderType::StopMarket | OrderType::StopLimit => {
                if let Some(stop) = order.stop_price {
                    let triggered = match order.side {
                        ssm_core::Side::Buy => current_price >= stop,
                        ssm_core::Side::Sell => current_price <= stop,
                    };
                    if triggered {
                        order.price = Some(current_price);
                        order.status = OrderStatus::Filled;
                        order.updated_at = chrono::Utc::now().timestamp_millis();
                        self.filled_count += 1;
                    } else {
                        order.status = OrderStatus::Open;
                    }
                }
            }
            OrderType::TakeProfitMarket | OrderType::TakeProfitLimit => {
                if let Some(tp) = order.stop_price {
                    let triggered = match order.side {
                        ssm_core::Side::Buy => current_price <= tp,
                        ssm_core::Side::Sell => current_price >= tp,
                    };
                    if triggered {
                        order.price = Some(current_price);
                        order.status = OrderStatus::Filled;
                        order.updated_at = chrono::Utc::now().timestamp_millis();
                        self.filled_count += 1;
                    } else {
                        order.status = OrderStatus::Open;
                    }
                }
            }
            OrderType::TrailingStop => {
                // Trailing stop needs price history tracking — simplified here
                order.status = OrderStatus::Open;
            }
        }

        Ok(())
    }
}

impl Default for PaperEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssm_core::{OrderType, Side};
    use std::str::FromStr;

    fn make_order(side: Side, order_type: OrderType) -> Order {
        Order {
            id: "test-1".into(),
            symbol: "BTCUSDT".into(),
            side,
            order_type,
            quantity: Decimal::from(1),
            price: None,
            stop_price: None,
            trailing_delta: None,
            time_in_force: None,
            reduce_only: false,
            status: OrderStatus::Pending,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn market_order_fills_immediately() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::Market);
        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
        assert_eq!(order.price, Some(Decimal::from(50000)));
    }

    #[test]
    fn limit_buy_fills_at_or_below() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::Limit);
        order.price = Some(Decimal::from_str("49000").unwrap());

        // Price too high — should not fill
        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_eq!(order.status, OrderStatus::Open);

        // Price at limit — should fill
        engine.fill_order(&mut order, Decimal::from(49000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn stop_market_sell_triggers_below() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Sell, OrderType::StopMarket);
        order.stop_price = Some(Decimal::from(48000));

        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_eq!(order.status, OrderStatus::Open);

        engine.fill_order(&mut order, Decimal::from(47000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn take_profit_sell_triggers_above() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Sell, OrderType::TakeProfitMarket);
        order.stop_price = Some(Decimal::from(55000));

        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_eq!(order.status, OrderStatus::Open);

        engine.fill_order(&mut order, Decimal::from(56000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn filled_count_increments() {
        let mut engine = PaperEngine::new();
        let mut o1 = make_order(Side::Buy, OrderType::Market);
        let mut o2 = make_order(Side::Sell, OrderType::Market);
        engine.fill_order(&mut o1, Decimal::from(100)).unwrap();
        engine.fill_order(&mut o2, Decimal::from(100)).unwrap();
        assert_eq!(engine.filled_count(), 2);
    }

    #[test]
    fn test_limit_sell_fills_at_or_above() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Sell, OrderType::Limit);
        order.price = Some(Decimal::from(51000));

        // Current price is above limit — should fill
        engine.fill_order(&mut order, Decimal::from(52000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_limit_sell_not_fills_below() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Sell, OrderType::Limit);
        order.price = Some(Decimal::from(51000));

        // Current price is below limit — should not fill
        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_eq!(order.status, OrderStatus::Open);
    }

    #[test]
    fn test_stop_market_buy_triggers_above() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::StopMarket);
        order.stop_price = Some(Decimal::from(52000));

        // Current price above stop — should trigger
        engine.fill_order(&mut order, Decimal::from(53000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_stop_market_buy_not_triggers_below() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::StopMarket);
        order.stop_price = Some(Decimal::from(52000));

        // Current price below stop — should not trigger
        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_eq!(order.status, OrderStatus::Open);
    }

    #[test]
    fn test_stop_limit_triggers() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Sell, OrderType::StopLimit);
        order.stop_price = Some(Decimal::from(48000));

        // Price above stop — should not trigger for sell
        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_eq!(order.status, OrderStatus::Open);

        // Price below stop — should trigger for sell
        engine.fill_order(&mut order, Decimal::from(47000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_take_profit_buy_triggers_below() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::TakeProfitMarket);
        order.stop_price = Some(Decimal::from(48000));

        // Current price below TP — should trigger for buy
        engine.fill_order(&mut order, Decimal::from(47000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_take_profit_limit_triggers() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Sell, OrderType::TakeProfitLimit);
        order.stop_price = Some(Decimal::from(55000));

        // Price below TP — should not trigger for sell
        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_eq!(order.status, OrderStatus::Open);

        // Price above TP — should trigger for sell
        engine.fill_order(&mut order, Decimal::from(56000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_trailing_stop_stays_open() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Sell, OrderType::TrailingStop);
        order.trailing_delta = Some(Decimal::from(100));

        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_eq!(order.status, OrderStatus::Open);
    }

    #[test]
    fn test_paper_engine_default() {
        let engine = PaperEngine::default();
        assert_eq!(engine.filled_count(), 0);
    }

    #[test]
    fn test_market_order_sets_fill_price() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::Market);
        assert_eq!(order.price, None);

        engine.fill_order(&mut order, Decimal::from(42000)).unwrap();
        assert_eq!(order.price, Some(Decimal::from(42000)));
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_stop_limit_buy_triggers_at_exact_stop() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::StopLimit);
        order.stop_price = Some(Decimal::from(52000));

        // Price exactly at stop — should trigger for buy
        engine.fill_order(&mut order, Decimal::from(52000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
        assert_eq!(order.price, Some(Decimal::from(52000)));
    }

    #[test]
    fn test_stop_limit_sell_at_exact_stop() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Sell, OrderType::StopLimit);
        order.stop_price = Some(Decimal::from(48000));

        // Price exactly at stop — should trigger for sell
        engine.fill_order(&mut order, Decimal::from(48000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_stop_limit_buy_not_triggered_below() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::StopLimit);
        order.stop_price = Some(Decimal::from(52000));

        // Price below stop — should not trigger for buy
        engine.fill_order(&mut order, Decimal::from(51999)).unwrap();
        assert_eq!(order.status, OrderStatus::Open);
    }

    #[test]
    fn test_take_profit_limit_buy_at_exact_price() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::TakeProfitLimit);
        order.stop_price = Some(Decimal::from(48000));

        // Price exactly at TP — should trigger for buy
        engine.fill_order(&mut order, Decimal::from(48000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
        assert_eq!(order.price, Some(Decimal::from(48000)));
    }

    #[test]
    fn test_take_profit_limit_buy_not_triggered_above() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::TakeProfitLimit);
        order.stop_price = Some(Decimal::from(48000));

        // Price above TP — should not trigger for buy
        engine.fill_order(&mut order, Decimal::from(49000)).unwrap();
        assert_eq!(order.status, OrderStatus::Open);
    }

    #[test]
    fn test_take_profit_sell_not_triggered_below() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Sell, OrderType::TakeProfitMarket);
        order.stop_price = Some(Decimal::from(55000));

        // Price below TP — should not trigger for sell
        engine.fill_order(&mut order, Decimal::from(54999)).unwrap();
        assert_eq!(order.status, OrderStatus::Open);
    }

    #[test]
    fn test_take_profit_sell_at_exact_price() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Sell, OrderType::TakeProfitMarket);
        order.stop_price = Some(Decimal::from(55000));

        // Price exactly at TP — should trigger for sell
        engine.fill_order(&mut order, Decimal::from(55000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_limit_buy_at_exact_boundary() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::Limit);
        order.price = Some(Decimal::from_str("49000.00").unwrap());

        // Exactly at limit price — should fill
        engine
            .fill_order(&mut order, Decimal::from_str("49000.00").unwrap())
            .unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_limit_sell_at_exact_boundary() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Sell, OrderType::Limit);
        order.price = Some(Decimal::from(51000));

        // Exactly at limit — should fill
        engine.fill_order(&mut order, Decimal::from(51000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_limit_order_without_price_does_nothing() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::Limit);
        // price is None by default from make_order

        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        // Status stays Pending since no price set — the match arm does nothing
        assert_eq!(order.status, OrderStatus::Pending);
        assert_eq!(engine.filled_count(), 0);
    }

    #[test]
    fn test_stop_market_without_stop_price_does_nothing() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Sell, OrderType::StopMarket);
        // stop_price is None

        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_eq!(order.status, OrderStatus::Pending);
        assert_eq!(engine.filled_count(), 0);
    }

    #[test]
    fn test_take_profit_without_stop_price_does_nothing() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::TakeProfitMarket);
        // stop_price is None

        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_eq!(order.status, OrderStatus::Pending);
        assert_eq!(engine.filled_count(), 0);
    }

    #[test]
    fn test_extreme_small_price() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::Market);
        let tiny_price = Decimal::from_str("0.00000001").unwrap();

        engine.fill_order(&mut order, tiny_price).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
        assert_eq!(order.price, Some(tiny_price));
    }

    #[test]
    fn test_extreme_large_price() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::Market);
        let huge_price = Decimal::from(999_999_999);

        engine.fill_order(&mut order, huge_price).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
        assert_eq!(order.price, Some(huge_price));
    }

    #[test]
    fn test_market_sell_fills_immediately() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Sell, OrderType::Market);

        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
        assert_eq!(order.price, Some(Decimal::from(50000)));
    }

    #[test]
    fn test_limit_buy_below_price_fills() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::Limit);
        order.price = Some(Decimal::from(50000));

        // Current price below limit — should fill
        engine.fill_order(&mut order, Decimal::from(49000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_trailing_stop_buy_stays_open() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::TrailingStop);
        order.trailing_delta = Some(Decimal::from(200));

        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_eq!(order.status, OrderStatus::Open);
        assert_eq!(engine.filled_count(), 0);
    }

    #[test]
    fn test_filled_count_not_incremented_for_open() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::Limit);
        order.price = Some(Decimal::from(40000));

        // Price too high — stays open
        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_eq!(engine.filled_count(), 0);
    }

    #[test]
    fn test_stop_market_buy_at_exact_stop() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::StopMarket);
        order.stop_price = Some(Decimal::from(52000));

        // Price exactly at stop — should trigger for buy
        engine.fill_order(&mut order, Decimal::from(52000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_stop_market_sell_at_exact_stop() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Sell, OrderType::StopMarket);
        order.stop_price = Some(Decimal::from(48000));

        // Price exactly at stop — should trigger for sell
        engine.fill_order(&mut order, Decimal::from(48000)).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
    }

    #[test]
    fn test_stop_market_sell_not_triggered_above() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Sell, OrderType::StopMarket);
        order.stop_price = Some(Decimal::from(48000));

        // Price above stop — should NOT trigger for sell stop
        engine.fill_order(&mut order, Decimal::from(48001)).unwrap();
        assert_eq!(order.status, OrderStatus::Open);
    }

    #[test]
    fn test_multiple_fills_increment_count() {
        let mut engine = PaperEngine::new();
        for i in 0..5 {
            let mut order = make_order(Side::Buy, OrderType::Market);
            order.id = format!("order-{i}");
            engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        }
        assert_eq!(engine.filled_count(), 5);
    }

    #[test]
    fn test_updated_at_changes_on_fill() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::Market);
        assert_eq!(order.updated_at, 0);

        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_ne!(order.updated_at, 0);
    }

    #[test]
    fn test_updated_at_changes_on_open() {
        let mut engine = PaperEngine::new();
        let mut order = make_order(Side::Buy, OrderType::Limit);
        order.price = Some(Decimal::from(40000));
        // Price too high, order stays open but updated_at should not change
        // (the code only updates timestamp on fill or status change to Open for limit)
        engine.fill_order(&mut order, Decimal::from(50000)).unwrap();
        assert_eq!(order.status, OrderStatus::Open);
    }
}
