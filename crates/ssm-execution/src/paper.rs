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
}
