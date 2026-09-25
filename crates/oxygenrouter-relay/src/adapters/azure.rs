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

/// The API version may be overridden per channel by appending it to the key as
/// `<key>|<api-version>`; otherwise `DEFAULT_API_VERSION` applies.
pub fn azure_api_version(info: &RelayInfo) -> String {
    info.credential_raw
        .split('|')
        .nth(1)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_API_VERSION)
        .to_string()
}

pub struct AzureAdaptor;

#[async_trait]
impl Adaptor for AzureAdaptor {
    fn name(&self) -> &'static str {
        "azure"
    }

    fn request_url(&self, info: &RelayInfo) -> Result<String, RelayError> {
        // Azure routes by deployment: the upstream model name IS the deployment
        // for the common single-deployment case. When `base_url` already carries
        // a `/openai/deployments/...` path (operator pinned a deployment), we
        // only append the leaf action; otherwise we build the full path.
        let base = info.base_url.trim_end_matches('/');
        let leaf = info
            .request_path
            .trim_start_matches('/')
            .trim_start_matches("v1/");
        let path = if base.contains("/openai/deployments/") {
            format!("{}/{}", base, leaf)
        } else {
            format!(
                "{}/openai/deployments/{}/{}",
                base, info.upstream_model, leaf
            )
        };
        let api_version = azure_api_version(info);
        let sep = if path.contains('?') { '&' } else { '?' };
        Ok(format!("{}{}api-version={}", path, sep, api_version))
    }

    fn setup_headers(&self, headers: &mut HeaderMap, info: &RelayInfo) -> Result<(), RelayError> {
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        if !info.api_key.is_empty() {
            let value = HeaderValue::from_str(&info.api_key)
                .map_err(|e| RelayError::Auth(format!("api-key header: {}", e)))?;
            headers.insert("api-key", value);
        }
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
