//! OpenAI chat request -> Gemini `generateContent` request.
//!
//! * `system`/`developer` messages -> `systemInstruction`.
//! * `role: assistant` -> `role: model`.
//! * `tool` messages -> `functionResponse` parts.
//! * assistant `tool_calls` -> `functionCall` parts.
//! * SDK config: temperature/topP/maxOutputTokens/stopSequences.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use serde_json::{json, Map, Value};

use crate::error::RelayError;

pub fn convert(body: &Value, _is_stream: bool) -> Result<Value, RelayError> {
    let obj = body
        .as_object()
        .ok_or_else(|| RelayError::InvalidRequest("request body must be a JSON object".into()))?;

    let mut out = Map::new();
    let mut gen_config = Map::new();

    if let Some(v) = obj.get("temperature") {
        gen_config.insert("temperature".into(), v.clone());
    }
    if let Some(v) = obj.get("top_p") {
        gen_config.insert("topP".into(), v.clone());
    }
    if let Some(v) = obj
        .get("max_completion_tokens")
        .or_else(|| obj.get("max_tokens"))
    {
        gen_config.insert("maxOutputTokens".into(), v.clone());
    }
    if let Some(stop) = obj.get("stop") {
        let seqs = match stop {
            Value::String(s) => vec![Value::String(s.clone())],
            Value::Array(a) => a.clone(),
            _ => vec![],
        };
        if !seqs.is_empty() {
            gen_config.insert("stopSequences".into(), Value::Array(seqs));
        }
    }
    if !gen_config.is_empty() {
        out.insert("generationConfig".into(), Value::Object(gen_config));
    }

    // Structured output.
    if let Some(rf) = obj.get("response_format").and_then(|v| v.as_object()) {
        if rf.get("type").and_then(|v| v.as_str()) == Some("json_object") {
            let mut gc = out
                .get("generationConfig")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            gc.insert("responseMimeType".into(), json!("application/json"));
            out.insert("generationConfig".into(), Value::Object(gc));
        }
    }

    // Tools.
    if let Some(tools) = obj.get("tools").and_then(|v| v.as_array()) {
        let decls: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                let f = t.get("function")?;
                Some(json!({
                    "name": f.get("name").and_then(|v| v.as_str()).unwrap_or("tool"),
                    "description": f.get("description").cloned().unwrap_or(Value::Null),
                    "parameters": f.get("parameters").cloned().unwrap_or(json!({"type":"object"})),
                }))
            })
            .collect();
        if !decls.is_empty() {
            out.insert("tools".into(), json!([{"functionDeclarations": decls}]));
        }
    }

    // Messages.
    let messages = obj
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut system_parts: Vec<Value> = Vec::new();
    let mut contents: Vec<Value> = Vec::new();

    for m in &messages {
        let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("user");
        match role {
            "system" | "developer" => {
                if let Some(t) = content_as_text(m.get("content")) {
                    system_parts.push(json!({"text": t}));
                }
            }
            "tool" => {
                let name = m
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool");
                let text = content_as_text(m.get("content")).unwrap_or_default();
                let response = serde_json::from_str::<Value>(&text)
                    .unwrap_or_else(|_| json!({"result": text}));
                let part = json!({"functionResponse": {"name": name, "response": response}});
                append_part(&mut contents, "user", part);
            }
            "assistant" => {
                let mut parts: Vec<Value> = Vec::new();
                if let Some(t) = content_as_text(m.get("content")) {
                    if !t.is_empty() {
                        parts.push(json!({"text": t}));
                    }
                }
                if let Some(tcs) = m.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in tcs {
                        let f = tc.get("function").cloned().unwrap_or(Value::Null);
                        let args = f
                            .get("arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}");
                        let arg_val: Value = serde_json::from_str(args).unwrap_or(json!({}));
                        parts.push(json!({"functionCall": {
                            "name": f.get("name").and_then(|v| v.as_str()).unwrap_or("tool"),
                            "args": arg_val,
                        }}));
                    }
                }
                if parts.is_empty() {
                    parts.push(json!({"text": "..."}));
                }
                append_parts(&mut contents, "model", parts);
            }
            _ => {
                let parts = convert_user_content(m.get("content"));
                append_parts(&mut contents, "user", parts);
            }
        }
    }

    if !system_parts.is_empty() {
        out.insert("systemInstruction".into(), json!({"parts": system_parts}));
    }
    out.insert("contents".into(), Value::Array(contents));

    Ok(Value::Object(out))
}

fn append_parts(contents: &mut Vec<Value>, role: &str, parts: Vec<Value>) {
    if let Some(last) = contents.last_mut() {
        if last.get("role").and_then(|v| v.as_str()) == Some(role) {
            if let Some(arr) = last.get_mut("parts").and_then(|v| v.as_array_mut()) {
                arr.extend(parts);
                return;
            }
        }
    }
    contents.push(json!({"role": role, "parts": parts}));
}

fn append_part(contents: &mut Vec<Value>, role: &str, part: Value) {
    append_parts(contents, role, vec![part]);
}

fn convert_user_content(content: Option<&Value>) -> Vec<Value> {
    match content {
        Some(Value::String(s)) => vec![json!({"text": s})],
        Some(Value::Array(parts)) => {
            let mut out = Vec::new();
            for p in parts {
                match p.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                            out.push(json!({"text": t}));
                        }
                    }
                    Some("image_url") => {
                        if let Some(url) = p
                            .get("image_url")
                            .and_then(|v| v.get("url"))
                            .and_then(|v| v.as_str())
                        {
                            out.push(image_part(url));
                        }
                    }
                    _ => {}
                }
            }
            if out.is_empty() {
                out.push(json!({"text": "..."}));
            }
            out
        }
        _ => vec![json!({"text": "..."})],
    }
}

fn image_part(url: &str) -> Value {
    if let Some(rest) = url.strip_prefix("data:") {
        if let Some((meta, data)) = rest.split_once(',') {
            let mime = meta.split(';').next().unwrap_or("image/png");
            return json!({"inlineData": {"mimeType": mime, "data": data}});
        }
    }
    json!({"fileData": {"mimeType": "image/png", "fileUri": url}})
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
