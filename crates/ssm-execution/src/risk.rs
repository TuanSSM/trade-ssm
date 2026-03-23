use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use ssm_core::Side;
use ssm_core::{Order, Position};
use std::collections::HashMap;

/// Risk management configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    /// Maximum position size per symbol (in base asset).
    pub max_position_size: Decimal,
    /// Maximum total exposure across all symbols (in quote currency).
    pub max_total_exposure: Decimal,
    /// Maximum drawdown percentage before circuit breaker triggers (e.g., 0.10 = 10%).
    pub max_drawdown_pct: Decimal,
    /// Maximum number of open positions.
    pub max_open_positions: usize,
    /// Fixed fractional position sizing (fraction of balance per trade).
    pub position_size_fraction: Decimal,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_position_size: Decimal::from(10),
            max_total_exposure: Decimal::from(1_000_000),
            max_drawdown_pct: Decimal::new(10, 2), // 10%
            max_open_positions: 5,
            position_size_fraction: Decimal::new(2, 2), // 2%
        }
    }
}

/// Risk check result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskCheck {
    Approved,
    Rejected(String),
}

/// Risk manager — validates orders against risk limits.
pub struct RiskManager {
    config: RiskConfig,
    peak_equity: Decimal,
    circuit_breaker_active: bool,
}

impl RiskManager {
    pub fn new(config: RiskConfig, initial_equity: Decimal) -> Self {
        Self {
            config,
            peak_equity: initial_equity,
            circuit_breaker_active: false,
        }
    }

    pub fn is_circuit_breaker_active(&self) -> bool {
        self.circuit_breaker_active
    }

    /// Update equity tracking. Activates circuit breaker if drawdown exceeds limit.
    pub fn update_equity(&mut self, current_equity: Decimal) {
        if current_equity > self.peak_equity {
            self.peak_equity = current_equity;
        }

        if self.peak_equity > Decimal::ZERO {
            let drawdown = (self.peak_equity - current_equity) / self.peak_equity;
            if drawdown >= self.config.max_drawdown_pct {
                if !self.circuit_breaker_active {
                    tracing::warn!(
                        drawdown = %drawdown,
                        limit = %self.config.max_drawdown_pct,
                        "circuit breaker activated"
                    );
                }
                self.circuit_breaker_active = true;
            }
        }
    }

    /// Reset circuit breaker (manual override).
    pub fn reset_circuit_breaker(&mut self, new_equity: Decimal) {
        self.circuit_breaker_active = false;
        self.peak_equity = new_equity;
        tracing::info!("circuit breaker reset");
    }

    /// Validate an order against all risk limits.
    pub fn check_order(
        &self,
        order: &Order,
        positions: &HashMap<String, Position>,
        current_price: Decimal,
    ) -> RiskCheck {
        if self.circuit_breaker_active {
            return RiskCheck::Rejected("circuit breaker active — max drawdown exceeded".into());
        }

        // Check max open positions
        if !positions.contains_key(&order.symbol)
            && positions.len() >= self.config.max_open_positions
        {
            return RiskCheck::Rejected(format!(
                "max open positions ({}) reached",
                self.config.max_open_positions
            ));
        }

        // Check max position size
        if let Some(pos) = positions.get(&order.symbol) {
            if pos.side == order.side {
                let new_size = pos.quantity + order.quantity;
                if new_size > self.config.max_position_size {
                    return RiskCheck::Rejected(format!(
                        "position size {} would exceed max {}",
                        new_size, self.config.max_position_size
                    ));
                }
            }
        } else if order.quantity > self.config.max_position_size {
            return RiskCheck::Rejected(format!(
                "order size {} exceeds max position size {}",
                order.quantity, self.config.max_position_size
            ));
        }

        // Check total exposure
        let current_exposure: Decimal =
            positions.values().map(|p| p.quantity * p.entry_price).sum();
        let new_exposure = current_exposure + order.quantity * current_price;
        if new_exposure > self.config.max_total_exposure {
            return RiskCheck::Rejected(format!(
                "total exposure {} would exceed max {}",
                new_exposure, self.config.max_total_exposure
            ));
        }

        RiskCheck::Approved
    }

    /// Calculate position size using fixed fractional method.
    ///
    /// size = (balance * fraction) / (entry_price * risk_per_unit)
    pub fn calculate_position_size(
        &self,
        balance: Decimal,
        entry_price: Decimal,
        stop_distance: Decimal,
    ) -> Decimal {
        if entry_price.is_zero() || stop_distance.is_zero() {
            return Decimal::ZERO;
        }

        let risk_amount = balance * self.config.position_size_fraction;
        let size = risk_amount / stop_distance;

        // Clamp to max position size
        size.min(self.config.max_position_size)
    }

