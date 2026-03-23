use serde::Serialize;
use ssm_core::Side;

/// A completed trade record for metrics calculation.
#[derive(Debug, Clone, Serialize)]
pub struct CompletedTrade {
    pub side: Side,
    pub entry_price: f64,
    pub exit_price: f64,
    pub duration: usize,
    pub pnl: f64,
    pub fees: f64,
}

/// Comprehensive episode performance metrics.
#[derive(Debug, Clone, Serialize)]
pub struct EpisodeMetrics {
    pub initial_balance: f64,
    pub final_balance: f64,
    pub equity_curve: Vec<f64>,

    // Return metrics
    pub total_return_pct: f64,
    pub buy_and_hold_return_pct: f64,
    pub alpha: f64,

    // Risk metrics
    pub max_drawdown_pct: f64,
    pub sharpe_ratio: f64,
    pub sortino_ratio: f64,

    // Trade metrics
    pub total_trades: u32,
    pub winning_trades: u32,
    pub losing_trades: u32,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub largest_win: f64,
    pub largest_loss: f64,
    pub avg_hold_duration: f64,
    pub total_fees_paid: f64,
}

/// Accumulates trade and equity data during an episode, then computes final metrics.
#[derive(Clone)]
pub struct MetricsAccumulator {
    trades: Vec<CompletedTrade>,
    equity_curve: Vec<f64>,
    initial_balance: f64,
    balance: f64,
    peak: f64,
    max_drawdown: f64,
    first_price: f64,
    last_price: f64,
    step_returns: Vec<f64>,
    prev_equity: f64,
}

impl MetricsAccumulator {
    pub fn new(initial_balance: f64, first_price: f64) -> Self {
        Self {
            trades: Vec::new(),
            equity_curve: vec![initial_balance],
            initial_balance,
            balance: initial_balance,
            peak: initial_balance,
            max_drawdown: 0.0,
            first_price,
            last_price: first_price,
            step_returns: Vec::new(),
            prev_equity: initial_balance,
        }
    }

    /// Record equity at each environment step.
    pub fn record_step(&mut self, equity: f64, current_price: f64) {
        self.equity_curve.push(equity);
        self.last_price = current_price;

        if equity > self.peak {
            self.peak = equity;
        }
        if self.peak > 0.0 {
            let drawdown = (self.peak - equity) / self.peak;
            if drawdown > self.max_drawdown {
                self.max_drawdown = drawdown;
            }
        }

        let step_return = if self.prev_equity > 0.0 {
            (equity - self.prev_equity) / self.prev_equity
        } else {
            0.0
        };
        self.step_returns.push(step_return);
        self.prev_equity = equity;
    }

    /// Record a completed trade.
    pub fn record_trade(&mut self, trade: CompletedTrade) {
        self.trades.push(trade);
    }

    /// Update the tracked balance.
    pub fn set_balance(&mut self, balance: f64) {
        self.balance = balance;
    }

    /// Compute all derived metrics. `steps_per_year` is used for Sharpe annualization.
    pub fn finalize(self, steps_per_year: f64) -> EpisodeMetrics {
        let final_balance = self.balance;
        let total_return_pct = if self.initial_balance > 0.0 {
            (final_balance - self.initial_balance) / self.initial_balance * 100.0
        } else {
            0.0
        };

        let buy_and_hold_return_pct = if self.first_price > 0.0 {
            (self.last_price - self.first_price) / self.first_price * 100.0
        } else {
            0.0
        };

        let alpha = total_return_pct - buy_and_hold_return_pct;

        let sharpe_ratio = compute_sharpe(&self.step_returns, steps_per_year);
        let sortino_ratio = compute_sortino(&self.step_returns, steps_per_year);

        let total_trades = self.trades.len() as u32;
        let mut gross_profit = 0.0_f64;
        let mut gross_loss = 0.0_f64;
        let mut winning_trades = 0u32;
        let mut losing_trades = 0u32;
        let mut total_win = 0.0_f64;
        let mut total_loss = 0.0_f64;
        let mut largest_win = 0.0_f64;
        let mut largest_loss = 0.0_f64;
        let mut total_duration = 0usize;
        let mut total_fees = 0.0_f64;

        for trade in &self.trades {
            total_fees += trade.fees;
            total_duration += trade.duration;
            if trade.pnl > 0.0 {
                winning_trades += 1;
                gross_profit += trade.pnl;
                total_win += trade.pnl;
                if trade.pnl > largest_win {
                    largest_win = trade.pnl;
                }
            } else if trade.pnl < 0.0 {
                losing_trades += 1;
                gross_loss += trade.pnl.abs();
                total_loss += trade.pnl.abs();
                if trade.pnl.abs() > largest_loss {
                    largest_loss = trade.pnl.abs();
                }
            }
        }

        let win_rate = if total_trades > 0 {
            winning_trades as f64 / total_trades as f64
        } else {
            0.0
        };

        let profit_factor = if gross_loss > 0.0 {
            gross_profit / gross_loss
        } else if gross_profit > 0.0 {
            f64::INFINITY
        } else {
            0.0
        };

        let avg_win = if winning_trades > 0 {
            total_win / winning_trades as f64
        } else {
            0.0
        };

        let avg_loss = if losing_trades > 0 {
            total_loss / losing_trades as f64
        } else {
            0.0
        };

        let avg_hold_duration = if total_trades > 0 {
            total_duration as f64 / total_trades as f64
        } else {
            0.0
        };

        EpisodeMetrics {
            initial_balance: self.initial_balance,
            final_balance,
            equity_curve: self.equity_curve,
            total_return_pct,
            buy_and_hold_return_pct,
            alpha,
            max_drawdown_pct: self.max_drawdown * 100.0,
            sharpe_ratio,
            sortino_ratio,
            total_trades,
            winning_trades,
            losing_trades,
            win_rate,
            profit_factor,
            avg_win,
            avg_loss,
            largest_win,
            largest_loss,
            avg_hold_duration,
            total_fees_paid: total_fees,
        }
    }
}

