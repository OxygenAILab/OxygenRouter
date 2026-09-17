//! OpenAI chat request -> Anthropic Messages request.
//!
//! Faithful port of NewAPI's `OpenAIChatRequestToClaudeMessages`. Key rules:
//! * `developer` -> `system`; `system` messages are hoisted to top-level `system`.
//! * consecutive same-role text messages are merged.
//! * first non-system message must be `user` (a placeholder is prepended).
//! * `tool` role -> a `user` message containing a `tool_result` block.
//! * assistant `tool_calls` -> `tool_use` content blocks.
//! * `max_tokens` is mandatory (default injected when absent).
//! * multimodal parts: text -> `{type:text}`; images -> base64 source blocks.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use serde_json::{json, Map, Value};

use crate::error::RelayError;

/// Default `max_tokens` when the client did not supply one (Anthropic requires it).
const DEFAULT_MAX_TOKENS: i64 = 8192;

pub fn convert(
    body: &Value,
    upstream_model: &str,
    is_stream: bool,
) -> Result<Value, RelayError> {
    let obj = body
        .as_object()
        .ok_or_else(|| RelayError::InvalidRequest("request body must be a JSON object".into()))?;

    let mut out = Map::new();
    out.insert("model".into(), Value::String(upstream_model.to_string()));

    // --- scalars -------------------------------------------------------------
    let max_tokens = obj
        .get("max_completion_tokens")
        .or_else(|| obj.get("max_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(DEFAULT_MAX_TOKENS);
    out.insert("max_tokens".into(), json!(max_tokens));

    if let Some(v) = obj.get("temperature") {
        out.insert("temperature".into(), v.clone());
    }
    if let Some(v) = obj.get("top_p") {
        out.insert("top_p".into(), v.clone());
    }
    if let Some(v) = obj.get("top_k") {
        out.insert("top_k".into(), v.clone());
    }
    if is_stream {
        out.insert("stream".into(), Value::Bool(true));
    }
    // stop -> stop_sequences
    if let Some(stop) = obj.get("stop") {
        let seqs = match stop {
            Value::String(s) => vec![Value::String(s.clone())],
            Value::Array(a) => a.clone(),
            _ => vec![],
        };
        if !seqs.is_empty() {
            out.insert("stop_sequences".into(), Value::Array(seqs));
        }
    }

    // --- tools ---------------------------------------------------------------
    if let Some(tools) = obj.get("tools").and_then(|v| v.as_array()) {
        let claude_tools: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                let f = t.get("function")?;
                Some(json!({
                    "name": f.get("name").and_then(|v| v.as_str()).unwrap_or("tool"),
                    "description": f.get("description").cloned().unwrap_or(Value::Null),
                    "input_schema": f.get("parameters").cloned()
                        .unwrap_or_else(|| json!({"type":"object","properties":{}})),
                }))
            })
            .collect();
        if !claude_tools.is_empty() {
            out.insert("tools".into(), Value::Array(claude_tools));
        }
    }
    if let Some(tc) = obj.get("tool_choice") {
        if let Some(v) = map_tool_choice(tc) {
            out.insert("tool_choice".into(), v);
        }
    }

    // --- messages ------------------------------------------------------------
    let messages = obj
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut system_blocks: Vec<Value> = Vec::new();
    // (role, blocks) with merging of consecutive same-role text.
    let mut claude_msgs: Vec<(String, Vec<Value>)> = Vec::new();

    for m in &messages {
        let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("user");
        let content = m.get("content");

        match role {
            "system" | "developer" => {
                if let Some(text) = content_as_text(content) {
                    system_blocks.push(json!({"type":"text","text":text}));
                }
                continue;
            }
            "tool" => {
                let tool_use_id = m
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let result_text = content_as_text(content).unwrap_or_default();
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": result_text,
                });
                push_user_block(&mut claude_msgs, block);
                continue;
            }
            "assistant" => {
                let mut blocks: Vec<Value> = Vec::new();
                if let Some(text) = content_as_text(content) {
                    if !text.is_empty() {
                        blocks.push(json!({"type":"text","text":text}));
                    }
                }
                if let Some(tcs) = m.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in tcs {
                        let f = tc.get("function").cloned().unwrap_or(Value::Null);
                        let args = f
                            .get("arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}");
                        let input: Value = serde_json::from_str(args).unwrap_or(json!({}));
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": tc.get("id").and_then(|v| v.as_str()).unwrap_or("toolu"),
                            "name": f.get("name").and_then(|v| v.as_str()).unwrap_or("tool"),
                            "input": input,
                        }));
                    }
                }
                if blocks.is_empty() {
                    blocks.push(json!({"type":"text","text":"..."}));
                }
                push_block(&mut claude_msgs, "assistant", blocks);
                continue;
            }
            _ => {
                // user (and unknown roles)
                let blocks = convert_user_content(content);
                push_block(&mut claude_msgs, "user", blocks);
            }
        }
    }

    if !system_blocks.is_empty() {
        out.insert("system".into(), Value::Array(system_blocks));
    }

    // Anthropic requires the first message to be `user`.
    if let Some((role, _)) = claude_msgs.first() {
        if role == "assistant" {
            claude_msgs.insert(
                0,
                (
                    "user".to_string(),
                    vec![json!({"type":"text","text":"..."})],
                ),
            );
        }
    }

    let msgs: Vec<Value> = claude_msgs
        .into_iter()
        .map(|(role, content)| json!({"role": role, "content": content}))
        .collect();
    out.insert("messages".into(), Value::Array(msgs));

    Ok(Value::Object(out))
}

