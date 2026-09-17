//! Azure OpenAI adapter.
//!
//! Azure uses `api-key` header (not Bearer) and deployment-style URLs:
//! `{base}/openai/deployments/{deployment}/chat/completions?api-version=...`
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::adaptor::{AdaptedResponse, Adaptor, UpstreamResponse};
use crate::error::RelayError;
use crate::usage::extract_openai_usage;
use crate::value::{RelayInfo, RelayValue};

pub const DEFAULT_API_VERSION: &str = "2024-10-21";

pub struct AzureAdaptor;

#[async_trait]
impl Adaptor for AzureAdaptor {
    fn name(&self) -> &'static str {
        "azure"
    }

    fn request_url(&self, info: &RelayInfo) -> Result<String, RelayError> {
        // The caller supplies base_url including the deployment path; we only
        // guarantee the api-version convention here.
        Ok(info.base_url.clone())
    }

    fn setup_headers(&self, headers: &mut HeaderMap, _info: &RelayInfo) -> Result<(), RelayError> {
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        headers.insert(
            "api-key",
            HeaderValue::from_str("").map_err(|e| RelayError::Auth(e.to_string()))?,
        );
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
