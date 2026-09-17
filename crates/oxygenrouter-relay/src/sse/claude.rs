//! Anthropic SSE event stream -> OpenAI SSE chunk stream.
//!
//! This is the subtle part. Anthropic emits typed events:
//! `message_start`, `content_block_start`, `content_block_delta`,
//! `content_block_stop`, `message_delta`, `message_stop`, `ping`, `error`.
//!
//! Key translation rules:
//! * `message_start` -> first chunk with `delta.role = "assistant"` (empty content).
//! * `content_block_start` (tool_use) -> tool_call chunk with the block index
//!   remapped from the **Anthropic content-block index** to a **dense
//!   tool-call index** (Anthropic counts text/thinking blocks too).
//! * `content_block_delta` text_delta -> `delta.content`.
//! * `content_block_delta` input_json_delta -> `delta.tool_calls[].function.arguments`.
//! * `content_block_delta` thinking_delta -> `delta.reasoning_content`.
//! * `message_delta` -> a chunk carrying `finish_reason` and the usage.
//! * final -> a `usage` chunk (if requested) then `data: [DONE]`.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use serde_json::{json, Value};

use crate::convert::claude_to_openai::map_stop_reason;
use crate::value::Usage;

/// Streaming converter holding per-stream state.
pub struct ClaudeToOpenAiStream<'a> {
    model: &'a str,
    id: String,
    created: i64,
    /// Anthropic content-block index -> dense OpenAI tool-call index.
    tool_index_by_block: std::collections::HashMap<u64, usize>,
    next_tool_index: usize,
    role_sent: bool,
    usage: Usage,
    finish_reason: Option<String>,
}

impl<'a> ClaudeToOpenAiStream<'a> {
    pub fn new(model: &'a str) -> Self {
        Self {
            model,
            id: "chatcmpl-claude".to_string(),
            created: chrono::Utc::now().timestamp(),
            tool_index_by_block: std::collections::HashMap::new(),
            next_tool_index: 0,
            role_sent: false,
            usage: Usage {
                semantic: "anthropic".to_string(),
                ..Default::default()
            },
            finish_reason: None,
        }
    }

    /// Consume a full buffered SSE payload and produce an OpenAI SSE payload.
    /// Returns the rewritten bytes and the extracted usage.
    pub fn run(&mut self, body: &[u8]) -> (bytes::Bytes, Usage) {
        let text = String::from_utf8_lossy(body);
        let mut out = String::new();

        for frame in parse_sse_frames(&text) {
            // Anthropic frames carry an `event:` line; we only need the data.
            let Some(data) = frame.data else { continue };
            if data.trim() == "[DONE]" {
                continue;
            }
            let Ok(ev) = serde_json::from_str::<Value>(&data) else {
                continue;
            };
            let etype = ev
                .get("type")
                .and_then(|v| v.as_str())
                .or(frame.event.as_deref())
                .unwrap_or("");
            self.handle_event(etype, &ev, &mut out);
        }

        out.push_str("data: [DONE]\n\n");
        (bytes::Bytes::from(out), self.usage.clone())
    }

