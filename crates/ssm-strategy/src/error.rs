use std::fmt;

/// Domain error type for strategy operations.
#[derive(Debug)]
pub enum StrategyError {
    /// Not enough candle data for analysis.
    InsufficientData { required: usize, available: usize },
    /// Strategy analysis failed.
    AnalysisFailed(String),
    /// Model prediction failed.
    PredictionFailed(String),
}

impl fmt::Display for StrategyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientData {
                required,
                available,
            } => write!(
                f,
                "insufficient data: need {required} candles, have {available}"
            ),
            Self::AnalysisFailed(msg) => write!(f, "analysis failed: {msg}"),
            Self::PredictionFailed(msg) => write!(f, "prediction failed: {msg}"),
        }
    }
}

impl std::error::Error for StrategyError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insufficient_data_display() {
        let err = StrategyError::InsufficientData {
            required: 20,
            available: 5,
        };
        let msg = err.to_string();
        assert!(msg.contains("20"));
        assert!(msg.contains("5"));
    }

    #[test]
    fn test_analysis_failed_display() {
        let err = StrategyError::AnalysisFailed("CVD computation error".into());
        assert!(err.to_string().contains("CVD computation error"));
    }

    #[test]
    fn test_prediction_failed_display() {
        let err = StrategyError::PredictionFailed("model not loaded".into());
        assert!(err.to_string().contains("model not loaded"));
    }

    #[test]
    fn test_into_anyhow() {
        let err = StrategyError::AnalysisFailed("test".into());
        let anyhow_err: anyhow::Error = err.into();
        assert!(anyhow_err.to_string().contains("test"));
    }
}
