use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use ssm_core::{Candle, FeatureRow};
use ssm_indicators::atr::atr;
use ssm_indicators::bollinger::bollinger_bands;
use ssm_indicators::cvd::analyze_cvd;
use ssm_indicators::ema::ema;
use ssm_indicators::macd::macd;
use ssm_indicators::obv::obv;
use ssm_indicators::rsi::rsi;
use ssm_indicators::vwap::vwap;

/// Number of features per row.
pub const FEATURE_COUNT: usize = 22;

/// Extract feature rows from candle data for ML/RL model training.
///
/// Features per row (22 total):
///    0: raw_open           (normalized to first candle)
///    1: raw_high
///    2: raw_low
///    3: raw_close
///    4: volume
///    5: buy_sell_ratio      (taker_buy / volume)
///    6: cvd_delta           (per-candle CVD)
///    7: cvd_cumulative      (running sum)
///    8: price_change_pct    (close/open - 1)
///    9: range_pct           ((high - low) / open)
///   10: rsi_14              (RSI period 14, 0-100 scaled to 0-1)
///   11: ema_ratio           (EMA-9 / EMA-21 crossover ratio)
///   12: macd_histogram      (MACD histogram normalized by price)
///   13: bb_pct_b            (Bollinger %B, 0-1 within bands)
///   14: bb_bandwidth        (Bollinger bandwidth)
///   15: atr_normalized      (ATR-14 / close price)
///   16: obv_delta           (OBV change normalized)
///   17: vwap_deviation      ((close - VWAP) / close)
///   18: rsi_slope           (RSI change over last 3 periods)
///   19: macd_signal_diff    (MACD line - signal line, normalized)
///   20: volume_sma_ratio    (volume / volume SMA-20)
///   21: close_vs_ema9       ((close - EMA9) / close)
pub fn extract_features(candles: &[Candle], cvd_window: usize) -> Vec<FeatureRow> {
    if candles.is_empty() {
        return vec![];
    }

    let base_price = candles[0].open.to_f64().unwrap_or(1.0);
    let cvd = analyze_cvd(candles, cvd_window);

    // Compute indicators on full candle history for accuracy
    let rsi_vals = rsi(candles, 14);
    let ema9 = ema(candles, 9);
    let ema21 = ema(candles, 21);
    let macd_result = macd(candles, 12, 26, 9);
    let bb = bollinger_bands(candles, 20, Decimal::from(2));
    let atr_vals = atr(candles, 14);
    let obv_vals = obv(candles);
    let vwap_result = vwap(candles);

    let start = candles.len().saturating_sub(cvd_window);
    let slice = &candles[start..];

    slice
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let candle_idx = start + i;
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

            // RSI: indicator returns values starting from `period` index
            // rsi_vals length = candles.len() - period, maps to candle indices [period..]
            let rsi_val = indicator_at(&rsi_vals, candle_idx, 14, candles.len()) / 100.0;

            // EMA crossover ratio: EMA-9 / EMA-21
            let ema9_val = indicator_at(&ema9, candle_idx, 9, candles.len());
            let ema21_val = indicator_at(&ema21, candle_idx, 21, candles.len());
            let ema_ratio = if ema21_val > 0.0 {
                ema9_val / ema21_val
            } else {
                1.0
            };

            // MACD histogram: normalized by price
            // macd result lengths are aligned to the slowest EMA
            let macd_hist = indicator_at(
                &macd_result.histogram,
                candle_idx,
                26 + 9 - 1,
                candles.len(),
            );
            let macd_hist_norm = if close > 0.0 { macd_hist / close } else { 0.0 };

            // MACD line - signal line
            let macd_line = indicator_at(&macd_result.macd, candle_idx, 26, candles.len());
            let macd_signal =
                indicator_at(&macd_result.signal, candle_idx, 26 + 9 - 1, candles.len());
            let macd_signal_diff = if close > 0.0 {
                (macd_line - macd_signal) / close
            } else {
                0.0
            };

            // Bollinger %B and bandwidth: bb values start at period index
            let bb_pct_b = indicator_at(&bb.pct_b, candle_idx, 20, candles.len());
            let bb_bw = indicator_at(&bb.bandwidth, candle_idx, 20, candles.len());

            // ATR normalized by close price
            let atr_val = indicator_at(&atr_vals, candle_idx, 14, candles.len());
            let atr_norm = if close > 0.0 { atr_val / close } else { 0.0 };

            // OBV delta (change from previous): obv has one value per candle
            let obv_current = obv_vals
                .get(candle_idx)
                .and_then(|d| d.to_f64())
                .unwrap_or(0.0);
            let obv_prev = if candle_idx > 0 {
                obv_vals
                    .get(candle_idx - 1)
                    .and_then(|d| d.to_f64())
                    .unwrap_or(0.0)
            } else {
                obv_current
            };
            let obv_delta = if vol > 0.0 {
                (obv_current - obv_prev) / vol
            } else {
                0.0
            };

            // VWAP deviation: (close - VWAP) / close
            let vwap_val = vwap_result
                .vwap
                .get(candle_idx)
                .and_then(|d| d.to_f64())
                .unwrap_or(close);
            let vwap_dev = if close > 0.0 {
                (close - vwap_val) / close
            } else {
                0.0
            };

            // RSI slope: change over last 3 periods
            let rsi_prev = if candle_idx >= 3 {
                indicator_at(&rsi_vals, candle_idx - 3, 14, candles.len()) / 100.0
            } else {
                rsi_val
            };
            let rsi_slope = rsi_val - rsi_prev;

            // Volume SMA ratio: current volume / 20-period volume SMA
            let vol_sma = volume_sma(candles, candle_idx, 20);
            let vol_sma_ratio = if vol_sma > 0.0 { vol / vol_sma } else { 1.0 };

            // Close vs EMA-9
            let close_vs_ema9 = if close > 0.0 {
                (close - ema9_val) / close
            } else {
                0.0
            };

            FeatureRow {
                timestamp: c.close_time,
                features: vec![
                    open / base_price,  // 0
                    high / base_price,  // 1
                    low / base_price,   // 2
                    close / base_price, // 3
                    vol,                // 4
                    buy_sell_ratio,     // 5
                    cvd_delta,          // 6
                    cvd_cum,            // 7
                    price_change,       // 8
                    range_pct,          // 9
                    rsi_val,            // 10
                    ema_ratio,          // 11
                    macd_hist_norm,     // 12
                    bb_pct_b,           // 13
                    bb_bw,              // 14
                    atr_norm,           // 15
                    obv_delta,          // 16
                    vwap_dev,           // 17
                    rsi_slope,          // 18
                    macd_signal_diff,   // 19
                    vol_sma_ratio,      // 20
                    close_vs_ema9,      // 21
                ],
                label: None,
            }
        })
        .collect()
}

