use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rust_decimal::Decimal;
use ssm_core::Candle;

fn make_candles(n: usize) -> Vec<Candle> {
    (0..n)
        .map(|i| {
            let price = Decimal::from(50000 + (i % 1000) as u64);
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

fn bench_cvd(c: &mut Criterion) {
    let candles = make_candles(1000);
    c.bench_function("cvd_1000_candles", |b| {
        b.iter(|| ssm_indicators::cvd::analyze_cvd(black_box(&candles), 1000));
    });
}

fn bench_rsi(c: &mut Criterion) {
    let candles = make_candles(1000);
    c.bench_function("rsi_1000_candles", |b| {
        b.iter(|| ssm_indicators::rsi::rsi(black_box(&candles), 14));
    });
}

fn bench_ema(c: &mut Criterion) {
    let candles = make_candles(1000);
    c.bench_function("ema_1000_candles", |b| {
        b.iter(|| ssm_indicators::ema::ema(black_box(&candles), 20));
    });
}

fn bench_macd(c: &mut Criterion) {
    let candles = make_candles(1000);
    c.bench_function("macd_1000_candles", |b| {
        b.iter(|| ssm_indicators::macd::macd(black_box(&candles), 12, 26, 9));
    });
}

criterion_group!(benches, bench_cvd, bench_rsi, bench_ema, bench_macd);
criterion_main!(benches);
