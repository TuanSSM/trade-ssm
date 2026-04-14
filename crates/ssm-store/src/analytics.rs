use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use ssm_core::TradeRecord;
use std::collections::HashMap;

/// Performance summary over a set of trades.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PerformanceSummary {
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub win_rate: f64,
    pub total_profit: Decimal,
    pub avg_profit: Decimal,
    pub avg_win: Decimal,
    pub avg_loss: Decimal,
    pub best_trade: Decimal,
    pub worst_trade: Decimal,
    pub profit_factor: f64,
    pub avg_duration_candles: f64,
}

/// Daily performance bucket.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DailyPerformance {
    pub date: String,
    pub trades: usize,
    pub profit: Decimal,
    pub wins: usize,
    pub losses: usize,
}

/// Per-strategy breakdown.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StrategyBreakdown {
    pub strategy: String,
    pub trades: usize,
    pub profit: Decimal,
    pub win_rate: f64,
}

/// Compute a performance summary from a set of trades.
pub fn summarize(trades: &[TradeRecord]) -> PerformanceSummary {
    if trades.is_empty() {
        return PerformanceSummary {
            total_trades: 0,
            winning_trades: 0,
            losing_trades: 0,
            win_rate: 0.0,
            total_profit: Decimal::ZERO,
            avg_profit: Decimal::ZERO,
            avg_win: Decimal::ZERO,
            avg_loss: Decimal::ZERO,
            best_trade: Decimal::ZERO,
            worst_trade: Decimal::ZERO,
            profit_factor: 0.0,
            avg_duration_candles: 0.0,
        };
    }

    let total = trades.len();
    let wins: Vec<&TradeRecord> = trades.iter().filter(|t| t.profit > Decimal::ZERO).collect();
    let losses: Vec<&TradeRecord> = trades
        .iter()
        .filter(|t| t.profit <= Decimal::ZERO)
        .collect();

    let total_profit: Decimal = trades.iter().map(|t| t.profit).sum();
    let gross_wins: Decimal = wins.iter().map(|t| t.profit).sum();
    let gross_losses: Decimal = losses.iter().map(|t| t.profit.abs()).sum();

    let avg_profit = total_profit / Decimal::from(total);
    let avg_win = if wins.is_empty() {
        Decimal::ZERO
    } else {
        gross_wins / Decimal::from(wins.len())
    };
    let avg_loss = if losses.is_empty() {
        Decimal::ZERO
    } else {
        gross_losses / Decimal::from(losses.len())
    };

    let best = trades.iter().map(|t| t.profit).max().unwrap_or_default();
    let worst = trades.iter().map(|t| t.profit).min().unwrap_or_default();

    let profit_factor = if gross_losses > Decimal::ZERO {
        gross_wins.to_f64().unwrap_or(0.0) / gross_losses.to_f64().unwrap_or(1.0)
    } else if gross_wins > Decimal::ZERO {
        f64::INFINITY
    } else {
        0.0
    };

    let total_duration: u64 = trades.iter().map(|t| t.duration_candles).sum();
    let avg_duration = total_duration as f64 / total as f64;

    PerformanceSummary {
        total_trades: total,
        winning_trades: wins.len(),
        losing_trades: losses.len(),
        win_rate: wins.len() as f64 / total as f64 * 100.0,
        total_profit,
        avg_profit,
        avg_win,
        avg_loss,
        best_trade: best,
        worst_trade: worst,
        profit_factor,
        avg_duration_candles: avg_duration,
    }
}

