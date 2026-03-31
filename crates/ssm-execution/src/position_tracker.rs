use rust_decimal::Decimal;
use ssm_core::{Order, Position, Side};
use std::collections::HashMap;

/// Tracks open positions per symbol.
pub struct PositionTracker {
    positions: HashMap<String, Position>,
    last_closed: Vec<String>,
}

impl PositionTracker {
    pub fn new() -> Self {
        Self {
            positions: HashMap::new(),
            last_closed: Vec::new(),
        }
    }

    pub fn get(&self, symbol: &str) -> Option<&Position> {
        self.positions.get(symbol)
    }

    pub fn all(&self) -> &HashMap<String, Position> {
        &self.positions
    }

    pub fn has_position(&self, symbol: &str) -> bool {
        self.positions
            .get(symbol)
            .map(|p| p.quantity > Decimal::ZERO)
            .unwrap_or(false)
    }

    /// Update position state after an order fill.
    /// After calling this, check `closed_symbols()` for positions that were fully closed.
    pub fn apply_fill(&mut self, order: &Order, fill_price: Decimal) {
        self.last_closed.clear();
        let now = chrono::Utc::now().timestamp_millis();

        // Determine what to do based on current position state
        enum Action {
            None,
            Remove,
            Flip(Position),
        }

        let action = if let Some(pos) = self.positions.get_mut(&order.symbol) {
            if pos.side == order.side {
                // Adding to position — average entry
                let total_cost = pos.entry_price * pos.quantity + fill_price * order.quantity;
                pos.quantity += order.quantity;
                if pos.quantity > Decimal::ZERO {
                    pos.entry_price = total_cost / pos.quantity;
                }
                Action::None
            } else {
                // Reducing position
                let pnl_per_unit = match pos.side {
                    Side::Buy => fill_price - pos.entry_price,
                    Side::Sell => pos.entry_price - fill_price,
                };
                let close_qty = order.quantity.min(pos.quantity);
                pos.realized_pnl += pnl_per_unit * close_qty;
                pos.quantity -= close_qty;

                if pos.quantity <= Decimal::ZERO {
                    let remainder = order.quantity - close_qty;
                    if remainder > Decimal::ZERO {
                        Action::Flip(Position {
                            symbol: order.symbol.clone(),
                            side: order.side,
                            entry_price: fill_price,
                            quantity: remainder,
                            unrealized_pnl: Decimal::ZERO,
                            realized_pnl: pos.realized_pnl,
                            leverage: pos.leverage,
                            opened_at: now,
                        })
                    } else {
                        Action::Remove
                    }
                } else {
                    Action::None
                }
            }
        } else {
            // New position
            self.positions.insert(
                order.symbol.clone(),
                Position {
                    symbol: order.symbol.clone(),
                    side: order.side,
                    entry_price: fill_price,
                    quantity: order.quantity,
                    unrealized_pnl: Decimal::ZERO,
                    realized_pnl: Decimal::ZERO,
                    leverage: 1,
                    opened_at: now,
                },
            );
            Action::None
        };

        match action {
            Action::Remove => {
                self.positions.remove(&order.symbol);
                self.last_closed.push(order.symbol.clone());
            }
            Action::Flip(new_pos) => {
                self.last_closed.push(order.symbol.clone());
                self.positions.insert(order.symbol.clone(), new_pos);
            }
            Action::None => {}
        }
    }

    /// Update unrealized PnL for all positions given current prices.
    pub fn mark_to_market(&mut self, prices: &HashMap<String, Decimal>) {
        for (symbol, pos) in &mut self.positions {
            if let Some(&price) = prices.get(symbol) {
                pos.unrealized_pnl = match pos.side {
                    Side::Buy => (price - pos.entry_price) * pos.quantity,
                    Side::Sell => (pos.entry_price - price) * pos.quantity,
                };
            }
        }
    }

    /// Restore a position from persistent storage (startup recovery).
    pub fn restore_position(&mut self, pos: Position) {
        self.positions.insert(pos.symbol.clone(), pos);
    }

    /// Returns the list of symbols that were removed (positions closed) in the last `apply_fill`.
    /// This is useful for persisting position removals to the store.
    pub fn closed_symbols(&self) -> &[String] {
        &self.last_closed
    }
}

