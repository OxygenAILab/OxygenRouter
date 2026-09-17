//! The `Adaptor` trait and its registry.
//!
//! An adaptor knows one upstream protocol: its URL shape, its auth header, how
//! to translate an inbound request into that protocol, and how to translate the
//! response (including SSE streams) back into the client's requested format.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::header::HeaderMap;

use crate::error::RelayError;
use crate::value::{RelayFormat, RelayInfo, RelayValue, Usage};

/// An upstream response, still in raw bytes plus status/headers.
pub struct UpstreamResponse {
    pub status: u16,
    pub headers: HeaderMap,
    /// Raw body (buffered). For streaming adaptors this is the full SSE payload.
    pub body: Bytes,
    /// True when the upstream body is an SSE stream.
    pub is_stream: bool,
}

impl UpstreamResponse {
    pub fn body_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }
}

/// The result of handling a response: the client-facing value plus billing usage.
pub struct AdaptedResponse {
    pub body: Bytes,
    pub usage: Usage,
}

/// One upstream protocol.
#[async_trait]
pub trait Adaptor: Send + Sync {
    /// Stable name, e.g. "openai", "anthropic".
    fn name(&self) -> &'static str;

    /// The URL to POST to for this request.
    fn request_url(&self, info: &RelayInfo) -> Result<String, RelayError>;

    /// Apply auth + provider headers to an outgoing request.
    fn setup_headers(&self, headers: &mut HeaderMap, info: &RelayInfo) -> Result<(), RelayError>;

    /// Translate the inbound request into this provider's wire format.
    ///
    /// `body` is the already-parsed inbound JSON. The returned value is what
    /// gets serialized and sent upstream (with `model` already rewritten to
    /// `info.upstream_model`).
    fn convert_request(
        &self,
        info: &RelayInfo,
        body: &serde_json::Value,
    ) -> Result<RelayValue, RelayError>;

    /// Translate the upstream response into the client's requested format.
    ///
    /// For streaming responses (`info.is_stream`), this rewrites the SSE frame
    /// stream from the provider format into `info.relay_format`. Returns the
    /// client-facing bytes and the extracted usage.
    fn convert_response(
        &self,
        info: &RelayInfo,
        resp: &UpstreamResponse,
    ) -> Result<AdaptedResponse, RelayError>;

    /// Models this adaptor knows about (used for `/v1/models` fallback).
    fn model_list(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Error emitted by an adaptor when it cannot handle a format at all.
pub fn unsupported(what: &str) -> RelayError {
    RelayError::Unsupported(what.to_string())
}

/// True when the adaptor can translate from `from` to its native format for the
/// given relay mode. Most adaptors support OpenAI-in; native-format-in support
/// is per-adaptor.
pub fn accepts_inbound(_adaptor: &str, _from: RelayFormat) -> bool {
    // Every adaptor accepts OpenAI-shaped inbound; native inbound is opt-in via
    // `convert_request` returning `unsupported`.
    true
}
