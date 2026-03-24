use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ssm_core::TradeRecord;
use std::collections::HashMap;

/// Per-strategy/pair edge statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeStats {
    pub pair: String,
    pub total_trades: usize,
    pub win_rate: Decimal,
    pub avg_win: Decimal,
    pub avg_loss: Decimal,
    pub expectancy: Decimal,
    pub risk_reward_ratio: Decimal,
}

/// Analyzes per-pair edge from completed trade records.
pub struct EdgeAnalyzer {
    min_trades: usize,
    min_expectancy: Decimal,
}

impl EdgeAnalyzer {
    pub fn new(min_trades: usize, min_expectancy: Decimal) -> Self {
        Self {
            min_trades,
            min_expectancy,
        }
    }

    /// Compute edge stats from trade records, grouped by pair (symbol).
    pub fn analyze(&self, trades: &[TradeRecord]) -> Vec<EdgeStats> {
        let mut grouped: HashMap<String, Vec<&TradeRecord>> = HashMap::new();
        for t in trades {
            grouped.entry(t.symbol.clone()).or_default().push(t);
        }

        let mut results: Vec<EdgeStats> = grouped
            .into_iter()
            .filter(|(_, group)| group.len() >= self.min_trades)
            .map(|(pair, group)| self.compute_stats(&pair, &group))
            .collect();

        results.sort_by(|a, b| b.expectancy.cmp(&a.expectancy));
        results
    }

    /// Filter pairs that don't meet minimum expectancy, returning only qualifying pair names.
    pub fn filter_pairs(&self, trades: &[TradeRecord]) -> Vec<String> {
        self.analyze(trades)
            .into_iter()
            .filter(|s| s.expectancy >= self.min_expectancy)
            .map(|s| s.pair)
            .collect()
    }

    /// Kelly criterion position size for a pair.
    ///
    /// Kelly fraction = W - (1 - W) / R
    /// where W = win_rate, R = risk_reward_ratio (avg_win / avg_loss).
    ///
    /// Returns zero for negative edge or zero risk-reward.
    /// Clamps to a maximum of 25% of balance (quarter-Kelly is common in practice).
    pub fn kelly_size(&self, stats: &EdgeStats, balance: Decimal) -> Decimal {
        if stats.risk_reward_ratio <= Decimal::ZERO {
            return Decimal::ZERO;
        }

        // Kelly fraction: W - (1 - W) / R
        let w = stats.win_rate;
        let r = stats.risk_reward_ratio;
        let kelly = w - (Decimal::ONE - w) / r;

        if kelly <= Decimal::ZERO {
            return Decimal::ZERO;
        }

        // Clamp to max 25% (quarter-Kelly for safety)
        let quarter = Decimal::new(25, 2); // 0.25
        let clamped = kelly.min(quarter);
        clamped * balance
    }