impl Default for PositionTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use ssm_core::{OrderStatus, OrderType};

    fn fill_order(symbol: &str, side: Side, qty: Decimal, price: Decimal) -> Order {
        Order {
            id: "t".into(),
            symbol: symbol.into(),
            side,
            order_type: OrderType::Market,
            quantity: qty,
            price: Some(price),
            stop_price: None,
            trailing_delta: None,
            time_in_force: None,
            reduce_only: false,
            status: OrderStatus::Filled,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn open_and_close_position() {
        let mut tracker = PositionTracker::new();

        // Open long
        let buy = fill_order("BTCUSDT", Side::Buy, Decimal::from(2), Decimal::from(50000));
        tracker.apply_fill(&buy, Decimal::from(50000));
        assert!(tracker.has_position("BTCUSDT"));
        assert_eq!(tracker.get("BTCUSDT").unwrap().quantity, Decimal::from(2));

        // Close long at profit
        let sell = fill_order(
            "BTCUSDT",
            Side::Sell,
            Decimal::from(2),
            Decimal::from(51000),
        );
        tracker.apply_fill(&sell, Decimal::from(51000));
        assert!(!tracker.has_position("BTCUSDT"));
    }

    #[test]
    fn partial_close_with_pnl() {
        let mut tracker = PositionTracker::new();

        let buy = fill_order("ETHUSDT", Side::Buy, Decimal::from(10), Decimal::from(3000));
        tracker.apply_fill(&buy, Decimal::from(3000));

        // Close half at profit
        let sell = fill_order("ETHUSDT", Side::Sell, Decimal::from(5), Decimal::from(3200));
        tracker.apply_fill(&sell, Decimal::from(3200));

        let pos = tracker.get("ETHUSDT").unwrap();
        assert_eq!(pos.quantity, Decimal::from(5));
        // PnL: (3200 - 3000) * 5 = 1000
        assert_eq!(pos.realized_pnl, Decimal::from(1000));
    }

    #[test]
    fn mark_to_market_updates_unrealized() {
        let mut tracker = PositionTracker::new();

        let buy = fill_order("BTCUSDT", Side::Buy, Decimal::from(1), Decimal::from(50000));
        tracker.apply_fill(&buy, Decimal::from(50000));

        let mut prices = HashMap::new();
        prices.insert("BTCUSDT".into(), Decimal::from(52000));
        tracker.mark_to_market(&prices);

        assert_eq!(
            tracker.get("BTCUSDT").unwrap().unrealized_pnl,
            Decimal::from(2000)
        );
    }

    #[test]
    fn position_flip() {
        let mut tracker = PositionTracker::new();

        // Open long 2
        let buy = fill_order("BTCUSDT", Side::Buy, Decimal::from(2), Decimal::from(50000));
        tracker.apply_fill(&buy, Decimal::from(50000));

        // Sell 3 — should close long and open short 1
        let sell = fill_order(
            "BTCUSDT",
            Side::Sell,
            Decimal::from(3),
            Decimal::from(51000),
        );
        tracker.apply_fill(&sell, Decimal::from(51000));

        let pos = tracker.get("BTCUSDT").unwrap();
        assert_eq!(pos.side, Side::Sell);
        assert_eq!(pos.quantity, Decimal::from(1));
    }

    #[test]
    fn test_no_position_initially() {
        let tracker = PositionTracker::new();
        assert!(tracker.get("BTCUSDT").is_none());
        assert!(!tracker.has_position("BTCUSDT"));
    }

    #[test]
    fn test_add_to_position() {
        let mut tracker = PositionTracker::new();

        let buy1 = fill_order("BTCUSDT", Side::Buy, Decimal::from(1), Decimal::from(50000));
        tracker.apply_fill(&buy1, Decimal::from(50000));

        let buy2 = fill_order("BTCUSDT", Side::Buy, Decimal::from(1), Decimal::from(50000));
        tracker.apply_fill(&buy2, Decimal::from(50000));

        let pos = tracker.get("BTCUSDT").unwrap();
        assert_eq!(pos.quantity, Decimal::from(2));
    }

    #[test]
    fn test_entry_price_averaging() {
        let mut tracker = PositionTracker::new();

        let buy1 = fill_order("BTCUSDT", Side::Buy, Decimal::from(1), Decimal::from(100));
        tracker.apply_fill(&buy1, Decimal::from(100));

        let buy2 = fill_order("BTCUSDT", Side::Buy, Decimal::from(1), Decimal::from(200));
        tracker.apply_fill(&buy2, Decimal::from(200));

        let pos = tracker.get("BTCUSDT").unwrap();
        assert_eq!(pos.entry_price, Decimal::from(150));
        assert_eq!(pos.quantity, Decimal::from(2));
    }

    #[test]
    fn test_short_position_pnl() {
        let mut tracker = PositionTracker::new();

        // Open short at 100
        let sell = fill_order("BTCUSDT", Side::Sell, Decimal::from(2), Decimal::from(100));
        tracker.apply_fill(&sell, Decimal::from(100));

        // Close short at 90 — profit
        let buy = fill_order("BTCUSDT", Side::Buy, Decimal::from(2), Decimal::from(90));
        tracker.apply_fill(&buy, Decimal::from(90));

        // Position closed, PnL = (100 - 90) * 2 = 20
        assert!(!tracker.has_position("BTCUSDT"));
    }

    #[test]
    fn test_short_position_loss() {
        let mut tracker = PositionTracker::new();

        // Open short at 100
        let sell = fill_order("BTCUSDT", Side::Sell, Decimal::from(1), Decimal::from(100));
        tracker.apply_fill(&sell, Decimal::from(100));

        // Partial close at 110 — loss
        let buy = fill_order("BTCUSDT", Side::Buy, Decimal::from(1), Decimal::from(110));
        tracker.apply_fill(&buy, Decimal::from(110));

        // Position closed with negative PnL
        assert!(!tracker.has_position("BTCUSDT"));
    }

    #[test]
    fn test_mark_to_market_short() {
        let mut tracker = PositionTracker::new();

        // Open short at 100
        let sell = fill_order("BTCUSDT", Side::Sell, Decimal::from(1), Decimal::from(100));
        tracker.apply_fill(&sell, Decimal::from(100));

        // Price drops to 90 — positive unrealized for short
        let mut prices = HashMap::new();
        prices.insert("BTCUSDT".into(), Decimal::from(90));
        tracker.mark_to_market(&prices);

        let pos = tracker.get("BTCUSDT").unwrap();
        // unrealized = (entry - current) * qty = (100 - 90) * 1 = 10
        assert_eq!(pos.unrealized_pnl, Decimal::from(10));
    }

    #[test]
    fn test_multiple_symbols() {
        let mut tracker = PositionTracker::new();

        let btc = fill_order("BTCUSDT", Side::Buy, Decimal::from(1), Decimal::from(50000));
        tracker.apply_fill(&btc, Decimal::from(50000));

        let eth = fill_order("ETHUSDT", Side::Sell, Decimal::from(5), Decimal::from(3000));
        tracker.apply_fill(&eth, Decimal::from(3000));

        assert!(tracker.has_position("BTCUSDT"));
        assert!(tracker.has_position("ETHUSDT"));
        assert_eq!(tracker.get("BTCUSDT").unwrap().side, Side::Buy);
        assert_eq!(tracker.get("ETHUSDT").unwrap().side, Side::Sell);
    }

    #[test]
    fn test_has_position_false_after_close() {
        let mut tracker = PositionTracker::new();

        let buy = fill_order("BTCUSDT", Side::Buy, Decimal::from(1), Decimal::from(50000));
        tracker.apply_fill(&buy, Decimal::from(50000));
        assert!(tracker.has_position("BTCUSDT"));

        let sell = fill_order(
            "BTCUSDT",
            Side::Sell,
            Decimal::from(1),
            Decimal::from(51000),
        );
        tracker.apply_fill(&sell, Decimal::from(51000));
        assert!(!tracker.has_position("BTCUSDT"));
    }

    #[test]
    fn test_all_returns_all_positions() {
        let mut tracker = PositionTracker::new();

        let btc = fill_order("BTCUSDT", Side::Buy, Decimal::from(1), Decimal::from(50000));
        tracker.apply_fill(&btc, Decimal::from(50000));

        let eth = fill_order("ETHUSDT", Side::Buy, Decimal::from(1), Decimal::from(3000));
        tracker.apply_fill(&eth, Decimal::from(3000));

        let sol = fill_order("SOLUSDT", Side::Sell, Decimal::from(10), Decimal::from(100));
        tracker.apply_fill(&sol, Decimal::from(100));

        assert_eq!(tracker.all().len(), 3);
        assert!(tracker.all().contains_key("BTCUSDT"));
        assert!(tracker.all().contains_key("ETHUSDT"));
        assert!(tracker.all().contains_key("SOLUSDT"));
    }

    #[test]
    fn test_position_flip_long_to_short() {
        let mut tracker = PositionTracker::new();

        // Open long 3
        let buy = fill_order("BTCUSDT", Side::Buy, Decimal::from(3), Decimal::from(50000));
        tracker.apply_fill(&buy, Decimal::from(50000));
        assert_eq!(tracker.get("BTCUSDT").unwrap().side, Side::Buy);

        // Sell 5 — close long 3, open short 2
        let sell = fill_order(
            "BTCUSDT",
            Side::Sell,
            Decimal::from(5),
            Decimal::from(52000),
        );
        tracker.apply_fill(&sell, Decimal::from(52000));

        let pos = tracker.get("BTCUSDT").unwrap();
        assert_eq!(pos.side, Side::Sell);
        assert_eq!(pos.quantity, Decimal::from(2));
        assert_eq!(pos.entry_price, Decimal::from(52000));
    }

    #[test]
    fn test_position_flip_short_to_long() {
        let mut tracker = PositionTracker::new();

        // Open short 2
        let sell = fill_order(
            "BTCUSDT",
            Side::Sell,
            Decimal::from(2),
            Decimal::from(50000),
        );
        tracker.apply_fill(&sell, Decimal::from(50000));
        assert_eq!(tracker.get("BTCUSDT").unwrap().side, Side::Sell);

        // Buy 5 — close short 2, open long 3
        let buy = fill_order("BTCUSDT", Side::Buy, Decimal::from(5), Decimal::from(48000));
        tracker.apply_fill(&buy, Decimal::from(48000));

        let pos = tracker.get("BTCUSDT").unwrap();
        assert_eq!(pos.side, Side::Buy);
        assert_eq!(pos.quantity, Decimal::from(3));
        assert_eq!(pos.entry_price, Decimal::from(48000));
    }

    #[test]
    fn test_position_flip_preserves_realized_pnl() {
        let mut tracker = PositionTracker::new();

        // Open long 2 at 50000
        let buy = fill_order("BTCUSDT", Side::Buy, Decimal::from(2), Decimal::from(50000));
        tracker.apply_fill(&buy, Decimal::from(50000));

        // Sell 3 at 52000 — close long 2 (profit 4000), open short 1
        let sell = fill_order(
            "BTCUSDT",
            Side::Sell,
            Decimal::from(3),
            Decimal::from(52000),
        );
        tracker.apply_fill(&sell, Decimal::from(52000));

        let pos = tracker.get("BTCUSDT").unwrap();
        // realized_pnl from closing long: (52000 - 50000) * 2 = 4000
        assert_eq!(pos.realized_pnl, Decimal::from(4000));
    }

    #[test]
    fn test_partial_fill_reduces_quantity() {
        let mut tracker = PositionTracker::new();

        let buy = fill_order(
            "BTCUSDT",
            Side::Buy,
            Decimal::from(10),
            Decimal::from(50000),
        );
        tracker.apply_fill(&buy, Decimal::from(50000));

        // Close 3 of 10
        let sell = fill_order(
            "BTCUSDT",
            Side::Sell,
            Decimal::from(3),
            Decimal::from(51000),
        );
        tracker.apply_fill(&sell, Decimal::from(51000));

        let pos = tracker.get("BTCUSDT").unwrap();
        assert_eq!(pos.quantity, Decimal::from(7));
        assert_eq!(pos.side, Side::Buy);
        // PnL: (51000 - 50000) * 3 = 3000
        assert_eq!(pos.realized_pnl, Decimal::from(3000));
    }

    #[test]
    fn test_multiple_partial_fills() {
        let mut tracker = PositionTracker::new();

        let buy = fill_order(
            "BTCUSDT",
            Side::Buy,
            Decimal::from(10),
            Decimal::from(50000),
        );
        tracker.apply_fill(&buy, Decimal::from(50000));

        // Close 3
        let sell1 = fill_order(
            "BTCUSDT",
            Side::Sell,
            Decimal::from(3),
            Decimal::from(51000),
        );
        tracker.apply_fill(&sell1, Decimal::from(51000));

        // Close 4
        let sell2 = fill_order(
            "BTCUSDT",
            Side::Sell,
            Decimal::from(4),
            Decimal::from(52000),
        );
        tracker.apply_fill(&sell2, Decimal::from(52000));

        let pos = tracker.get("BTCUSDT").unwrap();
        assert_eq!(pos.quantity, Decimal::from(3));
        // PnL: (51000-50000)*3 + (52000-50000)*4 = 3000 + 8000 = 11000
        assert_eq!(pos.realized_pnl, Decimal::from(11000));
    }

    #[test]
    fn test_mark_to_market_with_no_positions() {
        let mut tracker = PositionTracker::new();
        let mut prices = HashMap::new();
        prices.insert("BTCUSDT".into(), Decimal::from(50000));
        // Should not panic
        tracker.mark_to_market(&prices);
        assert!(tracker.all().is_empty());
    }

    #[test]
    fn test_mark_to_market_missing_price() {
        let mut tracker = PositionTracker::new();

        let buy = fill_order("BTCUSDT", Side::Buy, Decimal::from(1), Decimal::from(50000));
        tracker.apply_fill(&buy, Decimal::from(50000));

        // Mark to market with a different symbol — BTCUSDT unrealized should stay 0
        let mut prices = HashMap::new();
        prices.insert("ETHUSDT".into(), Decimal::from(3000));
        tracker.mark_to_market(&prices);

        assert_eq!(
            tracker.get("BTCUSDT").unwrap().unrealized_pnl,
            Decimal::ZERO
        );
    }

    #[test]
    fn test_mark_to_market_multiple_symbols() {
        let mut tracker = PositionTracker::new();

        let btc = fill_order("BTCUSDT", Side::Buy, Decimal::from(1), Decimal::from(50000));
        tracker.apply_fill(&btc, Decimal::from(50000));

        let eth = fill_order(
            "ETHUSDT",
            Side::Sell,
            Decimal::from(10),
            Decimal::from(3000),
        );
        tracker.apply_fill(&eth, Decimal::from(3000));

        let mut prices = HashMap::new();
        prices.insert("BTCUSDT".into(), Decimal::from(55000));
        prices.insert("ETHUSDT".into(), Decimal::from(2800));
        tracker.mark_to_market(&prices);

        // BTC long: (55000 - 50000) * 1 = 5000
        assert_eq!(
            tracker.get("BTCUSDT").unwrap().unrealized_pnl,
            Decimal::from(5000)
        );
        // ETH short: (3000 - 2800) * 10 = 2000
        assert_eq!(
            tracker.get("ETHUSDT").unwrap().unrealized_pnl,
            Decimal::from(2000)
        );
    }

    #[test]
    fn test_mark_to_market_negative_unrealized() {
        let mut tracker = PositionTracker::new();

        let buy = fill_order("BTCUSDT", Side::Buy, Decimal::from(2), Decimal::from(50000));
        tracker.apply_fill(&buy, Decimal::from(50000));

        let mut prices = HashMap::new();
        prices.insert("BTCUSDT".into(), Decimal::from(48000));
        tracker.mark_to_market(&prices);

        // (48000 - 50000) * 2 = -4000
        assert_eq!(
            tracker.get("BTCUSDT").unwrap().unrealized_pnl,
            Decimal::from(-4000)
        );
    }

    #[test]
    fn test_default_tracker() {
        let tracker = PositionTracker::default();
        assert!(tracker.all().is_empty());
        assert!(!tracker.has_position("BTCUSDT"));
    }

    #[test]
    fn test_entry_price_weighted_average_unequal_quantities() {
        let mut tracker = PositionTracker::new();

        // Buy 1 at 100
        let buy1 = fill_order("BTCUSDT", Side::Buy, Decimal::from(1), Decimal::from(100));
        tracker.apply_fill(&buy1, Decimal::from(100));

        // Buy 3 at 200
        let buy2 = fill_order("BTCUSDT", Side::Buy, Decimal::from(3), Decimal::from(200));
        tracker.apply_fill(&buy2, Decimal::from(200));

        let pos = tracker.get("BTCUSDT").unwrap();
        assert_eq!(pos.quantity, Decimal::from(4));
        // Weighted average: (100*1 + 200*3) / 4 = 700/4 = 175
        assert_eq!(pos.entry_price, Decimal::from(175));
    }

    #[test]
    fn test_exact_close_removes_position() {
        let mut tracker = PositionTracker::new();

        let buy = fill_order("BTCUSDT", Side::Buy, Decimal::from(5), Decimal::from(50000));
        tracker.apply_fill(&buy, Decimal::from(50000));

        let sell = fill_order(
            "BTCUSDT",
            Side::Sell,
            Decimal::from(5),
            Decimal::from(50000),
        );
        tracker.apply_fill(&sell, Decimal::from(50000));

        // Position should be completely removed
        assert!(tracker.get("BTCUSDT").is_none());
        assert!(!tracker.has_position("BTCUSDT"));
    }

    #[test]
    fn test_short_position_partial_close_pnl() {
        let mut tracker = PositionTracker::new();

        // Open short 10 at 100
        let sell = fill_order("BTCUSDT", Side::Sell, Decimal::from(10), Decimal::from(100));
        tracker.apply_fill(&sell, Decimal::from(100));

        // Close 4 at 90 — profit
        let buy = fill_order("BTCUSDT", Side::Buy, Decimal::from(4), Decimal::from(90));
        tracker.apply_fill(&buy, Decimal::from(90));

        let pos = tracker.get("BTCUSDT").unwrap();
        assert_eq!(pos.quantity, Decimal::from(6));
        assert_eq!(pos.side, Side::Sell);
        // PnL: (100 - 90) * 4 = 40
        assert_eq!(pos.realized_pnl, Decimal::from(40));
    }
}
