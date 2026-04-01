use proptest::prelude::*;
use rust_decimal::Decimal;
use ssm_core::{AIAction, Candle, Signal};
use ssm_execution::backtest::{BacktestConfig, BacktestEngine};
use std::collections::HashMap;

fn make_candle(close: u64, time_index: i64) -> Candle {
    let price = Decimal::from(close);
    Candle {
        open_time: time_index * 900_000,
        open: price,
        high: price + Decimal::from(10),
        low: price - Decimal::from(10),
        close: price,
        volume: Decimal::from(100),
        close_time: (time_index + 1) * 900_000 - 1,
        quote_volume: Decimal::ZERO,
        trades: 10,
        taker_buy_volume: Decimal::from(60),
        taker_sell_volume: Decimal::from(40),
    }
}

fn zero_fee_config() -> BacktestConfig {
    BacktestConfig {
        initial_balance: Decimal::from(10_000),
        fee_rate: Decimal::ZERO,
        leverage: 1,
        funding_rate: Decimal::ZERO,
        position_size_pct: Decimal::new(10, 2),
        slippage: Default::default(),
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

proptest! {
    /// No-signal strategy: final balance equals initial balance.
    #[test]
    fn no_signal_preserves_balance(
        n in 2usize..100
    ) {
        let candles: Vec<Candle> = (0..n).map(|i| make_candle(100 + (i % 50) as u64, i as i64)).collect();
        let mut engine = BacktestEngine::new(zero_fee_config());
        let result = engine.run(&candles, |_| None);
        prop_assert_eq!(result.final_balance, Decimal::from(10_000));
        prop_assert_eq!(result.total_trades, 0);
    }

    /// Win rate is always in [0, 1].
    #[test]
    fn win_rate_bounded(
        prices in prop::collection::vec(50u64..200, 4..50)
    ) {
        let candles: Vec<Candle> = prices.iter().enumerate()
            .map(|(i, &p)| make_candle(p, i as i64))
            .collect();
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
        prop_assert!(result.win_rate >= 0.0);
        prop_assert!(result.win_rate <= 1.0);
    }

    /// Max drawdown is always non-negative.
    #[test]
    fn drawdown_non_negative(
        prices in prop::collection::vec(50u64..200, 4..50)
    ) {
        let candles: Vec<Candle> = prices.iter().enumerate()
            .map(|(i, &p)| make_candle(p, i as i64))
            .collect();
        let mut engine = BacktestEngine::new(zero_fee_config());
        let result = engine.run(&candles, |closed| match closed.len() {
            1 => Some(default_signal(AIAction::EnterLong)),
            3 => Some(default_signal(AIAction::ExitLong)),
            _ => None,
        });
        prop_assert!(result.max_drawdown >= Decimal::ZERO);
        prop_assert!(result.max_drawdown_pct >= Decimal::ZERO);
    }

    /// Higher fee rates never increase final balance (for same trade sequence).
    #[test]
    fn higher_fees_never_increase_balance(
        fee_bps in 0u32..100
    ) {
        let candles: Vec<Candle> = (0..5).map(|i| make_candle(100 + i * 5, i as i64)).collect();
        let signal_fn = |closed: &[Candle]| -> Option<Signal> {
            if closed.len() == 1 {
                Some(default_signal(AIAction::EnterLong))
            } else if closed.len() == 3 {
                Some(default_signal(AIAction::ExitLong))
            } else {
                None
            }
        };

        let mut cfg_low = zero_fee_config();
        let mut engine_low = BacktestEngine::new(cfg_low.clone());
        let r_low = engine_low.run(&candles, signal_fn);

        cfg_low.fee_rate = Decimal::new(fee_bps as i64, 4);
        let mut engine_high = BacktestEngine::new(cfg_low);
        let r_high = engine_high.run(&candles, signal_fn);

        prop_assert!(r_high.final_balance <= r_low.final_balance);
    }
}