    fn compute_stats(&self, pair: &str, trades: &[&TradeRecord]) -> EdgeStats {
        let total = trades.len();

        let (winners, losers): (Vec<&&TradeRecord>, Vec<&&TradeRecord>) =
            trades.iter().partition(|t| t.profit > Decimal::ZERO);

        let win_count = winners.len();
        let loss_count = losers.len();

        let win_rate = if total > 0 {
            Decimal::from(win_count as u64) / Decimal::from(total as u64)
        } else {
            Decimal::ZERO
        };

        let avg_win = if win_count > 0 {
            winners.iter().map(|t| t.profit).sum::<Decimal>() / Decimal::from(win_count as u64)
        } else {
            Decimal::ZERO
        };

        let avg_loss = if loss_count > 0 {
            // avg_loss is stored as a positive value (absolute average loss)
            losers.iter().map(|t| t.profit.abs()).sum::<Decimal>()
                / Decimal::from(loss_count as u64)
        } else {
            Decimal::ZERO
        };

        let risk_reward_ratio = if avg_loss > Decimal::ZERO {
            avg_win / avg_loss
        } else if avg_win > Decimal::ZERO {
            // No losses: infinite edge, represent as a large number
            Decimal::new(999, 0)
        } else {
            Decimal::ZERO
        };

        // Expectancy = (win_rate * avg_win) - ((1 - win_rate) * avg_loss)
        let expectancy = (win_rate * avg_win) - ((Decimal::ONE - win_rate) * avg_loss);

        EdgeStats {
            pair: pair.to_string(),
            total_trades: total,
            win_rate,
            avg_win,
            avg_loss,
            expectancy,
            risk_reward_ratio,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssm_core::{ExitReason, Side};

    /// Helper to create Decimal from an integer.
    fn d(v: i64) -> Decimal {
        Decimal::new(v, 0)
    }

    /// Helper to create Decimal with scale (e.g., d_s(5, 1) = 0.5).
    fn d_s(v: i64, scale: u32) -> Decimal {
        Decimal::new(v, scale)
    }

    fn make_trade(symbol: &str, profit: Decimal) -> TradeRecord {
        TradeRecord {
            id: "t1".to_string(),
            symbol: symbol.to_string(),
            side: Side::Buy,
            entry_price: d(100),
            exit_price: d(100) + profit,
            quantity: Decimal::ONE,
            profit,
            profit_pct: profit / d(100),
            entry_time: 0,
            exit_time: 1000,
            duration_candles: 10,
            exit_reason: ExitReason::Signal,
            leverage: 1,
            fee: d_s(1, 1), // 0.1
        }
    }

    #[test]
    fn computes_correct_win_rate() {
        let trades = vec![
            make_trade("BTCUSDT", d(100)),
            make_trade("BTCUSDT", d(50)),
            make_trade("BTCUSDT", d(-30)),
            make_trade("BTCUSDT", d(80)),
        ];

        let analyzer = EdgeAnalyzer::new(1, Decimal::ZERO);
        let stats = analyzer.analyze(&trades);
        assert_eq!(stats.len(), 1);

        let s = &stats[0];
        assert_eq!(s.total_trades, 4);
        // 3 wins out of 4
        assert_eq!(s.win_rate, d(3) / d(4));
    }

    #[test]
    fn computes_correct_expectancy() {
        // 2 wins of +100 each, 2 losses of -50 each
        let trades = vec![
            make_trade("ETHUSDT", d(100)),
            make_trade("ETHUSDT", d(100)),
            make_trade("ETHUSDT", d(-50)),
            make_trade("ETHUSDT", d(-50)),
        ];

        let analyzer = EdgeAnalyzer::new(1, Decimal::ZERO);
        let stats = analyzer.analyze(&trades);
        let s = &stats[0];

        // win_rate = 0.5, avg_win = 100, avg_loss = 50
        // expectancy = 0.5 * 100 - 0.5 * 50 = 50 - 25 = 25
        assert_eq!(s.win_rate, d_s(5, 1));
        assert_eq!(s.avg_win, d(100));
        assert_eq!(s.avg_loss, d(50));
        assert_eq!(s.expectancy, d(25));
    }

    #[test]
    fn filters_low_expectancy_pairs() {
        let trades = vec![
            // BTCUSDT: positive edge
            make_trade("BTCUSDT", d(200)),
            make_trade("BTCUSDT", d(200)),
            make_trade("BTCUSDT", d(-50)),
            // ETHUSDT: negative edge
            make_trade("ETHUSDT", d(10)),
            make_trade("ETHUSDT", d(-100)),
            make_trade("ETHUSDT", d(-100)),
        ];

        let analyzer = EdgeAnalyzer::new(1, d(10));
        let pairs = analyzer.filter_pairs(&trades);

        assert!(pairs.contains(&"BTCUSDT".to_string()));
        assert!(!pairs.contains(&"ETHUSDT".to_string()));
    }

    #[test]
    fn filters_pairs_below_min_trades() {
        let trades = vec![
            make_trade("BTCUSDT", d(100)),
            // Only 1 trade for BTCUSDT
        ];

        let analyzer = EdgeAnalyzer::new(5, Decimal::ZERO);
        let stats = analyzer.analyze(&trades);
        assert!(stats.is_empty());
    }

    #[test]
    fn kelly_returns_zero_for_negative_edge() {
        let stats = EdgeStats {
            pair: "BTCUSDT".to_string(),
            total_trades: 10,
            win_rate: d_s(3, 1), // 0.3
            avg_win: d(50),
            avg_loss: d(100),
            expectancy: d(-55),
            risk_reward_ratio: d_s(5, 1), // 0.5
        };

        let analyzer = EdgeAnalyzer::new(1, Decimal::ZERO);
        let size = analyzer.kelly_size(&stats, d(10000));
        assert_eq!(size, Decimal::ZERO);
    }

    #[test]
    fn kelly_returns_zero_for_zero_risk_reward() {
        let stats = EdgeStats {
            pair: "BTCUSDT".to_string(),
            total_trades: 10,
            win_rate: d_s(5, 1), // 0.5
            avg_win: Decimal::ZERO,
            avg_loss: Decimal::ZERO,
            expectancy: Decimal::ZERO,
            risk_reward_ratio: Decimal::ZERO,
        };

        let analyzer = EdgeAnalyzer::new(1, Decimal::ZERO);
        let size = analyzer.kelly_size(&stats, d(10000));
        assert_eq!(size, Decimal::ZERO);
    }

    #[test]
    fn kelly_clamps_to_reasonable_values() {
        // Very high win rate + high RR -> kelly would be large, but we clamp to 25%
        let stats = EdgeStats {
            pair: "BTCUSDT".to_string(),
            total_trades: 100,
            win_rate: d_s(9, 1), // 0.9
            avg_win: d(200),
            avg_loss: d(10),
            expectancy: d(179),
            risk_reward_ratio: d(20),
        };

        let analyzer = EdgeAnalyzer::new(1, Decimal::ZERO);
        let size = analyzer.kelly_size(&stats, d(10000));

        // Kelly = 0.9 - 0.1/20 = 0.9 - 0.005 = 0.895 -> clamped to 0.25
        // Size = 0.25 * 10000 = 2500
        assert_eq!(size, d(2500));
    }

    #[test]
    fn kelly_normal_positive_edge() {
        // win_rate = 0.6, avg_win = 100, avg_loss = 80 => RR = 1.25
        // Kelly = 0.6 - 0.4/1.25 = 0.6 - 0.32 = 0.28 -> clamped to 0.25
        let stats = EdgeStats {
            pair: "BTCUSDT".to_string(),
            total_trades: 50,
            win_rate: d_s(6, 1), // 0.6
            avg_win: d(100),
            avg_loss: d(80),
            expectancy: d(28),
            risk_reward_ratio: d_s(125, 2), // 1.25
        };

        let analyzer = EdgeAnalyzer::new(1, Decimal::ZERO);
        let size = analyzer.kelly_size(&stats, d(10000));
        // Kelly = 0.28 -> clamped to 0.25 -> 2500
        assert_eq!(size, d(2500));
    }

    #[test]
    fn kelly_small_positive_edge() {
        // win_rate = 0.55, avg_win = 100, avg_loss = 100 => RR = 1.0
        // Kelly = 0.55 - 0.45/1.0 = 0.10
        // Size = 0.10 * 10000 = 1000
        let stats = EdgeStats {
            pair: "BTCUSDT".to_string(),
            total_trades: 50,
            win_rate: d_s(55, 2), // 0.55
            avg_win: d(100),
            avg_loss: d(100),
            expectancy: d(10),
            risk_reward_ratio: Decimal::ONE,
        };

        let analyzer = EdgeAnalyzer::new(1, Decimal::ZERO);
        let size = analyzer.kelly_size(&stats, d(10000));
        assert_eq!(size, d(1000));
    }
}
