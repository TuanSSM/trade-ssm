use serde::{Deserialize, Serialize};
use ssm_core::{Candle, FeatureRow};
use std::collections::HashMap;

use crate::features::extract_features;

/// Supported trading timeframes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Timeframe {
    M3,
    M15,
    H1,
    H4,
}

impl Timeframe {
    /// Duration of one candle in milliseconds.
    pub fn duration_ms(&self) -> i64 {
        match self {
            Timeframe::M3 => 3 * 60 * 1000,
            Timeframe::M15 => 15 * 60 * 1000,
            Timeframe::H1 => 60 * 60 * 1000,
            Timeframe::H4 => 4 * 60 * 60 * 1000,
        }
    }

    /// Parse from string (e.g., "3m", "15m", "1h", "4h").
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "3m" => Some(Timeframe::M3),
            "15m" => Some(Timeframe::M15),
            "1h" => Some(Timeframe::H1),
            "4h" => Some(Timeframe::H4),
            _ => None,
        }
    }

    /// String representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Timeframe::M3 => "3m",
            Timeframe::M15 => "15m",
            Timeframe::H1 => "1h",
            Timeframe::H4 => "4h",
        }
    }

    /// Approximate number of candles per year (for Sharpe annualization).
    pub fn steps_per_year(&self) -> f64 {
        let minutes_per_year = 365.25 * 24.0 * 60.0;
        match self {
            Timeframe::M3 => minutes_per_year / 3.0,
            Timeframe::M15 => minutes_per_year / 15.0,
            Timeframe::H1 => minutes_per_year / 60.0,
            Timeframe::H4 => minutes_per_year / 240.0,
        }
    }
}

impl std::fmt::Display for Timeframe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Resample finer-grained candles into a coarser timeframe.
///
/// Input candles must be sorted by `open_time`.
/// Uses floor-division on `open_time` to bucket candles into the target interval.
pub fn resample_candles(candles: &[Candle], target: Timeframe) -> Vec<Candle> {
    if candles.is_empty() {
        return Vec::new();
    }

    let interval_ms = target.duration_ms();
    let mut result: Vec<Candle> = Vec::new();
    let mut bucket: Option<(i64, Candle)> = None;

    for c in candles {
        let bucket_key = c.open_time / interval_ms;

        if let Some((key, ref mut agg)) = bucket {
            if key == bucket_key {
                // Aggregate into current bucket
                if c.high > agg.high {
                    agg.high = c.high;
                }
                if c.low < agg.low {
                    agg.low = c.low;
                }
                agg.close = c.close;
                agg.close_time = c.close_time;
                agg.volume += c.volume;
                agg.quote_volume += c.quote_volume;
                agg.trades += c.trades;
                agg.taker_buy_volume += c.taker_buy_volume;
                agg.taker_sell_volume += c.taker_sell_volume;
            } else {
                // Finalize previous bucket, start new one
                result.push(agg.clone());
                bucket = Some((bucket_key, c.clone()));
            }
        } else {
            bucket = Some((bucket_key, c.clone()));
        }
    }

    // Don't forget the last bucket
    if let Some((_, agg)) = bucket {
        result.push(agg);
    }

    result
}

/// Extract features at multiple timeframe resolutions.
///
/// Returns a map from `Timeframe` to feature vectors. Higher timeframe features
/// are extracted from resampled candles.
pub fn extract_multi_tf_features(
    candles: &[Candle],
    higher_tfs: &[Timeframe],
    cvd_window: usize,
) -> HashMap<Timeframe, Vec<FeatureRow>> {
    let mut result = HashMap::new();

    for &tf in higher_tfs {
        let resampled = resample_candles(candles, tf);
        let features = extract_features(&resampled, cvd_window);
        result.insert(tf, features);
    }

    result
}

