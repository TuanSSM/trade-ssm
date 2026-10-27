//! DCA-into-zones futures strategy.
//!
//! Enters on a volume spike at a confluence level (MA / inner band / outer band /
//! fast-FVG / POC). Scales into the DCA stack as price moves adversely across
//! Bollinger inner → outer. In hedge mode, overlays a counter-trend scalp back
//! to the band mean when price pokes an extreme opposite the primary trend.
//!
//! The strategy is intentionally compact; it reuses existing indicators,
//! orderflow primitives and the Strategy trait's lifecycle callbacks.

use anyhow::Result;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::RwLock;

use ssm_core::{
    AIAction, Candle, ExitReason, Order, Position, RoiEntry, Side, Signal, StoplossType,
};
use ssm_indicators::atr::atr;
use ssm_indicators::bollinger::bollinger_bands;
use ssm_indicators::cvd::{analyze_cvd, CvdTrend};
use ssm_indicators::ema::ema;
use ssm_indicators::fvg::{fast_fvg, FvgType};
use ssm_indicators::vol_delta::{detect_volume_spike, SpikeDirection};
use ssm_orderflow::imbalance::{detect_imbalances, ImbalanceConfig};

use crate::traits::Strategy;

/// Static configuration for the DCA-into-zones strategy.
#[derive(Debug, Clone)]
pub struct DcaZoneConfig {
    pub symbol: String,
    pub bb_period: usize,
    pub bb_inner_mult: Decimal,
    pub bb_outer_mult: Decimal,
    pub ma_period: usize,
    pub trend_ma_period: usize,
    pub atr_period: usize,
    pub cvd_window: usize,
    pub volume_spike_period: usize,
    pub volume_spike_mult: Decimal,
    pub spike_imbalance_frac: Decimal,
    pub imbalance: ImbalanceConfig,
    pub fvg_min_gap_atr: Decimal,
    pub level_proximity_atr: Decimal,
    pub min_confidence: f64,
    pub max_dca_adds: u8,
    pub dca_qty_pcts: [Decimal; 3],
    pub scalp_pct_b_high: Decimal,
    pub scalp_pct_b_low: Decimal,
    pub risk_per_trade_pct: Decimal,
    pub leverage: u32,
    pub stoploss_atr_mult: Decimal,
}

impl Default for DcaZoneConfig {
    fn default() -> Self {
        Self {
            symbol: String::new(),
            bb_period: 20,
            bb_inner_mult: Decimal::ONE,
            bb_outer_mult: Decimal::from(2),
            ma_period: 20,
            trend_ma_period: 50,
            atr_period: 14,
            cvd_window: 15,
            volume_spike_period: 20,
            volume_spike_mult: Decimal::new(25, 1),   // 2.5x
            spike_imbalance_frac: Decimal::new(6, 1), // 0.6
            imbalance: ImbalanceConfig::default(),
            fvg_min_gap_atr: Decimal::new(5, 1), // 0.5 ATR
            level_proximity_atr: Decimal::ONE,
            min_confidence: 0.55,
            max_dca_adds: 3,
            dca_qty_pcts: [
                Decimal::new(50, 2), // 0.50 × base at MA
                Decimal::new(75, 2), // 0.75 × base at inner
                Decimal::ONE,        // 1.00 × base at outer
            ],
            scalp_pct_b_high: Decimal::new(95, 2),
            scalp_pct_b_low: Decimal::new(5, 2),
            risk_per_trade_pct: Decimal::new(2, 2), // 2% balance per trade
            leverage: 5,
            stoploss_atr_mult: Decimal::from(2),
        }
    }
}

/// Per-symbol DCA tier bookkeeping stored by the strategy across callbacks.
#[derive(Debug, Clone)]
struct DcaState {
    side: Side,
    base_quantity: Decimal,
    tiers_filled: u8,
    tier_levels: [Decimal; 3],
}

/// DCA-into-zones futures strategy. See the module docstring.
pub struct DcaZoneStrategy {
    cfg: DcaZoneConfig,
    state: RwLock<HashMap<String, DcaState>>,
}

