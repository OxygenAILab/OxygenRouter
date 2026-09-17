//! OpenAI adapter — the reference implementation and universal fallback.
//!
//! Every OpenAI-compatible provider (OpenRouter, DeepSeek, Moonshot, vLLM,
//! SGLang, Xinference, LiteLLM, …) routes through this adaptor. It performs the
//! minimum work: rewrite `model`, inject param overrides, forward, and extract
//! usage from the response (non-stream `usage` object; stream final chunk).
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::adaptor::{AdaptedResponse, Adaptor, UpstreamResponse};
use crate::error::RelayError;
use crate::usage::extract_openai_usage;
use crate::value::{RelayInfo, RelayValue, Usage};

pub struct OpenAiAdaptor;

impl OpenAiAdaptor {
    /// Append the request path to the base URL without duplicating a `/v1`
    /// segment that the base already ends with.
    pub fn join_url(base: &str, path: &str) -> String {
        let base = base.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        if base.ends_with("/v1") && path.starts_with("v1/") {
            format!("{}/{}", base, path.trim_start_matches("v1/"))
        } else {
            format!("{}/{}", base, path)
        }
    }
}

#[async_trait]
impl Adaptor for OpenAiAdaptor {
    fn name(&self) -> &'static str {
        "openai"
    }

    fn request_url(&self, info: &RelayInfo) -> Result<String, RelayError> {
        // The concrete path is carried on the request itself; the adapter only
        // owns the auth/URL convention, which for OpenAI is base + path.
        Ok(info.base_url.clone())
    }

    fn setup_headers(&self, headers: &mut HeaderMap, _info: &RelayInfo) -> Result<(), RelayError> {
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        Ok(())
    }

    fn convert_request(
        &self,
        info: &RelayInfo,
        body: &serde_json::Value,
    ) -> Result<RelayValue, RelayError> {
        let mut obj = body.as_object().cloned().unwrap_or_default();
        obj.insert(
            "model".to_string(),
            serde_json::Value::String(info.upstream_model.clone()),
        );
        Ok(RelayValue::Raw(serde_json::Value::Object(obj)))
    }

    fn convert_response(
        &self,
        info: &RelayInfo,
        resp: &UpstreamResponse,
    ) -> Result<AdaptedResponse, RelayError> {
        if info.is_stream && resp.is_stream {
            let (body, usage) = crate::sse::openai::extract_stream_usage(&resp.body);
            return Ok(AdaptedResponse { body, usage });
        }
        let usage = extract_openai_usage(&resp.body);
        Ok(AdaptedResponse {
            body: resp.body.clone(),
            usage,
        })
    }

    fn model_list(&self) -> Vec<String> {
        vec![
            "gpt-4o",
            "gpt-4o-mini",
            "gpt-4.1",
            "gpt-4.1-mini",
            "gpt-4.1-nano",
            "o1",
            "o1-mini",
            "o3-mini",
            "text-embedding-3-small",
            "text-embedding-3-large",
            "dall-e-3",
            "tts-1",
            "whisper-1",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }
}

/// Default usage when nothing else is available.
pub fn zero_usage() -> Usage {
    Usage::default()
}
