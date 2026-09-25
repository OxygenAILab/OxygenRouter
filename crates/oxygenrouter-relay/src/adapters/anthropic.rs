//! Anthropic (Claude) adapter.
//!
//! The most-used non-OpenAI provider, so the conversion is fully implemented:
//! * inbound OpenAI chat -> Anthropic `/v1/messages` request
//! * Anthropic response -> OpenAI chat response
//! * Anthropic SSE event stream -> OpenAI SSE chunk stream
//!
//! Auth is `x-api-key` + `anthropic-version` (NOT `Authorization: Bearer`).
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::adaptor::{AdaptedResponse, Adaptor, UpstreamResponse};
use crate::convert::openai_to_claude;
use crate::error::RelayError;
use crate::sse::claude::ClaudeToOpenAiStream;
use crate::usage::extract_claude_usage;
use crate::value::{RelayFormat, RelayInfo, RelayValue, Usage};

/// Default Anthropic API version header.
pub const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicAdaptor;

#[async_trait]
impl Adaptor for AnthropicAdaptor {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    fn request_url(&self, info: &RelayInfo) -> Result<String, RelayError> {
        let base = info.base_url.trim_end_matches('/');
        Ok(format!("{}/v1/messages", base))
    }

    fn setup_headers(&self, headers: &mut HeaderMap, info: &RelayInfo) -> Result<(), RelayError> {
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static(DEFAULT_ANTHROPIC_VERSION),
        );
        if !info.api_key.is_empty() {
            let value = HeaderValue::from_str(&info.api_key)
                .map_err(|e| RelayError::Auth(format!("x-api-key header: {}", e)))?;
            headers.insert("x-api-key", value);
        }
        Ok(())
    }

    fn convert_request(
        &self,
        info: &RelayInfo,
        body: &serde_json::Value,
    ) -> Result<RelayValue, RelayError> {
        // If the client already speaks Anthropic format, pass through with the
        // model rewritten.
        if let Some(obj) = body.as_object() {
            let looks_claude = obj.contains_key("max_tokens")
                && obj.contains_key("messages")
                && !obj.contains_key("response_format")
                && obj.get("messages").and_then(|m| m.as_array()).is_some();
            // Heuristic: Anthropic bodies always carry `max_tokens` and never a
            // top-level `model` named like an OpenAI model with `n`.
            if looks_claude && !obj.contains_key("n") && !obj.contains_key("logprobs") {
                // Still potentially OpenAI. Only short-circuit when it also has
                // an Anthropic-only field.
                if obj.contains_key("system")
                    || obj.contains_key("anthropic_version")
                    || obj.contains_key("metadata")
                {
                    let mut out = obj.clone();
                    out.insert(
                        "model".to_string(),
                        serde_json::Value::String(info.upstream_model.clone()),
                    );
                    return Ok(RelayValue::Raw(serde_json::Value::Object(out)));
                }
            }
        }
        let claude = openai_to_claude::convert(body, &info.upstream_model, info.is_stream)?;
        Ok(RelayValue::Raw(claude))
    }

    fn convert_response(
        &self,
        info: &RelayInfo,
        resp: &UpstreamResponse,
    ) -> Result<AdaptedResponse, RelayError> {
        // Shape the response after what the *client* asked for, not what the
        // provider speaks. A client posting to `/v1/messages` expects Anthropic
        // shape, so the upstream body passes through unchanged; a client posting
        // OpenAI shape gets the translation. Returning OpenAI shape to an
        // Anthropic client is the bug this branch exists to prevent.
        let client_speaks_claude = info.relay_format == RelayFormat::Claude;

        if info.is_stream && resp.is_stream {
            if client_speaks_claude {
                // Already Anthropic SSE: forward verbatim, but still read the
                // usage out for billing.
                let (body, usage) = crate::sse::claude::pass_through_stream(&resp.body);
                return Ok(AdaptedResponse { body, usage });
            }
            let mut conv = ClaudeToOpenAiStream::new(&info.origin_model);
            let (body, usage) = conv.run(&resp.body);
            return Ok(AdaptedResponse {
                body: body.into(),
                usage,
            });
        }

        let usage = extract_claude_usage(&resp.body);
        if client_speaks_claude {
            return Ok(AdaptedResponse {
                body: resp.body.clone(),
                usage,
            });
        }
        let body = crate::convert::claude_to_openai::convert_response(&resp.body, &info.origin_model)?;
        Ok(AdaptedResponse {
            body: body.into(),
            usage,
        })
    }

    fn model_list(&self) -> Vec<String> {
        vec![
            "claude-3-5-sonnet-20241022",
            "claude-3-5-haiku-20241022",
            "claude-3-opus-20240229",
            "claude-3-haiku-20240307",
            "claude-sonnet-4-20250514",
            "claude-opus-4-20250514",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }
}

/// Unused placeholder kept for symmetry with the Go adaptor's usage return.
pub fn zero_usage() -> Usage {
    Usage::default()
}
