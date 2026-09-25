//! Gemini `generateContent` request -> OpenAI chat completion request.
//!
//! The inbound half of the Gemini dialect pair (`gemini_to_openai` covers the
//! response). A client that speaks Gemini posts to `/v1beta/models/<m>:<verb>`;
//! when the selected channel is OpenAI-compatible the body has to be translated
//! *and* the request aimed at `/v1/chat/completions`, because that upstream has no
//! `/v1beta` route at all. Forwarding Gemini shape there simply 404s.
//!
//! Rules mirror NewAPI's `GeminiGenerateContentRequestToOpenAIChat`:
//! * `contents[].role`: `model` -> `assistant`; everything else passes through.
//! * `systemInstruction` -> a leading `system` message.
//! * `thought: true` parts are reasoning, not answer text.
//! * `inlineData` / `fileData` -> `image_url` (a data URL or the raw URI).
//! * `functionCall` -> assistant `tool_calls`; `functionResponse` -> its own
//!   `tool` message, with the name resolved from the matching call.
//! * `generationConfig` -> `temperature` / `top_p` / `top_k` / `max_tokens` /
//!   `stop`.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::error::RelayError;
use crate::value::Usage;

/// Convert a Gemini `generateContent` request into an OpenAI chat request.
pub fn convert(body: &Value, upstream_model: &str, is_stream: bool) -> Result<Value, RelayError> {
    let obj = body
        .as_object()
        .ok_or_else(|| RelayError::InvalidRequest("request body must be a JSON object".into()))?;

    let mut out = Map::new();
    out.insert("model".into(), Value::String(upstream_model.to_string()));
    if is_stream {
        out.insert("stream".into(), Value::Bool(true));
        out.insert("stream_options".into(), json!({"include_usage": true}));
    }

    // --- generationConfig ----------------------------------------------------
    if let Some(cfg) = obj.get("generationConfig").and_then(|v| v.as_object()) {
        for (from, to) in [
            ("temperature", "temperature"),
            ("topP", "top_p"),
            ("topK", "top_k"),
        ] {
            if let Some(v) = cfg.get(from) {
                out.insert(to.into(), v.clone());
            }
        }
        if let Some(v) = cfg.get("maxOutputTokens") {
            out.insert("max_tokens".into(), v.clone());
        }
        if let Some(seqs) = cfg.get("stopSequences").and_then(|v| v.as_array()) {
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
    }

    // --- tools ---------------------------------------------------------------
    if let Some(decls) = obj
        .get("tools")
        .and_then(|v| v.as_array())
        .and_then(|tools| tools.first())
        .and_then(|t| t.get("functionDeclarations"))
        .and_then(|v| v.as_array())
    {
        let converted: Vec<Value> = decls
            .iter()
            .map(|d| {
                json!({
                    "type": "function",
                    "function": {
                        "name": d.get("name").and_then(|v| v.as_str()).unwrap_or("tool"),
                        "description": d.get("description").cloned().unwrap_or(Value::Null),
                        "parameters": d.get("parameters").cloned()
                            .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
                    }
                })
            })
            .collect();
        if !converted.is_empty() {
            out.insert("tools".into(), Value::Array(converted));
        }
    }
    if let Some(mode) = obj
        .get("toolConfig")
        .and_then(|v| v.get("functionCallingConfig"))
        .and_then(|v| v.get("mode"))
        .and_then(|v| v.as_str())
    {
        let choice = match mode {
            "NONE" => Some(json!("none")),
            "ANY" => Some(json!("required")),
            "AUTO" => Some(json!("auto")),
            _ => None,
        };
        if let Some(choice) = choice {
            out.insert("tool_choice".into(), choice);
        }
    }

    // --- messages ------------------------------------------------------------
    let mut messages: Vec<Value> = Vec::new();

    if let Some(system) = obj.get("systemInstruction") {
        let text = parts_text(system.get("parts"));
        if !text.is_empty() {
            messages.push(json!({"role": "system", "content": text}));
        }
    }

    // Gemini names the function on a `functionResponse`, but OpenAI's `tool`
    // message addresses the call by id, so ids are minted and remembered here.
    let mut call_ids: HashMap<String, String> = HashMap::new();
    let mut next_call = 0usize;

    let contents = obj
        .get("contents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for content in &contents {
        let role = match content.get("role").and_then(|v| v.as_str()) {
            Some("model") => "assistant",
            Some(other) => other,
            None => "user",
        };

        let Some(parts) = content.get("parts").and_then(|v| v.as_array()) else {
            continue;
        };

        let mut text_parts: Vec<String> = Vec::new();
        let mut reasoning_parts: Vec<String> = Vec::new();
        let mut media: Vec<Value> = Vec::new();
        let mut tool_calls: Vec<Value> = Vec::new();

        for part in parts {
            if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                if text.is_empty() {
                    continue;
                }
                if part.get("thought").and_then(|v| v.as_bool()).unwrap_or(false) {
                    reasoning_parts.push(text.to_string());
                } else {
                    text_parts.push(text.to_string());
                }
                continue;
            }

            if let Some(inline) = part.get("inlineData") {
                let mime = inline
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("image/png");
                let data = inline.get("data").and_then(|v| v.as_str()).unwrap_or("");
                media.push(json!({
                    "type": "image_url",
                    "image_url": {"url": format!("data:{};base64,{}", mime, data)}
                }));
                continue;
            }
            if let Some(file) = part.get("fileData") {
                if let Some(uri) = file.get("fileUri").and_then(|v| v.as_str()) {
                    media.push(json!({"type": "image_url", "image_url": {"url": uri}}));
                }
                continue;
            }

            if let Some(call) = part.get("functionCall") {
                let name = call
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool")
                    .to_string();
                let id = format!("call_{}", next_call);
                next_call += 1;
                call_ids.insert(name.clone(), id.clone());
                let arguments = call
                    .get("args")
                    .and_then(|a| serde_json::to_string(a).ok())
                    .unwrap_or_else(|| "{}".to_string());
                tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": {"name": name, "arguments": arguments},
                }));
                continue;
            }

            if let Some(response) = part.get("functionResponse") {
                let name = response
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let id = call_ids
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| format!("call_{}", next_call));
                let payload = response.get("response").cloned().unwrap_or(json!({}));
                let text = match payload {
                    Value::String(s) => s,
                    other => serde_json::to_string(&other).unwrap_or_default(),
                };
                let mut tool_message = Map::new();
                tool_message.insert("role".into(), Value::String("tool".into()));
                tool_message.insert("tool_call_id".into(), Value::String(id));
                tool_message.insert("content".into(), Value::String(text));
                if !name.is_empty() {
                    tool_message.insert("name".into(), Value::String(name));
                }
                messages.push(Value::Object(tool_message));
            }
        }

        // Reasoning and answer text share one OpenAI `reasoning_content` +
        // `content` pair, so they are flattened rather than split into blocks.
        if !tool_calls.is_empty() {
            let mut message = Map::new();
            message.insert("role".into(), Value::String(role.to_string()));
            message.insert("tool_calls".into(), Value::Array(tool_calls));
            message.insert(
                "content".into(),
                if text_parts.is_empty() {
                    Value::Null
                } else {
                    Value::String(text_parts.join("\n"))
                },
            );
            if !reasoning_parts.is_empty() {
                message.insert(
                    "reasoning_content".into(),
                    Value::String(reasoning_parts.join("\n")),
                );
            }
            messages.push(Value::Object(message));
        } else if !media.is_empty() {
            let mut parts_out: Vec<Value> = Vec::new();
            if !text_parts.is_empty() {
                parts_out.push(json!({"type": "text", "text": text_parts.join("\n")}));
            }
            parts_out.extend(media);
            let mut message = Map::new();
            message.insert("role".into(), Value::String(role.to_string()));
            message.insert("content".into(), Value::Array(parts_out));
            if !reasoning_parts.is_empty() {
                message.insert(
                    "reasoning_content".into(),
                    Value::String(reasoning_parts.join("\n")),
                );
            }
            messages.push(Value::Object(message));
        } else if !text_parts.is_empty() || !reasoning_parts.is_empty() {
            let mut message = Map::new();
            message.insert("role".into(), Value::String(role.to_string()));
            message.insert(
                "content".into(),
                if text_parts.is_empty() {
                    Value::Null
                } else {
                    Value::String(text_parts.join("\n"))
                },
            );
            if !reasoning_parts.is_empty() {
                message.insert(
                    "reasoning_content".into(),
                    Value::String(reasoning_parts.join("\n")),
                );
            }
            messages.push(Value::Object(message));
        }
    }

    out.insert("messages".into(), Value::Array(messages));
    Ok(Value::Object(out))
}

