//! Ollama adapter.
//!
//! Ollama exposes an OpenAI-compatible `/v1/chat/completions` since v0.1.24,
//! but its native API is `/api/chat`. We target the OpenAI-compatible shim
//! (simplest, and what NewAPI effectively does) with model rewritten.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::adaptor::{AdaptedResponse, Adaptor, UpstreamResponse};
use crate::error::RelayError;
use crate::value::{RelayInfo, RelayValue};

use super::openai_compat;

pub struct OllamaAdaptor;

#[async_trait]
impl Adaptor for OllamaAdaptor {
    fn name(&self) -> &'static str {
        "ollama"
    }

    fn request_url(&self, info: &RelayInfo) -> Result<String, RelayError> {
        // Ollama's OpenAI-compatible shim, so a native Anthropic/Gemini client is
        // re-aimed at the chat route exactly as it is for any other OpenAI channel.
        Ok(openai_compat::request_url(info))
    }

    fn setup_headers(&self, headers: &mut HeaderMap, info: &RelayInfo) -> Result<(), RelayError> {
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        // Ollama ignores auth locally, but a reverse-proxied Ollama may require
        // it, so forward the credential when the operator supplied one.
        if !info.api_key.is_empty() {
            if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", info.api_key)) {
                headers.insert("Authorization", value);
            }
        }
        Ok(())
    }

    fn convert_request(
        &self,
        info: &RelayInfo,
        body: &serde_json::Value,
    ) -> Result<RelayValue, RelayError> {
        if let Some(translated) = openai_compat::convert_request(info, body)? {
            return Ok(translated);
        }
        Ok(openai_compat::rewrite_model(info, body))
    }

    fn convert_response(
        &self,
        info: &RelayInfo,
        resp: &UpstreamResponse,
    ) -> Result<AdaptedResponse, RelayError> {
        openai_compat::convert_response(info, resp)
    }

    fn model_list(&self) -> Vec<String> {
        vec!["llama3.1", "llama3.2", "qwen2.5", "mistral", "phi3", "gemma2"]
            .into_iter()
            .map(String::from)
            .collect()
    }
}