    /// Calculate position size using Kelly criterion.
    ///
    /// kelly_fraction = (win_rate * avg_win - (1 - win_rate) * avg_loss) / avg_win
    pub fn kelly_position_size(
        &self,
        balance: Decimal,
        win_rate: Decimal,
        avg_win: Decimal,
        avg_loss: Decimal,
        entry_price: Decimal,
    ) -> Decimal {
        if avg_win.is_zero() || entry_price.is_zero() {
            return Decimal::ZERO;
        }

        let loss_rate = Decimal::ONE - win_rate;
        let kelly = (win_rate * avg_win - loss_rate * avg_loss) / avg_win;

        // Half-Kelly for safety
        let half_kelly = kelly / Decimal::from(2);
        if half_kelly <= Decimal::ZERO {
            return Decimal::ZERO;
        }

        let risk_amount = balance * half_kelly;
        let size = risk_amount / entry_price;

        size.min(self.config.max_position_size)
    }
}

/// A bracket order: entry + stop-loss + take-profit as an atomic unit.
#[derive(Debug, Clone)]
pub struct BracketOrder {
    pub entry: Order,
    pub stop_loss: Order,
    pub take_profit: Order,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssm_core::{OrderStatus, OrderType};

    fn make_order(symbol: &str, side: Side, qty: u32) -> Order {
        Order {
            id: "test".into(),
            symbol: symbol.into(),
            side,
            order_type: OrderType::Market,
            quantity: Decimal::from(qty),
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

    fn make_position(symbol: &str, side: Side, qty: u32, entry: u32) -> Position {
        Position {
            symbol: symbol.into(),
            side,
            entry_price: Decimal::from(entry),
            quantity: Decimal::from(qty),
            unrealized_pnl: Decimal::ZERO,
            realized_pnl: Decimal::ZERO,
            leverage: 1,
            opened_at: 0,
        }
    }

    #[test]
    fn approve_within_limits() {
        let rm = RiskManager::new(RiskConfig::default(), Decimal::from(100_000));
        let order = make_order("BTCUSDT", Side::Buy, 1);
        let positions = HashMap::new();
        assert_eq!(
            rm.check_order(&order, &positions, Decimal::from(50_000)),
            RiskCheck::Approved
        );
    }

    #[test]
    fn reject_exceeds_position_size() {
        let config = RiskConfig {
            max_position_size: Decimal::from(5),
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(100_000));
        let order = make_order("BTCUSDT", Side::Buy, 10);
        let positions = HashMap::new();
        assert!(matches!(
            rm.check_order(&order, &positions, Decimal::from(50_000)),
            RiskCheck::Rejected(_)
        ));
    }

    #[test]
    fn reject_max_positions() {
        let config = RiskConfig {
            max_open_positions: 2,
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(100_000));
        let mut positions = HashMap::new();
        positions.insert(
            "BTCUSDT".into(),
            make_position("BTCUSDT", Side::Buy, 1, 50000),
        );
        positions.insert(
            "ETHUSDT".into(),
            make_position("ETHUSDT", Side::Buy, 1, 3000),
        );

        let order = make_order("SOLUSDT", Side::Buy, 1);
        assert!(matches!(
            rm.check_order(&order, &positions, Decimal::from(100)),
            RiskCheck::Rejected(_)
        ));
    }

    #[test]
    fn circuit_breaker_triggers() {
        let config = RiskConfig {
            max_drawdown_pct: Decimal::new(10, 2), // 10%
            ..Default::default()
        };
        let mut rm = RiskManager::new(config, Decimal::from(100_000));

        rm.update_equity(Decimal::from(100_000)); // peak
        rm.update_equity(Decimal::from(89_000)); // 11% drawdown

        assert!(rm.is_circuit_breaker_active());

        let order = make_order("BTCUSDT", Side::Buy, 1);
        assert!(matches!(
            rm.check_order(&order, &HashMap::new(), Decimal::from(50_000)),
            RiskCheck::Rejected(_)
        ));
    }

    #[test]
    fn position_sizing_fixed_fractional() {
        let config = RiskConfig {
            position_size_fraction: Decimal::new(2, 2), // 2%
            max_position_size: Decimal::from(100),
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(100_000));

        let size = rm.calculate_position_size(
            Decimal::from(100_000),
            Decimal::from(50_000),
            Decimal::from(1_000), // $1000 stop distance
        );
        // risk = 100_000 * 0.02 = 2_000
        // size = 2_000 / 1_000 = 2
        assert_eq!(size, Decimal::from(2));
    }

    #[test]
    fn kelly_sizing() {
        let rm = RiskManager::new(RiskConfig::default(), Decimal::from(100_000));
        let size = rm.kelly_position_size(
            Decimal::from(100_000),
            Decimal::new(60, 2),   // 60% win rate
            Decimal::from(1_000),  // avg win
            Decimal::from(500),    // avg loss
            Decimal::from(50_000), // entry price
        );
        assert!(size > Decimal::ZERO);
    }

    #[test]
    fn circuit_breaker_reset() {
        let mut rm = RiskManager::new(RiskConfig::default(), Decimal::from(100_000));
        rm.update_equity(Decimal::from(85_000));
        assert!(rm.is_circuit_breaker_active());

        rm.reset_circuit_breaker(Decimal::from(85_000));
        assert!(!rm.is_circuit_breaker_active());
    }
}
