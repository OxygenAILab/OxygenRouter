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
    /// Headers to send upstream.
    ///
    /// Deliberately narrow: the caller's credential must not leak and the channel
    /// supplies its own. Only the flags a provider needs are carried.
    pub headers: Vec<(String, String)>,
    /// The caller's own headers, never sent anywhere.
    ///
    /// Kept apart from `headers` because the two answer different questions.
    /// Policy that inspects the request — channel affinity reading a session id,
    /// for one — needs what the *client* sent, and reading the upstream header
    /// list for it is how the first version of that feature silently did nothing:
    /// it looked for `X-Session-Id` in a list that only ever holds five
    /// provider flags.
    pub client_headers: Vec<(String, String)>,
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
    /// Every channel attempted before the one that succeeded, in order.
    ///
    /// Mirrors NewAPI's `use_channel` trail so a failover is auditable rather
    /// than invisible: a request that silently retried three times looks
    /// identical to one that worked first time without this.
    pub failed_channels: Vec<String>,
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

    /// The request itself is unusable, and no other channel would help.
    ///
    /// Distinct from `Internal` because it is the caller's configuration rather
    /// than a fault here: a model-mapping chain that cycles is an operator's
    /// mistake, and answering it with a retry loop would turn a fixable typo into
    /// a hang.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
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
            ProxyError::InvalidRequest(_) => 400,
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
            // Retrying cannot help: every channel applies the same broken rule.
            ProxyError::InvalidRequest(_) => false,
        }
    }

    /// Decide if error is "context length exceeded" (special: trigger long-context fallback)
    pub fn is_context_exceeded(&self) -> bool {
        matches!(self, ProxyError::ContextExceeded(_))
    }
}
