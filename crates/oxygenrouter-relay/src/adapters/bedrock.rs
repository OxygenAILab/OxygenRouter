//! AWS Bedrock adapter.
//!
//! Bedrock uses `InvokeModel` / `InvokeModelWithResponseStream` with the
//! **Anthropic Messages body** (`anthropic_version: "bedrock-2023-05-31"`) for
//! Claude models, or the `messages-v1` body for Nova. Auth is SigV4.
//!
//! Two credential modes:
//! * API-key mode: `Authorization: Bearer <key>`, region in the URL.
//! * AKSK mode: static creds `ak|sk|region`, SigV4 signing.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::adaptor::{AdaptedResponse, Adaptor, UpstreamResponse};
use crate::error::RelayError;
use crate::sigv4::{self, AwsCredentials};
use crate::sse::claude::ClaudeToOpenAiStream;
use crate::usage::extract_claude_usage;
use crate::value::{RelayInfo, RelayValue};

pub const BEDROCK_ANTHROPIC_VERSION: &str = "bedrock-2023-05-31";

#[derive(Debug, Clone)]
pub enum BedrockCredential {
    /// Single bearer token plus region.
    ApiKey { token: String, region: String },
    /// Static access key / secret / region (SigV4).
    AkSk(AwsCredentials),
    Missing,
}

/// Parse a Bedrock channel's `api_key` field.
/// * 2 parts `key|region` -> API-key mode.
/// * 3 parts `ak|sk|region` -> AKSK mode.
pub fn parse_credential(raw: &str, default_region: &str) -> BedrockCredential {
    let parts: Vec<&str> = raw.split('|').map(|s| s.trim()).collect();
    match parts.as_slice() {
        [token, region] if !token.is_empty() => BedrockCredential::ApiKey {
            token: token.to_string(),
            region: region.to_string(),
        },
        [ak, sk, region] if !ak.is_empty() && !sk.is_empty() => {
            BedrockCredential::AkSk(AwsCredentials {
                access_key: ak.to_string(),
                secret_key: sk.to_string(),
                region: region.to_string(),
                service: "bedrock".to_string(),
            })
        }
        _ => {
            if raw.trim().is_empty() {
                BedrockCredential::Missing
            } else {
                BedrockCredential::ApiKey {
                    token: raw.trim().to_string(),
                    region: default_region.to_string(),
                }
            }
        }
    }
}

/// Map a friendly Claude model name to a Bedrock model id.
pub fn to_bedrock_model_id(model: &str, region: &str) -> String {
    if model.contains("anthropic.") {
        return model.to_string();
    }
    let base = if model.starts_with("claude") {
        format!("anthropic.{}", model)
    } else {
        model.to_string()
    };
    let with_version = if base.contains(":") {
        base
    } else {
        format!("{}:0", base)
            .replace("-v1:0", ":0")
    };
    // Cross-region inference prefix.
    let prefix = match region.split('-').next().unwrap_or("us") {
        "eu" => "eu",
        "ap" => "apac",
        _ => "us",
    };
    format!("{}.{}", prefix, with_version)
}

pub struct BedrockAdaptor;

#[async_trait]
impl Adaptor for BedrockAdaptor {
    fn name(&self) -> &'static str {
        "aws"
    }

    fn request_url(&self, info: &RelayInfo) -> Result<String, RelayError> {
        // `info.base_url` carries the endpoint; the concrete path is injected by
        // the caller (Bedrock paths are model-specific).
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
        // Bedrock Claude expects the Anthropic body plus anthropic_version.
        let claude = crate::convert::openai_to_claude::convert(
            body,
            &info.upstream_model,
            info.is_stream,
        )?;
        let mut obj = claude.as_object().cloned().unwrap_or_default();
        obj.insert(
            "anthropic_version".into(),
            serde_json::Value::String(BEDROCK_ANTHROPIC_VERSION.to_string()),
        );
        Ok(RelayValue::Raw(serde_json::Value::Object(obj)))
    }

    fn convert_response(
        &self,
        info: &RelayInfo,
        resp: &UpstreamResponse,
    ) -> Result<AdaptedResponse, RelayError> {
        // Bedrock's body is Anthropic-shaped, so reuse the Claude converters.
        if info.is_stream && resp.is_stream {
            let mut conv = ClaudeToOpenAiStream::new(&info.origin_model);
            let (body, usage) = conv.run(&resp.body);
            return Ok(AdaptedResponse {
                body: body.into(),
                usage,
            });
        }
        let usage = extract_claude_usage(&resp.body);
        let body =
            crate::convert::claude_to_openai::convert_response(&resp.body, &info.origin_model)?;
        Ok(AdaptedResponse {
            body: body.into(),
            usage,
        })
    }
}

/// Sign an outgoing Bedrock request using SigV4 (AKSK mode).
pub fn sign_bedrock_request(
    cred: &AwsCredentials,
    method: &str,
    url: &str,
    headers: &mut HeaderMap,
    body: &[u8],
) -> Result<(), RelayError> {
    sigv4::sign(cred, method, url, headers, body).map_err(|e| RelayError::Auth(e))
}