/// Align higher-timeframe features to base-timeframe candles for anti-repainting.
///
/// For each base candle at index `i`, finds the most recent higher-TF feature
/// whose candle `close_time <= base_candle[i].open_time`.
pub fn align_higher_tf_features(
    base_candles: &[Candle],
    higher_candles: &[Candle],
    higher_features: &[FeatureRow],
) -> Vec<Option<usize>> {
    if higher_candles.is_empty() || higher_features.is_empty() {
        return vec![None; base_candles.len()];
    }

    let mut aligned = Vec::with_capacity(base_candles.len());
    let mut hi_idx = 0;

    for base in base_candles {
        // Advance higher-TF index as far as possible while still closed before base opens
        while hi_idx + 1 < higher_candles.len()
            && higher_candles[hi_idx + 1].close_time <= base.open_time
        {
            hi_idx += 1;
        }

        if higher_candles[hi_idx].close_time <= base.open_time && hi_idx < higher_features.len() {
            aligned.push(Some(hi_idx));
        } else {
            aligned.push(None);
        }
    }

    aligned
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn make_candle(open_time: i64, close_time: i64, price: &str, volume: &str) -> Candle {
        let p = Decimal::from_str(price).unwrap();
        let v = Decimal::from_str(volume).unwrap();
        Candle {
            open_time,
            open: p,
            high: p + Decimal::from(5),
            low: p - Decimal::from(5),
            close: p,
            volume: v,
            close_time,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: v / Decimal::from(2),
            taker_sell_volume: v / Decimal::from(2),
        }
    }

    #[test]
    fn timeframe_parse_roundtrip() {
        for tf in [Timeframe::M3, Timeframe::M15, Timeframe::H1, Timeframe::H4] {
            assert_eq!(Timeframe::parse(tf.as_str()), Some(tf));
        }
        assert_eq!(Timeframe::parse("invalid"), None);
    }

    #[test]
    fn steps_per_year_values() {
        let m15 = Timeframe::M15.steps_per_year();
        assert!((m15 - 35064.0).abs() < 100.0); // ~35040
        let h1 = Timeframe::H1.steps_per_year();
        assert!((h1 - 8766.0).abs() < 50.0); // ~8760
    }

    #[test]
    fn resample_3m_to_15m() {
        let interval_3m = 3 * 60 * 1000;
        // Create 5 x 3m candles (should produce 1 x 15m candle)
        let candles: Vec<Candle> = (0..5)
            .map(|i| {
                let open = i * interval_3m;
                let close = open + interval_3m - 1;
                make_candle(open, close, &format!("{}", 100 + i), "100")
            })
            .collect();

        let resampled = resample_candles(&candles, Timeframe::M15);
        assert_eq!(resampled.len(), 1);

        let r = &resampled[0];
        // First open
        assert_eq!(r.open, candles[0].open);
        // Last close
        assert_eq!(r.close, candles[4].close);
        // Max high
        assert!(r.high >= candles[4].high);
        // Min low
        assert!(r.low <= candles[0].low);
        // Sum volume
        let total_vol: Decimal = candles.iter().map(|c| c.volume).sum();
        assert_eq!(r.volume, total_vol);
    }

    #[test]
    fn resample_empty_input() {
        let result = resample_candles(&[], Timeframe::H1);
        assert!(result.is_empty());
    }

    #[test]
    fn resample_preserves_multiple_buckets() {
        let interval_3m = 3 * 60 * 1000;
        let interval_15m = 15 * 60 * 1000;
        // 10 x 3m candles spanning 2 x 15m buckets
        let candles: Vec<Candle> = (0..10)
            .map(|i| {
                let open = i * interval_3m;
                let close = open + interval_3m - 1;
                make_candle(open, close, "100", "100")
            })
            .collect();

        let resampled = resample_candles(&candles, Timeframe::M15);
        assert_eq!(resampled.len(), 2);

        // First bucket: candles 0-4 (0..15m)
        assert_eq!(resampled[0].open_time, 0);
        // Second bucket: candles 5-9 (15m..30m)
        assert_eq!(resampled[1].open_time, 5 * interval_3m);
        assert!(resampled[1].open_time >= interval_15m);
    }

    #[test]
    fn align_anti_repainting() {
        let interval_3m = 3 * 60 * 1000;

        // Base: 3m candles
        let base: Vec<Candle> = (0..10)
            .map(|i| {
                let open = i * interval_3m;
                let close = open + interval_3m - 1;
                make_candle(open, close, "100", "100")
            })
            .collect();

        // Higher: 15m candles (just 2)
        let higher = resample_candles(&base, Timeframe::M15);

        // Mock features (same count as higher candles)
        let features: Vec<FeatureRow> = higher
            .iter()
            .map(|c| FeatureRow {
                timestamp: c.open_time,
                features: vec![1.0],
                label: None,
            })
            .collect();

        let aligned = align_higher_tf_features(&base, &higher, &features);
        assert_eq!(aligned.len(), base.len());

        // First few base candles shouldn't see any higher-TF data
        // because the first 15m candle hasn't closed yet
        assert!(aligned[0].is_none());
    }

    #[test]
    fn duration_ms_values() {
        assert_eq!(Timeframe::M3.duration_ms(), 180_000);
        assert_eq!(Timeframe::M15.duration_ms(), 900_000);
        assert_eq!(Timeframe::H1.duration_ms(), 3_600_000);
        assert_eq!(Timeframe::H4.duration_ms(), 14_400_000);
    }
}
