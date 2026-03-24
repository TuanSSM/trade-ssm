use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ssm_core::Position;
use std::collections::HashMap;

/// Portfolio-level configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioConfig {
    pub max_open_trades: usize,
    pub max_total_exposure: Decimal,
    pub per_pair_max_exposure: Decimal,
    pub pairs: Vec<String>,
}

impl Default for PortfolioConfig {
    fn default() -> Self {
        Self {
            max_open_trades: 5,
            max_total_exposure: Decimal::from(100_000),
            per_pair_max_exposure: Decimal::from(20_000),
            pairs: vec!["BTCUSDT".into(), "ETHUSDT".into()],
        }
    }
}

/// Portfolio manager — tracks positions across multiple symbols.
pub struct PortfolioManager {
    config: PortfolioConfig,
    positions: HashMap<String, Position>,
    pair_pnl: HashMap<String, Decimal>,
}

impl PortfolioManager {
    pub fn new(config: PortfolioConfig) -> Self {
        Self {
            config,
            positions: HashMap::new(),
            pair_pnl: HashMap::new(),
        }
    }

    /// Check if a new trade is allowed for the given symbol.
    pub fn can_open_trade(&self, symbol: &str, quantity: Decimal, price: Decimal) -> bool {
        // Check max open trades (only counts as new if symbol not already open)
        if !self.positions.contains_key(symbol)
            && self.positions.len() >= self.config.max_open_trades
        {
            return false;
        }

        let new_exposure = quantity * price;

        // Check per-pair exposure
        let current_pair_exposure = self.pair_exposure(symbol);
        if current_pair_exposure + new_exposure > self.config.per_pair_max_exposure {
            return false;
        }

        // Check total exposure
        let current_total = self.total_exposure();
        if current_total + new_exposure > self.config.max_total_exposure {
            return false;
        }

        true
    }

    /// Get total exposure across all positions.
    pub fn total_exposure(&self) -> Decimal {
        self.positions
            .values()
            .map(|p| p.quantity * p.entry_price)
            .sum()
    }

    /// Get exposure for a specific pair.
    pub fn pair_exposure(&self, symbol: &str) -> Decimal {
        self.positions
            .get(symbol)
            .map(|p| p.quantity * p.entry_price)
            .unwrap_or(Decimal::ZERO)
    }

    /// Number of open positions.
    pub fn open_trade_count(&self) -> usize {
        self.positions.len()
    }

    /// Update position for a symbol.
    pub fn update_position(&mut self, symbol: &str, position: Option<Position>) {
        match position {
            Some(pos) => {
                self.positions.insert(symbol.to_string(), pos);
            }
            None => {
                self.positions.remove(symbol);
            }
        }
    }

    /// Get all active pairs.
    pub fn active_pairs(&self) -> Vec<&str> {
        self.positions.keys().map(|s| s.as_str()).collect()
    }

    /// Record PnL for a pair.
    pub fn record_pnl(&mut self, symbol: &str, pnl: Decimal) {
        let entry = self
            .pair_pnl
            .entry(symbol.to_string())
            .or_insert(Decimal::ZERO);
        *entry += pnl;
    }

    /// Get cumulative PnL for a pair.
    pub fn pair_pnl(&self, symbol: &str) -> Decimal {
        self.pair_pnl.get(symbol).copied().unwrap_or(Decimal::ZERO)
    }

    /// Get total portfolio PnL.
    pub fn total_pnl(&self) -> Decimal {
        self.pair_pnl.values().sum()
    }

    /// Correlation-aware check: returns true if new trade would be too correlated.
    /// Simple version: if we already have >3 positions on same quote currency, reject.
    pub fn is_too_correlated(&self, symbol: &str) -> bool {
        let quote = extract_quote_currency(symbol);

        let count = self
            .positions
            .keys()
            .filter(|s| extract_quote_currency(s) == quote)
            .count();

        count > 3
    }
}

