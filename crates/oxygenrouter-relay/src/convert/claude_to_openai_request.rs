//! Anthropic Messages request -> OpenAI chat completion request.
//!
//! The mirror image of `openai_to_claude`, and the missing half of the pair that
//! `openai_to_claude_response` completes. A client posting Anthropic shape to
//! `POST /v1/messages` may be routed to a channel that speaks OpenAI; that
//! upstream does not implement `/v1/messages`, so the request has to be both
//! translated here *and* addressed to `/v1/chat/completions`.
//!
//! Without this the relay forwarded an Anthropic body to an Anthropic *path* on
//! an OpenAI channel, and the reply was then fed to the OpenAI->Claude response
//! converter, which found no `choices` and answered with an empty content block.
//! Shape-only assertions pass on that, which is how it survived review.
//!
//! Rules mirror NewAPI's `ClaudeMessagesRequestToOpenAIChat`:
//! * `system` (string or block array) -> a leading `system` message.
//! * `stop_sequences` -> `stop` (a lone entry stays a scalar).
//! * `tools[].input_schema` -> `tools[].function.parameters`.
//! * `tool_use` blocks -> assistant `tool_calls`; `tool_result` blocks -> their
//!   own `tool` message, since OpenAI keeps those separate from user content.
//! * `image` blocks -> `image_url` data URLs.
//! * streaming asks for `stream_options.include_usage`, because an OpenAI
//!   upstream omits the terminal usage chunk otherwise and the request would
//!   bill zero.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::error::RelayError;

