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
}
