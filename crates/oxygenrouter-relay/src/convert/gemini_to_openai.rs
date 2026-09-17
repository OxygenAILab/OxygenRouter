//! Gemini `generateContent` response -> OpenAI chat completion.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use serde_json::{json, Value};

use crate::error::RelayError;

pub fn map_finish_reason(reason: &str) -> &'static str {
    match reason {
        "STOP" => "stop",
        "MAX_TOKENS" => "length",
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" | "OTHER" => {
            "content_filter"
        }
        _ => "stop",
    }
}

pub fn convert_response(body: &[u8], model: &str) -> Result<Vec<u8>, RelayError> {
    let v: Value = serde_json::from_slice(body)?;
    Ok(serde_json::to_vec(&convert_value(&v, model))?)
}

pub fn convert_value(v: &Value, model: &str) -> Value {
    let candidates = v
        .get("candidates")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();

    let mut choices: Vec<Value> = Vec::new();
    for (i, cand) in candidates.iter().enumerate() {
        let parts = cand
            .get("content")
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default();

        let mut text = String::new();
        let mut reasoning = String::new();
        let mut tool_calls: Vec<Value> = Vec::new();
        let mut tool_index = 0usize;

        for p in &parts {
            if let Some(t) = p.get("text").and_then(|x| x.as_str()) {
                if p.get("thought").and_then(|x| x.as_bool()).unwrap_or(false) {
                    reasoning.push_str(t);
                } else {
                    text.push_str(t);
                }
            }
            if let Some(fc) = p.get("functionCall") {
                let args = fc.get("args").cloned().unwrap_or(json!({}));
                tool_calls.push(json!({
                    "index": tool_index,
                    "id": format!("call_{}", tool_index),
                    "type": "function",
                    "function": {
                        "name": fc.get("name").and_then(|x| x.as_str()).unwrap_or("tool"),
                        "arguments": serde_json::to_string(&args).unwrap_or_else(|_| "{}".into()),
                    }
                }));
                tool_index += 1;
            }
            if let Some(inline) = p.get("inlineData") {
                let mime = inline.get("mimeType").and_then(|x| x.as_str()).unwrap_or("image/png");
                let data = inline.get("data").and_then(|x| x.as_str()).unwrap_or("");
                text.push_str(&format!("![image](data:{};base64,{})", mime, data));
            }
        }

        let finish = cand
            .get("finishReason")
            .and_then(|x| x.as_str())
            .map(map_finish_reason)
            .or(if tool_calls.is_empty() { None } else { Some("tool_calls") })
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

        choices.push(json!({
            "index": i,
            "message": message,
            "finish_reason": finish,
        }));
    }

    let usage = usage_from_metadata(v.get("usageMetadata"));
    json!({
        "id": format!("chatcmpl-gemini-{}", chrono::Utc::now().timestamp_millis()),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": model,
        "choices": choices,
        "usage": usage,
    })
}

pub fn usage_from_metadata(md: Option<&Value>) -> Value {
    let md = md.cloned().unwrap_or_default();
    let prompt = md.get("promptTokenCount").and_then(|x| x.as_i64()).unwrap_or(0)
        + md.get("toolUsePromptTokenCount").and_then(|x| x.as_i64()).unwrap_or(0);
    let completion = md.get("candidatesTokenCount").and_then(|x| x.as_i64()).unwrap_or(0);
    let thoughts = md.get("thoughtsTokenCount").and_then(|x| x.as_i64()).unwrap_or(0);
    json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion + thoughts,
        "total_tokens": md.get("totalTokenCount").and_then(|x| x.as_i64()).unwrap_or(prompt + completion + thoughts),
        "prompt_tokens_details": {
            "cached_tokens": md.get("cachedContentTokenCount").and_then(|x| x.as_i64()).unwrap_or(0),
        },
        "completion_tokens_details": {
            "reasoning_tokens": thoughts,
        }
    })
}