/// Group trades by day (UTC) and compute daily performance.
pub fn daily_performance(trades: &[TradeRecord]) -> Vec<DailyPerformance> {
    let mut by_day: HashMap<String, Vec<&TradeRecord>> = HashMap::new();

    for trade in trades {
        let dt = chrono::DateTime::from_timestamp_millis(trade.exit_time).unwrap_or_default();
        let date = dt.format("%Y-%m-%d").to_string();
        by_day.entry(date).or_default().push(trade);
    }

    let mut days: Vec<DailyPerformance> = by_day
        .into_iter()
        .map(|(date, day_trades)| {
            let profit: Decimal = day_trades.iter().map(|t| t.profit).sum();
            let wins = day_trades
                .iter()
                .filter(|t| t.profit > Decimal::ZERO)
                .count();
            let losses = day_trades.len() - wins;
            DailyPerformance {
                date,
                trades: day_trades.len(),
                profit,
                wins,
                losses,
            }
        })
        .collect();

    days.sort_by(|a, b| a.date.cmp(&b.date));
    days
}

/// Compute equity curve from trades (cumulative PnL).
pub fn equity_curve(trades: &[TradeRecord], initial_balance: Decimal) -> Vec<(i64, Decimal)> {
    let mut sorted: Vec<&TradeRecord> = trades.iter().collect();
    sorted.sort_by_key(|t| t.exit_time);

    let mut equity = initial_balance;
    let mut curve = vec![(0i64, equity)];

    for trade in sorted {
        equity += trade.profit;
        curve.push((trade.exit_time, equity));
    }
    curve
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssm_core::{ExitReason, Side};

    fn trade(id: &str, profit: i64, exit_time: i64, duration: u64) -> TradeRecord {
        TradeRecord {
            id: id.into(),
            symbol: "BTCUSDT".into(),
            side: Side::Buy,
            entry_price: Decimal::from(50000),
            exit_price: Decimal::from(51000),
            quantity: Decimal::from(1),
            profit: Decimal::from(profit),
            profit_pct: Decimal::from(2),
            entry_time: exit_time - 60000,
            exit_time,
            duration_candles: duration,
            exit_reason: ExitReason::Roi,
            leverage: 1,
            fee: Decimal::ZERO,
        }
    }

    #[test]
    fn summarize_empty() {
        let s = summarize(&[]);
        assert_eq!(s.total_trades, 0);
        assert_eq!(s.win_rate, 0.0);
    }

    #[test]
    fn summarize_mixed() {
        let trades = vec![
            trade("1", 1000, 1700000000000, 5),
            trade("2", -500, 1700001000000, 3),
            trade("3", 200, 1700002000000, 7),
        ];
        let s = summarize(&trades);
        assert_eq!(s.total_trades, 3);
        assert_eq!(s.winning_trades, 2);
        assert_eq!(s.losing_trades, 1);
        assert_eq!(s.total_profit, Decimal::from(700));
        assert_eq!(s.best_trade, Decimal::from(1000));
        assert_eq!(s.worst_trade, Decimal::from(-500));
        assert!(s.profit_factor > 1.0);
    }

    #[test]
    fn daily_groups_by_date() {
        let trades = vec![
            trade("1", 100, 1700000000000, 1),
            trade("2", 200, 1700000001000, 1),
            trade("3", -50, 1700090000000, 1),
        ];
        let days = daily_performance(&trades);
        assert!(!days.is_empty());
        // All trades should be grouped
        let total: usize = days.iter().map(|d| d.trades).sum();
        assert_eq!(total, 3);
    }

    #[test]
    fn equity_curve_accumulates() {
        let trades = vec![
            trade("1", 100, 1000, 1),
            trade("2", -50, 2000, 1),
            trade("3", 200, 3000, 1),
        ];
        let curve = equity_curve(&trades, Decimal::from(10000));
        assert_eq!(curve.len(), 4); // initial + 3 trades
        assert_eq!(curve[0].1, Decimal::from(10000));
        assert_eq!(curve[3].1, Decimal::from(10250)); // 10000 + 100 - 50 + 200
    }

    #[test]
    fn summarize_all_wins() {
        let trades = vec![trade("1", 500, 1000, 1), trade("2", 300, 2000, 1)];
        let s = summarize(&trades);
        assert_eq!(s.winning_trades, 2);
        assert_eq!(s.losing_trades, 0);
        assert!(s.profit_factor.is_infinite());
    }
}
