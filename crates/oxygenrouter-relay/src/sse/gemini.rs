//! Gemini SSE (`alt=sse`) -> OpenAI SSE chunk stream.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use serde_json::{json, Value};

use crate::convert::gemini_to_openai::usage_from_metadata;
use crate::value::Usage;

pub struct GeminiToOpenAiStream<'a> {
    model: &'a str,
    id: String,
    created: i64,
    usage: Usage,
    started: bool,
}

impl<'a> GeminiToOpenAiStream<'a> {
    pub fn new(model: &'a str) -> Self {
        Self {
            model,
            id: format!("chatcmpl-gemini-{}", chrono::Utc::now().timestamp_millis()),
            created: chrono::Utc::now().timestamp(),
            usage: Usage {
                semantic: "gemini".to_string(),
                ..Default::default()
            },
            started: false,
        }
    }

    pub fn run(&mut self, body: &[u8]) -> (bytes::Bytes, Usage) {
        let text = String::from_utf8_lossy(body);
        let mut out = String::new();

        for frame in crate::sse::claude::parse_sse_frames(&text) {
            let Some(data) = frame.data else { continue };
            let d = data.trim();
            if d.is_empty() || d == "[DONE]" {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(d) else {
                continue;
            };
            self.handle(&v, &mut out);
        }

        out.push_str("data: [DONE]\n\n");
        (bytes::Bytes::from(out), self.usage.clone())
    }

    fn handle(&mut self, v: &Value, out: &mut String) {
        if let Some(md) = v.get("usageMetadata") {
            self.merge_usage(md);
        }
        if !self.started {
            self.started = true;
            let chunk = self.chunk(json!({"role":"assistant","content":""}), Value::Null);
            push_chunk(out, &chunk);
        }

        let candidates = v
            .get("candidates")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        for cand in &candidates {
            let parts = cand
                .get("content")
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.as_array())
                .cloned()
                .unwrap_or_default();
            for p in &parts {
                if let Some(t) = p.get("text").and_then(|x| x.as_str()) {
                    if !t.is_empty() {
                        let key = if p.get("thought").and_then(|x| x.as_bool()).unwrap_or(false) {
                            "reasoning_content"
                        } else {
                            "content"
                        };
                        let chunk = self.chunk(json!({ key: t }), Value::Null);
                        push_chunk(out, &chunk);
                    }
                }
                if let Some(fc) = p.get("functionCall") {
                    let args = fc.get("args").cloned().unwrap_or(json!({}));
                    let chunk = self.chunk(
                        json!({"tool_calls":[{
                            "index":0,
                            "id":"call_0",
                            "type":"function",
                            "function":{
                                "name": fc.get("name").and_then(|x| x.as_str()).unwrap_or("tool"),
                                "arguments": serde_json::to_string(&args).unwrap_or_else(|_| "{}".into()),
                            }
                        }]}),
                        Value::Null,
                    );
                    push_chunk(out, &chunk);
                }
            }
            if let Some(reason) = cand.get("finishReason").and_then(|x| x.as_str()) {
                let fr = crate::convert::gemini_to_openai::map_finish_reason(reason);
                let chunk = self.chunk(json!({}), Value::String(fr.to_string()));
                push_chunk(out, &chunk);
            }
        }
    }

    fn merge_usage(&mut self, md: &Value) {
        let u = usage_from_metadata(Some(md));
        if let Some(p) = u.get("prompt_tokens").and_then(|x| x.as_i64()) {
            if p > 0 {
                self.usage.prompt_tokens = p;
            }
        }
        if let Some(c) = u.get("completion_tokens").and_then(|x| x.as_i64()) {
            if c > 0 {
                self.usage.completion_tokens = c;
            }
        }
        if let Some(t) = u.get("total_tokens").and_then(|x| x.as_i64()) {
            if t > 0 {
                self.usage.total_tokens = t;
            }
        }
    }

    fn chunk(&self, delta: Value, finish: Value) -> Value {
        json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]
        })
    }
}

fn push_chunk(out: &mut String, chunk: &Value) {
    out.push_str("data: ");
    out.push_str(&chunk.to_string());
    out.push_str("\n\n");
}
