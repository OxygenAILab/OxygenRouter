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
use crate::usage::extract_openai_usage;
use crate::value::{RelayInfo, RelayValue};

pub struct AdvancedCustomAdaptor;

#[async_trait]
impl Adaptor for AdvancedCustomAdaptor {
    fn name(&self) -> &'static str {
        "advanced_custom"
    }

    fn request_url(&self, info: &RelayInfo) -> Result<String, RelayError> {
        Ok(info
            .base_url
            .replace("{model}", &info.upstream_model))
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
}
