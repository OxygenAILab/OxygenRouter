//! OpenAI chat completion -> Anthropic Messages response.
//!
//! The inverse of `claude_to_openai`, used when an upstream speaks Anthropic and
//! the *client* speaks Anthropic too (`POST /v1/messages`). Without this the
//! client would receive an `object: "chat.completion"` body and reject it.
//!
//! The target shape is pinned by observation, not assumption. A live reference
//! instance answers `/v1/messages` with:
//!
//! ```json
//! {
//!   "id": "...", "type": "message", "role": "assistant",
//!   "content": [{"type": "thinking", "thinking": "..."}],
//!   "stop_reason": "max_tokens", "model": "...",
//!   "usage": {"input_tokens": 18, "cache_creation_input_tokens": 0,
//!             "cache_read_input_tokens": 0, "output_tokens": 30,
//!             "claude_cache_creation_5_m_tokens": 0,
//!             "claude_cache_creation_1_h_tokens": 0}
//! }
//! ```
//!
//! Note that reasoning is carried as a `thinking` content block, which is why
//! `reasoning_content` maps there rather than being dropped.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use serde_json::{json, Map, Value};

use crate::error::RelayError;

/// Map an OpenAI `finish_reason` to an Anthropic `stop_reason`.
pub fn map_finish_reason(reason: &str) -> &'static str {
    match reason {
        "stop" => "end_turn",
        "length" => "max_tokens",
        "tool_calls" | "function_call" => "tool_use",
        "content_filter" => "refusal",
        // Anthropic has no direct equivalent; `end_turn` is the safe terminal
        // stop a client can always handle.
        _ => "end_turn",
    }
}

/// Map an Anthropic `stop_reason` back to an OpenAI `finish_reason`.
pub fn map_stop_reason_to_openai(reason: &str) -> &'static str {
    match reason {
        "end_turn" | "stop_sequence" => "stop",
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        "refusal" => "content_filter",
        _ => "stop",
    }
}

/// Convert a complete OpenAI chat response body.
pub fn convert_response(body: &[u8], fallback_model: &str) -> Result<Vec<u8>, RelayError> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|e| RelayError::Conversion(format!("openai response is not JSON: {e}")))?;
    Ok(serde_json::to_vec(&convert_value(&value, fallback_model))?)
}

