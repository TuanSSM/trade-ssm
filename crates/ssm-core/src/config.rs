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

// ---------------------------------------------------------------------------
// TOML-based application configuration with hot-reload support
// ---------------------------------------------------------------------------

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Top-level configuration for trade-ssm services.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub trading: TradingConfig,
    #[serde(default)]
    pub risk: RiskConfig,
    #[serde(default)]
    pub notifications: NotificationConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingConfig {
    #[serde(default = "default_symbol_str")]
    pub symbol: String,
    #[serde(default = "default_interval_str")]
    pub interval: String,
    #[serde(default = "default_execution_mode_str")]
    pub execution_mode: String,
    #[serde(default = "default_quantity")]
    pub quantity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    #[serde(default = "default_max_drawdown")]
    pub max_drawdown_pct: f64,
    #[serde(default = "default_max_positions")]
    pub max_open_positions: u32,
    #[serde(default = "default_position_size_pct")]
    pub position_size_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub min_profit_threshold: Option<String>,
    #[serde(default)]
    pub quiet_hours_start: Option<u32>,
    #[serde(default)]
    pub quiet_hours_end: Option<u32>,
    #[serde(default = "default_cooldown")]
    pub cooldown_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default)]
    pub format: String,
}

// Default value functions for serde
fn default_symbol_str() -> String {
    DEFAULT_SYMBOL.to_string()
}
fn default_interval_str() -> String {
    DEFAULT_INTERVAL.to_string()
}
fn default_execution_mode_str() -> String {
    DEFAULT_EXECUTION_MODE.to_string()
}
fn default_quantity() -> String {
    "0.001".to_string()
}
fn default_max_drawdown() -> f64 {
    10.0
}
fn default_max_positions() -> u32 {
    3
}
fn default_position_size_pct() -> f64 {
    2.0
}
fn default_cooldown() -> u64 {
    60
}
fn default_log_level() -> String {
    "info".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        toml::from_str("").expect("empty TOML must parse to defaults")
    }
}

impl Default for TradingConfig {
    fn default() -> Self {
        Self {
            symbol: default_symbol_str(),
            interval: default_interval_str(),
            execution_mode: default_execution_mode_str(),
            quantity: default_quantity(),
        }
    }
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_drawdown_pct: default_max_drawdown(),
            max_open_positions: default_max_positions(),
            position_size_pct: default_position_size_pct(),
        }
    }
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_profit_threshold: None,
            quiet_hours_start: None,
            quiet_hours_end: None,
            cooldown_secs: default_cooldown(),
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: String::new(),
        }
    }
}

impl AppConfig {
    /// Load config from a TOML file. Falls back to defaults for missing fields.
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading config from {}: {e}", path.display()))?;
        let config: Self =
            toml::from_str(&content).map_err(|e| anyhow::anyhow!("parsing config TOML: {e}"))?;
        Ok(config)
    }

    /// Load from the `CONFIG_FILE` env var path, or return defaults.
    pub fn from_env_or_default() -> Self {
        match std::env::var("CONFIG_FILE") {
            Ok(path) => match Self::from_file(Path::new(&path)) {
                Ok(config) => {
                    tracing::info!(%path, "loaded config from file");
                    config
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to load config file, using defaults");
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    /// Reload config from file (for hot-reload).
    /// Returns the new config or an error if the file can't be parsed.
    pub fn reload(path: &Path) -> anyhow::Result<Self> {
        Self::from_file(path)
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

    #[test]
    fn default_config_has_sane_values() {
        let config = AppConfig::default();
        assert_eq!(config.trading.symbol, "BTCUSDT");
        assert_eq!(config.trading.interval, "15m");
        assert_eq!(config.trading.execution_mode, "paper");
        assert_eq!(config.risk.max_open_positions, 3);
    }

    #[test]
    fn parses_partial_toml() {
        let toml_str = r#"
[trading]
symbol = "ETHUSDT"
interval = "1h"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.trading.symbol, "ETHUSDT");
        assert_eq!(config.trading.interval, "1h");
        // Defaults for unspecified fields
        assert_eq!(config.trading.execution_mode, "paper");
        assert_eq!(config.risk.max_open_positions, 3);
    }

    #[test]
    fn parses_full_toml() {
        let toml_str = r#"
[trading]
symbol = "SOLUSDT"
interval = "5m"
execution_mode = "live"
quantity = "0.1"

[risk]
max_drawdown_pct = 5.0
max_open_positions = 1
position_size_pct = 1.0

[notifications]
enabled = true
min_profit_threshold = "50"
quiet_hours_start = 22
quiet_hours_end = 6
cooldown_secs = 120

[logging]
level = "debug"
format = "json"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.trading.symbol, "SOLUSDT");
        assert_eq!(config.risk.max_drawdown_pct, 5.0);
        assert!(config.notifications.enabled);
        assert_eq!(config.notifications.quiet_hours_start, Some(22));
    }
}
