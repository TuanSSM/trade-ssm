use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rust_decimal::Decimal;
use ssm_core::Side;
use ssm_engine::types::{EngineParams, PermissionFlags, SymbolBuf, TradeEvent};
use ssm_engine::{CoreEngine, GateResult, RingBuffer, SeqLock};

fn bench_gate_evaluation(c: &mut Criterion) {
    let params = EngineParams::default();
    c.bench_function("gate_buy_open", |b| {
        b.iter(|| {
            ssm_engine::gate::evaluate_buy_gate(
                black_box(PermissionFlags::ALL),
                black_box(Decimal::from(1)),
                black_box(Decimal::from(50000)),
                black_box(&params),
            )
        });
    });

    let mut blocked_params = EngineParams::default();
    blocked_params.circuit_breaker = true;
    c.bench_function("gate_buy_blocked", |b| {
        b.iter(|| {
            ssm_engine::gate::evaluate_buy_gate(
                black_box(PermissionFlags::ALL),
                black_box(Decimal::from(1)),
                black_box(Decimal::from(50000)),
                black_box(&blocked_params),
            )
        });
    });
}

fn bench_seqlock(c: &mut Criterion) {
    let params = EngineParams::default();
    let lock = SeqLock::new(params);
    let mut last_seq = 0u64;
    let mut cached = params;

    // Prime the cache
    lock.read_if_changed(&mut last_seq, &mut cached);

    c.bench_function("seqlock_cache_hit", |b| {
        b.iter(|| {
            black_box(lock.read_if_changed(black_box(&mut last_seq), black_box(&mut cached)))
        });
    });

    c.bench_function("seqlock_cache_miss", |b| {
        let mut seq = 0u64;
        let mut cache = params;
        b.iter(|| {
            lock.write(black_box(params));
            black_box(lock.read_if_changed(&mut seq, &mut cache));
        });
    });

    c.bench_function("seqlock_read", |b| {
        b.iter(|| black_box(lock.read()));
    });
}

fn bench_spsc(c: &mut Criterion) {
    let ring: RingBuffer<TradeEvent> = RingBuffer::new(4096);
    let event = TradeEvent {
        kind: ssm_engine::types::TradeEventKind::PositionOpened,
        symbol: SymbolBuf::new("BTCUSDT").unwrap(),
        side: Side::Buy,
        price: Decimal::from(50000),
        quantity: Decimal::from(1),
        realized_pnl: Decimal::ZERO,
        timestamp: 1234567890,
    };

    c.bench_function("spsc_push_pop", |b| {
        b.iter(|| {
            ring.push(black_box(event)).unwrap();
            black_box(ring.pop().unwrap());
        });
    });
}

fn bench_core_engine(c: &mut Criterion) {
    let sym = SymbolBuf::new("BTCUSDT").unwrap();
    let params = EngineParams::default();

    c.bench_function("core_on_tick_no_position", |b| {
        let mut engine = CoreEngine::new(sym, params);
        let seqlock = SeqLock::new(params);
        let ring: RingBuffer<TradeEvent> = RingBuffer::new(64);
        b.iter(|| {
            engine.on_tick(black_box(Decimal::from(50000)), &seqlock, &ring);
        });
    });

    c.bench_function("core_on_tick_with_position", |b| {
        let mut engine = CoreEngine::new(sym, params);
        let seqlock = SeqLock::new(params);
        let ring: RingBuffer<TradeEvent> = RingBuffer::new(4096);
        engine.on_signal(
            Side::Buy,
            Decimal::from(1),
            Decimal::from(50000),
            1000,
            &ring,
        );
        // Drain the open event
        ring.pop();
        b.iter(|| {
            engine.on_tick(black_box(Decimal::from(50100)), &seqlock, &ring);
        });
    });

    c.bench_function("core_apply_fill", |b| {
        b.iter_batched(
            || CoreEngine::new(sym, params),
            |mut engine| {
                black_box(engine.apply_fill(
                    Side::Buy,
                    Decimal::from(1),
                    Decimal::from(50000),
                    1000,
                ));
            },
            criterion::BatchSize::SmallInput,
        );
    });

    c.bench_function("core_full_signal_cycle", |b| {
        let ring: RingBuffer<TradeEvent> = RingBuffer::new(4096);
        let mut engine = CoreEngine::new(sym, params);
        let mut i = 0u64;
        b.iter(|| {
            i += 1;
            let result = engine.on_signal(
                black_box(Side::Buy),
                black_box(Decimal::from(1)),
                black_box(Decimal::from(50000)),
                black_box(i as i64),
                &ring,
            );
            // Drain event to prevent ring fill
            if result == GateResult::Open {
                ring.pop();
            }
        });
    });
}

fn bench_mark_to_market(c: &mut Criterion) {
    let sym = SymbolBuf::new("BTCUSDT").unwrap();
    let params = EngineParams::default();
    let mut engine = CoreEngine::new(sym, params);
    let ring: RingBuffer<TradeEvent> = RingBuffer::new(64);
    engine.on_signal(
        Side::Buy,
        Decimal::from(1),
        Decimal::from(50000),
        1000,
        &ring,
    );

    c.bench_function("mark_to_market", |b| {
        b.iter(|| {
            engine.mark_to_market(black_box(Decimal::from(50100)));
        });
    });
}

criterion_group!(
    benches,
    bench_gate_evaluation,
    bench_seqlock,
    bench_spsc,
    bench_core_engine,
    bench_mark_to_market,
);
criterion_main!(benches);
