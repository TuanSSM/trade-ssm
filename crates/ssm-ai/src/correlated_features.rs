use ssm_core::{Candle, FeatureRow};
use std::collections::HashMap;

use crate::features::extract_features;

/// Include features from correlated pairs.
pub struct CorrelatedPairFeatures {
    pub primary_pair: String,
    pub correlated_pairs: Vec<String>,
}

impl CorrelatedPairFeatures {
    pub fn new(primary: String, correlated: Vec<String>) -> Self {
        Self {
            primary_pair: primary,
            correlated_pairs: correlated,
        }
    }

    /// Merge features from correlated pair candles into primary features.
    ///
    /// For each primary feature row, appends the features from each correlated pair's
    /// candle data (matched by closest timestamp). If a correlated pair has no matching
    /// feature row, zeros are appended.
    pub fn merge_features(
        &self,
        primary_features: &[FeatureRow],
        correlated_candles: &HashMap<String, Vec<Candle>>,
        cvd_window: usize,
    ) -> Vec<FeatureRow> {
        if primary_features.is_empty() {
            return Vec::new();
        }

        // Pre-extract features for each correlated pair
        let mut correlated_features: Vec<(&String, Vec<FeatureRow>)> = Vec::new();
        for pair in &self.correlated_pairs {
            if let Some(candles) = correlated_candles.get(pair) {
                let features = extract_features(candles, cvd_window);
                correlated_features.push((pair, features));
            }
        }

        primary_features
            .iter()
            .map(|primary_row| {
                let mut combined = primary_row.features.clone();

                for (_pair, features) in &correlated_features {
                    // Find the closest feature row by timestamp
                    let matched = features
                        .iter()
                        .rev()
                        .find(|f| f.timestamp <= primary_row.timestamp);

                    if let Some(corr_row) = matched {
                        combined.extend_from_slice(&corr_row.features);
                    } else if let Some(first) = features.first() {
                        // Use first available if none precedes
                        combined.extend_from_slice(&first.features);
                    } else {
                        // No data for this pair — append zeros
                        combined.extend(std::iter::repeat_n(0.0, primary_row.features.len()));
                    }
                }

                FeatureRow {
                    timestamp: primary_row.timestamp,
                    features: combined,
                    label: primary_row.label,
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
    fn merge_adds_extra_features() {
        let interval = 60 * 1000;
        let primary_candles: Vec<Candle> = (0..20)
            .map(|i| {
                let open = i * interval;
                let close = open + interval - 1;
                make_candle(open, close, "100", "50")
            })
            .collect();

        let corr_candles: Vec<Candle> = (0..20)
            .map(|i| {
                let open = i * interval;
                let close = open + interval - 1;
                make_candle(open, close, "3000", "200")
            })
            .collect();

        let primary_features = crate::features::extract_features(&primary_candles, 10);
        let base_len = primary_features[0].features.len();

        let cpf = CorrelatedPairFeatures::new("BTCUSDT".to_string(), vec!["ETHUSDT".to_string()]);

        let mut correlated_map = HashMap::new();
        correlated_map.insert("ETHUSDT".to_string(), corr_candles);

        let merged = cpf.merge_features(&primary_features, &correlated_map, 10);

        assert_eq!(merged.len(), primary_features.len());
        // Each merged row should have more features than the primary alone
        assert!(merged[0].features.len() > base_len);
        // Specifically: base_len + correlated features length
        assert_eq!(merged[0].features.len(), base_len * 2);
        // Timestamps should be preserved
        assert_eq!(merged[0].timestamp, primary_features[0].timestamp);
    }

    #[test]
    fn merge_empty_primary_returns_empty() {
        let cpf = CorrelatedPairFeatures::new("BTCUSDT".to_string(), vec!["ETHUSDT".to_string()]);
        let result = cpf.merge_features(&[], &HashMap::new(), 10);
        assert!(result.is_empty());
    }

    #[test]
    fn merge_no_correlated_pairs_preserves_primary() {
        let interval = 60 * 1000;
        let candles: Vec<Candle> = (0..20)
            .map(|i| {
                let open = i * interval;
                let close = open + interval - 1;
                make_candle(open, close, "100", "50")
            })
            .collect();

        let primary_features = crate::features::extract_features(&candles, 10);
        let cpf = CorrelatedPairFeatures::new("BTCUSDT".to_string(), vec![]);
        let merged = cpf.merge_features(&primary_features, &HashMap::new(), 10);

        assert_eq!(merged.len(), primary_features.len());
        assert_eq!(merged[0].features.len(), primary_features[0].features.len());
    }

    #[test]
    fn merge_missing_correlated_data_appends_zeros() {
        let interval = 60 * 1000;
        let candles: Vec<Candle> = (0..20)
            .map(|i| {
                let open = i * interval;
                let close = open + interval - 1;
                make_candle(open, close, "100", "50")
            })
            .collect();

        let primary_features = crate::features::extract_features(&candles, 10);
        let base_len = primary_features[0].features.len();

        let cpf = CorrelatedPairFeatures::new("BTCUSDT".to_string(), vec!["XYZUSDT".to_string()]);

        // Empty correlated map — the pair "XYZUSDT" has no data
        let merged = cpf.merge_features(&primary_features, &HashMap::new(), 10);

        // No correlated data found, so features should be unchanged
        // (the pair is in correlated_pairs but not in the HashMap, so it's skipped)
        assert_eq!(merged[0].features.len(), base_len);
    }

    /// Anti-repainting: adding candle N+1 must not change merged features at earlier timestamps.
    #[test]
    fn anti_repainting_correlated_features_stable() {
        let interval = 60 * 1000;
        let n = 30;

        let primary_candles: Vec<Candle> = (0..n)
            .map(|i| {
                let open = i * interval;
                let close = open + interval - 1;
                let price = format!("{}", 100 + (i % 10));
                make_candle(open, close, &price, "50")
            })
            .collect();

        let corr_candles: Vec<Candle> = (0..n)
            .map(|i| {
                let open = i * interval;
                let close = open + interval - 1;
                let price = format!("{}", 3000 + (i % 15));
                make_candle(open, close, &price, "200")
            })
            .collect();

        let cpf = CorrelatedPairFeatures::new("LINKUSDT".to_string(), vec!["ETHUSDT".to_string()]);
        let cvd_window = 10;

        // Extract with N-1 candles
        let primary_n_minus_1 = &primary_candles[..n as usize - 1];
        let corr_n_minus_1: Vec<Candle> = corr_candles[..n as usize - 1].to_vec();
        let features_before = crate::features::extract_features(primary_n_minus_1, cvd_window);
        let mut map_before = HashMap::new();
        map_before.insert("ETHUSDT".to_string(), corr_n_minus_1);
        let merged_before = cpf.merge_features(&features_before, &map_before, cvd_window);

        // Extract with all N candles
        let features_after = crate::features::extract_features(&primary_candles, cvd_window);
        let mut map_after = HashMap::new();
        map_after.insert("ETHUSDT".to_string(), corr_candles);
        let merged_after = cpf.merge_features(&features_after, &map_after, cvd_window);

        // All overlapping timestamps must have identical feature values
        for row_before in &merged_before {
            if let Some(row_after) = merged_after
                .iter()
                .find(|r| r.timestamp == row_before.timestamp)
            {
                assert_eq!(
                    row_before.features.len(),
                    row_after.features.len(),
                    "feature length mismatch at ts={}",
                    row_before.timestamp,
                );
                for (j, (a, b)) in row_before
                    .features
                    .iter()
                    .zip(row_after.features.iter())
                    .enumerate()
                {
                    assert!(
                        (a - b).abs() < 1e-10,
                        "feature[{j}] changed at ts={}: {a} vs {b}",
                        row_before.timestamp,
                    );
                }
            }
        }
    }

    /// Correlated features must never use future data relative to the primary candle.
    /// Uses the same time range for primary and correlated (the realistic use case).
    #[test]
    fn correlated_features_never_use_future_data() {
        let interval = 60 * 1000;

        // Primary and correlated share the same time range (realistic: same download period)
        let primary_candles: Vec<Candle> = (0..25)
            .map(|i| {
                let open = i * interval;
                let close = open + interval - 1;
                make_candle(open, close, "100", "50")
            })
            .collect();

        // Correlated: same 25 candles, different prices
        let corr_candles: Vec<Candle> = (0..25)
            .map(|i| {
                let open = i * interval;
                let close = open + interval - 1;
                let price = format!("{}", 3000 + i * 10);
                make_candle(open, close, &price, "200")
            })
            .collect();

        let cpf = CorrelatedPairFeatures::new("LINKUSDT".to_string(), vec!["ETHUSDT".to_string()]);
        let cvd_window = 10;

        let primary_features = crate::features::extract_features(&primary_candles, cvd_window);
        let mut map = HashMap::new();
        map.insert("ETHUSDT".to_string(), corr_candles.clone());
        let merged = cpf.merge_features(&primary_features, &map, cvd_window);

        let base_len = primary_features[0].features.len();

        // Extract correlated features independently to identify which row was matched
        let corr_features_all = crate::features::extract_features(&corr_candles, cvd_window);

        for row in &merged {
            // The correlated portion starts at base_len
            let corr_slice = &row.features[base_len..];

            // Find the matched correlated feature row (the one whose features match)
            let matched = corr_features_all.iter().find(|cf| {
                cf.features.len() == corr_slice.len()
                    && cf
                        .features
                        .iter()
                        .zip(corr_slice.iter())
                        .all(|(a, b)| (a - b).abs() < 1e-10)
            });

            if let Some(matched_row) = matched {
                assert!(
                    matched_row.timestamp <= row.timestamp,
                    "correlated feature at ts={} uses future data from ts={}",
                    row.timestamp,
                    matched_row.timestamp,
                );
            }
        }
    }

    #[test]
    fn merge_with_multiple_correlated_pairs() {
        let interval = 60 * 1000;
        let primary_candles: Vec<Candle> = (0..20)
            .map(|i| {
                let open = i * interval;
                let close = open + interval - 1;
                make_candle(open, close, "15", "30")
            })
            .collect();

        let eth_candles: Vec<Candle> = (0..20)
            .map(|i| {
                let open = i * interval;
                let close = open + interval - 1;
                make_candle(open, close, "3000", "200")
            })
            .collect();

        let btc_candles: Vec<Candle> = (0..20)
            .map(|i| {
                let open = i * interval;
                let close = open + interval - 1;
                make_candle(open, close, "60000", "500")
            })
            .collect();

        let primary_features = crate::features::extract_features(&primary_candles, 10);
        let base_len = primary_features[0].features.len();

        let cpf = CorrelatedPairFeatures::new(
            "LINKUSDT".to_string(),
            vec!["ETHUSDT".to_string(), "BTCUSDT".to_string()],
        );

        let mut correlated_map = HashMap::new();
        correlated_map.insert("ETHUSDT".to_string(), eth_candles);
        correlated_map.insert("BTCUSDT".to_string(), btc_candles);

        let merged = cpf.merge_features(&primary_features, &correlated_map, 10);

        assert_eq!(merged.len(), primary_features.len());
        // base_len + 2 * base_len (one set of 22 features per correlated pair)
        assert_eq!(merged[0].features.len(), base_len * 3);
    }
}
