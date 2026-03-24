use rust_decimal::Decimal;
use ssm_core::{ExitReason, PairLock, TradeRecord};

// ---------------------------------------------------------------------------
// Protection trait
// ---------------------------------------------------------------------------

/// Protection plugin — decides whether to lock a pair based on trade history.
pub trait Protection: Send + Sync {
    fn name(&self) -> &str;
    fn should_lock(&self, trades: &[TradeRecord], pair: &str, now: i64) -> Option<PairLock>;
}

// ---------------------------------------------------------------------------
// StoplossGuard
// ---------------------------------------------------------------------------

/// Halt trading on a pair after N stop-losses within a time window.
pub struct StoplossGuard {
    pub max_stoplosses: usize,
    pub lookback_seconds: i64,
    pub lock_seconds: i64,
}

impl Protection for StoplossGuard {
    fn name(&self) -> &str {
        "StoplossGuard"
    }

    fn should_lock(&self, trades: &[TradeRecord], pair: &str, now: i64) -> Option<PairLock> {
        let cutoff = now - self.lookback_seconds;
        let count = trades
            .iter()
            .filter(|t| {
                t.symbol == pair && t.exit_reason == ExitReason::Stoploss && t.exit_time > cutoff
            })
            .count();

        if count >= self.max_stoplosses {
            Some(PairLock {
                symbol: pair.to_string(),
                reason: format!(
                    "StoplossGuard: {} stoplosses in last {}s",
                    count, self.lookback_seconds
                ),
                until: now + self.lock_seconds,
            })
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// MaxDrawdownProtection
// ---------------------------------------------------------------------------

/// Pause trading when cumulative loss in the lookback window exceeds a threshold.
pub struct MaxDrawdownProtection {
    pub max_drawdown_pct: Decimal,
    pub lookback_seconds: i64,
    pub lock_seconds: i64,
}

impl Protection for MaxDrawdownProtection {
    fn name(&self) -> &str {
        "MaxDrawdownProtection"
    }

    fn should_lock(&self, trades: &[TradeRecord], pair: &str, now: i64) -> Option<PairLock> {
        let cutoff = now - self.lookback_seconds;
        let total_profit_pct: Decimal = trades
            .iter()
            .filter(|t| t.symbol == pair && t.exit_time > cutoff)
            .map(|t| t.profit_pct)
            .sum();

        // Drawdown is negative profit; lock when loss exceeds threshold.
        if total_profit_pct < Decimal::ZERO && total_profit_pct.abs() >= self.max_drawdown_pct {
            Some(PairLock {
                symbol: pair.to_string(),
                reason: format!(
                    "MaxDrawdownProtection: cumulative {}% exceeds -{}% threshold",
                    total_profit_pct, self.max_drawdown_pct
                ),
                until: now + self.lock_seconds,
            })
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// CooldownPeriod
// ---------------------------------------------------------------------------

/// Lock a pair for N seconds after any exit.
pub struct CooldownPeriod {
    pub cooldown_seconds: i64,
}

impl Protection for CooldownPeriod {
    fn name(&self) -> &str {
        "CooldownPeriod"
    }

    fn should_lock(&self, trades: &[TradeRecord], pair: &str, now: i64) -> Option<PairLock> {
        let last_exit = trades
            .iter()
            .filter(|t| t.symbol == pair)
            .map(|t| t.exit_time)
            .max();

        if let Some(exit_time) = last_exit {
            if now - exit_time < self.cooldown_seconds {
                return Some(PairLock {
                    symbol: pair.to_string(),
                    reason: format!(
                        "CooldownPeriod: {}s cooldown after last exit",
                        self.cooldown_seconds
                    ),
                    until: exit_time + self.cooldown_seconds,
                });
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// LowProfitPairs
// ---------------------------------------------------------------------------

/// Lock pairs whose cumulative profit over the lookback window is below a
/// minimum threshold.
pub struct LowProfitPairs {
    pub lookback_seconds: i64,
    pub min_profit_pct: Decimal,
    pub lock_seconds: i64,
}

impl Protection for LowProfitPairs {
    fn name(&self) -> &str {
        "LowProfitPairs"
    }

    fn should_lock(&self, trades: &[TradeRecord], pair: &str, now: i64) -> Option<PairLock> {
        let cutoff = now - self.lookback_seconds;
        let matching: Vec<_> = trades
            .iter()
            .filter(|t| t.symbol == pair && t.exit_time > cutoff)
            .collect();

        if matching.is_empty() {
            return None;
        }

        let total_profit_pct: Decimal = matching.iter().map(|t| t.profit_pct).sum();

        if total_profit_pct < self.min_profit_pct {
            Some(PairLock {
                symbol: pair.to_string(),
                reason: format!(
                    "LowProfitPairs: cumulative {}% below {}% minimum",
                    total_profit_pct, self.min_profit_pct
                ),
                until: now + self.lock_seconds,
            })
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// ProtectionStack
// ---------------------------------------------------------------------------

/// Combines multiple protection plugins. Returns the first lock found.
pub struct ProtectionStack {
    protections: Vec<Box<dyn Protection>>,
}

impl ProtectionStack {
    pub fn new() -> Self {
        Self {
            protections: Vec::new(),
        }
    }

    pub fn add(&mut self, p: Box<dyn Protection>) {
        self.protections.push(p);
    }

    /// Check all protections, return first lock found.
    pub fn check(&self, trades: &[TradeRecord], pair: &str, now: i64) -> Option<PairLock> {
        for p in &self.protections {
            if let Some(lock) = p.should_lock(trades, pair, now) {
                return Some(lock);
            }
        }
        None
    }
}

impl Default for ProtectionStack {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ssm_core::Side;

    fn dec(v: i64) -> Decimal {
        Decimal::from(v)
    }

    fn make_trade(
        symbol: &str,
        exit_reason: ExitReason,
        exit_time: i64,
        profit_pct: Decimal,
    ) -> TradeRecord {
        TradeRecord {
            id: "test".to_string(),
            symbol: symbol.to_string(),
            side: Side::Buy,
            entry_price: Decimal::from(100),
            exit_price: Decimal::from(99),
            quantity: Decimal::from(1),
            profit: profit_pct,
            profit_pct,
            entry_time: exit_time - 60,
            exit_time,
            duration_candles: 1,
            exit_reason,
            leverage: 1,
            fee: Decimal::ZERO,
        }
    }

    // -- StoplossGuard ---------------------------------------------------------

    #[test]
    fn stoploss_guard_locks_after_n_stoplosses() {
        let guard = StoplossGuard {
            max_stoplosses: 3,
            lookback_seconds: 3600,
            lock_seconds: 600,
        };
        let now = 10_000;
        let trades = vec![
            make_trade("BTCUSDT", ExitReason::Stoploss, 9_000, dec(-1)),
            make_trade("BTCUSDT", ExitReason::Stoploss, 9_500, dec(-1)),
            make_trade("BTCUSDT", ExitReason::Stoploss, 9_800, dec(-1)),
        ];
        let lock = guard.should_lock(&trades, "BTCUSDT", now);
        assert!(lock.is_some());
        let lock = lock.unwrap();
        assert_eq!(lock.symbol, "BTCUSDT");
        assert_eq!(lock.until, now + 600);
    }

    #[test]
    fn stoploss_guard_no_lock_below_threshold() {
        let guard = StoplossGuard {
            max_stoplosses: 3,
            lookback_seconds: 3600,
            lock_seconds: 600,
        };
        let now = 10_000;
        let trades = vec![
            make_trade("BTCUSDT", ExitReason::Stoploss, 9_000, dec(-1)),
            make_trade("BTCUSDT", ExitReason::Stoploss, 9_500, dec(-1)),
        ];
        assert!(guard.should_lock(&trades, "BTCUSDT", now).is_none());
    }

    #[test]
    fn stoploss_guard_respects_lookback_window() {
        let guard = StoplossGuard {
            max_stoplosses: 3,
            lookback_seconds: 3600,
            lock_seconds: 600,
        };
        let now = 10_000;
        // Two stoplosses within window, one outside (exit_time 5000, cutoff = 6400)
        let trades = vec![
            make_trade("BTCUSDT", ExitReason::Stoploss, 5_000, dec(-1)),
            make_trade("BTCUSDT", ExitReason::Stoploss, 9_000, dec(-1)),
            make_trade("BTCUSDT", ExitReason::Stoploss, 9_500, dec(-1)),
        ];
        // Only 2 within window (9000, 9500); 5000 < cutoff 6400
        assert!(guard.should_lock(&trades, "BTCUSDT", now).is_none());
    }

    // -- MaxDrawdownProtection ------------------------------------------------

    #[test]
    fn max_drawdown_locks_on_excessive_losses() {
        let prot = MaxDrawdownProtection {
            max_drawdown_pct: dec(5),
            lookback_seconds: 3600,
            lock_seconds: 600,
        };
        let now = 10_000;
        let trades = vec![
            make_trade("BTCUSDT", ExitReason::Stoploss, 9_000, dec(-3)),
            make_trade("BTCUSDT", ExitReason::Stoploss, 9_500, dec(-3)),
        ];
        let lock = prot.should_lock(&trades, "BTCUSDT", now);
        assert!(lock.is_some());
        assert_eq!(lock.unwrap().until, now + 600);
    }

    #[test]
    fn max_drawdown_no_lock_when_within_threshold() {
        let prot = MaxDrawdownProtection {
            max_drawdown_pct: dec(10),
            lookback_seconds: 3600,
            lock_seconds: 600,
        };
        let now = 10_000;
        let trades = vec![
            make_trade("BTCUSDT", ExitReason::Stoploss, 9_000, dec(-3)),
            make_trade("BTCUSDT", ExitReason::Signal, 9_500, dec(2)),
        ];
        // Net = -1%, threshold is 10%
        assert!(prot.should_lock(&trades, "BTCUSDT", now).is_none());
    }

    // -- CooldownPeriod -------------------------------------------------------

    #[test]
    fn cooldown_locks_recently_exited_pair() {
        let prot = CooldownPeriod {
            cooldown_seconds: 300,
        };
        let now = 10_000;
        let trades = vec![make_trade("BTCUSDT", ExitReason::Signal, 9_900, dec(1))];
        let lock = prot.should_lock(&trades, "BTCUSDT", now);
        assert!(lock.is_some());
        assert_eq!(lock.unwrap().until, 9_900 + 300);
    }

    #[test]
    fn cooldown_allows_after_expiry() {
        let prot = CooldownPeriod {
            cooldown_seconds: 300,
        };
        let now = 10_300;
        let trades = vec![make_trade("BTCUSDT", ExitReason::Signal, 9_900, dec(1))];
        // 10300 - 9900 = 400 >= 300
        assert!(prot.should_lock(&trades, "BTCUSDT", now).is_none());
    }

    // -- LowProfitPairs -------------------------------------------------------

    #[test]
    fn low_profit_pairs_locks_negative_pairs() {
        let prot = LowProfitPairs {
            lookback_seconds: 3600,
            min_profit_pct: dec(0),
            lock_seconds: 600,
        };
        let now = 10_000;
        let trades = vec![
            make_trade("BTCUSDT", ExitReason::Stoploss, 9_000, dec(-2)),
            make_trade("BTCUSDT", ExitReason::Signal, 9_500, dec(1)),
        ];
        // Net = -1% < 0%
        let lock = prot.should_lock(&trades, "BTCUSDT", now);
        assert!(lock.is_some());
        assert_eq!(lock.unwrap().until, now + 600);
    }

    #[test]
    fn low_profit_pairs_allows_profitable_pair() {
        let prot = LowProfitPairs {
            lookback_seconds: 3600,
            min_profit_pct: dec(0),
            lock_seconds: 600,
        };
        let now = 10_000;
        let trades = vec![
            make_trade("BTCUSDT", ExitReason::Signal, 9_000, dec(3)),
            make_trade("BTCUSDT", ExitReason::Signal, 9_500, dec(1)),
        ];
        assert!(prot.should_lock(&trades, "BTCUSDT", now).is_none());
    }

    // -- ProtectionStack ------------------------------------------------------

    #[test]
    fn stack_composes_multiple_protections() {
        let mut stack = ProtectionStack::new();
        stack.add(Box::new(CooldownPeriod {
            cooldown_seconds: 300,
        }));
        stack.add(Box::new(StoplossGuard {
            max_stoplosses: 2,
            lookback_seconds: 3600,
            lock_seconds: 600,
        }));

        let now = 10_000;
        let trades = vec![make_trade("BTCUSDT", ExitReason::Signal, 9_900, dec(1))];
        // CooldownPeriod should fire first
        let lock = stack.check(&trades, "BTCUSDT", now);
        assert!(lock.is_some());
        assert!(lock.unwrap().reason.contains("CooldownPeriod"));
    }

    #[test]
    fn stack_returns_none_when_no_lock_needed() {
        let mut stack = ProtectionStack::new();
        stack.add(Box::new(CooldownPeriod {
            cooldown_seconds: 60,
        }));
        stack.add(Box::new(StoplossGuard {
            max_stoplosses: 5,
            lookback_seconds: 3600,
            lock_seconds: 600,
        }));

        let now = 10_000;
        // Trade exited long ago, no stoplosses
        let trades = vec![make_trade("BTCUSDT", ExitReason::Signal, 5_000, dec(1))];
        assert!(stack.check(&trades, "BTCUSDT", now).is_none());
    }
}
