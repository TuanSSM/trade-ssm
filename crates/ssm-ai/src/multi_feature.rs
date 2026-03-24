use ssm_core::{Candle, FeatureRow};

use crate::features::extract_features;
use crate::multi_timeframe::resample_candles;
use crate::multi_timeframe::Timeframe;

/// Expand features across multiple timeframes by resampling candles.
pub struct MultiTimeframeFeatures {
    pub timeframes: Vec<String>,
    pub cvd_window: usize,
}

impl MultiTimeframeFeatures {
    pub fn new(timeframes: Vec<String>, cvd_window: usize) -> Self {
        Self {
            timeframes,
            cvd_window,
        }
    }

    /// Resample candles to a higher timeframe by aggregating every `factor` candles.
    pub fn resample(candles: &[Candle], factor: usize) -> Vec<Candle> {
        if candles.is_empty() || factor == 0 {
            return Vec::new();
        }
        if factor == 1 {
            return candles.to_vec();
        }

        candles
            .chunks(factor)
            .map(|chunk| {
                let first = &chunk[0];
                let last = &chunk[chunk.len() - 1];
                let high = chunk.iter().map(|c| c.high).max().unwrap_or(first.high);
                let low = chunk.iter().map(|c| c.low).min().unwrap_or(first.low);
                let volume = chunk.iter().map(|c| c.volume).sum();
                let quote_volume = chunk.iter().map(|c| c.quote_volume).sum();
                let trades = chunk.iter().map(|c| c.trades).sum();
                let taker_buy_volume = chunk.iter().map(|c| c.taker_buy_volume).sum();
                let taker_sell_volume = chunk.iter().map(|c| c.taker_sell_volume).sum();

                Candle {
                    open_time: first.open_time,
                    open: first.open,
                    high,
                    low,
                    close: last.close,
                    volume,
                    close_time: last.close_time,
                    quote_volume,
                    trades,
                    taker_buy_volume,
                    taker_sell_volume,
                }
            })
            .collect()
    }

    /// Extract features from multiple timeframes, concatenating them.
    ///
    /// For each timeframe string, attempts to parse it as a known `Timeframe` variant
    /// and resamples accordingly. Falls back to factor-based resampling for numeric strings.
    /// Returns one `FeatureRow` per base-timeframe candle in the CVD window, with features
    /// from all timeframes concatenated.
    pub fn extract(&self, candles: &[Candle]) -> Vec<FeatureRow> {
        if candles.is_empty() {
            return Vec::new();
        }

        // Base features from original candles
        let base_features = extract_features(candles, self.cvd_window);
        if base_features.is_empty() {
            return Vec::new();
        }

        // Collect higher-timeframe feature sets
        let mut all_tf_features: Vec<Vec<FeatureRow>> = Vec::new();

        for tf_str in &self.timeframes {
            let resampled = if let Some(tf) = Timeframe::parse(tf_str) {
                resample_candles(candles, tf)
            } else {
                // Try parsing as a numeric factor
                if let Ok(factor) = tf_str.parse::<usize>() {
                    Self::resample(candles, factor)
                } else {
                    continue;
                }
            };

            let tf_features = extract_features(&resampled, self.cvd_window);
            all_tf_features.push(tf_features);
        }

        // Build output: for each base feature row, concatenate the last available
        // feature from each higher timeframe.
        base_features
            .iter()
            .map(|base_row| {
                let mut combined = base_row.features.clone();

                for tf_feats in &all_tf_features {
                    // Find the last higher-TF feature with timestamp <= base timestamp
                    let htf_row = tf_feats
                        .iter()
                        .rev()
                        .find(|f| f.timestamp <= base_row.timestamp);

                    if let Some(htf) = htf_row {
                        combined.extend_from_slice(&htf.features);
                    } else if let Some(first) = tf_feats.first() {
                        // Use first available if none precedes
                        combined.extend_from_slice(&first.features);
                    }
                }

                FeatureRow {
                    timestamp: base_row.timestamp,
                    features: combined,
                    label: base_row.label,
                }
            })
            .collect()
    }
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
    fn resample_produces_correct_number_of_candles() {
        let candles: Vec<Candle> = (0..12)
            .map(|i| make_candle(i * 1000, i * 1000 + 999, "100", "50"))
            .collect();

        // Factor 3: 12 candles -> 4 resampled
        let resampled = MultiTimeframeFeatures::resample(&candles, 3);
        assert_eq!(resampled.len(), 4);

        // Factor 5: 12 candles -> 3 (last chunk has only 2)
        let resampled = MultiTimeframeFeatures::resample(&candles, 5);
        assert_eq!(resampled.len(), 3);

        // Factor 1: no change
        let resampled = MultiTimeframeFeatures::resample(&candles, 1);
        assert_eq!(resampled.len(), 12);

        // Factor 0: empty
        let resampled = MultiTimeframeFeatures::resample(&candles, 0);
        assert!(resampled.is_empty());

        // Empty input
        let resampled = MultiTimeframeFeatures::resample(&[], 3);
        assert!(resampled.is_empty());
    }

