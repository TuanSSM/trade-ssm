use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use ssm_core::{AIAction, Candle, ExitReason, Side, Signal, TradeRecord};

use crate::slippage::SlippageModel;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BacktestConfig {
    pub initial_balance: Decimal,
    /// Trading fee rate per side, e.g. 0.0004 = 0.04%.
    pub fee_rate: Decimal,
    pub leverage: u32,
    /// Funding rate per 8 h period, e.g. 0.0001 = 0.01%.
    pub funding_rate: Decimal,
    /// Fraction of balance to risk per trade, e.g. 0.10 = 10%.
    pub position_size_pct: Decimal,
    /// Slippage model applied to fill prices.
    #[serde(skip)]
    pub slippage: SlippageModel,
}

impl Default for BacktestConfig {
    fn default() -> Self {
        Self {
            initial_balance: Decimal::from(10_000),
            fee_rate: Decimal::new(4, 4), // 0.04%
            leverage: 1,
            funding_rate: Decimal::new(1, 4),       // 0.01%
            position_size_pct: Decimal::new(10, 2), // 10%
            slippage: SlippageModel::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Result
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BacktestResult {
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub win_rate: f64,
    pub total_profit: Decimal,
    pub total_profit_pct: Decimal,
    pub avg_profit: Decimal,
    pub avg_duration_candles: f64,
    pub best_trade: Decimal,
    pub worst_trade: Decimal,
    pub max_drawdown: Decimal,
    pub max_drawdown_pct: Decimal,
    pub max_drawdown_duration: u64,
    pub sharpe_ratio: f64,
    pub sortino_ratio: f64,
    pub profit_factor: Decimal,
    pub final_balance: Decimal,
    pub trades: Vec<TradeRecord>,
}

// ---------------------------------------------------------------------------
// Internal open-position bookkeeping
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct OpenPosition {
    side: Side,
    entry_price: Decimal,
    quantity: Decimal,
    entry_candle_idx: usize,
    entry_time: i64,
    total_funding_fee: Decimal,
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

pub struct BacktestEngine {
    config: BacktestConfig,
}

impl BacktestEngine {
    pub fn new(config: BacktestConfig) -> Self {
        Self { config }
    }

    /// Run a backtest over `candles`.
    ///
    /// `signal_fn` receives **only closed candles** (`&candles[..i]` where `i`
    /// is the index of the current forming candle) and may return a `Signal`.
    /// Anti-repainting: the forming candle is never visible to the signal
    /// function; execution happens on the *next* candle's open (approximated
    /// by its close for simplicity in this candle-level simulation).
    pub fn run<F>(&mut self, candles: &[Candle], mut signal_fn: F) -> BacktestResult
    where
        F: FnMut(&[Candle]) -> Option<Signal>,
    {
        let cfg = &self.config;
        let mut balance = cfg.initial_balance;
        let mut position: Option<OpenPosition> = None;
        let mut trades: Vec<TradeRecord> = Vec::new();
        let mut trade_counter: u64 = 0;

        // Equity tracking for drawdown
        let mut peak_equity = balance;
        let mut max_dd = Decimal::ZERO;
        let mut max_dd_pct = Decimal::ZERO;
        // Drawdown duration: count of candles while in drawdown
        let mut current_dd_start: Option<usize> = None;
        let mut max_dd_duration: u64 = 0;

        // Per-trade returns for sharpe / sortino
        let mut trade_returns: Vec<f64> = Vec::new();

        // Funding: approximate 8 h as a candle count.
        // We derive candle duration from the first two candles (if available).
        let candle_duration_ms: i64 = if candles.len() >= 2 {
            (candles[1].open_time - candles[0].open_time).max(1)
        } else {
            900_000 // default 15 m
        };
        let eight_hours_ms: i64 = 8 * 3600 * 1000;
        let funding_interval_candles: usize = (eight_hours_ms / candle_duration_ms).max(1) as usize;

        // We need at least 2 candles (one closed + one forming) to do anything.
        if candles.len() < 2 {
            return Self::empty_result(balance);
        }

        // Main loop: i is the index of the "forming" candle.
        // Closed candles = candles[..i].
        for i in 1..candles.len() {
            let closed = &candles[..i];
            let forming = &candles[i];

            // --- Apply periodic funding fee to open position ---
            if let Some(ref mut pos) = position {
                // Every `funding_interval_candles` candles since entry, charge funding.
                let candles_held = i - pos.entry_candle_idx;
                if candles_held > 0 && candles_held % funding_interval_candles == 0 {
                    let notional = pos.entry_price * pos.quantity;
                    let funding_fee = notional * cfg.funding_rate;
                    pos.total_funding_fee += funding_fee;
                    balance -= funding_fee;
                }
            }

            // --- Get signal from closed candles ---
            let signal = signal_fn(closed);

            // --- Process signal ---
            if let Some(sig) = signal {
                match sig.action {
                    // Enter long: only if flat
                    AIAction::EnterLong if position.is_none() => {
                        let raw_price = forming.close;
                        let notional = balance * cfg.position_size_pct;
                        let exec_price = cfg.slippage.apply(
                            raw_price,
                            Side::Buy,
                            Some(notional * Decimal::from(cfg.leverage)),
                            Some(forming.volume),
                        );
                        let qty = (notional * Decimal::from(cfg.leverage)) / exec_price;
                        if qty > Decimal::ZERO {
                            let entry_fee = notional * Decimal::from(cfg.leverage) * cfg.fee_rate;
                            balance -= entry_fee;
                            position = Some(OpenPosition {
                                side: Side::Buy,
                                entry_price: exec_price,
                                quantity: qty,
                                entry_candle_idx: i,
                                entry_time: forming.open_time,
                                total_funding_fee: Decimal::ZERO,
                            });
                        }
                    }
                    // Enter short: only if flat
                    AIAction::EnterShort if position.is_none() => {
                        let raw_price = forming.close;
                        let notional = balance * cfg.position_size_pct;
                        let exec_price = cfg.slippage.apply(
                            raw_price,
                            Side::Sell,
                            Some(notional * Decimal::from(cfg.leverage)),
                            Some(forming.volume),
                        );
                        let qty = (notional * Decimal::from(cfg.leverage)) / exec_price;
                        if qty > Decimal::ZERO {
                            let entry_fee = notional * Decimal::from(cfg.leverage) * cfg.fee_rate;
                            balance -= entry_fee;
                            position = Some(OpenPosition {
                                side: Side::Sell,
                                entry_price: exec_price,
                                quantity: qty,
                                entry_candle_idx: i,
                                entry_time: forming.open_time,
                                total_funding_fee: Decimal::ZERO,
                            });
                        }
                    }
                    // Exit long
                    AIAction::ExitLong => {
                        if let Some(ref pos) = position {
                            if pos.side == Side::Buy {
                                let exec_price = cfg.slippage.apply(
                                    forming.close,
                                    Side::Sell,
                                    Some(pos.quantity * forming.close),
                                    Some(forming.volume),
                                );
                                let record = Self::close_position(
                                    pos,
                                    exec_price,
                                    i,
                                    forming.open_time,
                                    cfg,
                                    &mut trade_counter,
                                    &mut balance,
                                );
                                trade_returns.push(record.profit_pct.to_f64().unwrap_or(0.0));
                                trades.push(record);
                                position = None;
                            }
                        }
                    }
                    // Exit short
                    AIAction::ExitShort => {
                        if let Some(ref pos) = position {
                            if pos.side == Side::Sell {
                                let exec_price = cfg.slippage.apply(
                                    forming.close,
                                    Side::Buy,
                                    Some(pos.quantity * forming.close),
                                    Some(forming.volume),
                                );
                                let record = Self::close_position(
                                    pos,
                                    exec_price,
                                    i,
                                    forming.open_time,
                                    cfg,
                                    &mut trade_counter,
                                    &mut balance,
                                );
                                trade_returns.push(record.profit_pct.to_f64().unwrap_or(0.0));
                                trades.push(record);
                                position = None;
                            }
                        }
                    }
                    // Neutral or mismatched action — do nothing
                    _ => {}
                }
            }

            // --- Equity / drawdown tracking ---
            let equity = Self::current_equity(balance, &position, forming);
            if equity > peak_equity {
                peak_equity = equity;
                current_dd_start = None;
            }
            if peak_equity > Decimal::ZERO {
                let dd = peak_equity - equity;
                let dd_pct = dd / peak_equity * Decimal::from(100);
                if dd > max_dd {
                    max_dd = dd;
                }
                if dd_pct > max_dd_pct {
                    max_dd_pct = dd_pct;
                }
                if dd > Decimal::ZERO {
                    if current_dd_start.is_none() {
                        current_dd_start = Some(i);
                    }
                    let dur = (i - current_dd_start.unwrap()) as u64 + 1;
                    if dur > max_dd_duration {
                        max_dd_duration = dur;
                    }
                }
            }
        }

        // Force-close any remaining position on last candle
        if let Some(ref pos) = position {
            let last = candles.last().unwrap();
            let exec_price = last.close;
            let idx = candles.len() - 1;
            let record = Self::close_position(
                pos,
                exec_price,
                idx,
                last.open_time,
                &self.config,
                &mut trade_counter,
                &mut balance,
            );
            trade_returns.push(record.profit_pct.to_f64().unwrap_or(0.0));
            trades.push(record);
        }

        // --- Compute aggregate statistics ---
        let total_trades = trades.len();
        let winning_trades = trades.iter().filter(|t| t.profit > Decimal::ZERO).count();
        let losing_trades = trades.iter().filter(|t| t.profit < Decimal::ZERO).count();
        let win_rate = if total_trades > 0 {
            winning_trades as f64 / total_trades as f64
        } else {
            0.0
        };

        let total_profit = balance - cfg.initial_balance;
        let total_profit_pct = if cfg.initial_balance > Decimal::ZERO {
            total_profit / cfg.initial_balance * Decimal::from(100)
        } else {
            Decimal::ZERO
        };

        let avg_profit = if total_trades > 0 {
            total_profit / Decimal::from(total_trades as u64)
        } else {
            Decimal::ZERO
        };

        let avg_duration_candles = if total_trades > 0 {
            trades
                .iter()
                .map(|t| t.duration_candles as f64)
                .sum::<f64>()
                / total_trades as f64
        } else {
            0.0
        };

        let best_trade = trades
            .iter()
            .map(|t| t.profit)
            .max()
            .unwrap_or(Decimal::ZERO);
        let worst_trade = trades
            .iter()
            .map(|t| t.profit)
            .min()
            .unwrap_or(Decimal::ZERO);

        let gross_profit: Decimal = trades
            .iter()
            .filter(|t| t.profit > Decimal::ZERO)
            .map(|t| t.profit)
            .sum();
        let gross_loss: Decimal = trades
            .iter()
            .filter(|t| t.profit < Decimal::ZERO)
            .map(|t| t.profit.abs())
            .sum();
        let profit_factor = if gross_loss > Decimal::ZERO {
            gross_profit / gross_loss
        } else if gross_profit > Decimal::ZERO {
            Decimal::from(999)
        } else {
            Decimal::ZERO
        };

        let sharpe_ratio = Self::sharpe(&trade_returns);
        let sortino_ratio = Self::sortino(&trade_returns);

        BacktestResult {
            total_trades,
            winning_trades,
            losing_trades,
            win_rate,
            total_profit,
            total_profit_pct,
            avg_profit,
            avg_duration_candles,
            best_trade,
            worst_trade,
            max_drawdown: max_dd,
            max_drawdown_pct: max_dd_pct,
            max_drawdown_duration: max_dd_duration,
            sharpe_ratio,
            sortino_ratio,
            profit_factor,
            final_balance: balance,
            trades,
        }
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn close_position(
        pos: &OpenPosition,
        exit_price: Decimal,
        exit_candle_idx: usize,
        exit_time: i64,
        cfg: &BacktestConfig,
        counter: &mut u64,
        balance: &mut Decimal,
    ) -> TradeRecord {
        *counter += 1;
        let leverage = Decimal::from(cfg.leverage);
        let notional_exit = exit_price * pos.quantity;
        let exit_fee = notional_exit * cfg.fee_rate;

        // PnL is already amplified by leverage through the larger quantity
        // (qty = margin * leverage / price), so no extra multiplier needed.
        let raw_pnl = match pos.side {
            Side::Buy => (exit_price - pos.entry_price) * pos.quantity,
            Side::Sell => (pos.entry_price - exit_price) * pos.quantity,
        };
        let net_pnl = raw_pnl - exit_fee - pos.total_funding_fee;
        *balance += net_pnl;

        let notional_entry = pos.entry_price * pos.quantity;
        let total_fee = notional_entry * cfg.fee_rate + exit_fee + pos.total_funding_fee;
        let cost_basis = notional_entry / leverage; // margin used
        let profit_pct = if cost_basis > Decimal::ZERO {
            net_pnl / cost_basis * Decimal::from(100)
        } else {
            Decimal::ZERO
        };

        let duration_candles = (exit_candle_idx - pos.entry_candle_idx) as u64;

        TradeRecord {
            id: format!("bt-{counter}"),
            symbol: String::from("BACKTEST"),
            side: pos.side,
            entry_price: pos.entry_price,
            exit_price,
            quantity: pos.quantity,
            profit: net_pnl,
            profit_pct,
            entry_time: pos.entry_time,
            exit_time,
            duration_candles,
            exit_reason: ExitReason::Signal,
            leverage: cfg.leverage,
            fee: total_fee,
        }
    }

    fn current_equity(
        balance: Decimal,
        position: &Option<OpenPosition>,
        current_candle: &Candle,
    ) -> Decimal {
        match position {
            Some(pos) => {
                let unrealized = match pos.side {
                    Side::Buy => (current_candle.close - pos.entry_price) * pos.quantity,
                    Side::Sell => (pos.entry_price - current_candle.close) * pos.quantity,
                };
                balance + unrealized
            }
            None => balance,
        }
    }

    fn sharpe(returns: &[f64]) -> f64 {
        if returns.len() < 2 {
            return 0.0;
        }
        let n = returns.len() as f64;
        let mean = returns.iter().sum::<f64>() / n;
        let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
        let std = var.sqrt();
        if std == 0.0 {
            return 0.0;
        }
        mean / std
    }

    fn sortino(returns: &[f64]) -> f64 {
        if returns.len() < 2 {
            return 0.0;
        }
        let n = returns.len() as f64;
        let mean = returns.iter().sum::<f64>() / n;
        let downside_var = returns
            .iter()
            .map(|r| if *r < 0.0 { r.powi(2) } else { 0.0 })
            .sum::<f64>()
            / (n - 1.0);
        let downside_std = downside_var.sqrt();
        if downside_std == 0.0 {
            return 0.0;
        }
        mean / downside_std
    }

    fn empty_result(balance: Decimal) -> BacktestResult {
        BacktestResult {
            total_trades: 0,
            winning_trades: 0,
            losing_trades: 0,
            win_rate: 0.0,
            total_profit: Decimal::ZERO,
            total_profit_pct: Decimal::ZERO,
            avg_profit: Decimal::ZERO,
            avg_duration_candles: 0.0,
            best_trade: Decimal::ZERO,
            worst_trade: Decimal::ZERO,
            max_drawdown: Decimal::ZERO,
            max_drawdown_pct: Decimal::ZERO,
            max_drawdown_duration: 0,
            sharpe_ratio: 0.0,
            sortino_ratio: 0.0,
            profit_factor: Decimal::ZERO,
            final_balance: balance,
            trades: Vec::new(),
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::collections::HashMap;

    /// Build a candle with a given close price and open_time.
    fn make_candle(close: i64, time_index: i64) -> Candle {
        let price = Decimal::from(close);
        Candle {
            open_time: time_index * 900_000,
            open: price,
            high: price,
            low: price,
            close: price,
            volume: Decimal::from(100),
            close_time: (time_index + 1) * 900_000 - 1,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: Decimal::from(60),
            taker_sell_volume: Decimal::from(40),
        }
    }

    fn default_signal(action: AIAction) -> Signal {
        Signal {
            timestamp: 0,
            symbol: "BACKTEST".into(),
            action,
            confidence: 1.0,
            source: "test".into(),
            metadata: HashMap::new(),
        }
    }

    fn zero_fee_config() -> BacktestConfig {
        BacktestConfig {
            initial_balance: Decimal::from(10_000),
            fee_rate: Decimal::ZERO,
            leverage: 1,
            funding_rate: Decimal::ZERO,
            position_size_pct: Decimal::new(10, 2), // 10%
            slippage: SlippageModel::default(),
        }
    }

    // -----------------------------------------------------------------------
    // 1. Signals generate trades with correct PnL
    // -----------------------------------------------------------------------
    #[test]
    fn test_long_trade_profit() {
        // Candle sequence: 100, 100, 110 (enter on candle 1, exit on candle 2)
        let candles = vec![
            make_candle(100, 0),
            make_candle(100, 1),
            make_candle(110, 2),
        ];
        let mut engine = BacktestEngine::new(zero_fee_config());

        // After seeing 1 closed candle -> enter long
        // After seeing 2 closed candles -> exit long
        let result = engine.run(&candles, |closed| {
            if closed.len() == 1 {
                Some(default_signal(AIAction::EnterLong))
            } else if closed.len() == 2 {
                Some(default_signal(AIAction::ExitLong))
            } else {
                None
            }
        });

        assert_eq!(result.total_trades, 1);
        assert_eq!(result.winning_trades, 1);
        let trade = &result.trades[0];
        assert_eq!(trade.side, Side::Buy);
        assert_eq!(trade.entry_price, Decimal::from(100));
        assert_eq!(trade.exit_price, Decimal::from(110));
        // qty = (10000 * 0.10 * 1) / 100 = 10
        // profit = (110-100) * 10 = 100
        assert_eq!(trade.profit, Decimal::from(100));
        assert!(result.final_balance > result.trades[0].entry_price); // sanity
    }

    #[test]
    fn test_short_trade_profit() {
        let candles = vec![make_candle(100, 0), make_candle(100, 1), make_candle(90, 2)];
        let mut engine = BacktestEngine::new(zero_fee_config());

        let result = engine.run(&candles, |closed| {
            if closed.len() == 1 {
                Some(default_signal(AIAction::EnterShort))
            } else if closed.len() == 2 {
                Some(default_signal(AIAction::ExitShort))
            } else {
                None
            }
        });

        assert_eq!(result.total_trades, 1);
        assert_eq!(result.winning_trades, 1);
        let trade = &result.trades[0];
        assert_eq!(trade.side, Side::Sell);
        // profit = (100-90)*10 = 100
        assert_eq!(trade.profit, Decimal::from(100));
    }

    // -----------------------------------------------------------------------
    // 2. Fee deduction
    // -----------------------------------------------------------------------
    #[test]
    fn test_fee_deduction() {
        let candles = vec![
            make_candle(100, 0),
            make_candle(100, 1),
            make_candle(100, 2),
        ];
        let mut cfg = zero_fee_config();
        cfg.fee_rate = Decimal::new(1, 2); // 1% per side for easy math
        let mut engine = BacktestEngine::new(cfg);

        let result = engine.run(&candles, |closed| {
            if closed.len() == 1 {
                Some(default_signal(AIAction::EnterLong))
            } else if closed.len() == 2 {
                Some(default_signal(AIAction::ExitLong))
            } else {
                None
            }
        });

        assert_eq!(result.total_trades, 1);
        let trade = &result.trades[0];
        // entry notional = 10000*0.10 = 1000, qty = 1000/100 = 10
        // entry fee = 1000 * 0.01 = 10
        // exit notional = 100*10 = 1000, exit fee = 1000 * 0.01 = 10
        // raw pnl = 0 (price unchanged)
        // net pnl = 0 - 10 (exit fee) = -10
        // total fee recorded = entry_fee + exit_fee = 20
        assert_eq!(trade.fee, Decimal::from(20));
        assert!(trade.profit < Decimal::ZERO); // lost money to fees
        assert!(result.final_balance < Decimal::from(10_000));
    }

    // -----------------------------------------------------------------------
    // 3. Drawdown calculation
    // -----------------------------------------------------------------------
    #[test]
    fn test_drawdown_calculation() {
        // Price: 100, 100, 80, 100, 100
        // Enter long at candle 1 (price 100), price drops to 80 then back up
        let candles = vec![
            make_candle(100, 0),
            make_candle(100, 1),
            make_candle(80, 2),
            make_candle(100, 3),
            make_candle(100, 4),
        ];
        let mut engine = BacktestEngine::new(zero_fee_config());

        let result = engine.run(&candles, |closed| {
            if closed.len() == 1 {
                Some(default_signal(AIAction::EnterLong))
            } else if closed.len() == 4 {
                Some(default_signal(AIAction::ExitLong))
            } else {
                None
            }
        });

        // Max drawdown should be > 0 due to the dip to 80
        assert!(result.max_drawdown > Decimal::ZERO);
        assert!(result.max_drawdown_pct > Decimal::ZERO);
    }

    // -----------------------------------------------------------------------
    // 4. Win rate
    // -----------------------------------------------------------------------
    #[test]
    fn test_win_rate_calculation() {
        // Two trades: one winner, one loser
        let candles = vec![
            make_candle(100, 0),
            make_candle(100, 1),
            make_candle(110, 2), // exit first trade (win)
            make_candle(110, 3), // enter second trade
            make_candle(100, 4), // exit second trade (loss)
        ];
        let mut engine = BacktestEngine::new(zero_fee_config());

        let result = engine.run(&candles, |closed| match closed.len() {
            1 => Some(default_signal(AIAction::EnterLong)),
            2 => Some(default_signal(AIAction::ExitLong)),
            3 => Some(default_signal(AIAction::EnterLong)),
            4 => Some(default_signal(AIAction::ExitLong)),
            _ => None,
        });

        assert_eq!(result.total_trades, 2);
        assert_eq!(result.winning_trades, 1);
        assert_eq!(result.losing_trades, 1);
        assert!((result.win_rate - 0.5).abs() < 1e-9);
    }

    // -----------------------------------------------------------------------
    // 5. Anti-repainting: only closed candles passed to signal_fn
    // -----------------------------------------------------------------------
    #[test]
    fn test_anti_repainting() {
        let candles = vec![
            make_candle(100, 0),
            make_candle(200, 1),
            make_candle(300, 2),
            make_candle(400, 3),
        ];
        let mut max_closed_len = 0usize;
        let mut engine = BacktestEngine::new(zero_fee_config());

        // Collect the lengths of closed candle slices passed to signal_fn
        let mut seen_lengths: Vec<usize> = Vec::new();
        engine.run(&candles, |closed| {
            seen_lengths.push(closed.len());
            // Verify that the closed slice never includes the forming candle
            // The forming candle has index = closed.len(), so closed must be
            // strictly less than total candles.
            assert!(closed.len() < candles.len());
            // The last closed candle's index is closed.len()-1
            // which must be < the forming candle index = closed.len()
            if closed.len() > max_closed_len {
                max_closed_len = closed.len();
            }
            None
        });

        // We should have been called with lengths 1, 2, 3
        assert_eq!(seen_lengths, vec![1, 2, 3]);
        assert_eq!(max_closed_len, 3);
    }

    // -----------------------------------------------------------------------
    // 6. Leverage multiplier effect on PnL
    // -----------------------------------------------------------------------
    #[test]
    fn test_leverage_multiplier() {
        let candles = vec![
            make_candle(100, 0),
            make_candle(100, 1),
            make_candle(110, 2),
        ];

        // 1x leverage
        let mut cfg1 = zero_fee_config();
        cfg1.leverage = 1;
        let mut engine1 = BacktestEngine::new(cfg1);
        let r1 = engine1.run(&candles, |closed| {
            if closed.len() == 1 {
                Some(default_signal(AIAction::EnterLong))
            } else if closed.len() == 2 {
                Some(default_signal(AIAction::ExitLong))
            } else {
                None
            }
        });

        // 5x leverage
        let mut cfg5 = zero_fee_config();
        cfg5.leverage = 5;
        let mut engine5 = BacktestEngine::new(cfg5);
        let r5 = engine5.run(&candles, |closed| {
            if closed.len() == 1 {
                Some(default_signal(AIAction::EnterLong))
            } else if closed.len() == 2 {
                Some(default_signal(AIAction::ExitLong))
            } else {
                None
            }
        });

        // With 5x leverage, notional is 5x bigger, so PnL should be 5x
        let pnl_1x = r1.trades[0].profit;
        let pnl_5x = r5.trades[0].profit;
        assert_eq!(pnl_5x, pnl_1x * Decimal::from(5));
    }

    // -----------------------------------------------------------------------
    // 7. No trades when signal_fn returns None
    // -----------------------------------------------------------------------
    #[test]
    fn test_no_trades_when_no_signal() {
        let candles = vec![
            make_candle(100, 0),
            make_candle(110, 1),
            make_candle(120, 2),
        ];
        let mut engine = BacktestEngine::new(zero_fee_config());

        let result = engine.run(&candles, |_closed| None);

        assert_eq!(result.total_trades, 0);
        assert_eq!(result.winning_trades, 0);
        assert_eq!(result.losing_trades, 0);
        assert_eq!(result.final_balance, Decimal::from(10_000));
        assert_eq!(result.win_rate, 0.0);
    }

    // -----------------------------------------------------------------------
    // 8. Funding fee application
    // -----------------------------------------------------------------------
    #[test]
    fn test_funding_fee_application() {
        // 15 m candles -> 32 candles per 8 h. Hold for 32 candles to trigger funding.
        let num_candles = 34;
        let candles: Vec<Candle> = (0..num_candles)
            .map(|i| make_candle(100, i as i64))
            .collect();

        let mut cfg = zero_fee_config();
        cfg.funding_rate = Decimal::new(1, 2); // 1% per 8 h for easy math
        let mut engine = BacktestEngine::new(cfg);

        let result = engine.run(&candles, |closed| {
            if closed.len() == 1 {
                Some(default_signal(AIAction::EnterLong))
            } else if closed.len() == (num_candles - 1) as usize {
                Some(default_signal(AIAction::ExitLong))
            } else {
                None
            }
        });

        assert_eq!(result.total_trades, 1);
        let trade = &result.trades[0];
        // Funding fee should have been charged (balance should be less than initial)
        // Price didn't change, so only fees/funding reduced balance
        assert!(trade.fee > Decimal::ZERO);
        assert!(result.final_balance < Decimal::from(10_000));
    }

    // -----------------------------------------------------------------------
    // Additional: force-close at end
    // -----------------------------------------------------------------------
    #[test]
    fn test_force_close_at_end() {
        let candles = vec![
            make_candle(100, 0),
            make_candle(100, 1),
            make_candle(110, 2),
        ];
        let mut engine = BacktestEngine::new(zero_fee_config());

        // Enter but never explicitly exit
        let result = engine.run(&candles, |closed| {
            if closed.len() == 1 {
                Some(default_signal(AIAction::EnterLong))
            } else {
                None
            }
        });

        // Position should be force-closed at end
        assert_eq!(result.total_trades, 1);
        assert_eq!(result.trades[0].exit_price, Decimal::from(110));
    }

    // -----------------------------------------------------------------------
    // Neutral signal does nothing
    // -----------------------------------------------------------------------
    #[test]
    fn test_neutral_signal_does_nothing() {
        let candles = vec![
            make_candle(100, 0),
            make_candle(110, 1),
            make_candle(120, 2),
        ];
        let mut engine = BacktestEngine::new(zero_fee_config());

        let result = engine.run(&candles, |_| Some(default_signal(AIAction::Neutral)));

        assert_eq!(result.total_trades, 0);
        assert_eq!(result.final_balance, Decimal::from(10_000));
    }

    // -----------------------------------------------------------------------
    // Cannot enter while already in position
    // -----------------------------------------------------------------------
    #[test]
    fn test_no_double_entry() {
        let candles = vec![
            make_candle(100, 0),
            make_candle(100, 1),
            make_candle(100, 2),
            make_candle(110, 3),
        ];
        let mut engine = BacktestEngine::new(zero_fee_config());

        // Send EnterLong on every candle
        let result = engine.run(&candles, |_| Some(default_signal(AIAction::EnterLong)));

        // Only one trade (force-closed at end), not multiple entries
        assert_eq!(result.total_trades, 1);
    }

    // -----------------------------------------------------------------------
    // Sharpe and sortino are computed
    // -----------------------------------------------------------------------
    #[test]
    fn test_sharpe_sortino_with_mixed_trades() {
        let candles = vec![
            make_candle(100, 0),
            make_candle(100, 1),
            make_candle(110, 2), // win
            make_candle(110, 3),
            make_candle(100, 4), // loss
            make_candle(100, 5),
            make_candle(115, 6), // win
        ];
        let mut engine = BacktestEngine::new(zero_fee_config());

        let result = engine.run(&candles, |closed| match closed.len() {
            1 => Some(default_signal(AIAction::EnterLong)),
            2 => Some(default_signal(AIAction::ExitLong)),
            3 => Some(default_signal(AIAction::EnterLong)),
            4 => Some(default_signal(AIAction::ExitLong)),
            5 => Some(default_signal(AIAction::EnterLong)),
            6 => Some(default_signal(AIAction::ExitLong)),
            _ => None,
        });

        assert_eq!(result.total_trades, 3);
        // With mixed trades, sharpe and sortino should be finite
        assert!(result.sharpe_ratio.is_finite());
        assert!(result.sortino_ratio.is_finite());
    }

    // -----------------------------------------------------------------------
    // Profit factor
    // -----------------------------------------------------------------------
    #[test]
    fn test_profit_factor_all_winners() {
        let candles = vec![
            make_candle(100, 0),
            make_candle(100, 1),
            make_candle(110, 2),
        ];
        let mut engine = BacktestEngine::new(zero_fee_config());

        let result = engine.run(&candles, |closed| {
            if closed.len() == 1 {
                Some(default_signal(AIAction::EnterLong))
            } else if closed.len() == 2 {
                Some(default_signal(AIAction::ExitLong))
            } else {
                None
            }
        });

        // All winners => profit_factor = 999 (sentinel)
        assert_eq!(result.profit_factor, Decimal::from(999));
    }

    // -----------------------------------------------------------------------
    // Slippage reduces profitability
    // -----------------------------------------------------------------------
    #[test]
    fn test_slippage_reduces_profit() {
        let candles = vec![
            make_candle(100, 0),
            make_candle(100, 1),
            make_candle(110, 2),
        ];
        let signal_fn = |closed: &[Candle]| -> Option<Signal> {
            if closed.len() == 1 {
                Some(default_signal(AIAction::EnterLong))
            } else if closed.len() == 2 {
                Some(default_signal(AIAction::ExitLong))
            } else {
                None
            }
        };

        // Without slippage
        let mut no_slip = BacktestEngine::new(zero_fee_config());
        let r1 = no_slip.run(&candles, signal_fn);

        // With 50 bps slippage
        let mut slip_cfg = zero_fee_config();
        slip_cfg.slippage = SlippageModel::FixedBps(Decimal::from(50));
        let mut with_slip = BacktestEngine::new(slip_cfg);
        let r2 = with_slip.run(&candles, signal_fn);

        // Slippage should reduce final balance
        assert!(
            r2.final_balance < r1.final_balance,
            "slippage should reduce profit: {} < {}",
            r2.final_balance,
            r1.final_balance
        );
    }
}