/// Convert an Anthropic Messages request body into an OpenAI chat request.
pub fn convert(body: &Value, upstream_model: &str, is_stream: bool) -> Result<Value, RelayError> {
    let obj = body
        .as_object()
        .ok_or_else(|| RelayError::InvalidRequest("request body must be a JSON object".into()))?;

    let mut out = Map::new();
    out.insert("model".into(), Value::String(upstream_model.to_string()));

    for key in ["temperature", "top_p", "top_k"] {
        if let Some(v) = obj.get(key) {
            out.insert(key.into(), v.clone());
        }
    }
    if let Some(v) = obj.get("max_tokens") {
        out.insert("max_tokens".into(), v.clone());
    }

    if is_stream {
        out.insert("stream".into(), Value::Bool(true));
        // The upstream only reports token usage on a stream when asked to; an
        // empty usage block means the request settles at zero.
        out.insert("stream_options".into(), json!({"include_usage": true}));
    }

    // A single stop sequence stays a scalar, matching what OpenAI clients send.
    if let Some(seqs) = obj.get("stop_sequences").and_then(|v| v.as_array()) {
        match seqs.len() {
            0 => {}
            1 => {
                out.insert("stop".into(), seqs[0].clone());
            }
            _ => {
                out.insert("stop".into(), Value::Array(seqs.clone()));
            }
        }
    }

    if let Some(tools) = obj.get("tools").and_then(|v| v.as_array()) {
        let converted: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.get("name").and_then(|v| v.as_str()).unwrap_or("tool"),
                        "description": t.get("description").cloned().unwrap_or(Value::Null),
                        "parameters": t.get("input_schema").cloned()
                            .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
                    }
                })
            })
            .collect();
        if !converted.is_empty() {
            out.insert("tools".into(), Value::Array(converted));
        }
    }
    if let Some(choice) = obj.get("tool_choice") {
        if let Some(v) = map_tool_choice(choice) {
            out.insert("tool_choice".into(), v);
        }
    }

    let mut messages: Vec<Value> = Vec::new();

    if let Some(system) = obj.get("system") {
        let text = blocks_to_text(system);
        if !text.is_empty() {
            messages.push(json!({"role": "system", "content": text}));
        }
    }

    // `tool_result` blocks carry only a `tool_use_id`; OpenAI's `tool` message
    // optionally names the function, so ids are resolved against the `tool_use`
    // blocks seen earlier in the same conversation.
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let claude_messages = obj
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for message in &claude_messages {
        let role = message
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("user");
        let Some(content) = message.get("content") else {
            continue;
        };

        if let Some(text) = content.as_str() {
            messages.push(json!({"role": role, "content": text}));
            continue;
        }

        let Some(blocks) = content.as_array() else {
            continue;
        };

        let mut text_parts: Vec<String> = Vec::new();
        let mut media: Vec<Value> = Vec::new();
        let mut tool_calls: Vec<Value> = Vec::new();

        for block in blocks {
            match block.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "text" | "input_text" => {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        if !t.is_empty() {
                            text_parts.push(t.to_string());
                        }
                    }
                }
                "image" => {
                    if let Some(url) = image_url(block) {
                        media.push(json!({"type": "image_url", "image_url": {"url": url}}));
                    }
                }
                "tool_use" => {
                    let id = block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("tool")
                        .to_string();
                    if !id.is_empty() {
                        tool_names.insert(id.clone(), name.clone());
                    }
                    let arguments = block
                        .get("input")
                        .and_then(|i| serde_json::to_string(i).ok())
                        .unwrap_or_else(|| "{}".to_string());
                    tool_calls.push(json!({
                        "id": id,
                        "type": "function",
                        "function": {"name": name, "arguments": arguments},
                    }));
                }
                "tool_result" => {
                    let id = block
                        .get("tool_use_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let (text, images) = tool_result_content(block.get("content"));
                    let mut tool_message = Map::new();
                    tool_message.insert("role".into(), Value::String("tool".into()));
                    tool_message.insert("tool_call_id".into(), Value::String(id.to_string()));
                    tool_message.insert("content".into(), Value::String(text));
                    if let Some(name) = tool_names.get(id) {
                        tool_message.insert("name".into(), Value::String(name.clone()));
                    }
                    messages.push(Value::Object(tool_message));
                    // A `tool` message may only carry text, so images from a tool
                    // result ride on the surrounding message instead of being
                    // stringified into base64 the upstream tokenizer would count.
                    media.extend(images);
                }
                _ => {}
            }
        }

        let mut converted = Map::new();
        converted.insert("role".into(), Value::String(role.to_string()));
        if !tool_calls.is_empty() {
            converted.insert("tool_calls".into(), Value::Array(tool_calls));
            converted.insert(
                "content".into(),
                if text_parts.is_empty() {
                    Value::Null
                } else {
                    Value::String(text_parts.join("\n"))
                },
            );
            messages.push(Value::Object(converted));
        } else if !media.is_empty() {
            let mut parts: Vec<Value> = Vec::new();
            if !text_parts.is_empty() {
                parts.push(json!({"type": "text", "text": text_parts.join("\n")}));
            }
            parts.extend(media);
            converted.insert("content".into(), Value::Array(parts));
            messages.push(Value::Object(converted));
        } else if !text_parts.is_empty() {
            converted.insert("content".into(), Value::String(text_parts.join("\n")));
            messages.push(Value::Object(converted));
        }
        // A message that carried nothing but tool results is already emitted
        // above; an entirely empty one contributes no turn.
    }

    out.insert("messages".into(), Value::Array(messages));
    Ok(Value::Object(out))
}

/// Flatten a Claude `system` field (string, or array of text blocks) to text.
fn blocks_to_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Build the OpenAI `image_url` value for a Claude image block.
///
/// A base64 source becomes a data URL; a URL source passes through unchanged.
fn image_url(block: &Value) -> Option<String> {
    let source = block.get("source")?;
    if let Some(url) = source.get("url").and_then(|v| v.as_str()) {
        if !url.is_empty() {
            return Some(url.to_string());
        }
    }
    let media_type = source
        .get("media_type")
        .and_then(|v| v.as_str())
        .unwrap_or("image/png");
    let data = source.get("data").and_then(|v| v.as_str())?;
    Some(format!("data:{};base64,{}", media_type, data))
}

