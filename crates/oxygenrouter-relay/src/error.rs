//! Error types for the relay layer.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RelayError {
    #[error("unsupported: {0}")]
    Unsupported(String),

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("conversion failed: {0}")]
    Conversion(String),

    #[error("upstream error {status}: {body}")]
    Upstream { status: u16, body: String },

    #[error("network: {0}")]
    Network(String),

    #[error("auth error: {0}")]
    Auth(String),

    #[error("internal: {0}")]
    Internal(String),
}

impl RelayError {
    pub fn is_retryable(&self) -> bool {
        match self {
            RelayError::Upstream { status, .. } => {
                crate::retry::is_retryable_status(*status)
            }
            RelayError::Network(_) => true,
            _ => false,
        }
    }

    pub fn status_code(&self) -> u16 {
        match self {
            RelayError::Upstream { status, .. } => *status,
            RelayError::InvalidRequest(_) => 400,
            RelayError::Auth(_) => 401,
            RelayError::Unsupported(_) => 400,
            _ => 500,
        }
    }
}

impl From<serde_json::Error> for RelayError {
    fn from(e: serde_json::Error) -> Self {
        RelayError::Conversion(format!("json: {}", e))
    }
}

impl From<reqwest::Error> for RelayError {
    fn from(e: reqwest::Error) -> Self {
        RelayError::Network(e.to_string())
    }
}
