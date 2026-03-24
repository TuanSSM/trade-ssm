use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ssm_core::{MarginMode, Position, Side};

/// Leverage and margin configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeverageConfig {
    pub default_leverage: u32,
    pub max_leverage: u32,
    pub margin_mode: MarginMode,
    /// Liquidation buffer percentage, e.g., 0.05 = 5% buffer.
    pub liquidation_buffer_pct: Decimal,
}

impl Default for LeverageConfig {
    fn default() -> Self {
        Self {
            default_leverage: 1,
            max_leverage: 20,
            margin_mode: MarginMode::Isolated,
            liquidation_buffer_pct: Decimal::new(5, 2),
        }
    }
}

/// Leverage manager — calculates margin requirements and liquidation prices.
pub struct LeverageManager {
    config: LeverageConfig,
}

impl LeverageManager {
    pub fn new(config: LeverageConfig) -> Self {
        Self { config }
    }

    /// Calculate margin required for a position.
    ///
    /// margin = (quantity * price) / leverage
    pub fn margin_required(&self, quantity: Decimal, price: Decimal, leverage: u32) -> Decimal {
        if leverage == 0 {
            return Decimal::ZERO;
        }
        (quantity * price) / Decimal::from(leverage)
    }

    /// Calculate liquidation price for a position.
    ///
    /// For isolated margin:
    ///   Long:  liq_price = entry_price * (1 - 1/leverage)
    ///   Short: liq_price = entry_price * (1 + 1/leverage)
    ///
    /// For cross margin, liquidation depends on total account balance,
    /// so we use a more conservative estimate (same formula but with a smaller buffer).
    pub fn liquidation_price(
        &self,
        entry_price: Decimal,
        side: Side,
        leverage: u32,
        margin_mode: MarginMode,
    ) -> Decimal {
        if leverage == 0 {
            return Decimal::ZERO;
        }

        let lev = Decimal::from(leverage);
        let fraction = Decimal::ONE / lev;

        match margin_mode {
            MarginMode::Isolated => match side {
                Side::Buy => entry_price * (Decimal::ONE - fraction),
                Side::Sell => entry_price * (Decimal::ONE + fraction),
            },
            MarginMode::Cross => {
                // Cross margin uses a smaller movement to liquidation (more conservative estimate).
                // We use 80% of the isolated margin distance as a rough approximation.
                let cross_factor = Decimal::new(80, 2); // 0.80
                let adjusted_fraction = fraction * cross_factor;
                match side {
                    Side::Buy => entry_price * (Decimal::ONE - adjusted_fraction),
                    Side::Sell => entry_price * (Decimal::ONE + adjusted_fraction),
                }
            }
        }
    }

    /// Check if current price is within liquidation buffer.
    pub fn is_near_liquidation(&self, position: &Position, current_price: Decimal) -> bool {
        let liq_price = self.liquidation_price(
            position.entry_price,
            position.side,
            position.leverage,
            self.config.margin_mode,
        );

        if liq_price.is_zero() {
            return false;
        }

        // Calculate how close price is to liquidation relative to entry
        let distance_to_liq = match position.side {
            Side::Buy => {
                // For long, liquidation is below entry. Check if price is close to liq.
                if current_price <= liq_price {
                    return true; // Already past liquidation
                }
                (current_price - liq_price) / position.entry_price
            }
            Side::Sell => {
                // For short, liquidation is above entry. Check if price is close to liq.
                if current_price >= liq_price {
                    return true; // Already past liquidation
                }
                (liq_price - current_price) / position.entry_price
            }
        };

        distance_to_liq <= self.config.liquidation_buffer_pct
    }

    /// Calculate effective PnL with leverage.
    ///
    /// The PnL is calculated on the full notional value:
    ///   Long:  pnl = (exit - entry) * quantity
    ///   Short: pnl = (entry - exit) * quantity
    ///
    /// Leverage amplifies PnL relative to margin, but the absolute PnL
    /// is the same as the notional movement.
    pub fn leveraged_pnl(
        &self,
        entry_price: Decimal,
        exit_price: Decimal,
        quantity: Decimal,
        _leverage: u32,
        side: Side,
    ) -> Decimal {
        match side {
            Side::Buy => (exit_price - entry_price) * quantity,
            Side::Sell => (entry_price - exit_price) * quantity,
        }
    }

    /// Validate that leverage doesn't exceed maximum.
    pub fn validate_leverage(&self, leverage: u32) -> bool {
        leverage >= 1 && leverage <= self.config.max_leverage
    }