/// Map a `tool_result` content payload onto a `tool` message string plus any
/// images that have to be carried elsewhere.
fn tool_result_content(content: Option<&Value>) -> (String, Vec<Value>) {
    let Some(content) = content else {
        return (String::new(), Vec::new());
    };
    if let Some(text) = content.as_str() {
        return (text.to_string(), Vec::new());
    }
    let Some(blocks) = content.as_array() else {
        return (String::new(), Vec::new());
    };

    let mut texts: Vec<String> = Vec::new();
    let mut media: Vec<Value> = Vec::new();
    for block in blocks {
        match block.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "text" | "input_text" => {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    if !t.is_empty() {
                        texts.push(t.to_string());
                    }
                }
            }
            "image" => {
                if let Some(url) = image_url(block) {
                    media.push(json!({"type": "image_url", "image_url": {"url": url}}));
                }
            }
            _ => {}
        }
    }

    match (texts.is_empty(), media.is_empty()) {
        // Nothing structured survived: keep the original JSON rather than
        // silently dropping whatever the block held.
        (true, true) => (serde_json::to_string(content).unwrap_or_default(), media),
        // Upstreams reject an empty tool message; the images travel alongside.
        (true, false) => ("[image]".to_string(), media),
        _ => (texts.join("\n"), media),
    }
}