/// Join the `text` of a Gemini `parts` array.
fn parts_text(parts: Option<&Value>) -> String {
    parts
        .and_then(|p| p.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Map an OpenAI `finish_reason` onto a Gemini `finishReason`.
///
/// The inverse of [`crate::convert::gemini_to_openai::map_finish_reason`], with
/// one deliberate asymmetry the reference also has: `tool_calls` maps to `STOP`,
/// not to a tool-specific reason, because Gemini reports the call through the
/// parts and has no separate terminal reason for it.
pub fn map_openai_finish_reason(reason: &str) -> &'static str {
    match reason {
        "length" => "MAX_TOKENS",
        "content_filter" => "SAFETY",
        "RECITATION" => "RECITATION",
        // "stop", "tool_calls", and anything unknown.
        _ => "STOP",
    }
}

/// Convert an OpenAI chat completion body into a Gemini `generateContent` body.
///
/// Needed when a client speaks Gemini (`POST /v1beta/...`) but the selected
/// channel speaks OpenAI: the upstream answers OpenAI shape and the client will
/// only parse Gemini shape.
pub fn convert_response(body: &[u8], _model: &str) -> Result<Vec<u8>, RelayError> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|e| RelayError::Conversion(format!("openai response is not JSON: {e}")))?;
    Ok(serde_json::to_vec(&convert_response_value(&value))?)
}

