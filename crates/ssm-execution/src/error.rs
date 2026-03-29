use std::fmt;

/// Domain error type for execution operations.
#[derive(Debug)]
pub enum ExecutionError {
    /// Cannot submit order for a Neutral action.
    NeutralAction,
    /// Live engine not configured but required.
    NoLiveEngine,
    /// Preflight check failed.
    PreflightFailed(String),
    /// Order submission failed.
    OrderFailed(String),
    /// HMAC signing error.
    SigningError(String),
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NeutralAction => write!(f, "cannot submit order for Neutral action"),
            Self::NoLiveEngine => write!(f, "live engine not configured"),
            Self::PreflightFailed(msg) => write!(f, "preflight failed: {msg}"),
            Self::OrderFailed(msg) => write!(f, "order failed: {msg}"),
            Self::SigningError(msg) => write!(f, "signing error: {msg}"),
        }
    }
}

impl std::error::Error for ExecutionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_neutral_action_display() {
        let err = ExecutionError::NeutralAction;
        assert!(err.to_string().contains("Neutral"));
    }

    #[test]
    fn test_no_live_engine_display() {
        let err = ExecutionError::NoLiveEngine;
        assert!(err.to_string().contains("live engine not configured"));
    }

    #[test]
    fn test_preflight_failed_display() {
        let err = ExecutionError::PreflightFailed("no balance".into());
        assert!(err.to_string().contains("no balance"));
    }

    #[test]
    fn test_order_failed_display() {
        let err = ExecutionError::OrderFailed("rejected".into());
        assert!(err.to_string().contains("rejected"));
    }

    #[test]
    fn test_signing_error_display() {
        let err = ExecutionError::SigningError("bad key".into());
        assert!(err.to_string().contains("bad key"));
    }

    #[test]
    fn test_into_anyhow() {
        let err = ExecutionError::NeutralAction;
        let anyhow_err: anyhow::Error = err.into();
        assert!(anyhow_err.to_string().contains("Neutral"));
    }
}
