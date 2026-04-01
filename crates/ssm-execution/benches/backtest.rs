use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rust_decimal::Decimal;
use ssm_core::{AIAction, Candle, Signal};
use ssm_execution::backtest::{BacktestConfig, BacktestEngine};
use std::collections::HashMap;

fn make_candles(n: usize) -> Vec<Candle> {
    (0..n)
        .map(|i| {
            let base = 50000 + (i % 500) as u64;
            let price = Decimal::from(base);
            Candle {
                open_time: i as i64 * 900_000,
                open: price,
                high: price + Decimal::from(100),
                low: price - Decimal::from(100),
                close: price + Decimal::from(50),
                volume: Decimal::from(100),
                close_time: i as i64 * 900_000 + 899_999,
                quote_volume: Decimal::from(5_000_000),
                trades: 500,
                taker_buy_volume: Decimal::from(60),
                taker_sell_volume: Decimal::from(40),
            }
        })
        .collect()
}

fn make_signal(action: AIAction) -> Signal {
    Signal {
        timestamp: 0,
        symbol: "BENCH".into(),
        action,
        confidence: 1.0,
        source: "bench".into(),
        metadata: HashMap::new(),
    }
}

fn bench_backtest_10k(c: &mut Criterion) {
    let candles = make_candles(10_000);
    let config = BacktestConfig::default();

    c.bench_function("backtest_10k_candles_alternating", |b| {
        b.iter(|| {
            let mut engine = BacktestEngine::new(config.clone());
            engine.run(black_box(&candles), |closed| match closed.len() % 10 {
                1 => Some(make_signal(AIAction::EnterLong)),
                5 => Some(make_signal(AIAction::ExitLong)),
                _ => None,
            })
        });
    });
}

fn bench_backtest_neutral(c: &mut Criterion) {
    let candles = make_candles(10_000);
    let config = BacktestConfig::default();

    c.bench_function("backtest_10k_candles_no_trades", |b| {
        b.iter(|| {
            let mut engine = BacktestEngine::new(config.clone());
            engine.run(black_box(&candles), |_| None)
        });
    });
}

criterion_group!(benches, bench_backtest_10k, bench_backtest_neutral);
criterion_main!(benches);
