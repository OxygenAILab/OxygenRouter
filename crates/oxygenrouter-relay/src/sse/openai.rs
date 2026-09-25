//! OpenAI SSE helpers: usage extraction from a buffered stream.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use bytes::Bytes;

use crate::value::Usage;

/// Translate an OpenAI SSE stream into an Anthropic SSE stream.
///
/// Used when the client asked for Anthropic shape (`POST /v1/messages`) but the
/// selected upstream speaks OpenAI. The event order follows the reference
/// sequence pinned by NewAPI's `stream/openai_to_claude` golden snapshot:
///
/// ```text
/// message_start -> content_block_start -> content_block_delta{,...}
///               -> content_block_stop -> message_delta -> message_stop
/// ```
///
/// `reasoning_content` deltas become a leading `thinking` block (a separate
/// content block index), mirroring how the reference emits reasoning.
pub fn to_claude_events(body: &[u8]) -> Bytes {
    let text = String::from_utf8_lossy(body);
    let mut out = String::new();

    let mut message_id = String::from("msg_relay");
    let mut model = String::from("");
    let mut input_tokens: i64 = 0;
    let mut output_tokens: i64 = 0;
    let mut stop_reason = String::from("end_turn");

    // Block allocation. The thinking block is emitted only if reasoning arrives,
    // so a plain answer does not carry an empty one.
    let mut started = false;
    let mut thinking_index: Option<usize> = None;
    let mut text_index: Option<usize> = None;
    let mut next_index = 0usize;
    let mut text_open = false;
    let mut thinking_open = false;

    for frame in crate::sse::claude::parse_sse_frames(&text) {
        let Some(data) = frame.data else { continue };
        let trimmed = data.trim();
        if trimmed.is_empty() || trimmed == "[DONE]" {
            continue;
        }
        let Ok(ev) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };

        if let Some(id) = ev.get("id").and_then(|v| v.as_str()) {
            if !id.is_empty() {
                message_id = id.to_string();
            }
        }
        if let Some(m) = ev.get("model").and_then(|v| v.as_str()) {
            if !m.is_empty() {
                model = m.to_string();
            }
        }
        if let Some(u) = ev.get("usage") {
            if let Some(v) = u.get("prompt_tokens").and_then(|v| v.as_i64()) {
                input_tokens = v;
            }
            if let Some(v) = u.get("completion_tokens").and_then(|v| v.as_i64()) {
                output_tokens = v;
            }
        }

        let Some(choice) = ev
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|c| c.first())
        else {
            continue;
        };

        if !started {
            started = true;
            out.push_str(&crate::convert::openai_to_claude_response::sse_frame(
                &crate::convert::openai_to_claude_response::stream_message_start(
                    &message_id,
                    &model,
                    input_tokens,
                ),
            ));
        }

        // `delta` on a chunk, or `message` on the final non-streaming-shaped one.
        let delta = choice.get("delta").or_else(|| choice.get("message"));

        if let Some(reasoning) = delta
            .and_then(|d| d.get("reasoning_content"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            if thinking_index.is_none() {
                let index = next_index;
                next_index += 1;
                thinking_index = Some(index);
                thinking_open = true;
                out.push_str(&crate::convert::openai_to_claude_response::sse_frame(
                    &crate::convert::openai_to_claude_response::stream_block_start(
                        index,
                        serde_json::json!({"type": "thinking", "thinking": ""}),
                    ),
                ));
            }
            let index = thinking_index.unwrap_or(0);
            out.push_str(&crate::convert::openai_to_claude_response::sse_frame(
                &serde_json::json!({
                    "type": "content_block_delta",
                    "index": index,
                    "delta": {"type": "thinking_delta", "thinking": reasoning}
                }),
            ));
        }

        if let Some(text_delta) = delta
            .and_then(|d| d.get("content"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            // Reasoning and text are separate blocks, so close the thinking block
            // before opening the text one.
            if thinking_open {
                thinking_open = false;
                if let Some(index) = thinking_index {
                    out.push_str(&crate::convert::openai_to_claude_response::sse_frame(
                        &crate::convert::openai_to_claude_response::stream_block_stop(index),
                    ));
                }
            }
            if text_index.is_none() {
                let index = next_index;
                next_index += 1;
                text_index = Some(index);
                text_open = true;
                out.push_str(&crate::convert::openai_to_claude_response::sse_frame(
                    &crate::convert::openai_to_claude_response::stream_block_start(
                        index,
                        serde_json::json!({"type": "text", "text": ""}),
                    ),
                ));
            }
            let index = text_index.unwrap_or(0);
            out.push_str(&crate::convert::openai_to_claude_response::sse_frame(
                &crate::convert::openai_to_claude_response::stream_text_delta(index, text_delta),
            ));
        }

        if let Some(reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            stop_reason =
                crate::convert::openai_to_claude_response::map_finish_reason(reason).to_string();
        }
    }

    if !started {
        // The upstream produced nothing usable; still emit a well-formed stream
        // so the client can terminate instead of hanging.
        out.push_str(&crate::convert::openai_to_claude_response::sse_frame(
            &crate::convert::openai_to_claude_response::stream_message_start(
                &message_id,
                &model,
                input_tokens,
            ),
        ));
    }

    if thinking_open {
        if let Some(index) = thinking_index {
            out.push_str(&crate::convert::openai_to_claude_response::sse_frame(
                &crate::convert::openai_to_claude_response::stream_block_stop(index),
            ));
        }
    }
    if text_open {
        if let Some(index) = text_index {
            out.push_str(&crate::convert::openai_to_claude_response::sse_frame(
                &crate::convert::openai_to_claude_response::stream_block_stop(index),
            ));
        }
    }

    out.push_str(&crate::convert::openai_to_claude_response::sse_frame(
        &crate::convert::openai_to_claude_response::stream_message_delta(
            &stop_reason,
            output_tokens,
        ),
    ));
    out.push_str(&crate::convert::openai_to_claude_response::sse_frame(
        &crate::convert::openai_to_claude_response::stream_message_stop(),
    ));

    Bytes::from(out)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn openai_chunk(id: &str, model: &str, content: Option<&str>, reasoning: Option<&str>, finish: Option<&str>) -> String {
        let mut delta = serde_json::json!({});
        if let Some(c) = content {
            delta["content"] = serde_json::Value::String(c.to_string());
        }
        if let Some(r) = reasoning {
            delta["reasoning_content"] = serde_json::Value::String(r.to_string());
        }
        let chunk = serde_json::json!({
            "id": id,
            "model": model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
        });
        format!("data: {}\n\n", chunk)
    }

    fn types_in(sse: &str) -> Vec<String> {
        sse.lines()
            .filter(|l| l.starts_with("event: "))
            .map(|l| l.trim_start_matches("event: ").to_string())
            .collect()
    }

    #[test]
    fn a_text_answer_becomes_the_reference_event_sequence() {
        let mut sse = String::new();
        sse.push_str(&openai_chunk("chatcmpl-1", "m", Some("Hello"), None, None));
        sse.push_str(&openai_chunk("chatcmpl-1", "m", Some(" world"), None, None));
        sse.push_str(&openai_chunk("chatcmpl-1", "m", None, None, Some("stop")));
        sse.push_str("data: [DONE]\n\n");

        let out = String::from_utf8_lossy(&to_claude_events(sse.as_bytes())).to_string();
        let types = types_in(&out);
        // Mirrors NewAPI's stream/openai_to_claude golden ordering.
        assert_eq!("message_start", types[0]);
        assert_eq!("content_block_start", types[1]);
        assert_eq!("content_block_delta", types[2]);
        assert_eq!("content_block_delta", types[3]);
        assert_eq!("content_block_stop", types[4]);
        assert_eq!("message_delta", types[5]);
        assert_eq!("message_stop", types[6]);
        assert!(out.contains("\"text\":\"Hello\""));
        assert!(out.contains("end_turn"), "stop reason must be mapped");
    }

    #[test]
    fn reasoning_becomes_a_leading_thinking_block() {
        let mut sse = String::new();
        sse.push_str(&openai_chunk("c", "m", None, Some("thought"), None));
        sse.push_str(&openai_chunk("c", "m", Some("answer"), None, None));
        sse.push_str(&openai_chunk("c", "m", None, None, Some("stop")));

        let out = String::from_utf8_lossy(&to_claude_events(sse.as_bytes())).to_string();
        // Two blocks: thinking at index 0, text at index 1.
        assert!(out.contains("\"type\":\"thinking\""));
        assert!(out.contains("thinking_delta"));
        assert!(out.contains("\"index\":0"));
        assert!(out.contains("\"index\":1"));
    }

    #[test]
    fn finish_reason_length_maps_to_max_tokens() {
        let mut sse = String::new();
        sse.push_str(&openai_chunk("c", "m", Some("x"), None, None));
        sse.push_str(&openai_chunk("c", "m", None, None, Some("length")));
        let out = String::from_utf8_lossy(&to_claude_events(sse.as_bytes())).to_string();
        assert!(out.contains("max_tokens"));
    }

    #[test]
    fn an_empty_upstream_still_yields_a_well_formed_stream() {
        // The client must be able to terminate rather than hang.
        let out = String::from_utf8_lossy(&to_claude_events(b"")).to_string();
        let types = types_in(&out);
        assert_eq!(types.first().map(String::as_str), Some("message_start"));
        assert_eq!(types.last().map(String::as_str), Some("message_stop"));
    }

    #[test]
    fn usage_from_the_final_chunk_reaches_message_delta() {
        let mut sse = String::new();
        sse.push_str(&openai_chunk("c", "m", Some("x"), None, None));
        let last = serde_json::json!({
            "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 11, "completion_tokens": 7}
        });
        sse.push_str(&format!("data: {}\n\n", last));
        let out = String::from_utf8_lossy(&to_claude_events(sse.as_bytes())).to_string();
        // output_tokens is reported on the terminal message_delta.
        assert!(out.contains("\"output_tokens\":7"), "{out}");
    }

    #[test]
    fn every_frame_is_a_valid_sse_block() {
        let mut sse = String::new();
        sse.push_str(&openai_chunk("c", "m", Some("hi"), None, None));
        sse.push_str(&openai_chunk("c", "m", None, None, Some("stop")));
        let out = String::from_utf8_lossy(&to_claude_events(sse.as_bytes())).to_string();
        for block in out.split("\n\n").filter(|b| !b.trim().is_empty()) {
            assert!(block.starts_with("event: "), "bad frame: {block}");
            assert!(block.contains("\ndata: "), "bad frame: {block}");
            let data = block.split("\ndata: ").nth(1).expect("data line");
            assert!(serde_json::from_str::<serde_json::Value>(data).is_ok(), "bad json: {data}");
        }
    }
}