/// Convert an already-parsed OpenAI response value.
pub fn convert_response_value(value: &Value) -> Value {
    let mut candidates: Vec<Value> = Vec::new();

    if let Some(choices) = value.get("choices").and_then(|c| c.as_array()) {
        for (i, choice) in choices.iter().enumerate() {
            let message = choice.get("message").cloned().unwrap_or(json!({}));
            let mut parts: Vec<Value> = Vec::new();

            // Reasoning is carried as a `thought` part, which is how Gemini
            // distinguishes it from answer text on the way back out.
            if let Some(thinking) = message
                .get("reasoning_content")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                parts.push(json!({"text": thinking, "thought": true}));
            }
            if let Some(text) = message
                .get("content")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                parts.push(json!({"text": text}));
            }
            if let Some(calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
                for call in calls {
                    let function = call.get("function").cloned().unwrap_or(json!({}));
                    let arguments = function
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .unwrap_or_else(|| json!({}));
                    let mut part = Map::new();
                    let mut call_value = Map::new();
                    // `id` is optional in the Gemini protocol.
                    if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                        if !id.is_empty() {
                            call_value.insert("id".into(), Value::String(id.to_string()));
                        }
                    }
                    call_value.insert(
                        "name".into(),
                        Value::String(
                            function
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("tool")
                                .to_string(),
                        ),
                    );
                    call_value.insert("args".into(), arguments);
                    part.insert("functionCall".into(), Value::Object(call_value));
                    parts.push(Value::Object(part));
                }
            }

            let finish = choice
                .get("finish_reason")
                .and_then(|v| v.as_str())
                .map(map_openai_finish_reason)
                .unwrap_or("STOP");

            candidates.push(json!({
                "content": {"role": "model", "parts": parts},
                "finishReason": finish,
                "index": i,
                "safetyRatings": [],
            }));
        }
    }

    json!({
        "candidates": candidates,
        "usageMetadata": usage_metadata(value.get("usage")),
    })
}

