/// Shared configuration defaults and helpers for all trade-ssm services.
///
/// Each service can use these defaults and parsing utilities to avoid
/// duplicating env var handling across binaries.
/// Default trading symbol.
pub const DEFAULT_SYMBOL: &str = "BTCUSDT";

/// Default candlestick interval.
pub const DEFAULT_INTERVAL: &str = "15m";

/// Default CVD analysis window size.
pub const DEFAULT_CVD_WINDOW: usize = 15;

/// Default polling interval in seconds for the analyzer service.
pub const DEFAULT_CHECK_INTERVAL_SECS: u64 = 60;

/// Default number of days for historical data download.
pub const DEFAULT_DOWNLOAD_DAYS: u64 = 30;

/// Default data directory for downloaded candles.
pub const DEFAULT_DATADIR: &str = "user_data";

/// Default execution mode.
pub const DEFAULT_EXECUTION_MODE: &str = "paper";

/// Default maximum rolling candle buffer size for signal services.
pub const DEFAULT_MAX_CANDLES: usize = 200;

/// Read an environment variable with a default fallback.
pub fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Read an environment variable and parse it, falling back to a default.
pub fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Common service configuration shared across all trade-ssm binaries.
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub symbol: String,
    pub interval: String,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            symbol: DEFAULT_SYMBOL.to_string(),
            interval: DEFAULT_INTERVAL.to_string(),
        }
    }
}

impl ServiceConfig {
    /// Load from environment variables.
    pub fn from_env() -> Self {
        Self {
            symbol: env_or("SYMBOL", DEFAULT_SYMBOL),
            interval: env_or("INTERVAL", DEFAULT_INTERVAL),
        }
    }
}

/// Convert interval string to milliseconds.
pub fn interval_to_ms(interval: &str) -> i64 {
    match interval {
        "1m" => 60_000,
        "3m" => 180_000,
        "5m" => 300_000,
        "15m" => 900_000,
        "30m" => 1_800_000,
        "1h" => 3_600_000,
        "2h" => 7_200_000,
        "4h" => 14_400_000,
        "6h" => 21_600_000,
        "12h" => 43_200_000,
        "1d" => 86_400_000,
        "1w" => 604_800_000,
        _ => 900_000, // default 15m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        assert_eq!(DEFAULT_SYMBOL, "BTCUSDT");
        assert_eq!(DEFAULT_INTERVAL, "15m");
        assert_eq!(DEFAULT_CVD_WINDOW, 15);
        assert_eq!(DEFAULT_CHECK_INTERVAL_SECS, 60);
    }

    #[test]
    fn test_service_config_default() {
        let cfg = ServiceConfig::default();
        assert_eq!(cfg.symbol, "BTCUSDT");
        assert_eq!(cfg.interval, "15m");
    }

    #[test]
    fn test_interval_to_ms() {
        assert_eq!(interval_to_ms("1m"), 60_000);
        assert_eq!(interval_to_ms("15m"), 900_000);
        assert_eq!(interval_to_ms("1h"), 3_600_000);
        assert_eq!(interval_to_ms("4h"), 14_400_000);
        assert_eq!(interval_to_ms("1d"), 86_400_000);
        assert_eq!(interval_to_ms("unknown"), 900_000);
    }

    #[test]
    fn test_env_or_default() {
        // With no env var set, should return default
        let val = env_or("__TRADE_SSM_TEST_NONEXISTENT__", "fallback");
        assert_eq!(val, "fallback");
    }

    #[test]
    fn test_env_parse_default() {
        let val: u64 = env_parse("__TRADE_SSM_TEST_NONEXISTENT__", 42);
        assert_eq!(val, 42);
    }
}