    #[test]
    fn resampled_candle_ohlcv_is_correct() {
        let candles = vec![
            {
                let mut c = make_candle(0, 999, "100", "50");
                c.open = Decimal::from(100);
                c.high = Decimal::from(110);
                c.low = Decimal::from(90);
                c.close = Decimal::from(105);
                c.volume = Decimal::from(50);
                c.taker_buy_volume = Decimal::from(30);
                c.taker_sell_volume = Decimal::from(20);
                c
            },
            {
                let mut c = make_candle(1000, 1999, "105", "60");
                c.open = Decimal::from(105);
                c.high = Decimal::from(120);
                c.low = Decimal::from(95);
                c.close = Decimal::from(115);
                c.volume = Decimal::from(60);
                c.taker_buy_volume = Decimal::from(35);
                c.taker_sell_volume = Decimal::from(25);
                c
            },
            {
                let mut c = make_candle(2000, 2999, "115", "40");
                c.open = Decimal::from(115);
                c.high = Decimal::from(125);
                c.low = Decimal::from(100);
                c.close = Decimal::from(108);
                c.volume = Decimal::from(40);
                c.taker_buy_volume = Decimal::from(20);
                c.taker_sell_volume = Decimal::from(20);
                c
            },
        ];

        let resampled = MultiTimeframeFeatures::resample(&candles, 3);
        assert_eq!(resampled.len(), 1);
        let r = &resampled[0];

        // Open = first candle's open
        assert_eq!(r.open, Decimal::from(100));
        // High = max of all highs
        assert_eq!(r.high, Decimal::from(125));
        // Low = min of all lows
        assert_eq!(r.low, Decimal::from(90));
        // Close = last candle's close
        assert_eq!(r.close, Decimal::from(108));
        // Volume = sum
        assert_eq!(r.volume, Decimal::from(150));
        // Taker buy volume = sum
        assert_eq!(r.taker_buy_volume, Decimal::from(85));
        // Taker sell volume = sum
        assert_eq!(r.taker_sell_volume, Decimal::from(65));
        // Timestamps
        assert_eq!(r.open_time, 0);
        assert_eq!(r.close_time, 2999);
    }

    #[test]
    fn extract_concatenates_multi_tf_features() {
        let interval = 3 * 60 * 1000; // 3m
        let candles: Vec<Candle> = (0..30)
            .map(|i| {
                let open = i * interval;
                let close = open + interval - 1;
                make_candle(open, close, &format!("{}", 100 + i), "100")
            })
            .collect();

        let mtf = MultiTimeframeFeatures::new(vec!["15m".to_string()], 10);
        let features = mtf.extract(&candles);

        assert!(!features.is_empty());
        // Each row should have more features than base (22 base + 22 from 15m TF)
        let base_features = extract_features(&candles, 10);
        let base_feat_count = base_features[0].features.len();
        assert!(features[0].features.len() > base_feat_count);
    }
}
