use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ssm_core::Candle;

/// Delta analysis for a sequence of candles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaAnalysis {
    /// Per-candle delta (buy_vol - sell_vol).
    pub candle_deltas: Vec<Decimal>,
    /// Running cumulative delta.
    pub cumulative_delta: Vec<Decimal>,
    /// Detected divergences.
    pub divergences: Vec<DeltaDivergence>,
}

/// A divergence between price and delta.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaDivergence {
    /// Index in the candle array where divergence was detected.
    pub index: usize,
    pub divergence_type: DivergenceType,
    /// Price direction over the lookback window.
    pub price_change: Decimal,
    /// Delta direction over the lookback window.
    pub delta_change: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DivergenceType {
    /// Price rising, delta falling — bearish divergence.
    Bearish,
    /// Price falling, delta rising — bullish divergence.
    Bullish,
}

/// Analyze delta and detect divergences over a lookback window.
///
/// Anti-repainting: only analyzes closed candles.
pub fn analyze_delta(candles: &[Candle], lookback: usize) -> DeltaAnalysis {
    let mut candle_deltas = Vec::with_capacity(candles.len());
    let mut cumulative_delta = Vec::with_capacity(candles.len());
    let mut running = Decimal::ZERO;

    for c in candles {
        let delta = c.taker_buy_volume - c.taker_sell_volume;
        running += delta;
        candle_deltas.push(delta);
        cumulative_delta.push(running);
    }

    let mut divergences = Vec::new();

    if candles.len() >= lookback && lookback >= 2 {
        for i in lookback..candles.len() {
            let price_change = candles[i].close - candles[i - lookback].close;
            let delta_change = cumulative_delta[i] - cumulative_delta[i - lookback];

            if price_change > Decimal::ZERO && delta_change < Decimal::ZERO {
                divergences.push(DeltaDivergence {
                    index: i,
                    divergence_type: DivergenceType::Bearish,
                    price_change,
                    delta_change,
                });
            } else if price_change < Decimal::ZERO && delta_change > Decimal::ZERO {
                divergences.push(DeltaDivergence {
                    index: i,
                    divergence_type: DivergenceType::Bullish,
                    price_change,
                    delta_change,
                });
            }
        }
    }

    DeltaAnalysis {
        candle_deltas,
        cumulative_delta,
        divergences,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn candle(close: &str, buy_vol: &str, sell_vol: &str) -> Candle {
        let c = Decimal::from_str(close).unwrap();
        let bv = Decimal::from_str(buy_vol).unwrap();
        let sv = Decimal::from_str(sell_vol).unwrap();
        Candle {
            open_time: 0,
            open: c,
            high: c + Decimal::from(5),
            low: c - Decimal::from(5),
            close: c,
            volume: bv + sv,
            close_time: 0,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: bv,
            taker_sell_volume: sv,
        }
    }

    #[test]
    fn cumulative_delta_tracking() {
        let candles = vec![
            candle("100", "60", "40"), // delta +20
            candle("101", "70", "30"), // delta +40, cum +60
            candle("102", "50", "50"), // delta 0, cum +60
        ];
        let analysis = analyze_delta(&candles, 2);
        assert_eq!(analysis.candle_deltas.len(), 3);
        assert_eq!(analysis.cumulative_delta[0], Decimal::from(20));
        assert_eq!(analysis.cumulative_delta[1], Decimal::from(60));
        assert_eq!(analysis.cumulative_delta[2], Decimal::from(60));
    }

    #[test]
    fn bearish_divergence_detected() {
        // Price goes up but delta goes down (sellers absorbing)
        let candles = vec![
            candle("100", "70", "30"), // cum delta +40
            candle("101", "60", "40"), // cum delta +60
            candle("105", "30", "70"), // cum delta +20 (delta dropped from 60 to 20)
            candle("110", "25", "75"), // price up, delta down further
        ];
        let analysis = analyze_delta(&candles, 2);
        let bearish: Vec<_> = analysis
            .divergences
            .iter()
            .filter(|d| d.divergence_type == DivergenceType::Bearish)
            .collect();
        assert!(!bearish.is_empty(), "expected bearish divergence");
    }

    #[test]
    fn bullish_divergence_detected() {
        // Price goes down but delta goes up (buyers accumulating)
        let candles = vec![
            candle("110", "30", "70"), // cum delta -40
            candle("108", "40", "60"), // cum delta -60
            candle("105", "80", "20"), // cum delta  0 (delta recovered)
            candle("102", "75", "25"), // price down, delta up
        ];
        let analysis = analyze_delta(&candles, 2);
        let bullish: Vec<_> = analysis
            .divergences
            .iter()
            .filter(|d| d.divergence_type == DivergenceType::Bullish)
            .collect();
        assert!(!bullish.is_empty(), "expected bullish divergence");
    }

    #[test]
    fn no_repainting_delta() {
        let short = vec![candle("100", "60", "40"), candle("101", "70", "30")];
        let mut long = short.clone();
        long.push(candle("102", "50", "50"));

        let r_short = analyze_delta(&short, 2);
        let r_long = analyze_delta(&long, 2);

        for i in 0..r_short.cumulative_delta.len() {
            assert_eq!(
                r_short.cumulative_delta[i], r_long.cumulative_delta[i],
                "delta repainting at index {i}"
            );
        }
    }

    #[test]
    fn empty_candles_returns_empty_analysis() {
        let analysis = analyze_delta(&[], 5);
        assert!(analysis.candle_deltas.is_empty());
        assert!(analysis.cumulative_delta.is_empty());
        assert!(analysis.divergences.is_empty());
    }

    #[test]
    fn single_candle_no_divergence() {
        let candles = vec![candle("100", "70", "30")];
        let analysis = analyze_delta(&candles, 2);
        assert_eq!(analysis.candle_deltas.len(), 1);
        assert_eq!(analysis.candle_deltas[0], Decimal::from(40));
        assert_eq!(analysis.cumulative_delta[0], Decimal::from(40));
        // lookback=2 but only 1 candle, so no divergences possible
        assert!(analysis.divergences.is_empty());
    }

    #[test]
    fn lookback_of_one_produces_no_divergences() {
        // lookback < 2 should produce no divergences per the guard condition
        let candles = vec![
            candle("100", "70", "30"),
            candle("110", "30", "70"),
        ];
        let analysis = analyze_delta(&candles, 1);
        assert!(analysis.divergences.is_empty());
    }

    #[test]
    fn lookback_of_zero_produces_no_divergences() {
        let candles = vec![candle("100", "60", "40"), candle("101", "70", "30")];
        let analysis = analyze_delta(&candles, 0);
        assert!(analysis.divergences.is_empty());
    }

    #[test]
    fn no_divergence_when_price_and_delta_move_same_direction() {
        // Both price and delta go up — no divergence
        let candles = vec![
            candle("100", "60", "40"), // delta +20
            candle("101", "70", "30"), // delta +40, cum +60
            candle("105", "80", "20"), // delta +60, cum +120 — price up, delta up
        ];
        let analysis = analyze_delta(&candles, 2);
        assert!(analysis.divergences.is_empty());
    }

    #[test]
    fn lookback_larger_than_candle_count_produces_no_divergences() {
        let candles = vec![
            candle("100", "60", "40"),
            candle("101", "70", "30"),
        ];
        // lookback=5 but only 2 candles: guard `candles.len() >= lookback` fails
        let analysis = analyze_delta(&candles, 5);
        assert_eq!(analysis.candle_deltas.len(), 2);
        assert_eq!(analysis.cumulative_delta.len(), 2);
        assert!(analysis.divergences.is_empty());
    }

    #[test]
    fn flat_price_change_no_divergence() {
        // Price unchanged across lookback window — no divergence even if delta moves
        let candles = vec![
            candle("100", "60", "40"), // delta +20
            candle("100", "70", "30"), // delta +40, cum +60
            candle("100", "80", "20"), // same price, delta up — price_change == 0
        ];
        let analysis = analyze_delta(&candles, 2);
        assert!(analysis.divergences.is_empty());
    }

    #[test]
    fn flat_delta_change_no_divergence() {
        // Delta unchanged but price moves — no divergence (delta_change == 0)
        let candles = vec![
            candle("100", "50", "50"), // delta 0, cum 0
            candle("101", "50", "50"), // delta 0, cum 0
            candle("105", "50", "50"), // delta 0, cum 0 — price up, delta flat
        ];
        let analysis = analyze_delta(&candles, 2);
        assert!(analysis.divergences.is_empty());
    }

    #[test]
    fn multiple_divergences_detected_in_sequence() {
        // Multiple bearish divergences across a longer series
        let candles = vec![
            candle("100", "70", "30"), // delta +40, cum +40
            candle("101", "60", "40"), // delta +20, cum +60
            candle("105", "30", "70"), // delta -40, cum +20 — price up 4, delta down 40
            candle("108", "25", "75"), // delta -50, cum -30 — price up 7, delta down 90
            candle("112", "20", "80"), // delta -60, cum -90 — price up 7, delta down 110
        ];
        let analysis = analyze_delta(&candles, 2);
        // Indices 2, 3, 4 are all in the divergence scan window
        assert!(analysis.divergences.len() >= 2, "expected multiple divergences, got {}", analysis.divergences.len());
        for d in &analysis.divergences {
            assert_eq!(d.divergence_type, DivergenceType::Bearish);
        }
    }

    #[test]
    fn all_buy_volume_zero_sells() {
        // All volume is buying, zero sells => every delta is positive
        let candles = vec![
            candle("100", "100", "0"),
            candle("101", "50", "0"),
            candle("102", "200", "0"),
        ];
        let analysis = analyze_delta(&candles, 2);
        assert_eq!(analysis.candle_deltas[0], Decimal::from(100));
        assert_eq!(analysis.candle_deltas[1], Decimal::from(50));
        assert_eq!(analysis.candle_deltas[2], Decimal::from(200));
        assert_eq!(analysis.cumulative_delta[0], Decimal::from(100));
        assert_eq!(analysis.cumulative_delta[1], Decimal::from(150));
        assert_eq!(analysis.cumulative_delta[2], Decimal::from(350));
    }

    #[test]
    fn all_sell_volume_zero_buys() {
        // All volume is selling, zero buys => every delta is negative
        let candles = vec![
            candle("100", "0", "80"),
            candle("99", "0", "60"),
            candle("98", "0", "40"),
        ];
        let analysis = analyze_delta(&candles, 2);
        assert_eq!(analysis.candle_deltas[0], Decimal::from(-80));
        assert_eq!(analysis.candle_deltas[1], Decimal::from(-60));
        assert_eq!(analysis.candle_deltas[2], Decimal::from(-40));
        assert_eq!(analysis.cumulative_delta[2], Decimal::from(-180));
    }

    #[test]
    fn cumulative_delta_over_many_candles() {
        // 20 candles alternating +10 and -5 delta
        let candles: Vec<Candle> = (0..20)
            .map(|i| {
                if i % 2 == 0 {
                    candle(&format!("{}", 100 + i), "55", "45") // delta +10
                } else {
                    candle(&format!("{}", 100 + i), "47", "52") // delta -5
                }
            })
            .collect();
        let analysis = analyze_delta(&candles, 2);
        assert_eq!(analysis.candle_deltas.len(), 20);
        // 10 candles with +10 and 10 with -5 = 100 - 50 = 50
        assert_eq!(analysis.cumulative_delta[19], Decimal::from(50));
    }

    #[test]
    fn divergence_price_change_and_delta_change_values() {
        // Verify the actual price_change and delta_change values in a divergence
        let candles = vec![
            candle("100", "70", "30"), // delta +40, cum +40
            candle("101", "60", "40"), // delta +20, cum +60
            candle("105", "20", "80"), // delta -60, cum  0
        ];
        let analysis = analyze_delta(&candles, 2);
        assert_eq!(analysis.divergences.len(), 1);
        let d = &analysis.divergences[0];
        assert_eq!(d.index, 2);
        assert_eq!(d.divergence_type, DivergenceType::Bearish);
        // price_change = close[2] - close[0] = 105 - 100 = 5
        assert_eq!(d.price_change, Decimal::from(5));
        // delta_change = cum[2] - cum[0] = 0 - 40 = -40
        assert_eq!(d.delta_change, Decimal::from(-40));
    }

    #[test]
    fn larger_lookback_window() {
        // lookback=4 on 5 candles: only index 4 is checked
        let candles = vec![
            candle("100", "70", "30"), // cum +40
            candle("101", "60", "40"), // cum +60
            candle("102", "50", "50"), // cum +60
            candle("103", "40", "60"), // cum +40
            candle("110", "30", "70"), // cum  0 -- price up 10 from [0], delta down 40
        ];
        let analysis = analyze_delta(&candles, 4);
        assert_eq!(analysis.divergences.len(), 1);
        assert_eq!(analysis.divergences[0].index, 4);
        assert_eq!(analysis.divergences[0].divergence_type, DivergenceType::Bearish);
    }

    #[test]
    fn exactly_lookback_candles_no_divergence_check() {
        // candles.len() == lookback: loop range is lookback..len which is empty
        let candles = vec![
            candle("100", "30", "70"),
            candle("95", "80", "20"),
        ];
        let analysis = analyze_delta(&candles, 2);
        // No divergences because loop doesn't execute when len == lookback
        assert_eq!(analysis.divergences.len(), 0);
        // But deltas are still computed
        assert_eq!(analysis.candle_deltas.len(), 2);
        assert_eq!(analysis.cumulative_delta.len(), 2);
    }

    #[test]
    fn one_more_than_lookback_detects_divergence() {
        // candles.len() == lookback + 1: loop runs once at index=lookback
        let candles = vec![
            candle("100", "30", "70"), // cum -40
            candle("105", "30", "70"), // cum -80
            candle("95", "80", "20"),  // cum -20
        ];
        let analysis = analyze_delta(&candles, 2);
        // i=2: price_change = 95 - 105 = -10, delta_change = -20 - (-80) = 60
        // price down, delta up => bullish divergence
        assert_eq!(analysis.divergences.len(), 1);
        assert_eq!(analysis.divergences[0].divergence_type, DivergenceType::Bullish);
    }

    #[test]
    fn negative_cumulative_delta_stays_negative() {
        // All sell-heavy candles: cumulative should always be negative
        let candles = vec![
            candle("100", "10", "90"), // delta -80
            candle("99", "20", "80"),  // delta -60, cum -140
            candle("98", "15", "85"),  // delta -70, cum -210
        ];
        let analysis = analyze_delta(&candles, 2);
        for cum in &analysis.cumulative_delta {
            assert!(*cum < Decimal::ZERO, "cumulative delta should be negative, got {}", cum);
        }
    }
}
