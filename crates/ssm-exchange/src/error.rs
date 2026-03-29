use std::fmt;

/// Domain error type for exchange operations.
#[derive(Debug)]
pub enum ExchangeError {
    /// API returned a non-success HTTP status.
    ApiError { status: String, body: String },
    /// Exchange-specific error code.
    ExchangeApiError { code: i32, message: String },
    /// Data parsing failed.
    ParseError(String),
    /// Feature not yet implemented for this exchange.
    Unimplemented(String),
    /// Unknown exchange name.
    UnknownExchange(String),
    /// Network or request error.
    Network(String),
}

impl fmt::Display for ExchangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiError { status, body } => write!(f, "API returned {status}: {body}"),
            Self::ExchangeApiError { code, message } => {
                write!(f, "exchange error {code}: {message}")
            }
            Self::ParseError(msg) => write!(f, "parse error: {msg}"),
            Self::Unimplemented(msg) => write!(f, "not implemented: {msg}"),
            Self::UnknownExchange(name) => write!(f, "unknown exchange: {name}"),
            Self::Network(msg) => write!(f, "network error: {msg}"),
        }
    }
}

impl std::error::Error for ExchangeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_error_display() {
        let err = ExchangeError::ApiError {
            status: "404".into(),
            body: "not found".into(),
        };
        assert!(err.to_string().contains("404"));
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_exchange_api_error_display() {
        let err = ExchangeError::ExchangeApiError {
            code: -1121,
            message: "Invalid symbol".into(),
        };
        assert!(err.to_string().contains("-1121"));
    }

    #[test]
    fn test_parse_error_display() {
        let err = ExchangeError::ParseError("bad decimal".into());
        assert!(err.to_string().contains("bad decimal"));
    }

    #[test]
    fn test_unimplemented_display() {
        let err = ExchangeError::Unimplemented("list_pairs for Binance".into());
        assert!(err.to_string().contains("list_pairs"));
    }

    #[test]
    fn test_unknown_exchange_display() {
        let err = ExchangeError::UnknownExchange("kraken".into());
        assert!(err.to_string().contains("kraken"));
    }

    #[test]
    fn test_network_error_display() {
        let err = ExchangeError::Network("connection refused".into());
        assert!(err.to_string().contains("connection refused"));
    }

    #[test]
    fn test_into_anyhow() {
        let err = ExchangeError::UnknownExchange("test".into());
        let anyhow_err: anyhow::Error = err.into();
        assert!(anyhow_err.to_string().contains("test"));
    }
}
