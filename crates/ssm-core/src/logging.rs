/// Initialize structured logging for all trade-ssm services.
/// Set `LOG_FORMAT=json` for JSON output (suitable for log aggregation).
/// Defaults to human-readable format.
pub fn init_logging() {
    let env_filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    let json_mode = std::env::var("LOG_FORMAT")
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    if json_mode {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    }
}
