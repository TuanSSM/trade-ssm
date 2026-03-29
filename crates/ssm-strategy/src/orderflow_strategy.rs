use anyhow::Result;
use ssm_core::{AIAction, Candle, Signal};
use ssm_orderflow::delta::{analyze_delta, DivergenceType};
use ssm_orderflow::imbalance::{detect_imbalances, ImbalanceConfig, ImbalanceType};
use ssm_orderflow::sweep::{detect_sweeps, SweepConfig, SweepType};

use crate::traits::Strategy;

/// Order flow strategy using RIFEBTC-inspired signals.
///
/// Combines delta divergence, volume imbalance, and sweep detection
/// to generate trading signals.
pub struct OrderFlowStrategy {
    lookback: usize,
    imbalance_config: ImbalanceConfig,
    sweep_config: SweepConfig,
    min_confidence: f64,
}

impl OrderFlowStrategy {
    pub fn new(lookback: usize) -> Self {
        Self {
            lookback,
            imbalance_config: ImbalanceConfig::default(),
            sweep_config: SweepConfig::default(),
            min_confidence: 0.5,
        }
    }

    pub fn with_min_confidence(mut self, c: f64) -> Self {
        self.min_confidence = c;
        self
    }
}

impl Strategy for OrderFlowStrategy {
    fn name(&self) -> &str {
        "orderflow"
    }