/// Convert an already-parsed OpenAI response value.
pub fn convert_value(value: &Value, fallback_model: &str) -> Value {
    let choice = value
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first());

    let message = choice.and_then(|c| c.get("message"));
    let model = value
        .get("model")
        .and_then(|m| m.as_str())
        .filter(|m| !m.is_empty())
        .unwrap_or(fallback_model);

    let mut content: Vec<Value> = Vec::new();

    // Reasoning first: Anthropic clients render `thinking` before the answer, and
    // the live reference emits it as a leading block.
    if let Some(thinking) = message
        .and_then(|m| m.get("reasoning_content"))
        .and_then(|v| v.as_str())
        .filter(|t| !t.is_empty())
    {
        content.push(json!({"type": "thinking", "thinking": thinking}));
    }

    if let Some(text) = message.and_then(|m| m.get("content")) {
        match text {
            // A plain string is the common case.
            Value::String(s) if !s.is_empty() => {
                content.push(json!({"type": "text", "text": s}));
            }
            // A multimodal array: keep text parts, carry images through.
            Value::Array(parts) => {
                for part in parts {
                    match part {
                        Value::String(s) if !s.is_empty() => {
                            content.push(json!({"type": "text", "text": s}));
                        }
                        Value::Object(obj) => {
                            if let Some(t) = obj.get("text").and_then(|v| v.as_str()) {
                                if !t.is_empty() {
                                    content.push(json!({"type": "text", "text": t}));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    // Tool calls become `tool_use` blocks; the arguments string is parsed so the
    // block carries an object, which is what Anthropic defines.
    if let Some(calls) = message
        .and_then(|m| m.get("tool_calls"))
        .and_then(|v| v.as_array())
    {
        for call in calls {
            let function = call.get("function");
            let name = function
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let arguments = function
                .and_then(|f| f.get("arguments"))
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
            let input = serde_json::from_str::<Value>(arguments).unwrap_or_else(|_| {
                // A malformed argument string must not abort the response: keep
                // the raw text so the client can still see what the model sent.
                json!({"_raw": arguments})
            });
            content.push(json!({
                "type": "tool_use",
                "id": call.get("id").and_then(|v| v.as_str()).unwrap_or_default(),
                "name": name,
                "input": input,
            }));
        }
    }

    // An assistant turn always carries at least one block, even when empty, so a
    // client that indexes content[0] does not fail.
    if content.is_empty() {
        content.push(json!({"type": "text", "text": ""}));
    }

    let stop_reason = choice
        .and_then(|c| c.get("finish_reason"))
        .and_then(|v| v.as_str())
        .map(map_finish_reason)
        .unwrap_or("end_turn");

    let usage = convert_usage(value.get("usage"));

    json!({
        "id": value.get("id").and_then(|v| v.as_str()).unwrap_or(""),
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": content,
        "stop_reason": stop_reason,
        "stop_sequence": Value::Null,
        "usage": usage,
    })
}

/// Convert an OpenAI usage object into Anthropic's shape.
///
/// OpenAI reports cached tokens as a subset of the prompt total; Anthropic
/// reports the categories separately. So `input_tokens` is the uncached
/// remainder, which is what keeps a client's own accounting consistent.
pub fn convert_usage(usage: Option<&Value>) -> Value {
    let cached = usage
        .and_then(|u| u.get("prompt_tokens_details"))
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let prompt = usage
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let completion = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let uncached = (prompt - cached).max(0);
    json!({
        "input_tokens": uncached,
        "output_tokens": completion,
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": cached,
    })
}

/// Build the `message_start` payload for a streamed response.
pub fn stream_message_start(id: &str, model: &str, input_tokens: i64) -> Value {
    json!({
        "type": "message_start",
        "message": {
            "type": "message",
            "id": id,
            "role": "assistant",
            "model": model,
            "content": [],
            "stop_reason": Value::Null,
            "stop_sequence": Value::Null,
            "usage": {
                "input_tokens": input_tokens,
                "output_tokens": 0,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
            },
        }
    })
}

/// Build a `content_block_start` event.
pub fn stream_block_start(index: usize, block: Value) -> Value {
    json!({"type": "content_block_start", "index": index, "content_block": block})
}

/// Build a text `content_block_delta`.
pub fn stream_text_delta(index: usize, text: &str) -> Value {
    json!({
        "type": "content_block_delta",
        "index": index,
        "delta": {"type": "text_delta", "text": text}
    })
}

/// Build a `content_block_stop` event.
pub fn stream_block_stop(index: usize) -> Value {
    json!({"type": "content_block_stop", "index": index})
}

/// Build the terminal `message_delta` carrying the stop reason and usage.
pub fn stream_message_delta(stop_reason: &str, output_tokens: i64) -> Value {
    json!({
        "type": "message_delta",
        "delta": {"stop_reason": stop_reason, "stop_sequence": Value::Null},
        "usage": {"output_tokens": output_tokens}
    })
}

/// Build the `message_stop` event.
pub fn stream_message_stop() -> Value {
    json!({"type": "message_stop"})
}

/// Serialize an event as one SSE frame.
pub fn sse_frame(event: &Value) -> String {
    format!(
        "event: {}\ndata: {}\n\n",
        event.get("type").and_then(|v| v.as_str()).unwrap_or("message"),
        serde_json::to_string(event).unwrap_or_default()
    )
}

/// Extract the `model` field from a parsed body, for origin tracking.
pub fn body_model(value: &Value) -> Option<&str> {
    value
        .get("model")
        .and_then(|v| v.as_str())
        .filter(|m| !m.is_empty())
}

/// Helper for callers that need to read a string field defensively.
pub fn string_field<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(|v| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(v: Value) -> Value {
        convert_value(&v, "fallback-model")
    }

    #[test]
    fn a_plain_text_answer_becomes_one_text_block() {
        let out = convert(json!({
            "id": "chatcmpl-1",
            "model": "gpt-4o",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hello"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12}
        }));
        assert_eq!(out["type"], "message");
        assert_eq!(out["role"], "assistant");
        assert_eq!(out["model"], "gpt-4o");
        assert_eq!(out["content"][0]["type"], "text");
        assert_eq!(out["content"][0]["text"], "Hello");
        assert_eq!(out["stop_reason"], "end_turn");
        assert_eq!(out["usage"]["input_tokens"], 10);
        assert_eq!(out["usage"]["output_tokens"], 2);
    }

    #[test]
    fn reasoning_is_emitted_as_a_leading_thinking_block() {
        // The live reference emits `{"type":"thinking","thinking":"..."}`.
        let out = convert(json!({
            "choices": [{
                "message": {"role": "assistant", "content": "42", "reasoning_content": "Let me think."},
                "finish_reason": "stop"
            }]
        }));
        assert_eq!(out["content"][0]["type"], "thinking");
        assert_eq!(out["content"][0]["thinking"], "Let me think.");
        assert_eq!(out["content"][1]["type"], "text");
        assert_eq!(out["content"][1]["text"], "42");
    }

    #[test]
    fn tool_calls_become_tool_use_blocks_with_parsed_input() {
        let out = convert(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": "{\"city\":\"Paris\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        }));
        assert_eq!(out["content"][0]["type"], "tool_use");
        assert_eq!(out["content"][0]["id"], "call_1");
        assert_eq!(out["content"][0]["name"], "get_weather");
        // `input` must be an object, not the raw JSON string.
        assert_eq!(out["content"][0]["input"]["city"], "Paris");
        assert_eq!(out["stop_reason"], "tool_use");
    }

    #[test]
    fn malformed_tool_arguments_do_not_abort_the_response() {
        let out = convert(json!({
            "choices": [{
                "message": {"role": "assistant", "tool_calls": [
                    {"id": "c", "function": {"name": "f", "arguments": "{not json"}}
                ]},
                "finish_reason": "tool_calls"
            }]
        }));
        assert_eq!(out["content"][0]["type"], "tool_use");
        // The raw text is preserved rather than the whole conversion failing.
        assert_eq!(out["content"][0]["input"]["_raw"], "{not json");
    }

    #[test]
    fn an_empty_answer_still_has_one_block() {
        // A client that indexes content[0] must not fail.
        let out = convert(json!({
            "choices": [{"message": {"role": "assistant", "content": ""}, "finish_reason": "stop"}]
        }));
        assert!(out["content"].as_array().expect("array").len() >= 1);
        assert_eq!(out["content"][0]["type"], "text");
    }

    #[test]
    fn a_missing_model_falls_back() {
        let out = convert(json!({"choices": [{"message": {"content": "x"}}]}));
        assert_eq!(out["model"], "fallback-model");
    }

    #[test]
    fn finish_reasons_map_to_anthropic_stop_reasons() {
        assert_eq!(map_finish_reason("stop"), "end_turn");
        assert_eq!(map_finish_reason("length"), "max_tokens");
        assert_eq!(map_finish_reason("tool_calls"), "tool_use");
        assert_eq!(map_finish_reason("content_filter"), "refusal");
        // An unknown reason must still be a terminal stop a client can handle.
        assert_eq!(map_finish_reason("something_new"), "end_turn");
    }

    #[test]
    fn stop_reasons_round_trip_back_to_openai() {
        for openai in ["stop", "length", "tool_calls", "content_filter"] {
            let anthropic = map_finish_reason(openai);
            assert_eq!(map_stop_reason_to_openai(anthropic), openai, "{openai}");
        }
    }

    #[test]
    fn cached_tokens_move_out_of_the_input_total() {
        // OpenAI counts cached tokens inside prompt_tokens; Anthropic counts them
        // separately, so the input figure is the uncached remainder.
        let usage = convert_usage(Some(&json!({
            "prompt_tokens": 100,
            "completion_tokens": 5,
            "prompt_tokens_details": {"cached_tokens": 40}
        })));
        assert_eq!(usage["input_tokens"], 60);
        assert_eq!(usage["cache_read_input_tokens"], 40);
        assert_eq!(usage["output_tokens"], 5);
    }

    #[test]
    fn usage_tolerates_absent_fields() {
        let usage = convert_usage(None);
        assert_eq!(usage["input_tokens"], 0);
        assert_eq!(usage["output_tokens"], 0);
    }

    #[test]
    fn usage_never_reports_a_negative_input() {
        // Defensive: a provider that over-reports cached tokens must not produce
        // a negative input count.
        let usage = convert_usage(Some(&json!({
            "prompt_tokens": 10,
            "prompt_tokens_details": {"cached_tokens": 50}
        })));
        assert_eq!(usage["input_tokens"], 0);
    }

    #[test]
    fn the_full_body_shape_matches_the_observed_reference() {
        let out = convert(json!({
            "id": "chatcmpl-x", "model": "m",
            "choices": [{"message": {"role": "assistant", "content": "hi"}, "finish_reason": "length"}],
            "usage": {"prompt_tokens": 18, "completion_tokens": 30}
        }));
        // Every key the reference returned must be present.
        for key in ["id", "type", "role", "content", "stop_reason", "model", "usage"] {
            assert!(out.get(key).is_some(), "missing `{key}`");
        }
        assert_eq!(out["stop_reason"], "max_tokens");
    }

    #[test]
    fn stream_events_serialize_as_sse_frames() {
        let frame = sse_frame(&stream_text_delta(0, "Hello"));
        assert!(frame.starts_with("event: content_block_delta\n"));
        assert!(frame.contains("data: "));
        assert!(frame.ends_with("\n\n"));
        assert!(frame.contains("\"text\":\"Hello\""));
    }

    #[test]
    fn the_stream_sequence_has_the_expected_event_types() {
        // Mirrors the reference stream: start -> block start -> deltas ->
        // block stop -> message delta -> message stop.
        let events = vec![
            stream_message_start("id", "m", 4),
            stream_block_start(0, json!({"type": "text", "text": ""})),
            stream_text_delta(0, "Hello"),
            stream_text_delta(0, " world"),
            stream_block_stop(0),
            stream_message_delta("end_turn", 2),
            stream_message_stop(),
        ];
        let types: Vec<&str> = events
            .iter()
            .map(|e| e["type"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(
            types,
            vec![
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop",
            ]
        );
    }

    #[test]
    fn message_start_declares_the_model_and_role() {
        let start = stream_message_start("msg_1", "claude-x", 12);
        assert_eq!(start["message"]["role"], "assistant");
        assert_eq!(start["message"]["model"], "claude-x");
        assert_eq!(start["message"]["usage"]["input_tokens"], 12);
        assert_eq!(start["message"]["content"], json!([]));
    }

    #[test]
    fn message_delta_carries_the_stop_reason() {
        let delta = stream_message_delta("max_tokens", 30);
        assert_eq!(delta["delta"]["stop_reason"], "max_tokens");
        assert_eq!(delta["usage"]["output_tokens"], 30);
    }

    #[test]
    fn body_model_ignores_blanks() {
        assert_eq!(body_model(&json!({"model": "m"})), Some("m"));
        assert_eq!(body_model(&json!({"model": ""})), None);
        assert_eq!(body_model(&json!({})), None);
    }
}