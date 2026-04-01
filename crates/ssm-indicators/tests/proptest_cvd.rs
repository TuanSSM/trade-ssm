use proptest::prelude::*;
use rust_decimal::Decimal;
use ssm_core::Candle;
use ssm_indicators::cvd::analyze_cvd;

fn arb_candle() -> impl Strategy<Value = Candle> {
    (1i64..=1000, 1u64..=200, 1u64..=200).prop_map(|(time, buy, sell)| {
        let bv = Decimal::from(buy);
        let sv = Decimal::from(sell);
        Candle {
            open_time: time * 900_000,
            open: Decimal::from(100),
            high: Decimal::from(110),
            low: Decimal::from(90),
            close: Decimal::from(105),
            volume: bv + sv,
            close_time: time * 900_000 + 899_999,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: bv,
            taker_sell_volume: sv,
        }
    })
}

proptest! {
    /// CVD is deterministic: same input → same output.
    #[test]
    fn cvd_deterministic(candles in prop::collection::vec(arb_candle(), 1..50)) {
        let a = analyze_cvd(&candles, candles.len());
        let b = analyze_cvd(&candles, candles.len());
        prop_assert_eq!(a.total_cvd, b.total_cvd);
        prop_assert_eq!(a.deltas.len(), b.deltas.len());
        for i in 0..a.deltas.len() {
            prop_assert_eq!(a.deltas[i], b.deltas[i]);
            prop_assert_eq!(a.cumulative[i], b.cumulative[i]);
        }
    }

    /// Anti-repainting: adding one candle must not change previous cumulative values.
    #[test]
    fn cvd_no_repainting(candles in prop::collection::vec(arb_candle(), 2..30)) {
        let n = candles.len() - 1;
        let short = &candles[..n];
        let long = &candles;
        let r_short = analyze_cvd(short, n);
        let r_long = analyze_cvd(long, long.len());

        for i in 0..r_short.cumulative.len() {
            prop_assert_eq!(
                r_short.cumulative[i],
                r_long.cumulative[i],
                "repainting at index {}", i
            );
        }
    }

    /// Never panics on any input.
    #[test]
    fn cvd_never_panics(
        candles in prop::collection::vec(arb_candle(), 0..100),
        window in 0usize..200
    ) {
        let _ = analyze_cvd(&candles, window);
    }

    /// When all candles have buy > sell, final CVD should be positive.
    #[test]
    fn cvd_positive_when_all_buys_dominate(
        count in 2usize..30
    ) {
        let candles: Vec<Candle> = (0..count).map(|i| {
            Candle {
                open_time: i as i64 * 900_000,
                open: Decimal::from(100),
                high: Decimal::from(110),
                low: Decimal::from(90),
                close: Decimal::from(105),
                volume: Decimal::from(100),
                close_time: i as i64 * 900_000 + 899_999,
                quote_volume: Decimal::ZERO,
                trades: 10,
                taker_buy_volume: Decimal::from(70),
                taker_sell_volume: Decimal::from(30),
            }
        }).collect();

        let a = analyze_cvd(&candles, candles.len());
        prop_assert!(a.total_cvd > Decimal::ZERO);
    }
}