impl DcaZoneStrategy {
    pub fn new(cfg: DcaZoneConfig) -> Self {
        Self {
            cfg,
            state: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_symbol(mut self, symbol: &str) -> Self {
        self.cfg.symbol = symbol.to_string();
        self
    }

    fn min_candles(&self) -> usize {
        self.cfg
            .bb_period
            .max(self.cfg.trend_ma_period)
            .max(self.cfg.atr_period + 1)
            .max(self.cfg.volume_spike_period)
            .max(self.cfg.cvd_window)
            + 2
    }

    /// Current trend bias from the slope of the trend EMA.
    fn trend_bias(&self, candles: &[Candle]) -> TrendBias {
        let series = ema(candles, self.cfg.trend_ma_period);
        if series.len() < 3 {
            return TrendBias::Neutral;
        }
        let last = series[series.len() - 1];
        let prev = series[series.len() - 3];
        if last > prev {
            TrendBias::Bull
        } else if last < prev {
            TrendBias::Bear
        } else {
            TrendBias::Neutral
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrendBias {
    Bull,
    Bear,
    Neutral,
}

impl Strategy for DcaZoneStrategy {
    fn name(&self) -> &str {
        "dca_zone"
    }

    fn analyze(&self, candles: &[Candle]) -> Result<Option<Signal>> {
        if candles.len() < self.min_candles() {
            return Ok(None);
        }

        let last = candles.last().expect("length checked above");
        let last_close = last.close;

        let bb_inner = bollinger_bands(candles, self.cfg.bb_period, self.cfg.bb_inner_mult);
        let bb_outer = bollinger_bands(candles, self.cfg.bb_period, self.cfg.bb_outer_mult);
        let ma = ema(candles, self.cfg.ma_period);
        let atr_series = atr(candles, self.cfg.atr_period);

        if bb_inner.middle.is_empty()
            || bb_outer.upper.is_empty()
            || ma.is_empty()
            || atr_series.is_empty()
        {
            return Ok(None);
        }

        let last_atr = *atr_series.last().expect("non-empty");
        let last_mid = *bb_inner.middle.last().expect("non-empty");
        let last_inner_up = *bb_inner.upper.last().expect("non-empty");
        let last_inner_lo = *bb_inner.lower.last().expect("non-empty");
        let last_outer_up = *bb_outer.upper.last().expect("non-empty");
        let last_outer_lo = *bb_outer.lower.last().expect("non-empty");
        let last_pct_b = *bb_inner.pct_b.last().expect("non-empty");
        let last_ma = *ma.last().expect("non-empty");

        let bias = self.trend_bias(candles);
        let cvd = analyze_cvd(candles, self.cfg.cvd_window);
        let spike = detect_volume_spike(
            candles,
            self.cfg.volume_spike_period,
            self.cfg.volume_spike_mult,
            self.cfg.spike_imbalance_frac,
        );

        // --- Counter-trend scalp (hedge mode co-exists with trend DCA) ---
        //
        // Price pokes an outer band opposite the trend → mean-revert to BB middle.
        if bias == TrendBias::Bull && last_pct_b >= self.cfg.scalp_pct_b_high {
            return Ok(Some(self.build_signal(
                last.close_time,
                AIAction::EnterShort,
                0.65,
                "scalp",
                last_mid,
                [last_ma, last_inner_up, last_outer_up],
            )));
        }
        if bias == TrendBias::Bear && last_pct_b <= self.cfg.scalp_pct_b_low {
            return Ok(Some(self.build_signal(
                last.close_time,
                AIAction::EnterLong,
                0.65,
                "scalp",
                last_mid,
                [last_ma, last_inner_lo, last_outer_lo],
            )));
        }

        // --- Big signal: volume spike at a level in the trend direction ---
        let proximity = last_atr * self.cfg.level_proximity_atr;
        let fvgs = fast_fvg(candles, &atr_series, self.cfg.fvg_min_gap_atr);
        let imbalances = detect_imbalances(candles, &self.cfg.imbalance);

        let bullish_setup = spike == SpikeDirection::Bullish
            && cvd.trend == CvdTrend::Bullish
            && (bias == TrendBias::Bull || bias == TrendBias::Neutral)
            && price_near_any(
                last_close,
                proximity,
                &[last_ma, last_inner_lo, last_outer_lo],
            );
        let bearish_setup = spike == SpikeDirection::Bearish
            && cvd.trend == CvdTrend::Bearish
            && (bias == TrendBias::Bear || bias == TrendBias::Neutral)
            && price_near_any(
                last_close,
                proximity,
                &[last_ma, last_inner_up, last_outer_up],
            );

        if !bullish_setup && !bearish_setup {
            return Ok(None);
        }

        let confidence = self.confidence_score(
            &cvd,
            spike,
            last_pct_b,
            last_close,
            last_ma,
            last_atr,
            &fvgs,
            &imbalances,
        );
        if confidence < self.cfg.min_confidence {
            return Ok(None);
        }

        if bullish_setup {
            Ok(Some(self.build_signal(
                last.close_time,
                AIAction::EnterLong,
                confidence,
                "trend",
                last_close,
                [last_ma, last_inner_lo, last_outer_lo],
            )))
        } else {
            Ok(Some(self.build_signal(
                last.close_time,
                AIAction::EnterShort,
                confidence,
                "trend",
                last_close,
                [last_ma, last_inner_up, last_outer_up],
            )))
        }
    }

    fn on_trade_enter(&self, signal: &Signal, _position: Option<&Position>) -> bool {
        if signal.confidence < self.cfg.min_confidence
            && signal.metadata.get("role").map(String::as_str) != Some("scalp")
        {
            return false;
        }
        // Seed DCA state for trend signals. Scalps don't DCA.
        if signal.metadata.get("role").map(String::as_str) == Some("trend") {
            let side = match signal.action {
                AIAction::EnterLong => Side::Buy,
                AIAction::EnterShort => Side::Sell,
                _ => return true,
            };
            let tier_levels = [
                decimal_from_meta(signal, "zone_ma"),
                decimal_from_meta(signal, "zone_inner"),
                decimal_from_meta(signal, "zone_outer"),
            ];
            self.state.write().unwrap().insert(
                signal.symbol.clone(),
                DcaState {
                    side,
                    base_quantity: Decimal::ZERO,
                    tiers_filled: 0,
                    tier_levels,
                },
            );
        }
        true
    }

    fn on_order_filled(&self, order: &Order, position: &Position) {
        // Seed base quantity on the very first fill.
        let mut map = self.state.write().unwrap();
        if let Some(state) = map.get_mut(&order.symbol) {
            if state.base_quantity.is_zero() {
                state.base_quantity = position.quantity;
            }
        }
    }

    fn on_trade_exit(&self, position: &Position, candles: &[Candle]) -> Option<ExitReason> {
        // Clean DCA state when the strategy decides to exit.
        if candles.len() < self.cfg.cvd_window {
            return None;
        }
        let cvd = analyze_cvd(candles, self.cfg.cvd_window);
        let reversal = matches!(
            (position.side, cvd.trend),
            (Side::Buy, CvdTrend::Bearish) | (Side::Sell, CvdTrend::Bullish)
        );
        if reversal {
            self.state.write().unwrap().remove(&position.symbol);
            return Some(ExitReason::CustomExit("cvd_reversal".into()));
        }
        None
    }

    fn custom_position_size(&self, signal: &Signal, balance: Decimal) -> Option<Decimal> {
        // Scalps use a fraction of trend sizing.
        let base = balance * self.cfg.risk_per_trade_pct;
        let is_scalp = signal.metadata.get("role").map(String::as_str) == Some("scalp");
        if is_scalp {
            Some(base / Decimal::from(2))
        } else {
            Some(base)
        }
    }

    fn should_adjust_position(&self, position: &Position, candles: &[Candle]) -> Option<Decimal> {
        let map = self.state.read().unwrap();
        let state = map.get(&position.symbol)?;
        if state.tiers_filled >= self.cfg.max_dca_adds {
            return None;
        }
        let tier_idx = state.tiers_filled as usize;
        let target_level = state.tier_levels.get(tier_idx)?;
        if target_level.is_zero() {
            return None;
        }
        let last_close = candles.last()?.close;
        // DCA triggers when price trades through the next tier against the position.
        let crossed = match state.side {
            Side::Buy => last_close <= *target_level,
            Side::Sell => last_close >= *target_level,
        };
        if !crossed {
            return None;
        }

        // Confirming spike in the position direction keeps us from catching knives.
        let spike = detect_volume_spike(
            candles,
            self.cfg.volume_spike_period,
            self.cfg.volume_spike_mult,
            self.cfg.spike_imbalance_frac,
        );
        let confirming = matches!(
            (state.side, spike),
            (Side::Buy, SpikeDirection::Bullish) | (Side::Sell, SpikeDirection::Bearish)
        );
        if !confirming {
            return None;
        }

        drop(map);
        let mut map = self.state.write().unwrap();
        let state = map.get_mut(&position.symbol)?;
        let qty_mult = self.cfg.dca_qty_pcts.get(tier_idx).copied()?;
        let base = if state.base_quantity.is_zero() {
            position.quantity
        } else {
            state.base_quantity
        };
        state.tiers_filled += 1;
        Some(base * qty_mult)
    }

    fn custom_stoploss(
        &self,
        _position: &Position,
        candles: &[Candle],
        _candles_in_trade: usize,
    ) -> Option<Decimal> {
        let series = atr(candles, self.cfg.atr_period);
        let last_atr = *series.last()?;
        let close = candles.last()?.close;
        if close.is_zero() {
            return None;
        }
        let pct = (last_atr * self.cfg.stoploss_atr_mult) / close;
        Some(pct)
    }

    fn stoploss_type(&self) -> Option<StoplossType> {
        Some(StoplossType::AtrTrailing {
            multiplier: self.cfg.stoploss_atr_mult,
            atr_period: self.cfg.atr_period,
        })
    }

    fn roi_table(&self) -> Vec<RoiEntry> {
        vec![
            RoiEntry {
                minutes: 0,
                roi_pct: Decimal::new(3, 2),
            },
            RoiEntry {
                minutes: 60,
                roi_pct: Decimal::new(15, 3), // 1.5%
            },
            RoiEntry {
                minutes: 240,
                roi_pct: Decimal::new(5, 3), // 0.5%
            },
        ]
    }

    fn leverage(&self, _pair: &str, _signal: &Signal) -> Decimal {
        Decimal::from(self.cfg.leverage)
    }
}

impl DcaZoneStrategy {
    #[allow(clippy::too_many_arguments)]
    fn build_signal(
        &self,
        timestamp: i64,
        action: AIAction,
        confidence: f64,
        role: &str,
        base_level: Decimal,
        tier_levels: [Decimal; 3],
    ) -> Signal {
        let mut meta: HashMap<String, String> = HashMap::new();
        meta.insert("role".into(), role.into());
        meta.insert("base_level".into(), base_level.to_string());
        meta.insert("zone_ma".into(), tier_levels[0].to_string());
        meta.insert("zone_inner".into(), tier_levels[1].to_string());
        meta.insert("zone_outer".into(), tier_levels[2].to_string());
        Signal {
            timestamp,
            symbol: self.cfg.symbol.clone(),
            action,
            confidence,
            source: self.name().into(),
            metadata: meta,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn confidence_score(
        &self,
        cvd: &ssm_indicators::cvd::CvdAnalysis,
        spike: SpikeDirection,
        pct_b: Decimal,
        close: Decimal,
        ma: Decimal,
        atr_val: Decimal,
        fvgs: &[ssm_indicators::fvg::FvgZone],
        imbalances: &[ssm_orderflow::imbalance::ImbalanceZone],
    ) -> f64 {
        let mut score = 0.4_f64;
        if spike != SpikeDirection::None {
            let magnitude = cvd.total_cvd.abs().to_f64().unwrap_or(0.0);
            let per_candle = magnitude / self.cfg.cvd_window.max(1) as f64;
            score += per_candle.min(0.3);
        }
        // %B near an extreme nudges confidence.
        let pct_b_f = pct_b.to_f64().unwrap_or(0.5);
        if !(0.3..=0.7).contains(&pct_b_f) {
            score += 0.1;
        }
        // Price near MA is a confluent reversion target.
        let dist = (close - ma).abs();
        if atr_val > Decimal::ZERO && dist <= atr_val {
            score += 0.05;
        }
        // FVG confluence close to the trigger.
        if let Some(fvg) = fvgs.last() {
            let near = match fvg.fvg_type {
                FvgType::Bullish => close >= fvg.bottom && close <= fvg.top + atr_val,
                FvgType::Bearish => close <= fvg.top && close >= fvg.bottom - atr_val,
            };
            if near {
                score += 0.1;
            }
        }
        // Fresh imbalance in last 3 candles adds a small boost.
        let recent = imbalances
            .iter()
            .rev()
            .take(3)
            .any(|z| z.ratio >= self.cfg.imbalance.min_ratio);
        if recent {
            score += 0.05;
        }
        score.clamp(0.0, 1.0)
    }
}

fn price_near_any(price: Decimal, tolerance: Decimal, levels: &[Decimal]) -> bool {
    levels
        .iter()
        .any(|lvl| !lvl.is_zero() && (price - *lvl).abs() <= tolerance)
}

fn decimal_from_meta(signal: &Signal, key: &str) -> Decimal {
    signal
        .metadata
        .get(key)
        .and_then(|s| Decimal::from_str_exact(s).ok())
        .unwrap_or(Decimal::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssm_core::OrderStatus;
    use std::str::FromStr;

    #[allow(clippy::too_many_arguments)]
    fn candle(
        time: i64,
        open: &str,
        high: &str,
        low: &str,
        close: &str,
        volume: &str,
        buy: &str,
        sell: &str,
    ) -> Candle {
        Candle {
            open_time: time,
            open: Decimal::from_str(open).unwrap(),
            high: Decimal::from_str(high).unwrap(),
            low: Decimal::from_str(low).unwrap(),
            close: Decimal::from_str(close).unwrap(),
            volume: Decimal::from_str(volume).unwrap(),
            close_time: time + 899_999,
            quote_volume: Decimal::ZERO,
            trades: 50,
            taker_buy_volume: Decimal::from_str(buy).unwrap(),
            taker_sell_volume: Decimal::from_str(sell).unwrap(),
        }
    }

    fn flat_candles(n: usize, price: &str) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let t = (i as i64) * 900_000;
                candle(t, price, price, price, price, "100", "50", "50")
            })
            .collect()
    }

    #[test]
    fn insufficient_candles_returns_none() {
        let strategy = DcaZoneStrategy::new(DcaZoneConfig::default());
        let candles = flat_candles(10, "100");
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn flat_market_no_signal() {
        let strategy = DcaZoneStrategy::new(DcaZoneConfig::default());
        let candles = flat_candles(80, "100");
        assert!(strategy.analyze(&candles).unwrap().is_none());
    }

    #[test]
    fn bullish_spike_at_ma_triggers_entry() {
        let cfg = DcaZoneConfig {
            min_confidence: 0.4,
            symbol: "BTCUSDT".into(),
            ..DcaZoneConfig::default()
        };
        let strategy = DcaZoneStrategy::new(cfg);

        let mut candles: Vec<Candle> = Vec::new();
        // 60 upward-drifting candles so trend bias = Bull and MA sits below price.
        for i in 0..60 {
            let base = 100 + i;
            let t = (i as i64) * 900_000;
            candles.push(candle(
                t,
                &format!("{base}"),
                &format!("{}", base + 2),
                &format!("{}", base - 2),
                &format!("{}", base + 1),
                "100",
                "55",
                "45",
            ));
        }
        // Spike bar: 5× volume, strong bullish delta, closes near MA.
        candles.push(candle(
            60 * 900_000,
            "140",
            "160",
            "138",
            "158",
            "600",
            "500",
            "100",
        ));

        let signal = strategy.analyze(&candles).unwrap();
        assert!(signal.is_some(), "expected a signal");
        let signal = signal.unwrap();
        assert!(
            matches!(signal.action, AIAction::EnterLong | AIAction::EnterShort),
            "action was {:?}",
            signal.action
        );
    }

    #[test]
    fn no_repainting_dca_zone() {
        let strategy = DcaZoneStrategy::new(DcaZoneConfig::default());
        let short = flat_candles(80, "100");
        let mut long = short.clone();
        long.push(candle(
            80 * 900_000,
            "100",
            "100",
            "100",
            "100",
            "100",
            "50",
            "50",
        ));
        let r_short = strategy.analyze(&short).unwrap();
        let r_long_prefix = strategy.analyze(&short).unwrap();
        // Analyzing the same prefix twice must yield the same result.
        assert_eq!(r_short.is_some(), r_long_prefix.is_some());
    }

    #[test]
    fn roi_table_populated() {
        let strategy = DcaZoneStrategy::new(DcaZoneConfig::default());
        let roi = strategy.roi_table();
        assert_eq!(roi.len(), 3);
        assert_eq!(roi[0].minutes, 0);
    }

    #[test]
    fn stoploss_type_is_atr_trailing() {
        let strategy = DcaZoneStrategy::new(DcaZoneConfig::default());
        let sl = strategy.stoploss_type();
        assert!(matches!(sl, Some(StoplossType::AtrTrailing { .. })));
    }

    #[test]
    fn leverage_matches_config() {
        let cfg = DcaZoneConfig {
            leverage: 7,
            ..DcaZoneConfig::default()
        };
        let strategy = DcaZoneStrategy::new(cfg);
        let signal = Signal {
            timestamp: 0,
            symbol: "BTCUSDT".into(),
            action: AIAction::EnterLong,
            confidence: 0.9,
            source: "test".into(),
            metadata: HashMap::new(),
        };
        assert_eq!(strategy.leverage("BTCUSDT", &signal), Decimal::from(7));
    }

    #[test]
    fn custom_position_size_halves_for_scalp() {
        let strategy = DcaZoneStrategy::new(DcaZoneConfig::default());
        let mut trend = Signal {
            timestamp: 0,
            symbol: "BTCUSDT".into(),
            action: AIAction::EnterLong,
            confidence: 0.9,
            source: "test".into(),
            metadata: HashMap::new(),
        };
        trend.metadata.insert("role".into(), "trend".into());
        let trend_size = strategy.custom_position_size(&trend, Decimal::from(10_000));

        let mut scalp = trend.clone();
        scalp.metadata.insert("role".into(), "scalp".into());
        let scalp_size = strategy.custom_position_size(&scalp, Decimal::from(10_000));

        assert!(scalp_size.unwrap() < trend_size.unwrap());
    }

    #[test]
    fn should_adjust_position_triggers_on_tier_crossing() {
        let cfg = DcaZoneConfig {
            symbol: "BTCUSDT".into(),
            ..DcaZoneConfig::default()
        };
        let strategy = DcaZoneStrategy::new(cfg);

        // Seed state as if the entry had happened.
        {
            let mut map = strategy.state.write().unwrap();
            map.insert(
                "BTCUSDT".into(),
                DcaState {
                    side: Side::Buy,
                    base_quantity: Decimal::ONE,
                    tiers_filled: 0,
                    tier_levels: [Decimal::from(95), Decimal::from(90), Decimal::from(85)],
                },
            );
        }

        // Craft candles so the last candle has a bullish volume spike at price 95.
        let mut candles: Vec<Candle> = (0..19)
            .map(|i| {
                candle(
                    i as i64 * 900_000,
                    "100",
                    "100",
                    "100",
                    "100",
                    "100",
                    "50",
                    "50",
                )
            })
            .collect();
        candles.push(candle(
            19 * 900_000,
            "95",
            "95",
            "95",
            "95",
            "600",
            "500",
            "100",
        ));

        let position = Position {
            symbol: "BTCUSDT".into(),
            side: Side::Buy,
            entry_price: Decimal::from(100),
            quantity: Decimal::ONE,
            unrealized_pnl: Decimal::ZERO,
            realized_pnl: Decimal::ZERO,
            leverage: 1,
            opened_at: 0,
        };

        let adj = strategy.should_adjust_position(&position, &candles);
        assert_eq!(adj, Some(Decimal::new(50, 2))); // 0.5 of base
    }

    #[test]
    fn on_order_filled_captures_base_quantity() {
        let cfg = DcaZoneConfig {
            symbol: "BTCUSDT".into(),
            ..DcaZoneConfig::default()
        };
        let strategy = DcaZoneStrategy::new(cfg);
        strategy.state.write().unwrap().insert(
            "BTCUSDT".into(),
            DcaState {
                side: Side::Buy,
                base_quantity: Decimal::ZERO,
                tiers_filled: 0,
                tier_levels: [Decimal::from(99), Decimal::from(95), Decimal::from(90)],
            },
        );

        let order = Order {
            id: "ord-1".into(),
            symbol: "BTCUSDT".into(),
            side: Side::Buy,
            order_type: ssm_core::OrderType::Market,
            quantity: Decimal::ONE,
            price: None,
            stop_price: None,
            trailing_delta: None,
            time_in_force: None,
            reduce_only: false,
            status: OrderStatus::Filled,
            created_at: 0,
            updated_at: 0,
        };
        let position = Position {
            symbol: "BTCUSDT".into(),
            side: Side::Buy,
            entry_price: Decimal::from(100),
            quantity: Decimal::ONE,
            unrealized_pnl: Decimal::ZERO,
            realized_pnl: Decimal::ZERO,
            leverage: 1,
            opened_at: 0,
        };
        strategy.on_order_filled(&order, &position);
        let base = strategy
            .state
            .read()
            .unwrap()
            .get("BTCUSDT")
            .map(|s| s.base_quantity)
            .unwrap_or(Decimal::ZERO);
        assert_eq!(base, Decimal::ONE);
    }
}
