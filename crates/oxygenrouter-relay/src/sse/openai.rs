//! OpenAI SSE helpers: usage extraction from a buffered stream.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use bytes::Bytes;

use crate::value::Usage;

/// Scan an OpenAI SSE payload for the final `usage` object. OpenAI includes a
/// usage block in the final chunk when `stream_options.include_usage` is set,
/// and some providers always include it.
pub fn extract_stream_usage(body: &[u8]) -> (Bytes, Usage) {
    let text = String::from_utf8_lossy(body);
    let mut usage = Usage::default();
    let mut found = false;

    for frame in crate::sse::claude::parse_sse_frames(&text) {
        let Some(data) = frame.data else { continue };
        let d = data.trim();
        if d.is_empty() || d == "[DONE]" {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(d) else {
            continue;
        };
        if let Some(u) = v.get("usage") {
            if let Some(p) = u.get("prompt_tokens").and_then(|x| x.as_i64()) {
                usage.prompt_tokens = p;
                found = true;
            }
            if let Some(c) = u.get("completion_tokens").and_then(|x| x.as_i64()) {
                usage.completion_tokens = c;
            }
            if let Some(t) = u.get("total_tokens").and_then(|x| x.as_i64()) {
                usage.total_tokens = t;
            }
            if let Some(pd) = u.get("prompt_tokens_details") {
                if let Some(c) = pd.get("cached_tokens").and_then(|x| x.as_i64()) {
                    usage.cached_tokens = c;
                }
            }
            if let Some(cd) = u.get("completion_tokens_details") {
                if let Some(r) = cd.get("reasoning_tokens").and_then(|x| x.as_i64()) {
                    usage.reasoning_tokens = r;
                }
            }
            usage.semantic = "openai".to_string();
        }
    }

    if !found {
        usage.semantic = "openai".to_string();
    }
    (body.to_vec().into(), usage)
}