/// Map a Claude `tool_choice` onto its OpenAI equivalent.
fn map_tool_choice(choice: &Value) -> Option<Value> {
    match choice.get("type").and_then(|v| v.as_str()) {
        Some("auto") => Some(json!("auto")),
        Some("any") => Some(json!("required")),
        Some("none") => Some(json!("none")),
        Some("tool") => {
            let name = choice
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("tool");
            Some(json!({"type": "function", "function": {"name": name}}))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_user_turn_becomes_a_plain_openai_message() {
        let body = json!({
            "model": "claude-3-5-sonnet",
            "max_tokens": 64,
            "messages": [{"role": "user", "content": "hi"}]
        });
        let out = convert(&body, "gpt-test", false).unwrap();
        assert_eq!(out["model"], "gpt-test");
        assert_eq!(out["max_tokens"], 64);
        assert_eq!(out["messages"][0]["role"], "user");
        assert_eq!(out["messages"][0]["content"], "hi");
        // A non-streaming request must not ask for stream options.
        assert!(out.get("stream_options").is_none());
    }

    #[test]
    fn streaming_asks_the_upstream_to_report_usage() {
        let body = json!({
            "max_tokens": 8,
            "messages": [{"role": "user", "content": "hi"}]
        });
        let out = convert(&body, "m", true).unwrap();
        assert_eq!(out["stream"], true);
        // Without this the terminal usage chunk is absent and the request bills
        // zero, which is the defect this guards.
        assert_eq!(out["stream_options"]["include_usage"], true);
    }

    #[test]
    fn system_is_hoisted_to_a_leading_message() {
        let body = json!({
            "max_tokens": 8,
            "system": [{"type": "text", "text": "be brief"}],
            "messages": [{"role": "user", "content": "hi"}]
        });
        let out = convert(&body, "m", false).unwrap();
        assert_eq!(out["messages"][0]["role"], "system");
        assert_eq!(out["messages"][0]["content"], "be brief");
        assert_eq!(out["messages"][1]["role"], "user");
    }

    #[test]
    fn a_lone_stop_sequence_stays_a_scalar() {
        let body = json!({
            "max_tokens": 8,
            "stop_sequences": ["STOP"],
            "messages": [{"role": "user", "content": "hi"}]
        });
        let out = convert(&body, "m", false).unwrap();
        assert_eq!(out["stop"], "STOP");
    }

    #[test]
    fn several_stop_sequences_become_an_array() {
        let body = json!({
            "max_tokens": 8,
            "stop_sequences": ["A", "B"],
            "messages": [{"role": "user", "content": "hi"}]
        });
        let out = convert(&body, "m", false).unwrap();
        assert_eq!(out["stop"], json!(["A", "B"]));
    }

    #[test]
    fn tool_use_and_tool_result_split_into_assistant_and_tool_turns() {
        let body = json!({
            "max_tokens": 64,
            "messages": [
                {"role": "user", "content": "weather?"},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "toolu_1", "name": "get_weather",
                     "input": {"city": "Paris"}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "15 degrees"}
                ]}
            ]
        });
        let out = convert(&body, "m", false).unwrap();
        let msgs = out["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);

        let assistant = &msgs[1];
        assert_eq!(assistant["role"], "assistant");
        assert_eq!(assistant["tool_calls"][0]["id"], "toolu_1");
        assert_eq!(assistant["tool_calls"][0]["function"]["name"], "get_weather");
        assert!(assistant["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap()
            .contains("\"city\":\"Paris\""));

        let tool = &msgs[2];
        assert_eq!(tool["role"], "tool");
        assert_eq!(tool["tool_call_id"], "toolu_1");
        assert_eq!(tool["content"], "15 degrees");
        // The name is recovered from the matching tool_use block.
        assert_eq!(tool["name"], "get_weather");
    }

    #[test]
    fn a_tool_result_image_is_not_stringified_into_the_tool_message() {
        let body = json!({
            "max_tokens": 64,
            "messages": [
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "toolu_s", "name": "shot", "input": {}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_s", "content": [
                        {"type": "image", "source": {
                            "type": "base64", "media_type": "image/png", "data": "AAAA"}}
                    ]}
                ]}
            ]
        });
        let out = convert(&body, "m", false).unwrap();
        let msgs = out["messages"].as_array().unwrap();
        // assistant tool_calls, then the tool message, then the user turn
        // carrying the image.
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0]["role"], "assistant");
        assert_eq!(msgs[1]["role"], "tool");
        assert_eq!(msgs[1]["content"], "[image]");
        assert!(!msgs[1]["content"].as_str().unwrap().contains("AAAA"));
        let last = msgs.last().unwrap();
        assert_eq!(last["role"], "user");
        assert_eq!(last["content"][0]["type"], "image_url");
        assert_eq!(
            last["content"][0]["image_url"]["url"],
            "data:image/png;base64,AAAA"
        );
    }

    #[test]
    fn claude_tools_become_openai_function_tools() {
        let body = json!({
            "max_tokens": 64,
            "tools": [{
                "name": "get_weather",
                "description": "Get weather by city",
                "input_schema": {"type": "object", "properties": {"city": {"type": "string"}}}
            }],
            "tool_choice": {"type": "any"},
            "messages": [{"role": "user", "content": "hi"}]
        });
        let out = convert(&body, "m", false).unwrap();
        assert_eq!(out["tools"][0]["type"], "function");
        assert_eq!(out["tools"][0]["function"]["name"], "get_weather");
        assert_eq!(out["tools"][0]["function"]["parameters"]["type"], "object");
        assert_eq!(out["tool_choice"], "required");
    }

    #[test]
    fn text_blocks_in_a_user_turn_join_into_one_string() {
        let body = json!({
            "max_tokens": 8,
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "first"},
                {"type": "text", "text": "second"}
            ]}]
        });
        let out = convert(&body, "m", false).unwrap();
        assert_eq!(out["messages"][0]["content"], "first\nsecond");
    }

    #[test]
    fn a_multimodal_user_turn_keeps_text_and_image() {
        let body = json!({
            "max_tokens": 8,
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "what is this"},
                {"type": "image", "source": {
                    "type": "base64", "media_type": "image/png", "data": "BBBB"}}
            ]}]
        });
        let out = convert(&body, "m", false).unwrap();
        let parts = out["messages"][0]["content"].as_array().unwrap();
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[0]["text"], "what is this");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,BBBB");
    }

    #[test]
    fn a_url_image_source_passes_through() {
        let body = json!({
            "max_tokens": 8,
            "messages": [{"role": "user", "content": [
                {"type": "image", "source": {"type": "url", "url": "https://x/y.png"}}
            ]}]
        });
        let out = convert(&body, "m", false).unwrap();
        assert_eq!(
            out["messages"][0]["content"][0]["image_url"]["url"],
            "https://x/y.png"
        );
    }
}
