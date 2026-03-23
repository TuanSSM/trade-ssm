use rust_decimal::Decimal;
use ssm_core::Candle;

/// Compute Simple Moving Average over the last `period` candles.
pub fn sma(candles: &[Candle], period: usize) -> Vec<Decimal> {
    if candles.len() < period || period == 0 {
        return vec![];
    }

    let mut result = Vec::with_capacity(candles.len() - period + 1);
    let mut sum: Decimal = candles[..period].iter().map(|c| c.close).sum();
    result.push(sum / Decimal::from(period as u64));

    for i in period..candles.len() {
        sum += candles[i].close - candles[i - period].close;
        result.push(sum / Decimal::from(period as u64));
    }

    result
}

/// Compute Exponential Moving Average.
///
/// Multiplier: `2 / (period + 1)`
/// EMA[0] = SMA of first `period` candles.
/// EMA[i] = close * multiplier + EMA[i-1] * (1 - multiplier)
pub fn ema(candles: &[Candle], period: usize) -> Vec<Decimal> {
    if candles.len() < period || period == 0 {
        return vec![];
    }

    let multiplier = Decimal::from(2) / Decimal::from((period + 1) as u64);
    let one_minus = Decimal::ONE - multiplier;

    // Seed with SMA
    let initial_sma: Decimal =
        candles[..period].iter().map(|c| c.close).sum::<Decimal>() / Decimal::from(period as u64);

    let mut result = Vec::with_capacity(candles.len() - period + 1);
    result.push(initial_sma);

    let mut prev = initial_sma;
    for c in &candles[period..] {
        let val = c.close * multiplier + prev * one_minus;
        result.push(val);
        prev = val;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn candle_close(close: &str) -> Candle {
        let c = Decimal::from_str(close).unwrap();
        Candle {
            open_time: 0,
            open: c,
            high: c,
            low: c,
            close: c,
            volume: Decimal::from(100),
            close_time: 0,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: Decimal::from(50),
            taker_sell_volume: Decimal::from(50),
        }
    }

    #[test]
    fn sma_basic() {
        let candles = vec![
            candle_close("10"),
            candle_close("20"),
            candle_close("30"),
            candle_close("40"),
            candle_close("50"),
        ];
        let result = sma(&candles, 3);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], Decimal::from(20)); // (10+20+30)/3
        assert_eq!(result[1], Decimal::from(30)); // (20+30+40)/3
        assert_eq!(result[2], Decimal::from(40)); // (30+40+50)/3
    }

    #[test]
    fn ema_basic() {
        let candles = vec![
            candle_close("100"),
            candle_close("110"),
            candle_close("120"),
            candle_close("130"),
        ];
        let result = ema(&candles, 3);
        assert_eq!(result.len(), 2);
        // First value is SMA(3) = (100+110+120)/3 = 110
        assert_eq!(result[0], Decimal::from(110));
        // Second: 130 * 0.5 + 110 * 0.5 = 120
        assert_eq!(result[1], Decimal::from(120));
    }

    #[test]
    fn insufficient_candles() {
        let candles = vec![candle_close("100"), candle_close("110")];
        assert!(sma(&candles, 5).is_empty());
        assert!(ema(&candles, 5).is_empty());
    }

    #[test]
    fn no_repainting_sma() {
        let short = vec![candle_close("10"), candle_close("20"), candle_close("30")];
        let mut long = short.clone();
        long.push(candle_close("40"));

        let r_short = sma(&short, 3);
        let r_long = sma(&long, 3);

        assert_eq!(r_short[0], r_long[0], "SMA repainting");
    }

    #[test]
    fn no_repainting_ema() {
        let short = vec![
            candle_close("100"),
            candle_close("110"),
            candle_close("120"),
        ];
        let mut long = short.clone();
        long.push(candle_close("130"));

        let r_short = ema(&short, 3);
        let r_long = ema(&long, 3);

        assert_eq!(r_short[0], r_long[0], "EMA repainting");
    }

    #[test]
    fn test_sma_single_period() {
        let candles = vec![
            candle_close("10"),
            candle_close("20"),
            candle_close("30"),
        ];
        let result = sma(&candles, 1);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], Decimal::from(10));
        assert_eq!(result[1], Decimal::from(20));
        assert_eq!(result[2], Decimal::from(30));
    }

    #[test]
    fn test_sma_period_equals_len() {
        let candles = vec![
            candle_close("10"),
            candle_close("20"),
            candle_close("30"),
        ];
        let result = sma(&candles, 3);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], Decimal::from(20)); // (10+20+30)/3
    }

    #[test]
    fn test_sma_zero_period() {
        let candles = vec![candle_close("10"), candle_close("20")];
        let result = sma(&candles, 0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_ema_zero_period() {
        let candles = vec![candle_close("10"), candle_close("20")];
        let result = ema(&candles, 0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_ema_converges() {
        // Constant price of 100 → EMA should converge to 100
        let candles: Vec<_> = (0..50).map(|_| candle_close("100")).collect();
        let result = ema(&candles, 10);
        assert!(!result.is_empty());
        for val in &result {
            assert_eq!(*val, Decimal::from(100));
        }
    }

    #[test]
    fn test_sma_output_length() {
        let candles: Vec<_> = (0..10)
            .map(|i| candle_close(&format!("{}", 100 + i)))
            .collect();
        let period = 4;
        let result = sma(&candles, period);
        assert_eq!(result.len(), candles.len() - period + 1);
    }
}