/// Build a Gemini `usageMetadata` from an OpenAI `usage` object.
///
/// OpenAI reports reasoning tokens inside the completion total; Gemini reports
/// them separately, so `candidatesTokenCount` is the non-reasoning remainder.
pub fn usage_metadata(usage: Option<&Value>) -> Value {
    let prompt = usage
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let completion = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let thoughts = usage
        .and_then(|u| u.get("completion_tokens_details"))
        .and_then(|d| d.get("reasoning_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let cached = usage
        .and_then(|u| u.get("prompt_tokens_details"))
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let total = usage
        .and_then(|u| u.get("total_tokens"))
        .and_then(|v| v.as_i64())
        .unwrap_or(prompt + completion);

    json!({
        "promptTokenCount": prompt,
        "toolUsePromptTokenCount": 0,
        "candidatesTokenCount": (completion - thoughts).max(0),
        "totalTokenCount": total,
        "thoughtsTokenCount": thoughts,
        "cachedContentTokenCount": cached,
    })
}

/// Translate an OpenAI SSE stream into a Gemini SSE stream.
pub struct OpenAiToGeminiStream {
    usage: Usage,
}

impl OpenAiToGeminiStream {
    pub fn new() -> Self {
        Self {
            usage: Usage::default(),
        }
    }

    /// Translate the buffered stream, returning the client-facing bytes and the
    /// usage accumulated along the way.
    pub fn run(&mut self, body: &[u8]) -> (bytes::Bytes, Usage) {
        let text = String::from_utf8_lossy(body);
        let mut out = String::new();

        for frame in crate::sse::claude::parse_sse_frames(&text) {
            let Some(data) = frame.data else { continue };
            let trimmed = data.trim();
            if trimmed.is_empty() || trimmed == "[DONE]" {
                continue;
            }
            let Ok(chunk) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            self.handle(&chunk, &mut out);
        }

        (bytes::Bytes::from(out), self.usage.clone())
    }

    fn handle(&mut self, chunk: &Value, out: &mut String) {
        if let Some(usage) = chunk.get("usage") {
            self.absorb_usage(usage);
        }

        let choices = chunk
            .get("choices")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();

        for choice in &choices {
            let delta = choice.get("delta").cloned().unwrap_or(json!({}));
            let mut parts: Vec<Value> = Vec::new();

            if let Some(thinking) = delta
                .get("reasoning_content")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                parts.push(json!({"text": thinking, "thought": true}));
            }
            if let Some(text) = delta
                .get("content")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                parts.push(json!({"text": text}));
            }
            if let Some(calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                for call in calls {
                    let function = call.get("function").cloned().unwrap_or(json!({}));
                    // Streamed arguments arrive as a fragment, not complete JSON;
                    // `partialArgs` is Gemini's field for exactly that.
                    if let Some(args) = function.get("arguments").and_then(|v| v.as_str()) {
                        if !args.is_empty() {
                            let mut call_value = Map::new();
                            if let Some(name) =
                                function.get("name").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
                            {
                                call_value.insert("name".into(), Value::String(name.to_string()));
                            }
                            call_value.insert(
                                "partialArgs".into(),
                                json!([{"stringValue": args}]),
                            );
                            parts.push(json!({"functionCall": Value::Object(call_value)}));
                        }
                    }
                }
            }

            let finish = choice
                .get("finish_reason")
                .and_then(|v| v.as_str())
                .map(map_openai_finish_reason);

            // Mirror the reference: a chunk carrying neither content nor a finish
            // reason is skipped, which is what filters OpenAI's opening
            // role-only delta out of the Gemini stream.
            if parts.is_empty() && finish.is_none() {
                continue;
            }

            let mut candidate = Map::new();
            if !parts.is_empty() {
                candidate.insert(
                    "content".into(),
                    json!({"role": "model", "parts": parts}),
                );
            }
            candidate.insert(
                "finishReason".into(),
                finish.map(|f| Value::String(f.to_string())).unwrap_or(Value::Null),
            );
            candidate.insert("index".into(), json!(0));
            candidate.insert("safetyRatings".into(), json!([]));

            let mut event = Map::new();
            event.insert("candidates".into(), Value::Array(vec![Value::Object(candidate)]));
            if finish.is_some() {
                event.insert("usageMetadata".into(), self.usage_json());
            }
            push_gemini_frame(out, &Value::Object(event));
        }
    }

    fn absorb_usage(&mut self, usage: &Value) {
        if let Some(p) = usage.get("prompt_tokens").and_then(|v| v.as_i64()) {
            self.usage.prompt_tokens = p;
        }
        if let Some(c) = usage.get("completion_tokens").and_then(|v| v.as_i64()) {
            self.usage.completion_tokens = c;
        }
        if let Some(t) = usage.get("total_tokens").and_then(|v| v.as_i64()) {
            self.usage.total_tokens = t;
        }
        if let Some(r) = usage
            .get("completion_tokens_details")
            .and_then(|d| d.get("reasoning_tokens"))
            .and_then(|v| v.as_i64())
        {
            self.usage.reasoning_tokens = r;
        }
        if let Some(c) = usage
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(|v| v.as_i64())
        {
            self.usage.cached_tokens = c;
        }
        self.usage.semantic = "gemini".to_string();
        if self.usage.total_tokens == 0 {
            self.usage.total_tokens = self.usage.prompt_tokens + self.usage.completion_tokens;
        }
    }

    fn usage_json(&self) -> Value {
        json!({
            "promptTokenCount": self.usage.prompt_tokens,
            "toolUsePromptTokenCount": 0,
            "candidatesTokenCount": (self.usage.completion_tokens - self.usage.reasoning_tokens).max(0),
            "totalTokenCount": self.usage.total_tokens,
            "thoughtsTokenCount": self.usage.reasoning_tokens,
            "cachedContentTokenCount": self.usage.cached_tokens,
        })
    }
}

