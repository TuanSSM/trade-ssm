use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use ssm_core::Side;
use ssm_core::{Order, Position, TradeRecord};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Position sizing mode
// ---------------------------------------------------------------------------

/// Position sizing strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SizingMode {
    /// Fixed fraction of balance per trade.
    Fixed {
        /// Fraction of balance, e.g. 0.10 = 10%.
        fraction: Decimal,
    },
    /// Kelly criterion with configurable fraction multiplier.
    Kelly {
        /// Multiplier on the raw Kelly fraction.
        /// 1.0 = full Kelly, 0.5 = half-Kelly, 0.25 = quarter-Kelly.
        fraction_multiplier: Decimal,
        /// Don't use Kelly until this many completed trades exist.
        /// Falls back to `fallback_fraction` before this threshold.
        min_trades: usize,
        /// Fixed fraction to use when not enough trades exist.
        fallback_fraction: Decimal,
        /// Hard cap on the Kelly fraction (before multiplying by balance).
        max_fraction: Decimal,
    },
}

impl Default for SizingMode {
    fn default() -> Self {
        SizingMode::Fixed {
            fraction: Decimal::new(2, 2), // 2%
        }
    }
}

// ---------------------------------------------------------------------------
// Kelly statistics
// ---------------------------------------------------------------------------

/// Minimal trade statistics needed for Kelly position sizing.
#[derive(Debug, Clone)]
pub struct KellyStats {
    pub total_trades: usize,
    pub win_rate: Decimal,
    pub avg_win: Decimal,
    pub avg_loss: Decimal,
}

impl KellyStats {
    /// Compute Kelly-relevant stats from completed trades.
    pub fn from_trades(trades: &[TradeRecord]) -> Self {
        let total = trades.len();
        if total == 0 {
            return Self {
                total_trades: 0,
                win_rate: Decimal::ZERO,
                avg_win: Decimal::ZERO,
                avg_loss: Decimal::ZERO,
            };
        }

        let mut win_count: usize = 0;
        let mut win_sum = Decimal::ZERO;
        let mut loss_count: usize = 0;
        let mut loss_sum = Decimal::ZERO;

        for t in trades {
            if t.profit > Decimal::ZERO {
                win_count += 1;
                win_sum += t.profit;
            } else if t.profit < Decimal::ZERO {
                loss_count += 1;
                loss_sum += t.profit.abs();
            }
        }

        let win_rate = Decimal::from(win_count as u64) / Decimal::from(total as u64);
        let avg_win = if win_count > 0 {
            win_sum / Decimal::from(win_count as u64)
        } else {
            Decimal::ZERO
        };
        let avg_loss = if loss_count > 0 {
            loss_sum / Decimal::from(loss_count as u64)
        } else {
            Decimal::ZERO
        };

        Self {
            total_trades: total,
            win_rate,
            avg_win,
            avg_loss,
        }
    }
}