/// Extract the quote currency from a trading pair symbol.
/// Supports common quote currencies: USDT, BUSD, USDC, BTC, ETH, BNB.
fn extract_quote_currency(symbol: &str) -> &str {
    for suffix in &["USDT", "BUSD", "USDC", "BTC", "ETH", "BNB"] {
        if symbol.ends_with(suffix) {
            return suffix;
        }
    }
    symbol
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use ssm_core::Side;

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
    fn can_open_trade_respects_max_open_trades() {
        let config = PortfolioConfig {
            max_open_trades: 2,
            max_total_exposure: Decimal::from(1_000_000),
            per_pair_max_exposure: Decimal::from(1_000_000),
            ..Default::default()
        };
        let mut pm = PortfolioManager::new(config);
        pm.update_position(
            "BTCUSDT",
            Some(make_position("BTCUSDT", Side::Buy, 1, 50000)),
        );
        pm.update_position(
            "ETHUSDT",
            Some(make_position("ETHUSDT", Side::Buy, 1, 3000)),
        );

        // Third pair should be rejected
        assert!(!pm.can_open_trade("SOLUSDT", Decimal::from(1), Decimal::from(100)));

        // Existing pair should be allowed (adding to position)
        assert!(pm.can_open_trade("BTCUSDT", Decimal::from(1), Decimal::from(100)));
    }

    #[test]
    fn can_open_trade_respects_max_total_exposure() {
        let config = PortfolioConfig {
            max_total_exposure: Decimal::from(60_000),
            per_pair_max_exposure: Decimal::from(100_000),
            ..Default::default()
        };
        let mut pm = PortfolioManager::new(config);
        pm.update_position(
            "BTCUSDT",
            Some(make_position("BTCUSDT", Side::Buy, 1, 50000)),
        );

        // Adding 15000 exposure would exceed 60000 total
        assert!(!pm.can_open_trade("ETHUSDT", Decimal::from(5), Decimal::from(3000)));

        // Adding 2000 exposure would be within limit (50000 + 2000 = 52000)
        assert!(pm.can_open_trade("ETHUSDT", Decimal::from(1), Decimal::from(2000)));
    }

    #[test]
    fn can_open_trade_respects_per_pair_max_exposure() {
        let config = PortfolioConfig {
            per_pair_max_exposure: Decimal::from(10_000),
            max_total_exposure: Decimal::from(1_000_000),
            ..Default::default()
        };
        let mut pm = PortfolioManager::new(config);
        pm.update_position(
            "BTCUSDT",
            Some(make_position("BTCUSDT", Side::Buy, 1, 8000)),
        );

        // Adding 3000 to BTCUSDT would exceed per-pair limit (8000 + 3000 = 11000)
        assert!(!pm.can_open_trade("BTCUSDT", Decimal::from(1), Decimal::from(3000)));

        // Adding 1000 to BTCUSDT is within per-pair limit (8000 + 1000 = 9000)
        assert!(pm.can_open_trade("BTCUSDT", Decimal::from(1), Decimal::from(1000)));
    }

    #[test]
    fn total_exposure_calculation() {
        let mut pm = PortfolioManager::new(PortfolioConfig::default());
        pm.update_position(
            "BTCUSDT",
            Some(make_position("BTCUSDT", Side::Buy, 2, 50000)),
        );
        pm.update_position(
            "ETHUSDT",
            Some(make_position("ETHUSDT", Side::Sell, 10, 3000)),
        );

        // 2*50000 + 10*3000 = 100000 + 30000 = 130000
        assert_eq!(pm.total_exposure(), Decimal::from(130_000));
    }

    #[test]
    fn update_position_and_active_pairs() {
        let mut pm = PortfolioManager::new(PortfolioConfig::default());
        assert_eq!(pm.open_trade_count(), 0);
        assert!(pm.active_pairs().is_empty());

        pm.update_position(
            "BTCUSDT",
            Some(make_position("BTCUSDT", Side::Buy, 1, 50000)),
        );
        pm.update_position(
            "ETHUSDT",
            Some(make_position("ETHUSDT", Side::Sell, 1, 3000)),
        );

        assert_eq!(pm.open_trade_count(), 2);
        let mut pairs = pm.active_pairs();
        pairs.sort();
        assert_eq!(pairs, vec!["BTCUSDT", "ETHUSDT"]);

        // Remove a position
        pm.update_position("BTCUSDT", None);
        assert_eq!(pm.open_trade_count(), 1);
        assert_eq!(pm.active_pairs(), vec!["ETHUSDT"]);
    }

    #[test]
    fn is_too_correlated() {
        let mut pm = PortfolioManager::new(PortfolioConfig::default());

        pm.update_position(
            "BTCUSDT",
            Some(make_position("BTCUSDT", Side::Buy, 1, 50000)),
        );
        pm.update_position(
            "ETHUSDT",
            Some(make_position("ETHUSDT", Side::Buy, 1, 3000)),
        );
        pm.update_position("SOLUSDT", Some(make_position("SOLUSDT", Side::Buy, 1, 100)));

        // 3 USDT pairs — not too correlated yet (threshold is >3)
        assert!(!pm.is_too_correlated("AVAXUSDT"));

        // Add a 4th USDT pair
        pm.update_position("DOTUSDT", Some(make_position("DOTUSDT", Side::Buy, 1, 10)));

        // Now 4 USDT pairs — adding another would be too correlated
        assert!(pm.is_too_correlated("AVAXUSDT"));

        // BTC-quoted pair should not be correlated with USDT pairs
        assert!(!pm.is_too_correlated("ETHBTC"));
    }

    #[test]
    fn pnl_tracking() {
        let mut pm = PortfolioManager::new(PortfolioConfig::default());

        assert_eq!(pm.pair_pnl("BTCUSDT"), Decimal::ZERO);
        assert_eq!(pm.total_pnl(), Decimal::ZERO);

        pm.record_pnl("BTCUSDT", Decimal::from(500));
        pm.record_pnl("ETHUSDT", Decimal::from(-200));
        pm.record_pnl("BTCUSDT", Decimal::from(300));

        assert_eq!(pm.pair_pnl("BTCUSDT"), Decimal::from(800));
        assert_eq!(pm.pair_pnl("ETHUSDT"), Decimal::from(-200));
        assert_eq!(pm.total_pnl(), Decimal::from(600));
    }

    #[test]
    fn default_config_sensible() {
        let config = PortfolioConfig::default();
        assert_eq!(config.max_open_trades, 5);
        assert_eq!(config.max_total_exposure, Decimal::from(100_000));
        assert_eq!(config.per_pair_max_exposure, Decimal::from(20_000));
        assert_eq!(config.pairs.len(), 2);
        assert!(config.pairs.contains(&"BTCUSDT".to_string()));
        assert!(config.pairs.contains(&"ETHUSDT".to_string()));
    }

    #[test]
    fn pair_exposure_for_unknown_symbol() {
        let pm = PortfolioManager::new(PortfolioConfig::default());
        assert_eq!(pm.pair_exposure("UNKNOWN"), Decimal::ZERO);
    }

    #[test]
    fn extract_quote_currency_works() {
        assert_eq!(extract_quote_currency("BTCUSDT"), "USDT");
        assert_eq!(extract_quote_currency("ETHBUSD"), "BUSD");
        assert_eq!(extract_quote_currency("ETHBTC"), "BTC");
        assert_eq!(extract_quote_currency("BNBUSDC"), "USDC");
        assert_eq!(extract_quote_currency("SOLETH"), "ETH");
        assert_eq!(extract_quote_currency("UNKNOWN"), "UNKNOWN");
    }
}
