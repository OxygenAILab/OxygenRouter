//! AdvancedCustom adapter — operator-defined URL templates with variable
//! interpolation, covering vLLM/SGLang/self-hosted OpenAI-compatible servers
//! with nonstandard paths.
//!
//! Supported template variables (substituted into `base_url`):
//! * `{model}`   — upstream model name
//! * `{action}`  — endpoint action suffix (e.g. `chat/completions`)
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::adaptor::{AdaptedResponse, Adaptor, UpstreamResponse};
use crate::error::RelayError;
use crate::value::{RelayInfo, RelayValue};

use super::openai_compat;

pub struct AdvancedCustomAdaptor;

#[async_trait]
impl Adaptor for AdvancedCustomAdaptor {
    fn name(&self) -> &'static str {
        "advanced_custom"
    }

    fn request_url(&self, info: &RelayInfo) -> Result<String, RelayError> {
        // A native Anthropic/Gemini client must be sent to the chat-completions
        // action, the same re-aiming every OpenAI-compatible channel now does.
        let action = match openai_compat::dialect_target_path(info.relay_format) {
            Some(path) => path.trim_start_matches('/').to_string(),
            None => info.request_path.trim_start_matches('/').to_string(),
        };
        Ok(info
            .base_url
            .replace("{model}", &info.upstream_model)
            .replace("{action}", &action))
    }

    fn setup_headers(&self, headers: &mut HeaderMap, info: &RelayInfo) -> Result<(), RelayError> {
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        if !info.api_key.is_empty() {
            let value = HeaderValue::from_str(&format!("Bearer {}", info.api_key))
                .map_err(|e| RelayError::Auth(format!("authorization header: {}", e)))?;
            headers.insert("Authorization", value);
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
}
