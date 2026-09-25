//! Proxy request/result types and shared error type
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use bytes::Bytes;
use thiserror::Error;

pub use oxygenrouter_relay::Usage;

#[derive(Debug, Clone)]
pub struct ProxyRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub model: String,
    pub stream: bool,
}

#[derive(Debug, Clone)]
pub struct ProxyResult {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    pub model_used: String,
    pub channel_id: String,
    /// Token usage the upstream reported, for billing.
    ///
    /// Zero-valued when the upstream reported none; the caller decides whether
    /// to fall back to an estimate.
    pub usage: Usage,
    /// The adaptor that served the request, e.g. `anthropic`.
    pub adaptor: String,
}

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("no available channel")]
    NoChannel,

    #[error("context length exceeded: {0}")]
    ContextExceeded(String),

    #[error("upstream HTTP {status}: {body}")]
    Upstream { status: u16, body: String },

    #[error("network: {0}")]
    Network(String),

    #[error("internal: {0}")]
    Internal(String),
}

#[derive(Debug, Clone)]
pub struct ModelContextExceeded {
    pub model: String,
    pub message: String,
}

impl ProxyError {
    pub fn status(&self) -> u16 {
        match self {
            ProxyError::NoChannel => 503,
            ProxyError::ContextExceeded(_) => 413,
            ProxyError::Upstream { status, .. } => *status,
            ProxyError::Network(_) => 502,
            ProxyError::Internal(_) => 500,
        }
    }

    /// Decide if error is retryable (channel switch should happen)
    pub fn is_retryable(&self) -> bool {
        match self {
            ProxyError::NoChannel => false,
            ProxyError::ContextExceeded(_) => true,
            ProxyError::Upstream { status, .. } => {
                *status == 408
                    || *status == 425
                    || *status == 429
                    || *status == 500
                    || *status == 502
                    || *status == 503
                    || *status == 504
                    || *status == 401
                    || *status == 403
            }
            ProxyError::Network(_) => true,
            ProxyError::Internal(_) => false,
        }
    }

    /// Decide if error is "context length exceeded" (special: trigger long-context fallback)
    pub fn is_context_exceeded(&self) -> bool {
        matches!(self, ProxyError::ContextExceeded(_))
    }
}