/// Merge a block into the last message when the role matches; else push new.
fn push_block(msgs: &mut Vec<(String, Vec<Value>)>, role: &str, blocks: Vec<Value>) {
    if let Some((last_role, last_blocks)) = msgs.last_mut() {
        if last_role == role {
            last_blocks.extend(blocks);
            return;
        }
    }
    msgs.push((role.to_string(), blocks));
}

/// Append a block to a `user` message (merging with a trailing user message).
fn push_user_block(msgs: &mut Vec<(String, Vec<Value>)>, block: Value) {
    push_block(msgs, "user", vec![block]);
}

/// Convert an OpenAI user message's content (string or multimodal array).
fn convert_user_content(content: Option<&Value>) -> Vec<Value> {
    match content {
        Some(Value::String(s)) => vec![json!({"type":"text","text":s})],
        Some(Value::Array(parts)) => {
            let mut blocks = Vec::new();
            for p in parts {
                match p.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                            blocks.push(json!({"type":"text","text":t}));
                        }
                    }
                    Some("image_url") => {
                        if let Some(url) = p
                            .get("image_url")
                            .and_then(|v| v.get("url"))
                            .and_then(|v| v.as_str())
                        {
                            blocks.push(image_block(url));
                        }
                    }
                    _ => {}
                }
            }
            if blocks.is_empty() {
                blocks.push(json!({"type":"text","text":"..."}));
            }
            blocks
        }
        _ => vec![json!({"type":"text","text":"..."})],
    }
}

/// Build an Anthropic image block. Data URLs become base64 sources; remote URLs
/// become `url` sources (Anthropic accepts both since 2024).
fn image_block(url: &str) -> Value {
    if let Some(rest) = url.strip_prefix("data:") {
        // data:<media_type>;base64,<data>
        if let Some((meta, data)) = rest.split_once(',') {
            let media_type = meta.split(';').next().unwrap_or("image/png");
            return json!({
                "type": "image",
                "source": {"type": "base64", "media_type": media_type, "data": data}
            });
        }
    }
    json!({"type":"image","source":{"type":"url","url":url}})
}