fn compute_sharpe(returns: &[f64], steps_per_year: f64) -> f64 {
    if returns.is_empty() {
        return 0.0;
    }
    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;
    let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n;
    let std_dev = variance.sqrt();
    if std_dev < 1e-12 {
        return 0.0;
    }
    (mean / std_dev) * steps_per_year.sqrt()
}

fn compute_sortino(returns: &[f64], steps_per_year: f64) -> f64 {
    if returns.is_empty() {
        return 0.0;
    }
    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;
    let downside_variance = returns.iter().map(|r| r.min(0.0).powi(2)).sum::<f64>() / n;
    let downside_dev = downside_variance.sqrt();
    if downside_dev < 1e-12 {
        return 0.0;
    }
    (mean / downside_dev) * steps_per_year.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_episode_metrics() {
        let acc = MetricsAccumulator::new(10_000.0, 100.0);
        let m = acc.finalize(35040.0);
        assert!((m.initial_balance - 10_000.0).abs() < f64::EPSILON);
        assert!((m.final_balance - 10_000.0).abs() < f64::EPSILON);
        assert!((m.total_return_pct - 0.0).abs() < f64::EPSILON);
        assert_eq!(m.total_trades, 0);
        assert!((m.win_rate - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn single_winning_trade() {
        let mut acc = MetricsAccumulator::new(10_000.0, 100.0);
        acc.set_balance(10_500.0);
        acc.record_trade(CompletedTrade {
            side: Side::Buy,
            entry_price: 100.0,
            exit_price: 105.0,
            duration: 5,
            pnl: 500.0,
            fees: 0.0,
        });
        acc.record_step(10_500.0, 105.0);
        let m = acc.finalize(35040.0);
        assert_eq!(m.total_trades, 1);
        assert_eq!(m.winning_trades, 1);
        assert!((m.win_rate - 1.0).abs() < f64::EPSILON);
        assert!(m.profit_factor.is_infinite());
        assert!((m.avg_hold_duration - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn drawdown_calculation() {
        let mut acc = MetricsAccumulator::new(10_000.0, 100.0);
        // Equity goes: 10000 -> 12000 -> 9000 -> 11000
        acc.record_step(12_000.0, 110.0);
        acc.record_step(9_000.0, 90.0);
        acc.record_step(11_000.0, 105.0);
        acc.set_balance(11_000.0);
        let m = acc.finalize(35040.0);
        // Max drawdown: (12000 - 9000) / 12000 = 25%
        assert!((m.max_drawdown_pct - 25.0).abs() < 0.1);
    }

    #[test]
    fn buy_and_hold_calculation() {
        let mut acc = MetricsAccumulator::new(10_000.0, 100.0);
        acc.record_step(10_000.0, 120.0);
        let m = acc.finalize(35040.0);
        // B&H: (120 - 100) / 100 * 100 = 20%
        assert!((m.buy_and_hold_return_pct - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn sharpe_ratio_flat_returns() {
        let returns = vec![0.0; 100];
        let sharpe = compute_sharpe(&returns, 35040.0);
        assert!((sharpe - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn profit_factor_with_wins_and_losses() {
        let mut acc = MetricsAccumulator::new(10_000.0, 100.0);
        acc.set_balance(10_200.0);
        acc.record_trade(CompletedTrade {
            side: Side::Buy,
            entry_price: 100.0,
            exit_price: 110.0,
            duration: 3,
            pnl: 1000.0,
            fees: 0.0,
        });
        acc.record_trade(CompletedTrade {
            side: Side::Buy,
            entry_price: 110.0,
            exit_price: 102.0,
            duration: 4,
            pnl: -800.0,
            fees: 0.0,
        });
        let m = acc.finalize(35040.0);
        assert_eq!(m.total_trades, 2);
        assert_eq!(m.winning_trades, 1);
        assert_eq!(m.losing_trades, 1);
        // profit_factor = 1000 / 800 = 1.25
        assert!((m.profit_factor - 1.25).abs() < 0.01);
        assert!((m.win_rate - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn all_losing_trades() {
        let mut acc = MetricsAccumulator::new(10_000.0, 100.0);
        acc.set_balance(9_000.0);
        acc.record_trade(CompletedTrade {
            side: Side::Buy,
            entry_price: 100.0,
            exit_price: 95.0,
            duration: 2,
            pnl: -500.0,
            fees: 1.0,
        });
        acc.record_trade(CompletedTrade {
            side: Side::Sell,
            entry_price: 95.0,
            exit_price: 100.0,
            duration: 3,
            pnl: -500.0,
            fees: 1.0,
        });
        let m = acc.finalize(35040.0);
        assert_eq!(m.winning_trades, 0);
        assert_eq!(m.losing_trades, 2);
        assert!((m.win_rate - 0.0).abs() < f64::EPSILON);
        assert!((m.profit_factor - 0.0).abs() < f64::EPSILON);
        assert!((m.total_fees_paid - 2.0).abs() < f64::EPSILON);
        assert!((m.largest_loss - 500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn sortino_with_negative_returns() {
        // All negative returns: sortino should be negative
        let returns = vec![-0.01, -0.02, -0.005, -0.015];
        let sortino = compute_sortino(&returns, 35040.0);
        assert!(sortino < 0.0, "sortino should be negative for all-loss returns");
    }

    #[test]
    fn sharpe_with_positive_returns() {
        let returns = vec![0.01, 0.02, 0.015, 0.012];
        let sharpe = compute_sharpe(&returns, 35040.0);
        assert!(sharpe > 0.0, "sharpe should be positive for all-gain returns");
    }

    #[test]
    fn sharpe_empty_returns() {
        assert!((compute_sharpe(&[], 35040.0) - 0.0).abs() < f64::EPSILON);
        assert!((compute_sortino(&[], 35040.0) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn zero_initial_balance_metrics() {
        let acc = MetricsAccumulator::new(0.0, 100.0);
        let m = acc.finalize(35040.0);
        assert!((m.total_return_pct - 0.0).abs() < f64::EPSILON);
        assert!((m.initial_balance - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn equity_curve_starts_with_initial_balance() {
        let acc = MetricsAccumulator::new(10_000.0, 100.0);
        let m = acc.finalize(35040.0);
        assert_eq!(m.equity_curve.len(), 1);
        assert!((m.equity_curve[0] - 10_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn multiple_step_equity_curve_length() {
        let mut acc = MetricsAccumulator::new(10_000.0, 100.0);
        for i in 0..10 {
            acc.record_step(10_000.0 + i as f64 * 100.0, 100.0 + i as f64);
        }
        let m = acc.finalize(35040.0);
        // Initial + 10 steps = 11
        assert_eq!(m.equity_curve.len(), 11);
    }

    #[test]
    fn no_drawdown_when_equity_only_rises() {
        let mut acc = MetricsAccumulator::new(10_000.0, 100.0);
        acc.record_step(11_000.0, 110.0);
        acc.record_step(12_000.0, 120.0);
        acc.record_step(13_000.0, 130.0);
        acc.set_balance(13_000.0);
        let m = acc.finalize(35040.0);
        assert!((m.max_drawdown_pct - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn alpha_calculation() {
        let mut acc = MetricsAccumulator::new(10_000.0, 100.0);
        // Final price = 120 (B&H = 20%), but we only made 10% return
        acc.record_step(11_000.0, 120.0);
        acc.set_balance(11_000.0);
        let m = acc.finalize(35040.0);
        // alpha = total_return - buy_and_hold = 10% - 20% = -10%
        assert!((m.alpha - (-10.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn sortino_all_positive_returns_is_zero() {
        // When all returns are positive, downside deviation is zero, sortino should be 0
        let returns = vec![0.01, 0.02, 0.03];
        let sortino = compute_sortino(&returns, 35040.0);
        // All positive means no downside, so downside_dev ~ 0 => sortino = 0
        assert!((sortino - 0.0).abs() < f64::EPSILON);
    }
}