/// Get indicator value at a given candle index.
/// Indicators return vectors starting from their `period` offset.
/// Returns 0.0 if the candle index is before the indicator has data.
fn indicator_at(
    values: &[Decimal],
    candle_idx: usize,
    _period: usize,
    total_candles: usize,
) -> f64 {
    // Indicator values start at index `period` in candle space.
    // The indicator vector has length = total_candles - period (approximately).
    // Map candle_idx to indicator index: indicator_idx = candle_idx - (total_candles - values.len())
    if values.is_empty() {
        return 0.0;
    }
    let offset = total_candles.saturating_sub(values.len());
    if candle_idx < offset {
        return 0.0;
    }
    let idx = candle_idx - offset;
    values.get(idx).and_then(|d| d.to_f64()).unwrap_or(0.0)
}

/// Compute simple moving average of volume over `period` candles ending at `candle_idx`.
fn volume_sma(candles: &[Candle], candle_idx: usize, period: usize) -> f64 {
    if candle_idx + 1 < period {
        // Not enough history — use what we have
        let start = 0;
        let end = candle_idx + 1;
        let sum: f64 = candles[start..end]
            .iter()
            .map(|c| c.volume.to_f64().unwrap_or(0.0))
            .sum();
        return sum / (end - start) as f64;
    }
    let start = candle_idx + 1 - period;
    let end = candle_idx + 1;
    let sum: f64 = candles[start..end]
        .iter()
        .map(|c| c.volume.to_f64().unwrap_or(0.0))
        .sum();
    sum / period as f64
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
        assert_eq!(features[0].features.len(), FEATURE_COUNT);
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

    #[test]
    fn anti_repainting_indicator_features_stable() {
        // Compute features on N candles, then on N+1 candles.
        // Indicator-derived features (indices 10-21) must not change for overlapping candles.
        let candles: Vec<_> = (0..50)
            .map(|i| {
                let p = Decimal::from_str(&format!("{}", 100 + (i % 10))).unwrap();
                let bv = Decimal::from_str(&format!("{}", 50 + (i % 5))).unwrap();
                let sv = Decimal::from_str(&format!("{}", 50 - (i % 5))).unwrap();
                Candle {
                    open_time: (i as i64) * 900_000,
                    open: p,
                    high: p + Decimal::from(5),
                    low: p - Decimal::from(5),
                    close: p,
                    volume: bv + sv,
                    close_time: (i as i64) * 900_000 + 899_999,
                    quote_volume: Decimal::ZERO,
                    trades: 100,
                    taker_buy_volume: bv,
                    taker_sell_volume: sv,
                }
            })
            .collect();

        let window = 10;
        let features_n = extract_features(&candles[..49], window);
        let features_n1 = extract_features(&candles[..50], window);

        // Find the matching feature row by timestamp
        let last_n = features_n.last().unwrap();
        let matching = features_n1.iter().find(|f| f.timestamp == last_n.timestamp);
        assert!(matching.is_some(), "no matching timestamp found");
        let m = matching.unwrap();
        // Check indicator features (10-21) for anti-repainting stability
        for j in 10..FEATURE_COUNT {
            assert!(
                (last_n.features[j] - m.features[j]).abs() < 1e-10,
                "indicator feature {j} changed: {} vs {} (anti-repainting violation)",
                last_n.features[j],
                m.features[j]
            );
        }
    }

    #[test]
    fn all_indicator_features_populated() {
        // With enough candles, all features should be non-zero for varying data
        let candles: Vec<_> = (0..50)
            .map(|i| {
                let p = format!("{}", 100 + i);
                candle_at(&p, "60", "40")
            })
            .collect();
        let features = extract_features(&candles, 10);
        assert!(!features.is_empty());
        assert_eq!(features[0].features.len(), FEATURE_COUNT);
        // RSI, EMA ratio, etc. should have values when enough history
        let last = features.last().unwrap();
        // RSI should be between 0 and 1 (scaled)
        assert!(last.features[10] >= 0.0 && last.features[10] <= 1.0);
        // EMA ratio should be positive
        assert!(last.features[11] > 0.0);
    }
}
