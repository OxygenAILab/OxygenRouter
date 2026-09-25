//! Behaviour shared by every OpenAI-compatible channel.
//!
//! `OpenAiAdaptor` is the reference implementation, but it is not the only
//! channel that speaks OpenAI: `Ollama`, `AdvancedCustom` and every
//! `ApiType::OpenAi` fallback (OpenRouter, DeepSeek, vLLM, SGLang, LiteLLM …) do
//! too. A client may still arrive speaking Anthropic or Gemini.
//!
//! Keeping that handling here rather than in each adaptor is what stops the
//! following defect from coming back on a channel that was not part of the
//! original fix: the client's own path was appended to the base URL, so a request
//! for `/v1/messages` landed on a route the OpenAI upstream does not serve — or
//! serves in a different dialect — and the reply was then converted as if it were
//! OpenAI, producing a well-formed but **empty** result.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use crate::adaptor::{AdaptedResponse, UpstreamResponse};
use crate::error::RelayError;
use crate::usage::extract_openai_usage;
use crate::value::{RelayFormat, RelayInfo, RelayValue};

use super::openai::OpenAiAdaptor;

/// The route every OpenAI-compatible upstream serves chat completions on.
pub const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";

/// Whether the client's dialect differs from the upstream's.
///
/// True for Anthropic (`/v1/messages`) and Gemini (`/v1beta/models/<m>:<verb>`),
/// neither of which an OpenAI-compatible upstream implements.
pub fn needs_translation(format: RelayFormat) -> bool {
    matches!(format, RelayFormat::Claude | RelayFormat::Gemini)
}

/// The OpenAI path this request must be re-aimed at, or `None` when the client's
/// own path is already correct for an OpenAI upstream.
pub fn dialect_target_path(format: RelayFormat) -> Option<&'static str> {
    needs_translation(format).then_some(CHAT_COMPLETIONS_PATH)
}

/// The URL an OpenAI-compatible adaptor should post to.
pub fn request_url(info: &RelayInfo) -> String {
    let path = dialect_target_path(info.relay_format).unwrap_or(&info.request_path);
    OpenAiAdaptor::join_url(&info.base_url, path)
}

/// Translate a native Anthropic/Gemini request into OpenAI chat shape.
///
/// Returns `None` when the body is already OpenAI-shaped, which the caller then
/// forwards with only the model rewritten.
pub fn convert_request(
    info: &RelayInfo,
    body: &serde_json::Value,
) -> Result<Option<RelayValue>, RelayError> {
    match info.relay_format {
        RelayFormat::Claude => Ok(Some(RelayValue::Raw(
            crate::convert::claude_to_openai_request::convert(
                body,
                &info.upstream_model,
                info.is_stream,
            )?,
        ))),
        RelayFormat::Gemini => Ok(Some(RelayValue::Raw(
            crate::convert::gemini_to_openai_request::convert(
                body,
                &info.upstream_model,
                info.is_stream,
            )?,
        ))),
        _ => Ok(None),
    }
}

/// Rewrite `model` in place and leave the rest of an OpenAI-shaped body alone.
pub fn rewrite_model(info: &RelayInfo, body: &serde_json::Value) -> RelayValue {
    let mut obj = body.as_object().cloned().unwrap_or_default();
    obj.insert(
        "model".to_string(),
        serde_json::Value::String(info.upstream_model.clone()),
    );
    RelayValue::Raw(serde_json::Value::Object(obj))
}

/// Shape an OpenAI-compatible upstream response after what the client asked for.
///
/// For an OpenAI client this is a pass-through; for a native-dialect client the
/// body (and, when streaming, the whole SSE frame stream) is translated.
pub fn convert_response(
    info: &RelayInfo,
    resp: &UpstreamResponse,
) -> Result<AdaptedResponse, RelayError> {
    if info.is_stream && resp.is_stream {
        let (body, usage) = crate::sse::openai::extract_stream_usage(&resp.body);
        match info.relay_format {
            RelayFormat::Claude => Ok(AdaptedResponse {
                body: crate::sse::openai::to_claude_events(&body).into(),
                usage,
            }),
            RelayFormat::Gemini => {
                // Re-derived rather than reused: the Gemini frame carries usage in
                // its own shape, so the OpenAI reading of it does not apply.
                let mut conv =
                    crate::convert::gemini_to_openai_request::OpenAiToGeminiStream::new();
                let (converted, gemini_usage) = conv.run(&body);
                Ok(AdaptedResponse {
                    body: converted,
                    usage: gemini_usage,
                })
            }
            _ => Ok(AdaptedResponse { body, usage }),
        }
    } else {
        let usage = extract_openai_usage(&resp.body);
        match info.relay_format {
            RelayFormat::Claude => Ok(AdaptedResponse {
                body: crate::convert::openai_to_claude_response::convert_response(
                    &resp.body,
                    &info.origin_model,
                )?
                .into(),
                usage,
            }),
            RelayFormat::Gemini => Ok(AdaptedResponse {
                body: crate::convert::gemini_to_openai_request::convert_response(
                    &resp.body,
                    &info.origin_model,
                )?
                .into(),
                usage,
            }),
            _ => Ok(AdaptedResponse {
                body: resp.body.clone(),
                usage,
            }),
        }
    }
}