fn content_as_text(content: Option<&Value>) -> Option<String> {
    match content {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(parts)) => {
            let mut buf = String::new();
            for p in parts {
                if p.get("type").and_then(|v| v.as_str()) == Some("text") {
                    if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                        buf.push_str(t);
                    }
                }
            }
            if buf.is_empty() {
                None
            } else {
                Some(buf)
            }
        }
        _ => None,
    }
}

fn map_tool_choice(tc: &Value) -> Option<Value> {
    match tc {
        Value::String(s) => match s.as_str() {
            "auto" => Some(json!({"type":"auto"})),
            "required" => Some(json!({"type":"any"})),
            "none" => None,
            _ => Some(json!({"type":"auto"})),
        },
        Value::Object(o) => {
            if o.get("type").and_then(|v| v.as_str()) == Some("function") {
                let name = o
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool");
                Some(json!({"type":"tool","name":name}))
            } else {
                Some(json!({"type":"auto"}))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_hoisted_and_roles_mapped() {
        let body = json!({
            "model": "claude-3-5-sonnet-20241022",
            "max_tokens": 100,
            "messages": [
                {"role": "system", "content": "be brief"},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"},
                {"role": "user", "content": "bye"}
            ]
        });
        let out = convert(&body, "claude-3-5-sonnet-20241022", false).unwrap();
        assert_eq!(out["system"][0]["text"], "be brief");
        assert_eq!(out["messages"][0]["role"], "user");
        assert_eq!(out["messages"].as_array().unwrap().len(), 3);
        assert_eq!(out["max_tokens"], 100);
    }

    #[test]
    fn first_message_assistant_gets_placeholder_user() {
        let body = json!({
            "max_tokens": 10,
            "messages": [
                {"role": "assistant", "content": "alone"}
            ]
        });
        let out = convert(&body, "m", false).unwrap();
        let msgs = out["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
    }

    #[test]
    fn tool_calls_become_tool_use_blocks() {
        let body = json!({
            "max_tokens": 10,
            "messages": [
                {"role": "user", "content": "weather?"},
                {"role": "assistant", "content": "", "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {
                        "name": "get_weather", "arguments": "{\"city\":\"SF\"}"
                    }}
                ]},
                {"role": "tool", "tool_call_id": "call_1", "content": "sunny"}
            ]
        });
        let out = convert(&body, "m", false).unwrap();
        let msgs = out["messages"].as_array().unwrap();
        // assistant message carries tool_use block
        let asst = &msgs[1];
        assert_eq!(asst["content"][0]["type"], "tool_use");
        assert_eq!(asst["content"][0]["name"], "get_weather");
        assert_eq!(asst["content"][0]["input"]["city"], "SF");
        // tool message becomes user tool_result block
        let tool_msg = &msgs[2];
        assert_eq!(tool_msg["role"], "user");
        assert_eq!(tool_msg["content"][0]["type"], "tool_result");
        assert_eq!(tool_msg["content"][0]["tool_use_id"], "call_1");
    }

    #[test]
    fn consecutive_same_role_merged() {
        let body = json!({
            "max_tokens": 10,
            "messages": [
                {"role": "user", "content": "a"},
                {"role": "user", "content": "b"}
            ]
        });
        let out = convert(&body, "m", false).unwrap();
        let msgs = out["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["content"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn multimodal_image_data_url_becomes_base64_source() {
        let body = json!({
            "max_tokens": 10,
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "what is this"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}}
                ]}
            ]
        });
        let out = convert(&body, "m", false).unwrap();
        let content = out["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["source"]["data"], "AAAA");
    }

    #[test]
    fn max_tokens_defaults_when_missing() {
        let body = json!({"messages": [{"role": "user", "content": "hi"}]});
        let out = convert(&body, "m", false).unwrap();
        assert_eq!(out["max_tokens"], DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn tool_choice_required_maps_to_any() {
        let body = json!({
            "max_tokens": 10,
            "tool_choice": "required",
            "messages": [{"role": "user", "content": "hi"}]
        });
        let out = convert(&body, "m", false).unwrap();
        assert_eq!(out["tool_choice"]["type"], "any");
    }
}
