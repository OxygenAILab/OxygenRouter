//! Anthropic Messages response -> OpenAI chat completion response.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use serde_json::{json, Value};

use crate::error::RelayError;

/// Map an Anthropic `stop_reason` to an OpenAI `finish_reason`.
pub fn map_stop_reason(reason: &str) -> &'static str {
    match reason {
        "end_turn" | "stop_sequence" => "stop",
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        _ => "stop",
    }
}

/// Convert a non-streaming Anthropic response body.
pub fn convert_response(body: &[u8], model: &str) -> Result<Vec<u8>, RelayError> {
    let v: Value = serde_json::from_slice(body)?;
    let converted = convert_value(&v, model);
    Ok(serde_json::to_vec(&converted)?)
}

pub fn convert_value(v: &Value, model: &str) -> Value {
    let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("chatcmpl-claude");
    let content = v
        .get("content")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();

    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    let mut tool_index = 0usize;

    for block in &content {
        match block.get("type").and_then(|x| x.as_str()) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(|x| x.as_str()) {
                    text.push_str(t);
                }
            }
            Some("thinking") => {
                if let Some(t) = block.get("thinking").and_then(|x| x.as_str()) {
                    reasoning.push_str(t);
                }
            }
            Some("tool_use") => {
                let args = block.get("input").cloned().unwrap_or(json!({}));
                tool_calls.push(json!({
                    "index": tool_index,
                    "id": block.get("id").and_then(|x| x.as_str()).unwrap_or("toolu"),
                    "type": "function",
                    "function": {
                        "name": block.get("name").and_then(|x| x.as_str()).unwrap_or("tool"),
                        "arguments": serde_json::to_string(&args).unwrap_or_else(|_| "{}".into()),
                    }
                }));
                tool_index += 1;
            }
            _ => {}
        }
    }

    let finish = v
        .get("stop_reason")
        .and_then(|x| x.as_str())
        .map(map_stop_reason)
        .unwrap_or("stop");

    let mut message = json!({
        "role": "assistant",
        "content": if text.is_empty() { Value::Null } else { Value::String(text) },
    });
    if !reasoning.is_empty() {
        message["reasoning_content"] = Value::String(reasoning);
    }
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls);
    }

    let usage = v.get("usage").cloned().unwrap_or(json!({}));
    let input = usage.get("input_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
    let output = usage.get("output_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
    let cache_read = usage.get("cache_read_input_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
    let cache_creation = usage.get("cache_creation_input_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
    let prompt_total = input + cache_read + cache_creation;

    json!({
        "id": id,
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish,
        }],
        "usage": {
            "prompt_tokens": prompt_total,
            "completion_tokens": output,
            "total_tokens": prompt_total + output,
            "prompt_tokens_details": {
                "cached_tokens": cache_read,
                "cache_creation_tokens": cache_creation,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_response_conversion() {
        let resp = json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "thinking", "thinking": "let me think"},
                {"type": "text", "text": "The answer"},
                {"type": "tool_use", "id": "toolu_9", "name": "calc", "input": {"x": 1}}
            ],
            "stop_reason": "tool_use",
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_read_input_tokens": 20,
                "cache_creation_input_tokens": 10
            }
        });
        let out = convert_value(&resp, "claude-3-5-sonnet");
        assert_eq!(out["object"], "chat.completion");
        assert_eq!(out["model"], "claude-3-5-sonnet");
        let msg = &out["choices"][0]["message"];
        assert_eq!(msg["content"], "The answer");
        assert_eq!(msg["reasoning_content"], "let me think");
        assert_eq!(msg["tool_calls"][0]["id"], "toolu_9");
        assert_eq!(msg["tool_calls"][0]["function"]["name"], "calc");
        assert!(msg["tool_calls"][0]["function"]["arguments"].as_str().unwrap().contains("\"x\":1"));
        assert_eq!(out["choices"][0]["finish_reason"], "tool_calls");
        // prompt includes cache read + creation
        assert_eq!(out["usage"]["prompt_tokens"], 130);
        assert_eq!(out["usage"]["completion_tokens"], 50);
        assert_eq!(out["usage"]["prompt_tokens_details"]["cached_tokens"], 20);
    }

    #[test]
    fn stop_reasons_map() {
        assert_eq!(map_stop_reason("end_turn"), "stop");
        assert_eq!(map_stop_reason("stop_sequence"), "stop");
        assert_eq!(map_stop_reason("max_tokens"), "length");
        assert_eq!(map_stop_reason("tool_use"), "tool_calls");
    }
}