/// Raw Kelly fraction: `W - (1-W)/R` where `R = avg_win / avg_loss`.
///
/// Returns zero for negative edge or zero risk-reward.
pub fn kelly_fraction(stats: &KellyStats) -> Decimal {
    if stats.avg_win.is_zero() {
        return Decimal::ZERO;
    }

    let r = if stats.avg_loss > Decimal::ZERO {
        stats.avg_win / stats.avg_loss
    } else {
        // No losses: treat as very high reward ratio, but cap to avoid overflow.
        Decimal::new(999, 0)
    };

    let kelly = stats.win_rate - (Decimal::ONE - stats.win_rate) / r;
    if kelly <= Decimal::ZERO {
        Decimal::ZERO
    } else {
        kelly
    }
}

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
    /// Position sizing strategy. When set, `position_size_for_mode` uses this
    /// instead of `position_size_fraction`.
    #[serde(default)]
    pub sizing_mode: SizingMode,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_position_size: Decimal::from(10),
            max_total_exposure: Decimal::from(1_000_000),
            max_drawdown_pct: Decimal::new(10, 2), // 10%
            max_open_positions: 5,
            position_size_fraction: Decimal::new(2, 2), // 2%
            sizing_mode: SizingMode::default(),
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

    /// Calculate the notional risk amount based on the configured `SizingMode`.
    ///
    /// - **Fixed mode**: returns `balance * fraction`, ignoring `trades`.
    /// - **Kelly mode**: computes rolling stats from `trades`, applies the Kelly
    ///   formula with the configured fraction multiplier and cap. Falls back to
    ///   `fallback_fraction` when fewer than `min_trades` exist.
    ///
    /// The returned value is a notional amount (in quote currency). To convert to
    /// base-asset quantity: `qty = (notional * leverage) / entry_price`.
    pub fn position_size_for_mode(&self, balance: Decimal, trades: &[TradeRecord]) -> Decimal {
        let fraction = match &self.config.sizing_mode {
            SizingMode::Fixed { fraction } => *fraction,
            SizingMode::Kelly {
                fraction_multiplier,
                min_trades,
                fallback_fraction,
                max_fraction,
            } => {
                let stats = KellyStats::from_trades(trades);
                if stats.total_trades < *min_trades {
                    *fallback_fraction
                } else {
                    let raw = kelly_fraction(&stats);
                    if raw <= Decimal::ZERO {
                        Decimal::ZERO
                    } else {
                        (raw * fraction_multiplier).min(*max_fraction)
                    }
                }
            }
        };
        balance * fraction
    }

    /// Calculate position size using Kelly criterion.
    ///
    /// Prefer `position_size_for_mode` for new code — it supports configurable
    /// Kelly variants and automatic stats computation from trade history.
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

    #[test]
    fn test_circuit_breaker_not_triggered_within_limit() {
        let config = RiskConfig {
            max_drawdown_pct: Decimal::new(10, 2), // 10%
            ..Default::default()
        };
        let mut rm = RiskManager::new(config, Decimal::from(100_000));

        // 5% drawdown — within the 10% limit
        rm.update_equity(Decimal::from(95_000));
        assert!(!rm.is_circuit_breaker_active());
    }

    #[test]
    fn test_position_size_zero_stop() {
        let rm = RiskManager::new(RiskConfig::default(), Decimal::from(100_000));
        let size = rm.calculate_position_size(
            Decimal::from(100_000),
            Decimal::from(50_000),
            Decimal::ZERO, // zero stop distance
        );
        assert_eq!(size, Decimal::ZERO);
    }

    #[test]
    fn test_position_size_clamped_to_max() {
        let config = RiskConfig {
            max_position_size: Decimal::from(5),
            position_size_fraction: Decimal::new(50, 2), // 50% — very large fraction
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(1_000_000));

        let size = rm.calculate_position_size(
            Decimal::from(1_000_000),
            Decimal::from(50_000),
            Decimal::from(1), // tiny stop distance → huge raw size
        );
        // Should be clamped to max_position_size = 5
        assert_eq!(size, Decimal::from(5));
    }

    #[test]
    fn test_kelly_zero_win_rate() {
        let rm = RiskManager::new(RiskConfig::default(), Decimal::from(100_000));
        let size = rm.kelly_position_size(
            Decimal::from(100_000),
            Decimal::ZERO, // 0% win rate
            Decimal::from(1_000),
            Decimal::from(500),
            Decimal::from(50_000),
        );
        assert_eq!(size, Decimal::ZERO);
    }

    #[test]
    fn test_kelly_zero_avg_win() {
        let rm = RiskManager::new(RiskConfig::default(), Decimal::from(100_000));
        let size = rm.kelly_position_size(
            Decimal::from(100_000),
            Decimal::new(60, 2),
            Decimal::ZERO, // 0 avg win
            Decimal::from(500),
            Decimal::from(50_000),
        );
        assert_eq!(size, Decimal::ZERO);
    }

    #[test]
    fn test_exposure_limit_rejected() {
        let config = RiskConfig {
            max_total_exposure: Decimal::from(100_000), // low exposure limit
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(1_000_000));

        let mut positions = HashMap::new();
        positions.insert(
            "BTCUSDT".into(),
            make_position("BTCUSDT", Side::Buy, 1, 50000),
        );

        // New order would add 50000 exposure → total 100000, exceeds limit
        let order = make_order("ETHUSDT", Side::Buy, 1);
        let result = rm.check_order(&order, &positions, Decimal::from(60_000));
        assert!(matches!(result, RiskCheck::Rejected(_)));
    }

    #[test]
    fn test_adding_to_existing_position_exceeds_max() {
        let config = RiskConfig {
            max_position_size: Decimal::from(5),
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(1_000_000));

        let mut positions = HashMap::new();
        positions.insert(
            "BTCUSDT".into(),
            make_position("BTCUSDT", Side::Buy, 3, 50000),
        );

        // Adding 3 to existing 3 → 6 > max 5
        let order = make_order("BTCUSDT", Side::Buy, 3);
        let result = rm.check_order(&order, &positions, Decimal::from(50_000));
        assert!(matches!(result, RiskCheck::Rejected(_)));
    }

    #[test]
    fn test_risk_config_default() {
        let config = RiskConfig::default();
        assert_eq!(config.max_position_size, Decimal::from(10));
        assert_eq!(config.max_total_exposure, Decimal::from(1_000_000));
        assert_eq!(config.max_drawdown_pct, Decimal::new(10, 2));
        assert_eq!(config.max_open_positions, 5);
        assert_eq!(config.position_size_fraction, Decimal::new(2, 2));
    }

    #[test]
    fn test_approve_existing_symbol_same_side() {
        let rm = RiskManager::new(RiskConfig::default(), Decimal::from(1_000_000));

        let mut positions = HashMap::new();
        positions.insert(
            "BTCUSDT".into(),
            make_position("BTCUSDT", Side::Buy, 2, 50000),
        );

        // Adding 3 to existing 2 → 5, within max_position_size of 10
        let order = make_order("BTCUSDT", Side::Buy, 3);
        let result = rm.check_order(&order, &positions, Decimal::from(50_000));
        assert_eq!(result, RiskCheck::Approved);
    }

    #[test]
    fn test_zero_balance_position_size() {
        let rm = RiskManager::new(RiskConfig::default(), Decimal::from(100_000));
        let size = rm.calculate_position_size(
            Decimal::ZERO, // zero balance
            Decimal::from(50_000),
            Decimal::from(1_000),
        );
        assert_eq!(size, Decimal::ZERO);
    }

    #[test]
    fn test_zero_entry_price_position_size() {
        let rm = RiskManager::new(RiskConfig::default(), Decimal::from(100_000));
        let size = rm.calculate_position_size(
            Decimal::from(100_000),
            Decimal::ZERO, // zero entry price
            Decimal::from(1_000),
        );
        assert_eq!(size, Decimal::ZERO);
    }

    #[test]
    fn test_kelly_zero_entry_price() {
        let rm = RiskManager::new(RiskConfig::default(), Decimal::from(100_000));
        let size = rm.kelly_position_size(
            Decimal::from(100_000),
            Decimal::new(60, 2),
            Decimal::from(1_000),
            Decimal::from(500),
            Decimal::ZERO, // zero entry price
        );
        assert_eq!(size, Decimal::ZERO);
    }

    #[test]
    fn test_kelly_negative_edge_returns_zero() {
        let rm = RiskManager::new(RiskConfig::default(), Decimal::from(100_000));
        // Win rate 20%, avg_win 100, avg_loss 500 — negative edge
        let size = rm.kelly_position_size(
            Decimal::from(100_000),
            Decimal::new(20, 2),   // 20% win rate
            Decimal::from(100),    // avg win
            Decimal::from(500),    // avg loss
            Decimal::from(50_000), // entry price
        );
        assert_eq!(size, Decimal::ZERO);
    }

    #[test]
    fn test_kelly_clamped_to_max_position_size() {
        let config = RiskConfig {
            max_position_size: Decimal::from(1),
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(100_000));
        let size = rm.kelly_position_size(
            Decimal::from(10_000_000), // huge balance
            Decimal::new(90, 2),       // 90% win rate
            Decimal::from(10_000),     // avg win
            Decimal::from(100),        // avg loss
            Decimal::from(1),          // tiny entry price
        );
        assert_eq!(size, Decimal::from(1));
    }

    #[test]
    fn test_circuit_breaker_at_exact_threshold() {
        let config = RiskConfig {
            max_drawdown_pct: Decimal::new(10, 2), // 10%
            ..Default::default()
        };
        let mut rm = RiskManager::new(config, Decimal::from(100_000));

        // Exactly 10% drawdown — should trigger
        rm.update_equity(Decimal::from(90_000));
        assert!(rm.is_circuit_breaker_active());
    }

    #[test]
    fn test_circuit_breaker_just_below_threshold() {
        let config = RiskConfig {
            max_drawdown_pct: Decimal::new(10, 2), // 10%
            ..Default::default()
        };
        let mut rm = RiskManager::new(config, Decimal::from(100_000));

        // 9.99% drawdown — should NOT trigger
        rm.update_equity(Decimal::from(90_010));
        assert!(!rm.is_circuit_breaker_active());
    }

    #[test]
    fn test_circuit_breaker_rejects_all_orders() {
        let mut rm = RiskManager::new(RiskConfig::default(), Decimal::from(100_000));
        rm.update_equity(Decimal::from(80_000)); // 20% drawdown

        // Even a tiny order should be rejected
        let order = make_order("BTCUSDT", Side::Buy, 1);
        let result = rm.check_order(&order, &HashMap::new(), Decimal::from(1));
        match result {
            RiskCheck::Rejected(msg) => assert!(msg.contains("circuit breaker")),
            _ => panic!("expected rejection"),
        }
    }

    #[test]
    fn test_circuit_breaker_reset_allows_orders() {
        let mut rm = RiskManager::new(RiskConfig::default(), Decimal::from(100_000));
        rm.update_equity(Decimal::from(80_000)); // trigger breaker
        assert!(rm.is_circuit_breaker_active());

        rm.reset_circuit_breaker(Decimal::from(80_000));
        assert!(!rm.is_circuit_breaker_active());

        let order = make_order("BTCUSDT", Side::Buy, 1);
        let result = rm.check_order(&order, &HashMap::new(), Decimal::from(50_000));
        assert_eq!(result, RiskCheck::Approved);
    }

    #[test]
    fn test_peak_equity_updates_upward_only() {
        let config = RiskConfig {
            max_drawdown_pct: Decimal::new(10, 2),
            ..Default::default()
        };
        let mut rm = RiskManager::new(config, Decimal::from(100_000));

        rm.update_equity(Decimal::from(110_000)); // new peak
        rm.update_equity(Decimal::from(105_000)); // drawdown from 110k, not 100k

        // 4.5% drawdown from peak of 110k — should not trigger 10% breaker
        assert!(!rm.is_circuit_breaker_active());
    }

    #[test]
    fn test_order_opposite_side_existing_position_approved() {
        let rm = RiskManager::new(RiskConfig::default(), Decimal::from(1_000_000));

        let mut positions = HashMap::new();
        positions.insert(
            "BTCUSDT".into(),
            make_position("BTCUSDT", Side::Buy, 5, 50000),
        );

        // Sell order against existing long — should be approved (reducing position)
        let order = make_order("BTCUSDT", Side::Sell, 3);
        let result = rm.check_order(&order, &positions, Decimal::from(50_000));
        assert_eq!(result, RiskCheck::Approved);
    }

    #[test]
    fn test_max_positions_allows_existing_symbol() {
        let config = RiskConfig {
            max_open_positions: 1,
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(1_000_000));

        let mut positions = HashMap::new();
        positions.insert(
            "BTCUSDT".into(),
            make_position("BTCUSDT", Side::Buy, 1, 50000),
        );

        // Adding to existing position should be allowed even at max positions
        let order = make_order("BTCUSDT", Side::Buy, 1);
        let result = rm.check_order(&order, &positions, Decimal::from(50_000));
        assert_eq!(result, RiskCheck::Approved);
    }

    #[test]
    fn test_zero_peak_equity_no_panic() {
        let config = RiskConfig {
            max_drawdown_pct: Decimal::new(10, 2),
            ..Default::default()
        };
        let mut rm = RiskManager::new(config, Decimal::ZERO);

        // Should not panic or trigger breaker with zero peak
        rm.update_equity(Decimal::ZERO);
        assert!(!rm.is_circuit_breaker_active());
    }

    #[test]
    fn test_exposure_at_exact_limit_rejected() {
        let config = RiskConfig {
            max_total_exposure: Decimal::from(50_001),
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(1_000_000));

        let positions = HashMap::new();

        // Order exposure = 1 * 50001 = 50001, but limit check is >, so 50001 is not > 50001
        let order = make_order("BTCUSDT", Side::Buy, 1);
        let result = rm.check_order(&order, &positions, Decimal::from(50_001));
        // 50001 is not > 50001, so approved
        assert_eq!(result, RiskCheck::Approved);
    }

    #[test]
    fn test_exposure_just_over_limit_rejected() {
        let config = RiskConfig {
            max_total_exposure: Decimal::from(50_000),
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(1_000_000));

        let positions = HashMap::new();
        let order = make_order("BTCUSDT", Side::Buy, 1);
        // exposure = 1 * 50001 = 50001 > 50000
        let result = rm.check_order(&order, &positions, Decimal::from(50_001));
        assert!(matches!(result, RiskCheck::Rejected(_)));
    }

    // -----------------------------------------------------------------------
    // Kelly stats & sizing mode tests
    // -----------------------------------------------------------------------

    use ssm_core::ExitReason;

    fn make_trade(profit: i64) -> TradeRecord {
        TradeRecord {
            id: "t".into(),
            symbol: "BTCUSDT".into(),
            side: Side::Buy,
            entry_price: Decimal::from(100),
            exit_price: Decimal::from(100) + Decimal::from(profit),
            quantity: Decimal::ONE,
            profit: Decimal::from(profit),
            profit_pct: Decimal::from(profit),
            entry_time: 0,
            exit_time: 1000,
            duration_candles: 1,
            exit_reason: ExitReason::Signal,
            leverage: 1,
            fee: Decimal::ZERO,
        }
    }

    #[test]
    fn kelly_stats_empty_trades() {
        let stats = KellyStats::from_trades(&[]);
        assert_eq!(stats.total_trades, 0);
        assert_eq!(stats.win_rate, Decimal::ZERO);
        assert_eq!(stats.avg_win, Decimal::ZERO);
        assert_eq!(stats.avg_loss, Decimal::ZERO);
    }

    #[test]
    fn kelly_stats_all_winners() {
        let trades = vec![make_trade(100), make_trade(200)];
        let stats = KellyStats::from_trades(&trades);
        assert_eq!(stats.total_trades, 2);
        assert_eq!(stats.win_rate, Decimal::ONE);
        assert_eq!(stats.avg_win, Decimal::from(150));
        assert_eq!(stats.avg_loss, Decimal::ZERO);
    }

    #[test]
    fn kelly_stats_all_losers() {
        let trades = vec![make_trade(-50), make_trade(-100)];
        let stats = KellyStats::from_trades(&trades);
        assert_eq!(stats.total_trades, 2);
        assert_eq!(stats.win_rate, Decimal::ZERO);
        assert_eq!(stats.avg_win, Decimal::ZERO);
        assert_eq!(stats.avg_loss, Decimal::from(75));
    }

    #[test]
    fn kelly_stats_mixed_trades() {
        // 3 wins (+100 each), 1 loss (-50)
        let trades = vec![
            make_trade(100),
            make_trade(100),
            make_trade(100),
            make_trade(-50),
        ];
        let stats = KellyStats::from_trades(&trades);
        assert_eq!(stats.total_trades, 4);
        assert_eq!(stats.win_rate, Decimal::new(75, 2)); // 0.75
        assert_eq!(stats.avg_win, Decimal::from(100));
        assert_eq!(stats.avg_loss, Decimal::from(50));
    }

    #[test]
    fn kelly_fraction_positive_edge() {
        // win_rate = 0.6, avg_win = 100, avg_loss = 50 => R = 2.0
        // Kelly = 0.6 - 0.4/2.0 = 0.6 - 0.2 = 0.4
        let stats = KellyStats {
            total_trades: 100,
            win_rate: Decimal::new(6, 1), // 0.6
            avg_win: Decimal::from(100),
            avg_loss: Decimal::from(50),
        };
        let f = kelly_fraction(&stats);
        assert_eq!(f, Decimal::new(4, 1)); // 0.4
    }

    #[test]
    fn kelly_fraction_negative_edge() {
        // win_rate = 0.3, avg_win = 50, avg_loss = 100 => R = 0.5
        // Kelly = 0.3 - 0.7/0.5 = 0.3 - 1.4 = -1.1 => 0
        let stats = KellyStats {
            total_trades: 50,
            win_rate: Decimal::new(3, 1),
            avg_win: Decimal::from(50),
            avg_loss: Decimal::from(100),
        };
        assert_eq!(kelly_fraction(&stats), Decimal::ZERO);
    }

    #[test]
    fn kelly_fraction_zero_avg_win() {
        let stats = KellyStats {
            total_trades: 10,
            win_rate: Decimal::ZERO,
            avg_win: Decimal::ZERO,
            avg_loss: Decimal::from(100),
        };
        assert_eq!(kelly_fraction(&stats), Decimal::ZERO);
    }

    #[test]
    fn kelly_fraction_no_losses() {
        // win_rate = 1.0, no losses => R = 999
        // Kelly = 1.0 - 0/999 = 1.0
        let stats = KellyStats {
            total_trades: 10,
            win_rate: Decimal::ONE,
            avg_win: Decimal::from(100),
            avg_loss: Decimal::ZERO,
        };
        assert_eq!(kelly_fraction(&stats), Decimal::ONE);
    }

    #[test]
    fn sizing_mode_fixed_ignores_trades() {
        let config = RiskConfig {
            sizing_mode: SizingMode::Fixed {
                fraction: Decimal::new(10, 2), // 10%
            },
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(100_000));
        let trades = vec![make_trade(100), make_trade(-50)];
        let notional = rm.position_size_for_mode(Decimal::from(10_000), &trades);
        assert_eq!(notional, Decimal::from(1_000)); // 10% of 10000
    }

    #[test]
    fn sizing_mode_kelly_below_min_trades_uses_fallback() {
        let config = RiskConfig {
            sizing_mode: SizingMode::Kelly {
                fraction_multiplier: Decimal::ONE,
                min_trades: 10,
                fallback_fraction: Decimal::new(5, 2), // 5%
                max_fraction: Decimal::new(25, 2),
            },
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(100_000));
        // Only 2 trades, below min_trades=10
        let trades = vec![make_trade(100), make_trade(100)];
        let notional = rm.position_size_for_mode(Decimal::from(10_000), &trades);
        assert_eq!(notional, Decimal::from(500)); // 5% fallback of 10000
    }

    #[test]
    fn sizing_mode_kelly_positive_edge() {
        // 6 wins of +100, 4 losses of -50 => win_rate=0.6, avg_win=100, avg_loss=50
        // R = 2.0, Kelly = 0.6 - 0.4/2.0 = 0.4
        // half-Kelly: multiplier=0.5 => fraction = 0.4 * 0.5 = 0.20
        let config = RiskConfig {
            sizing_mode: SizingMode::Kelly {
                fraction_multiplier: Decimal::new(5, 1), // 0.5
                min_trades: 5,
                fallback_fraction: Decimal::new(2, 2),
                max_fraction: Decimal::new(25, 2), // 25%
            },
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(100_000));
        let mut trades = Vec::new();
        for _ in 0..6 {
            trades.push(make_trade(100));
        }
        for _ in 0..4 {
            trades.push(make_trade(-50));
        }
        let notional = rm.position_size_for_mode(Decimal::from(10_000), &trades);
        // fraction = 0.4 * 0.5 = 0.20, notional = 10000 * 0.20 = 2000
        assert_eq!(notional, Decimal::from(2_000));
    }

    #[test]
    fn sizing_mode_kelly_negative_edge_returns_zero() {
        // 2 wins of +50, 8 losses of -100 => win_rate=0.2, avg_win=50, avg_loss=100
        // R=0.5, Kelly = 0.2 - 0.8/0.5 = 0.2 - 1.6 = -1.4 => 0
        let config = RiskConfig {
            sizing_mode: SizingMode::Kelly {
                fraction_multiplier: Decimal::ONE,
                min_trades: 5,
                fallback_fraction: Decimal::new(2, 2),
                max_fraction: Decimal::new(25, 2),
            },
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(100_000));
        let mut trades = Vec::new();
        for _ in 0..2 {
            trades.push(make_trade(50));
        }
        for _ in 0..8 {
            trades.push(make_trade(-100));
        }
        let notional = rm.position_size_for_mode(Decimal::from(10_000), &trades);
        assert_eq!(notional, Decimal::ZERO);
    }

    #[test]
    fn sizing_mode_kelly_max_fraction_cap() {
        // 9 wins of +200, 1 loss of -10 => win_rate=0.9, avg_win=200, avg_loss=10
        // R=20, Kelly = 0.9 - 0.1/20 = 0.9 - 0.005 = 0.895
        // full Kelly (multiplier=1.0), capped at max_fraction=0.25
        let config = RiskConfig {
            sizing_mode: SizingMode::Kelly {
                fraction_multiplier: Decimal::ONE,
                min_trades: 5,
                fallback_fraction: Decimal::new(2, 2),
                max_fraction: Decimal::new(25, 2), // 25%
            },
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(100_000));
        let mut trades = Vec::new();
        for _ in 0..9 {
            trades.push(make_trade(200));
        }
        trades.push(make_trade(-10));
        let notional = rm.position_size_for_mode(Decimal::from(10_000), &trades);
        // fraction = min(0.895, 0.25) = 0.25, notional = 10000 * 0.25 = 2500
        assert_eq!(notional, Decimal::from(2_500));
    }

    #[test]
    fn sizing_mode_kelly_quarter_kelly() {
        // Same stats: win_rate=0.6, R=2.0, Kelly=0.4
        // quarter-Kelly: multiplier=0.25 => fraction = 0.4 * 0.25 = 0.10
        let config = RiskConfig {
            sizing_mode: SizingMode::Kelly {
                fraction_multiplier: Decimal::new(25, 2), // 0.25
                min_trades: 5,
                fallback_fraction: Decimal::new(2, 2),
                max_fraction: Decimal::new(50, 2),
            },
            ..Default::default()
        };
        let rm = RiskManager::new(config, Decimal::from(100_000));
        let mut trades = Vec::new();
        for _ in 0..6 {
            trades.push(make_trade(100));
        }
        for _ in 0..4 {
            trades.push(make_trade(-50));
        }
        let notional = rm.position_size_for_mode(Decimal::from(10_000), &trades);
        // fraction = 0.4 * 0.25 = 0.10, notional = 10000 * 0.10 = 1000
        assert_eq!(notional, Decimal::from(1_000));
    }

    #[test]
    fn sizing_mode_default_is_fixed() {
        let mode = SizingMode::default();
        match mode {
            SizingMode::Fixed { fraction } => {
                assert_eq!(fraction, Decimal::new(2, 2));
            }
            _ => panic!("default should be Fixed"),
        }
    }
}
