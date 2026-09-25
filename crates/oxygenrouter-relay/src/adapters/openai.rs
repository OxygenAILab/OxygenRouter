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
use crate::convert::claude_to_openai_request;
use crate::error::RelayError;
use crate::usage::extract_openai_usage;
use crate::value::{RelayFormat, RelayInfo, RelayValue, Usage};

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
        // A Claude client posts to `/v1/messages`, which an OpenAI upstream does
        // not implement. Appending that path verbatim sent an Anthropic body to
        // whatever answered `/v1/messages` there, and the reply was then run
        // through the OpenAI->Claude converter, which found no `choices` and
        // returned an empty content block. The request must be aimed at the chat
        // completions route the translated body is shaped for.
        if info.relay_format == RelayFormat::Claude {
            return Ok(Self::join_url(&info.base_url, "/v1/chat/completions"));
        }
        // OpenAI convention is `{base}/{path}`, collapsing a duplicated `/v1`.
        Ok(Self::join_url(&info.base_url, &info.request_path))
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
        // Anthropic inbound has to be translated, not forwarded: the body and
        // the URL above are both OpenAI chat completions shape.
        if info.relay_format == RelayFormat::Claude {
            return Ok(RelayValue::Raw(claude_to_openai_request::convert(
                body,
                &info.upstream_model,
                info.is_stream,
            )?));
        }
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
            // A Claude client posting to /v1/messages must receive Anthropic SSE
            // even when the upstream speaks OpenAI.
            if info.relay_format == RelayFormat::Claude {
                let converted = crate::sse::openai::to_claude_events(&body);
                return Ok(AdaptedResponse {
                    body: converted.into(),
                    usage,
                });
            }
            return Ok(AdaptedResponse { body, usage });
        }
        let usage = extract_openai_usage(&resp.body);
        if info.relay_format == RelayFormat::Claude {
            let body = crate::convert::openai_to_claude_response::convert_response(
                &resp.body,
                &info.origin_model,
            )?;
            return Ok(AdaptedResponse {
                body: body.into(),
                usage,
            });
        }
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