    /// Estimate funding fee for a period (8h).
    ///
    /// funding_fee = position_value * funding_rate
    pub fn funding_fee(&self, position_value: Decimal, funding_rate: Decimal) -> Decimal {
        position_value * funding_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn make_position(symbol: &str, side: Side, qty: u32, entry: u32, leverage: u32) -> Position {
        Position {
            symbol: symbol.into(),
            side,
            entry_price: Decimal::from(entry),
            quantity: Decimal::from(qty),
            unrealized_pnl: Decimal::ZERO,
            realized_pnl: Decimal::ZERO,
            leverage,
            opened_at: 0,
        }
    }

    #[test]
    fn margin_required_calculation() {
        let lm = LeverageManager::new(LeverageConfig::default());

        // 1 BTC at 50000 with 10x leverage -> margin = 5000
        let margin = lm.margin_required(Decimal::from(1), Decimal::from(50000), 10);
        assert_eq!(margin, Decimal::from(5000));

        // 2 BTC at 50000 with 5x leverage -> margin = 20000
        let margin = lm.margin_required(Decimal::from(2), Decimal::from(50000), 5);
        assert_eq!(margin, Decimal::from(20000));

        // 1x leverage -> margin = full notional
        let margin = lm.margin_required(Decimal::from(1), Decimal::from(50000), 1);
        assert_eq!(margin, Decimal::from(50000));
    }

    #[test]
    fn margin_required_zero_leverage() {
        let lm = LeverageManager::new(LeverageConfig::default());
        let margin = lm.margin_required(Decimal::from(1), Decimal::from(50000), 0);
        assert_eq!(margin, Decimal::ZERO);
    }

    #[test]
    fn liquidation_price_long_isolated() {
        let lm = LeverageManager::new(LeverageConfig::default());

        // Long at 50000 with 10x: liq = 50000 * (1 - 1/10) = 45000
        let liq = lm.liquidation_price(Decimal::from(50000), Side::Buy, 10, MarginMode::Isolated);
        assert_eq!(liq, Decimal::from(45000));

        // Long at 50000 with 5x: liq = 50000 * (1 - 1/5) = 40000
        let liq = lm.liquidation_price(Decimal::from(50000), Side::Buy, 5, MarginMode::Isolated);
        assert_eq!(liq, Decimal::from(40000));
    }

    #[test]
    fn liquidation_price_short_isolated() {
        let lm = LeverageManager::new(LeverageConfig::default());

        // Short at 50000 with 10x: liq = 50000 * (1 + 1/10) = 55000
        let liq = lm.liquidation_price(Decimal::from(50000), Side::Sell, 10, MarginMode::Isolated);
        assert_eq!(liq, Decimal::from(55000));

        // Short at 50000 with 5x: liq = 50000 * (1 + 1/5) = 60000
        let liq = lm.liquidation_price(Decimal::from(50000), Side::Sell, 5, MarginMode::Isolated);
        assert_eq!(liq, Decimal::from(60000));
    }

    #[test]
    fn liquidation_price_cross_margin() {
        let lm = LeverageManager::new(LeverageConfig::default());

        // Long at 50000 with 10x cross: fraction = 0.1 * 0.80 = 0.08
        // liq = 50000 * (1 - 0.08) = 46000
        let liq = lm.liquidation_price(Decimal::from(50000), Side::Buy, 10, MarginMode::Cross);
        assert_eq!(liq, Decimal::from(46000));

        // Short at 50000 with 10x cross: liq = 50000 * (1 + 0.08) = 54000
        let liq = lm.liquidation_price(Decimal::from(50000), Side::Sell, 10, MarginMode::Cross);
        assert_eq!(liq, Decimal::from(54000));
    }

    #[test]
    fn is_near_liquidation_long() {
        let config = LeverageConfig {
            liquidation_buffer_pct: Decimal::new(5, 2), // 5%
            margin_mode: MarginMode::Isolated,
            ..Default::default()
        };
        let lm = LeverageManager::new(config);

        // Long at 50000 with 10x -> liq at 45000
        let pos = make_position("BTCUSDT", Side::Buy, 1, 50000, 10);

        // Price at 47000: distance = (47000-45000)/50000 = 0.04 = 4% < 5% buffer -> near
        assert!(lm.is_near_liquidation(&pos, Decimal::from(47000)));

        // Price at 49000: distance = (49000-45000)/50000 = 0.08 = 8% > 5% buffer -> not near
        assert!(!lm.is_near_liquidation(&pos, Decimal::from(49000)));

        // Price below liquidation -> definitely near
        assert!(lm.is_near_liquidation(&pos, Decimal::from(44000)));
    }

    #[test]
    fn is_near_liquidation_short() {
        let config = LeverageConfig {
            liquidation_buffer_pct: Decimal::new(5, 2), // 5%
            margin_mode: MarginMode::Isolated,
            ..Default::default()
        };
        let lm = LeverageManager::new(config);

        // Short at 50000 with 10x -> liq at 55000
        let pos = make_position("BTCUSDT", Side::Sell, 1, 50000, 10);

        // Price at 53500: distance = (55000-53500)/50000 = 0.03 = 3% < 5% buffer -> near
        assert!(lm.is_near_liquidation(&pos, Decimal::from(53500)));

        // Price at 51000: distance = (55000-51000)/50000 = 0.08 = 8% > 5% buffer -> not near
        assert!(!lm.is_near_liquidation(&pos, Decimal::from(51000)));

        // Price above liquidation -> definitely near
        assert!(lm.is_near_liquidation(&pos, Decimal::from(56000)));
    }

    #[test]
    fn leveraged_pnl_long() {
        let lm = LeverageManager::new(LeverageConfig::default());

        // Long: buy at 50000, sell at 52000, qty 2
        let pnl = lm.leveraged_pnl(
            Decimal::from(50000),
            Decimal::from(52000),
            Decimal::from(2),
            10,
            Side::Buy,
        );
        assert_eq!(pnl, Decimal::from(4000));

        // Long: buy at 50000, sell at 48000 (loss), qty 1
        let pnl = lm.leveraged_pnl(
            Decimal::from(50000),
            Decimal::from(48000),
            Decimal::from(1),
            10,
            Side::Buy,
        );
        assert_eq!(pnl, Decimal::from(-2000));
    }

    #[test]
    fn leveraged_pnl_short() {
        let lm = LeverageManager::new(LeverageConfig::default());

        // Short: sell at 50000, buy at 48000, qty 2
        let pnl = lm.leveraged_pnl(
            Decimal::from(50000),
            Decimal::from(48000),
            Decimal::from(2),
            10,
            Side::Sell,
        );
        assert_eq!(pnl, Decimal::from(4000));

        // Short: sell at 50000, buy at 52000 (loss), qty 1
        let pnl = lm.leveraged_pnl(
            Decimal::from(50000),
            Decimal::from(52000),
            Decimal::from(1),
            10,
            Side::Sell,
        );
        assert_eq!(pnl, Decimal::from(-2000));
    }

    #[test]
    fn validate_leverage_valid() {
        let config = LeverageConfig {
            max_leverage: 20,
            ..Default::default()
        };
        let lm = LeverageManager::new(config);

        assert!(lm.validate_leverage(1));
        assert!(lm.validate_leverage(10));
        assert!(lm.validate_leverage(20));
    }

    #[test]
    fn validate_leverage_invalid() {
        let config = LeverageConfig {
            max_leverage: 20,
            ..Default::default()
        };
        let lm = LeverageManager::new(config);

        assert!(!lm.validate_leverage(0));
        assert!(!lm.validate_leverage(21));
        assert!(!lm.validate_leverage(100));
    }

    #[test]
    fn funding_fee_calculation() {
        let lm = LeverageManager::new(LeverageConfig::default());

        // Position value 100000, funding rate 0.01% = 0.0001
        let fee = lm.funding_fee(Decimal::from(100_000), Decimal::new(1, 4));
        assert_eq!(fee, Decimal::from(10));

        // Position value 50000, funding rate -0.005% = -0.00005
        let fee = lm.funding_fee(Decimal::from(50_000), Decimal::new(-5, 5));
        assert_eq!(fee, Decimal::new(-25, 1));
    }

    #[test]
    fn default_config_sensible() {
        let config = LeverageConfig::default();
        assert_eq!(config.default_leverage, 1);
        assert_eq!(config.max_leverage, 20);
        assert_eq!(config.margin_mode, MarginMode::Isolated);
        assert_eq!(config.liquidation_buffer_pct, Decimal::new(5, 2));
    }

    #[test]
    fn liquidation_price_zero_leverage() {
        let lm = LeverageManager::new(LeverageConfig::default());
        let liq = lm.liquidation_price(Decimal::from(50000), Side::Buy, 0, MarginMode::Isolated);
        assert_eq!(liq, Decimal::ZERO);
    }

    #[test]
    fn liquidation_price_1x_leverage() {
        let lm = LeverageManager::new(LeverageConfig::default());

        // 1x long: liq = 50000 * (1 - 1) = 0
        let liq = lm.liquidation_price(Decimal::from(50000), Side::Buy, 1, MarginMode::Isolated);
        assert_eq!(liq, Decimal::ZERO);

        // 1x short: liq = 50000 * (1 + 1) = 100000
        let liq = lm.liquidation_price(Decimal::from(50000), Side::Sell, 1, MarginMode::Isolated);
        assert_eq!(liq, Decimal::from(100_000));
    }
}