    fn handle_event(&mut self, etype: &str, ev: &Value, out: &mut String) {
        match etype {
            "message_start" => {
                if let Some(msg) = ev.get("message") {
                    if let Some(id) = msg.get("id").and_then(|v| v.as_str()) {
                        self.id = id.to_string();
                    }
                    self.accumulate_usage(msg.get("usage"));
                }
                if !self.role_sent {
                    self.role_sent = true;
                    let chunk = self.chunk(json!({"role":"assistant","content":""}), Value::Null);
                    push_chunk(out, &chunk);
                }
            }
            "content_block_start" => {
                let block = ev.get("content_block").cloned().unwrap_or(json!({}));
                if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                    let block_idx = ev.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                    let tool_idx = self.next_tool_index;
                    self.next_tool_index += 1;
                    self.tool_index_by_block.insert(block_idx, tool_idx);
                    let delta = json!({
                        "tool_calls": [{
                            "index": tool_idx,
                            "id": block.get("id").and_then(|v| v.as_str()).unwrap_or("toolu"),
                            "type": "function",
                            "function": {
                                "name": block.get("name").and_then(|v| v.as_str()).unwrap_or("tool"),
                                "arguments": ""
                            }
                        }]
                    });
                    let chunk = self.chunk(delta, Value::Null);
                    push_chunk(out, &chunk);
                } else if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    if !t.is_empty() {
                        let chunk = self.chunk(json!({"content": t}), Value::Null);
                        push_chunk(out, &chunk);
                    }
                }
            }
            "content_block_delta" => {
                let delta = ev.get("delta").cloned().unwrap_or(json!({}));
                let dtype = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let block_idx = ev.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                match dtype {
                    "text_delta" => {
                        let t = delta.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        if !t.is_empty() {
                            let chunk = self.chunk(json!({"content": t}), Value::Null);
                            push_chunk(out, &chunk);
                        }
                    }
                    "input_json_delta" => {
                        let partial = delta
                            .get("partial_json")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let tool_idx = self
                            .tool_index_by_block
                            .get(&block_idx)
                            .copied()
                            .unwrap_or(0);
                        let chunk = self.chunk(
                            json!({"tool_calls":[{"index":tool_idx,"function":{"arguments":partial}}]}),
                            Value::Null,
                        );
                        push_chunk(out, &chunk);
                    }
                    "thinking_delta" => {
                        let t = delta.get("thinking").and_then(|v| v.as_str()).unwrap_or("");
                        if !t.is_empty() {
                            let chunk = self.chunk(json!({"reasoning_content": t}), Value::Null);
                            push_chunk(out, &chunk);
                        }
                    }
                    "signature_delta" => {
                        let chunk = self.chunk(json!({"reasoning_content": "\n"}), Value::Null);
                        push_chunk(out, &chunk);
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(reason) = ev
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(|v| v.as_str())
                {
                    self.finish_reason = Some(map_stop_reason(reason).to_string());
                }
                self.accumulate_usage(ev.get("usage"));
                // OpenAI carries finish_reason on the final delta chunk.
                let finish = self
                    .finish_reason
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Null);
                let chunk = self.chunk(json!({}), finish);
                push_chunk(out, &chunk);
            }
            "message_stop" | "ping" | "content_block_stop" => {}
            "error" => {
                let err = ev.get("error").cloned().unwrap_or(json!({}));
                let chunk = json!({
                    "error": {
                        "message": err.get("message").and_then(|v| v.as_str()).unwrap_or("upstream error"),
                        "type": err.get("type").and_then(|v| v.as_str()).unwrap_or("upstream_error"),
                    }
                });
                push_chunk(out, &chunk);
            }
            _ => {}
        }
    }

    fn accumulate_usage(&mut self, usage: Option<&Value>) {
        let Some(u) = usage else { return };
        if let Some(v) = u.get("input_tokens").and_then(|x| x.as_i64()) {
            if v > 0 {
                self.usage.prompt_tokens = v;
            }
        }
        if let Some(v) = u.get("output_tokens").and_then(|x| x.as_i64()) {
            if v > 0 {
                self.usage.completion_tokens = v;
            }
        }
        let cache_read = u
            .get("cache_read_input_tokens")
            .and_then(|x| x.as_i64())
            .unwrap_or(0);
        let cache_creation = u
            .get("cache_creation_input_tokens")
            .and_then(|x| x.as_i64())
            .unwrap_or(0);
        if cache_read > 0 {
            self.usage.cached_tokens = cache_read;
        }
        if cache_creation > 0 {
            self.usage.cache_creation_tokens = cache_creation;
        }
        self.usage.total_tokens = self.usage.prompt_tokens
            + self.usage.cached_tokens
            + self.usage.cache_creation_tokens
            + self.usage.completion_tokens;
    }

    fn chunk(&self, delta: Value, finish: Value) -> Value {
        json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [{
                "index": 0,
                "delta": delta,
                "finish_reason": finish,
            }]
        })
    }
}

fn push_chunk(out: &mut String, chunk: &Value) {
    out.push_str("data: ");
    out.push_str(&chunk.to_string());
    out.push_str("\n\n");
}