impl Default for OpenAiToGeminiStream {
    fn default() -> Self {
        Self::new()
    }
}

/// Serialize one Gemini SSE frame. `alt=sse` uses a bare `data:` line with no
/// `event:` name, which is what the Gemini client libraries parse.
fn push_gemini_frame(out: &mut String, value: &Value) {
    out.push_str("data: ");
    out.push_str(&value.to_string());
    out.push_str("\n\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_user_turn_becomes_a_plain_openai_message() {
        let body = json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        });
        let out = convert(&body, "gpt-test", false).unwrap();
        assert_eq!(out["model"], "gpt-test");
        assert_eq!(out["messages"][0]["role"], "user");
        assert_eq!(out["messages"][0]["content"], "hi");
    }

    #[test]
    fn the_model_role_becomes_assistant() {
        let body = json!({
            "contents": [
                {"role": "user", "parts": [{"text": "hi"}]},
                {"role": "model", "parts": [{"text": "hello"}]}
            ]
        });
        let out = convert(&body, "m", false).unwrap();
        assert_eq!(out["messages"][1]["role"], "assistant");
        assert_eq!(out["messages"][1]["content"], "hello");
    }

    #[test]
    fn system_instruction_is_hoisted_to_a_leading_message() {
        let body = json!({
            "systemInstruction": {"parts": [{"text": "be brief"}]},
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        });
        let out = convert(&body, "m", false).unwrap();
        assert_eq!(out["messages"][0]["role"], "system");
        assert_eq!(out["messages"][0]["content"], "be brief");
        assert_eq!(out["messages"][1]["role"], "user");
    }

    #[test]
    fn generation_config_maps_onto_openai_sampling_fields() {
        let body = json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "generationConfig": {
                "temperature": 0.7, "topP": 0.9, "topK": 40,
                "maxOutputTokens": 128, "stopSequences": ["STOP"]
            }
        });
        let out = convert(&body, "m", false).unwrap();
        assert_eq!(out["temperature"], 0.7);
        assert_eq!(out["top_p"], 0.9);
        assert_eq!(out["top_k"], 40);
        assert_eq!(out["max_tokens"], 128);
        assert_eq!(out["stop"], "STOP");
    }

    #[test]
    fn thought_parts_become_reasoning_content() {
        let body = json!({
            "contents": [{"role": "model", "parts": [
                {"text": "let me think", "thought": true},
                {"text": "42"}
            ]}]
        });
        let out = convert(&body, "m", false).unwrap();
        assert_eq!(out["messages"][0]["reasoning_content"], "let me think");
        assert_eq!(out["messages"][0]["content"], "42");
    }

    #[test]
    fn inline_data_becomes_a_data_url_image() {
        let body = json!({
            "contents": [{"role": "user", "parts": [
                {"text": "what is this"},
                {"inlineData": {"mimeType": "image/png", "data": "AAAA"}}
            ]}]
        });
        let out = convert(&body, "m", false).unwrap();
        let parts = out["messages"][0]["content"].as_array().unwrap();
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,AAAA");
    }

    #[test]
    fn a_file_data_uri_passes_through() {
        let body = json!({
            "contents": [{"role": "user", "parts": [
                {"fileData": {"mimeType": "image/png", "fileUri": "https://x/y.png"}}
            ]}]
        });
        let out = convert(&body, "m", false).unwrap();
        assert_eq!(
            out["messages"][0]["content"][0]["image_url"]["url"],
            "https://x/y.png"
        );
    }

    #[test]
    fn a_function_call_becomes_an_assistant_tool_call() {
        let body = json!({
            "contents": [{"role": "model", "parts": [
                {"functionCall": {"name": "get_weather", "args": {"city": "Paris"}}}
            ]}]
        });
        let out = convert(&body, "m", false).unwrap();
        let call = &out["messages"][0]["tool_calls"][0];
        assert_eq!(call["function"]["name"], "get_weather");
        assert!(call["function"]["arguments"]
            .as_str()
            .unwrap()
            .contains("\"city\":\"Paris\""));
    }

    #[test]
    fn a_function_response_becomes_a_tool_message_addressed_to_the_call() {
        let body = json!({
            "contents": [
                {"role": "model", "parts": [
                    {"functionCall": {"name": "get_weather", "args": {}}}
                ]},
                {"role": "user", "parts": [
                    {"functionResponse": {"name": "get_weather",
                                          "response": {"temp": 15}}}
                ]}
            ]
        });
        let out = convert(&body, "m", false).unwrap();
        let messages = out["messages"].as_array().unwrap();
        // The tool message must reuse the id minted for the matching call, or
        // the upstream cannot pair them.
        let call_id = messages[0]["tool_calls"][0]["id"].as_str().unwrap();
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], call_id);
        assert_eq!(messages[1]["name"], "get_weather");
        assert!(messages[1]["content"].as_str().unwrap().contains("15"));
    }

    #[test]
    fn function_declarations_become_openai_tools() {
        let body = json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "tools": [{"functionDeclarations": [{
                "name": "get_weather",
                "description": "Get weather",
                "parameters": {"type": "object", "properties": {}}
            }]}],
            "toolConfig": {"functionCallingConfig": {"mode": "ANY"}}
        });
        let out = convert(&body, "m", false).unwrap();
        assert_eq!(out["tools"][0]["type"], "function");
        assert_eq!(out["tools"][0]["function"]["name"], "get_weather");
        assert_eq!(out["tool_choice"], "required");
    }

    #[test]
    fn streaming_asks_the_upstream_to_report_usage() {
        let body = json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]});
        let out = convert(&body, "m", true).unwrap();
        assert_eq!(out["stream"], true);
        assert_eq!(out["stream_options"]["include_usage"], true);
    }
}
