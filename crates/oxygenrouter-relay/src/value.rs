//! `RelayValue` — the format-polymorphic request/response envelope.
//!
//! Go's NewAPI passes `any` across the adaptor boundary and type-asserts.
//! Rust has no such ergonomics, so we use a tagged enum that every converter
//! can exhaustively match on. `Raw` is the escape hatch for unknown shapes and
//! for pass-through providers that speak OpenAI already.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use serde::{Deserialize, Serialize};

use crate::channel_type::{ApiType, ChannelType};

/// The wire format spoken by a client or an upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelayFormat {
    /// OpenAI `/v1/chat/completions` and friends.
    OpenAiChat,
    /// OpenAI `/v1/completions` (legacy).
    OpenAiText,
    /// OpenAI `/v1/embeddings`.
    OpenAiEmbedding,
    /// OpenAI `/v1/responses`.
    OpenAiResponses,
    /// Anthropic `/v1/messages`.
    Claude,
    /// Google `generateContent`.
    Gemini,
    /// Anything we do not model — forwarded verbatim.
    Raw,
}

impl RelayFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            RelayFormat::OpenAiChat => "openai_chat",
            RelayFormat::OpenAiText => "openai_text",
            RelayFormat::OpenAiEmbedding => "openai_embedding",
            RelayFormat::OpenAiResponses => "openai_responses",
            RelayFormat::Claude => "claude",
            RelayFormat::Gemini => "gemini",
            RelayFormat::Raw => "raw",
        }
    }
}

/// A request or response in one of the known wire formats, or raw JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RelayValue {
    /// Raw JSON — used for unknown formats and pure pass-through.
    Raw(serde_json::Value),
}

impl RelayValue {
    pub fn raw(v: serde_json::Value) -> Self {
        RelayValue::Raw(v)
    }

    pub fn as_raw(&self) -> &serde_json::Value {
        match self {
            RelayValue::Raw(v) => v,
        }
    }

    /// Consume the envelope and return the underlying JSON.
    pub fn into_raw(self) -> serde_json::Value {
        match self {
            RelayValue::Raw(v) => v,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut serde_json::Map<String, serde_json::Value>> {
        match self {
            RelayValue::Raw(serde_json::Value::Object(m)) => Some(m),
            _ => None,
        }
    }
}

impl From<serde_json::Value> for RelayValue {
    fn from(v: serde_json::Value) -> Self {
        RelayValue::Raw(v)
    }
}

/// Dialect-preserving token usage.
///
/// Billing needs the *exact* upstream numbers (Anthropic cache-creation 5m/1h
/// splits, Gemini thoughts tokens, per-modality detail), so we never flatten to
/// a single normalized struct at the boundary. `prompt`/`completion` are the
/// OpenAI-compatible totals; `source` records where the numbers came from.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    /// "openai" | "anthropic" | "gemini" | "estimate"
    #[serde(default)]
    pub semantic: String,

    #[serde(default)]
    pub cached_tokens: i64,
    #[serde(default)]
    pub cache_creation_tokens: i64,
    #[serde(default)]
    pub cache_creation_5m_tokens: i64,
    #[serde(default)]
    pub cache_creation_1h_tokens: i64,
    #[serde(default)]
    pub reasoning_tokens: i64,
    #[serde(default)]
    pub image_tokens: i64,
    #[serde(default)]
    pub audio_tokens: i64,
}

impl Usage {
    pub fn openai(prompt: i64, completion: i64) -> Self {
        Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
            semantic: "openai".to_string(),
            ..Default::default()
        }
    }
}

/// Per-request context passed to adaptors and converters.
#[derive(Debug, Clone)]
pub struct RelayInfo {
    pub channel_id: String,
    pub channel_type: ChannelType,
    pub api_type: ApiType,
    pub base_url: String,
    /// Credential selected for this attempt (one line of a multi-key channel).
    ///
    /// Populated by the caller before `setup_headers` runs; adaptors use it to
    /// emit their provider-specific auth. Never logged.
    pub api_key: String,
    /// Raw channel credential, for providers whose auth is not a single bearer
    /// token (`ak|sk|region` for Bedrock, `project|location|token` for Vertex).
    /// Equal to `api_key` when the channel has no compound credential.
    pub credential_raw: String,
    /// The client-facing path, e.g. `/v1/chat/completions`. Adaptors that own
    /// the whole URL (Gemini, Cohere) ignore it; adaptors that forward the path
    /// (OpenAI, Azure) append it to `base_url`.
    pub request_path: String,
    /// The model the client asked for (before mapping).
    pub origin_model: String,
    /// The model to send upstream (after mapping).
    pub upstream_model: String,
    /// Inbound wire format.
    pub relay_format: RelayFormat,
    pub is_stream: bool,
    pub group: String,
    pub user_agent: String,
}

impl Default for RelayInfo {
    fn default() -> Self {
        Self {
            channel_id: String::new(),
            channel_type: ChannelType::OpenAI,
            api_type: ApiType::OpenAi,
            base_url: String::new(),
            api_key: String::new(),
            credential_raw: String::new(),
            request_path: String::new(),
            origin_model: String::new(),
            upstream_model: String::new(),
            relay_format: RelayFormat::OpenAiChat,
            is_stream: false,
            group: "default".to_string(),
            user_agent: String::new(),
        }
    }
}
