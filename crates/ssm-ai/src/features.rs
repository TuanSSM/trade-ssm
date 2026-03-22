use rust_decimal::prelude::ToPrimitive;
use ssm_core::{Candle, FeatureRow};
use ssm_indicators::cvd::analyze_cvd;

/// Extract feature rows from candle data for ML/RL model training.
///
/// Features per row (FreqAI-inspired raw features):
///   0: raw_open   (normalized to first candle)
///   1: raw_high
///   2: raw_low
///   3: raw_close
///   4: volume
///   5: buy_sell_ratio  (taker_buy / volume)
///   6: cvd_delta       (per-candle CVD)
///   7: cvd_cumulative  (running sum)
///   8: price_change_pct (close/open - 1)
///   9: range_pct       ((high - low) / open)
pub fn extract_features(candles: &[Candle], cvd_window: usize) -> Vec<FeatureRow> {
    if candles.is_empty() {
        return vec![];
    }

    let base_price = candles[0].open.to_f64().unwrap_or(1.0);
    let cvd = analyze_cvd(candles, cvd_window);

    let start = candles.len().saturating_sub(cvd_window);
    let slice = &candles[start..];

    slice
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let open = c.open.to_f64().unwrap_or(0.0);
            let high = c.high.to_f64().unwrap_or(0.0);
            let low = c.low.to_f64().unwrap_or(0.0);
            let close = c.close.to_f64().unwrap_or(0.0);
            let vol = c.volume.to_f64().unwrap_or(0.0);
            let buy_vol = c.taker_buy_volume.to_f64().unwrap_or(0.0);

            let buy_sell_ratio = if vol > 0.0 { buy_vol / vol } else { 0.5 };
            let price_change = if open > 0.0 { close / open - 1.0 } else { 0.0 };
            let range_pct = if open > 0.0 { (high - low) / open } else { 0.0 };

            let cvd_delta = cvd.deltas.get(i).and_then(|d| d.to_f64()).unwrap_or(0.0);
            let cvd_cum = cvd
                .cumulative
                .get(i)
                .and_then(|d| d.to_f64())
                .unwrap_or(0.0);

            FeatureRow {
                timestamp: c.close_time,
                features: vec![
                    open / base_price,
                    high / base_price,
                    low / base_price,
                    close / base_price,
                    vol,
                    buy_sell_ratio,
                    cvd_delta,
                    cvd_cum,
                    price_change,
                    range_pct,
                ],
                label: None,
            }
        })
        .collect()
}

/// Label feature rows with future price movement for supervised training.
/// Label: 1.0 if close[i+horizon] > close[i], -1.0 if lower, 0.0 if flat.
pub fn label_features(features: &mut [FeatureRow], candles: &[Candle], horizon: usize) {
    let start = candles.len().saturating_sub(features.len());
    for (i, row) in features.iter_mut().enumerate() {
        let candle_idx = start + i;
        if candle_idx + horizon < candles.len() {
            let current = candles[candle_idx].close.to_f64().unwrap_or(0.0);
            let future = candles[candle_idx + horizon].close.to_f64().unwrap_or(0.0);
            row.label = Some(if future > current {
                1.0
            } else if future < current {
                -1.0
            } else {
                0.0
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn candle_at(close: &str, buy: &str, sell: &str) -> Candle {
        let bv = Decimal::from_str(buy).unwrap();
        let sv = Decimal::from_str(sell).unwrap();
        Candle {
            open_time: 0,
            open: Decimal::from_str(close).unwrap(),
            high: Decimal::from_str(close).unwrap() + Decimal::from(5),
            low: Decimal::from_str(close).unwrap() - Decimal::from(5),
            close: Decimal::from_str(close).unwrap(),
            volume: bv + sv,
            close_time: 1000,
            quote_volume: Decimal::ZERO,
            trades: 100,
            taker_buy_volume: bv,
            taker_sell_volume: sv,
        }
    }

    #[test]
    fn features_have_correct_dimensions() {
        let candles: Vec<_> = (0..20).map(|_| candle_at("50000", "60", "40")).collect();
        let features = extract_features(&candles, 15);
        assert_eq!(features.len(), 15);
        assert_eq!(features[0].features.len(), 10);
    }

    #[test]
    fn buy_sell_ratio_correct() {
        let candles: Vec<_> = (0..5).map(|_| candle_at("100", "70", "30")).collect();
        let features = extract_features(&candles, 5);
        // buy_sell_ratio = 70/100 = 0.7
        let ratio = features[0].features[5];
        assert!((ratio - 0.7).abs() < 0.001);
    }

    #[test]
    fn label_future_movement() {
        let candles = vec![
            candle_at("100", "50", "50"),
            candle_at("100", "50", "50"),
            candle_at("110", "50", "50"), // price went up
        ];
        let mut features = extract_features(&candles, 3);
        label_features(&mut features, &candles, 1);

        // First feature should see price go flat (100 → 100)
        assert_eq!(features[0].label, Some(0.0));
        // Second should see price go up (100 → 110)
        assert_eq!(features[1].label, Some(1.0));
        // Third has no future
        assert_eq!(features[2].label, None);
    }

    #[test]
    fn empty_candles_returns_empty() {
        let features = extract_features(&[], 15);
        assert!(features.is_empty());
    }
}