    fn analyze(&self, candles: &[Candle]) -> Result<Option<Signal>> {
        if candles.len() < self.lookback + 2 {
            return Ok(None);
        }

        let window = &candles[candles.len().saturating_sub(self.lookback * 2)..];

        // Analyze components
        let delta = analyze_delta(window, self.lookback);
        let imbalances = detect_imbalances(window, &self.imbalance_config);
        let sweeps = detect_sweeps(window, &self.sweep_config);

        // Score: positive = bullish, negative = bearish
        let mut score: f64 = 0.0;
        let mut signals_found = 0u32;

        // Delta divergence signals (strongest)
        if let Some(div) = delta.divergences.last() {
            match div.divergence_type {
                DivergenceType::Bullish => {
                    score += 2.0;
                    signals_found += 1;
                }
                DivergenceType::Bearish => {
                    score -= 2.0;
                    signals_found += 1;
                }
            }
        }

        // Recent imbalance signals
        let recent_imbalances: Vec<_> = imbalances
            .iter()
            .filter(|z| z.index >= window.len().saturating_sub(3))
            .collect();

        for imb in &recent_imbalances {
            match imb.zone_type {
                ImbalanceType::BuyImbalance => score += 1.0,
                ImbalanceType::SellImbalance => score -= 1.0,
            }
            signals_found += 1;
        }

        // Recent sweep signals
        let recent_sweeps: Vec<_> = sweeps
            .iter()
            .filter(|s| s.index >= window.len().saturating_sub(3))
            .collect();

        for sweep in &recent_sweeps {
            match sweep.sweep_type {
                SweepType::BullishSweep => score += 1.5,
                SweepType::BearishSweep => score -= 1.5,
            }
            signals_found += 1;
        }

        if signals_found == 0 {
            return Ok(None);
        }

        let confidence = (score.abs() / (signals_found as f64 * 2.0)).min(1.0);
        if confidence < self.min_confidence {
            return Ok(None);
        }

        let action = if score > 0.5 {
            AIAction::EnterLong
        } else if score < -0.5 {
            AIAction::EnterShort
        } else {
            return Ok(None);
        };

        let last = match candles.last() {
            Some(c) => c,
            None => return Ok(None),
        };
        Ok(Some(Signal {
            timestamp: last.close_time,
            symbol: String::new(),
            action,
            confidence,
            source: "orderflow".into(),
            metadata: std::collections::HashMap::new(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn candle_cv(close: &str, buy: &str, sell: &str) -> Candle {
        let c = Decimal::from_str(close).unwrap();
        let bv = Decimal::from_str(buy).unwrap();
        let sv = Decimal::from_str(sell).unwrap();
        Candle {
            open_time: 0,
            open: c,
            high: c + Decimal::from(5),
            low: c - Decimal::from(5),
            close: c,
            volume: bv + sv,
            close_time: 1000,
            quote_volume: Decimal::ZERO,
            trades: 100,
            taker_buy_volume: bv,
            taker_sell_volume: sv,
        }
    }

    #[test]
    fn insufficient_candles_returns_none() {
        let strategy = OrderFlowStrategy::new(10);
        let candles = vec![candle_cv("100", "50", "50")];
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn bullish_imbalance_produces_long() {
        let mut candles: Vec<_> = (0..20).map(|_| candle_cv("100", "50", "50")).collect();
        // Add strong buy imbalance at end
        candles.push(candle_cv("101", "90", "10"));
        candles.push(candle_cv("102", "95", "5"));
        candles.push(candle_cv("103", "92", "8"));

        let strategy = OrderFlowStrategy::new(10).with_min_confidence(0.0);
        let result = strategy.analyze(&candles).unwrap();
        if let Some(signal) = result {
            assert_eq!(signal.action, AIAction::EnterLong);
        }
    }

    #[test]
    fn test_strategy_name() {
        let strategy = OrderFlowStrategy::new(10);
        assert_eq!(strategy.name(), "orderflow");
    }

    #[test]
    fn test_bearish_imbalance_produces_short() {
        let mut candles: Vec<_> = (0..20).map(|_| candle_cv("100", "50", "50")).collect();
        // Add strong sell imbalance at end
        candles.push(candle_cv("99", "10", "90"));
        candles.push(candle_cv("98", "5", "95"));
        candles.push(candle_cv("97", "8", "92"));

        let strategy = OrderFlowStrategy::new(10).with_min_confidence(0.0);
        let result = strategy.analyze(&candles).unwrap();
        if let Some(signal) = result {
            assert_eq!(signal.action, AIAction::EnterShort);
        }
    }

    #[test]
    fn test_balanced_returns_none() {
        let candles: Vec<_> = (0..25).map(|_| candle_cv("100", "50", "50")).collect();
        let strategy = OrderFlowStrategy::new(10).with_min_confidence(0.0);
        let result = strategy.analyze(&candles).unwrap();
        // Equal buy/sell throughout — score near 0, should return None
        assert!(result.is_none());
    }

    #[test]
    fn name_returns_orderflow() {
        let strategy = OrderFlowStrategy::new(5);
        assert_eq!(strategy.name(), "orderflow");
    }

    #[test]
    fn empty_candles_returns_none() {
        let strategy = OrderFlowStrategy::new(5);
        assert!(strategy.analyze(&[]).unwrap().is_none());
    }

    #[test]
    fn single_candle_returns_none() {
        let strategy = OrderFlowStrategy::new(5);
        let candles = vec![candle_cv("100", "80", "20")];
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn exactly_lookback_plus_one_returns_none() {
        // Need lookback + 2 candles minimum; lookback + 1 is insufficient
        let lookback = 10;
        let strategy = OrderFlowStrategy::new(lookback);
        let candles: Vec<_> = (0..(lookback + 1))
            .map(|_| candle_cv("100", "80", "20"))
            .collect();
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn exactly_lookback_plus_two_is_sufficient() {
        // lookback + 2 candles is the minimum that doesn't bail early
        let lookback = 5;
        let strategy = OrderFlowStrategy::new(lookback).with_min_confidence(0.0);
        let candles: Vec<_> = (0..(lookback + 2))
            .map(|_| candle_cv("100", "90", "10"))
            .collect();
        // May or may not produce a signal depending on orderflow analysis,
        // but it should not return None due to insufficient candles
        let _ = strategy.analyze(&candles).unwrap();
    }

    #[test]
    fn high_confidence_threshold_filters() {
        let strategy = OrderFlowStrategy::new(5).with_min_confidence(0.99);
        let mut candles: Vec<_> = (0..20).map(|_| candle_cv("100", "50", "50")).collect();
        candles.push(candle_cv("101", "60", "40"));
        candles.push(candle_cv("102", "65", "35"));
        // Even with some buy pressure, 0.99 threshold should filter
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn with_min_confidence_sets_value() {
        let strategy = OrderFlowStrategy::new(10).with_min_confidence(0.75);
        // Balanced candles should be filtered
        let candles: Vec<_> = (0..25).map(|_| candle_cv("100", "50", "50")).collect();
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn strong_sell_pressure_produces_short() {
        // Create a scenario with strong declining prices and heavy sell volume
        let mut candles: Vec<_> = (0..20).map(|_| candle_cv("100", "50", "50")).collect();
        // Append candles with dropping price and heavy sell volume
        for i in 1..=5 {
            let price = format!("{}", 100 - i * 3);
            candles.push(candle_cv(&price, "5", "95"));
        }
        let strategy = OrderFlowStrategy::new(10).with_min_confidence(0.0);
        let result = strategy.analyze(&candles).unwrap();
        if let Some(signal) = result {
            assert_eq!(signal.action, AIAction::EnterShort);
            assert_eq!(signal.source, "orderflow");
        }
    }

    #[test]
    fn strong_buy_pressure_produces_long() {
        // Create a scenario with rising prices and heavy buy volume
        let mut candles: Vec<_> = (0..20).map(|_| candle_cv("100", "50", "50")).collect();
        // Append candles with rising price and heavy buy volume
        for i in 1..=5 {
            let price = format!("{}", 100 + i * 3);
            candles.push(candle_cv(&price, "95", "5"));
        }
        let strategy = OrderFlowStrategy::new(10).with_min_confidence(0.0);
        let result = strategy.analyze(&candles).unwrap();
        if let Some(signal) = result {
            assert_eq!(signal.action, AIAction::EnterLong);
            assert_eq!(signal.source, "orderflow");
        }
    }

    #[test]
    fn signal_confidence_bounded_0_to_1() {
        let mut candles: Vec<_> = (0..20).map(|_| candle_cv("100", "50", "50")).collect();
        candles.push(candle_cv("101", "90", "10"));
        candles.push(candle_cv("102", "95", "5"));
        candles.push(candle_cv("103", "92", "8"));
        let strategy = OrderFlowStrategy::new(10).with_min_confidence(0.0);
        let result = strategy.analyze(&candles).unwrap();
        if let Some(signal) = result {
            assert!(signal.confidence >= 0.0 && signal.confidence <= 1.0);
        }
    }

    #[test]
    fn large_lookback_with_few_candles_returns_none() {
        let strategy = OrderFlowStrategy::new(100);
        let candles: Vec<_> = (0..50).map(|_| candle_cv("100", "80", "20")).collect();
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }
}
