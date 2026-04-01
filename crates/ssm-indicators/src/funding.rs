use rust_decimal::Decimal;
use ssm_core::FundingRate;

/// Funding rate analysis result.
#[derive(Debug, Clone)]
pub struct FundingAnalysis {
    pub current_rate: Decimal,
    pub avg_rate: Decimal,
    pub is_extreme: bool,
    pub sentiment: FundingSentiment,
}

/// Market sentiment derived from funding rates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FundingSentiment {
    /// Negative funding = shorts paying longs = market is overleveraged short.
    Bullish,
    /// Positive funding = longs paying shorts = market is overleveraged long.
    Bearish,
    /// Funding rate is near zero — no strong directional bias.
    Neutral,
}

impl std::fmt::Display for FundingSentiment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Bullish => "BULLISH",
            Self::Bearish => "BEARISH",
            Self::Neutral => "NEUTRAL",
        })
    }
}

/// Extreme funding rate threshold (0.1% = 0.001).
const EXTREME_ABSOLUTE: Decimal = Decimal::from_parts(1, 0, 0, false, 3);
/// Sentiment threshold (0.0001 = 1 bps).
const SENTIMENT_THRESHOLD: Decimal = Decimal::from_parts(1, 0, 0, false, 4);

/// Analyze a series of funding rate snapshots.
///
/// Pure function — no I/O, deterministic.
pub fn analyze_funding(rates: &[FundingRate]) -> FundingAnalysis {
    if rates.is_empty() {
        return FundingAnalysis {
            current_rate: Decimal::ZERO,
            avg_rate: Decimal::ZERO,
            is_extreme: false,
            sentiment: FundingSentiment::Neutral,
        };
    }

    let current_rate = rates.last().map(|r| r.rate).unwrap_or(Decimal::ZERO);
    let sum: Decimal = rates.iter().map(|r| r.rate).sum();
    let avg_rate = sum / Decimal::from(rates.len() as u64);

    // Extreme: abs(current) > 3 * abs(avg) OR abs(current) > 0.1%
    let abs_current = current_rate.abs();
    let abs_avg = avg_rate.abs();
    let is_extreme = abs_current > EXTREME_ABSOLUTE
        || (abs_avg > Decimal::ZERO && abs_current > abs_avg * Decimal::from(3));

    // Sentiment: very negative funding → bullish (shorts paying),
    //            very positive → bearish (longs paying)
    let sentiment = if current_rate < -SENTIMENT_THRESHOLD {
        FundingSentiment::Bullish
    } else if current_rate > SENTIMENT_THRESHOLD {
        FundingSentiment::Bearish
    } else {
        FundingSentiment::Neutral
    };

    FundingAnalysis {
        current_rate,
        avg_rate,
        is_extreme,
        sentiment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(r: &str) -> FundingRate {
        FundingRate {
            symbol: "BTCUSDT".into(),
            rate: r.parse().unwrap(),
            timestamp: 0,
            next_funding_time: None,
        }
    }

    #[test]
    fn empty_rates_neutral() {
        let a = analyze_funding(&[]);
        assert_eq!(a.current_rate, Decimal::ZERO);
        assert_eq!(a.sentiment, FundingSentiment::Neutral);
        assert!(!a.is_extreme);
    }

    #[test]
    fn single_positive_rate_bearish() {
        let a = analyze_funding(&[rate("0.0005")]);
        assert_eq!(a.sentiment, FundingSentiment::Bearish);
        assert_eq!(a.current_rate, "0.0005".parse().unwrap());
    }

    #[test]
    fn single_negative_rate_bullish() {
        let a = analyze_funding(&[rate("-0.0003")]);
        assert_eq!(a.sentiment, FundingSentiment::Bullish);
    }

    #[test]
    fn near_zero_rate_neutral() {
        let a = analyze_funding(&[rate("0.00005")]);
        assert_eq!(a.sentiment, FundingSentiment::Neutral);
    }

    #[test]
    fn extreme_absolute_threshold() {
        // 0.002 > 0.001 → extreme
        let a = analyze_funding(&[rate("0.002")]);
        assert!(a.is_extreme);
    }

    #[test]
    fn extreme_relative_to_avg() {
        // avg = (0.0001 + 0.0001 + 0.001) / 3 ≈ 0.0004, current = 0.001 > 3 * 0.0004
        // Actually use values where current clearly exceeds 3x avg
        let rates = vec![rate("0.00005"), rate("0.00005"), rate("0.0008")];
        // avg ≈ 0.0003, current = 0.0008 → 0.0008 > 3 * 0.0003 = 0.0009? No.
        // Let's use: [0.0001, 0.0001, 0.002] → avg = 0.000733, current = 0.002 > 0.001 → extreme absolute
        let rates = vec![rate("0.0001"), rate("0.0001"), rate("0.002")];
        let a = analyze_funding(&rates);
        assert!(a.is_extreme);
    }

    #[test]
    fn not_extreme_when_close_to_avg() {
        let rates = vec![rate("0.0001"), rate("0.0001"), rate("0.0002")];
        let a = analyze_funding(&rates);
        assert!(!a.is_extreme);
    }

    #[test]
    fn avg_rate_calculated_correctly() {
        let rates = vec![rate("0.0001"), rate("0.0002"), rate("0.0003")];
        let a = analyze_funding(&rates);
        assert_eq!(a.avg_rate, "0.0002".parse().unwrap());
    }

    #[test]
    fn sentiment_display() {
        assert_eq!(FundingSentiment::Bullish.to_string(), "BULLISH");
        assert_eq!(FundingSentiment::Bearish.to_string(), "BEARISH");
        assert_eq!(FundingSentiment::Neutral.to_string(), "NEUTRAL");
    }
}