/// Minimal SSE frame parser.
pub struct SseFrame {
    pub event: Option<String>,
    pub data: Option<String>,
}

pub fn parse_sse_frames(input: &str) -> Vec<SseFrame> {
    let mut frames = Vec::new();
    let mut event: Option<String> = None;
    let mut data_lines: Vec<String> = Vec::new();

    let flush = |frames: &mut Vec<SseFrame>,
                 event: &mut Option<String>,
                 data_lines: &mut Vec<String>| {
        if event.is_some() || !data_lines.is_empty() {
            frames.push(SseFrame {
                event: event.take(),
                data: if data_lines.is_empty() {
                    None
                } else {
                    Some(data_lines.join("\n"))
                },
            });
        }
        data_lines.clear();
    };

    for line in input.lines() {
        if line.is_empty() {
            flush(&mut frames, &mut event, &mut data_lines);
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            event = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
    }
    flush(&mut frames, &mut event, &mut data_lines);
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    const STREAM: &str = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_01","role":"assistant","usage":{"input_tokens":25,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" world"}}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"get_weather"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"city\":"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"SF\"}"}}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":42}}

event: message_stop
data: {"type":"message_stop"}

"#;

    #[test]
    fn frames_parse_with_events() {
        let frames = parse_sse_frames(STREAM);
        assert_eq!(frames.len(), 9);
        assert_eq!(frames[0].event.as_deref(), Some("message_start"));
        assert!(frames[0].data.as_deref().unwrap().contains("message_start"));
    }

    #[test]
    fn stream_translates_to_openai_chunks() {
        let mut conv = ClaudeToOpenAiStream::new("claude-3-5-sonnet-20241022");
        let (body, usage) = conv.run(STREAM.as_bytes());
        let text = String::from_utf8(body.to_vec()).unwrap();

        let chunks: Vec<Value> = text
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .filter(|d| *d != "[DONE]")
            .map(|d| serde_json::from_str::<Value>(d).unwrap())
            .collect();

        // role chunk first
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], "assistant");
        // text deltas
        assert_eq!(chunks[1]["choices"][0]["delta"]["content"], "Hello");
        assert_eq!(chunks[2]["choices"][0]["delta"]["content"], " world");
        // tool call start with dense index 0
        let tc = &chunks[3]["choices"][0]["delta"]["tool_calls"][0];
        assert_eq!(tc["index"], 0);
        assert_eq!(tc["id"], "toolu_1");
        assert_eq!(tc["function"]["name"], "get_weather");
        // tool argument deltas keep index 0
        assert_eq!(chunks[4]["choices"][0]["delta"]["tool_calls"][0]["index"], 0);
        assert!(chunks[4]["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap()
            .contains("city"));
        // finish chunk carries tool_calls finish reason
        let finish_chunk = chunks.iter().rev().find(|c| {
            c["choices"][0]["finish_reason"].is_string()
        }).unwrap();
        assert_eq!(finish_chunk["choices"][0]["finish_reason"], "tool_calls");
        // ends with DONE
        assert!(text.ends_with("data: [DONE]\n\n"));

        // usage: prompt 25, completion 42
        assert_eq!(usage.prompt_tokens, 25);
        assert_eq!(usage.completion_tokens, 42);
        assert_eq!(usage.semantic, "anthropic");
    }

    #[test]
    fn thinking_delta_becomes_reasoning_content() {
        let stream = r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}

"#;
        let mut conv = ClaudeToOpenAiStream::new("m");
        let (body, _) = conv.run(stream.as_bytes());
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("reasoning_content"));
        assert!(text.contains("hmm"));
    }

    #[test]
    fn usage_includes_cache_tokens() {
        let stream = r#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":5,"cache_creation_input_tokens":3,"output_tokens":0}}}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}

"#;
        let mut conv = ClaudeToOpenAiStream::new("m");
        let (_, usage) = conv.run(stream.as_bytes());
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.cached_tokens, 5);
        assert_eq!(usage.cache_creation_tokens, 3);
        assert_eq!(usage.completion_tokens, 7);
        assert_eq!(usage.total_tokens, 10 + 5 + 3 + 7);
    }
}
