//! Live-captured SSE shaping check.
//!
//! `fixtures/live_openai_sse.txt` is a real OpenAI SSE stream captured from the
//! reference instance through this relay. Feeding the actual bytes back through
//! the translator is the only way to catch a bug that hand-written fixtures miss
//! (they tend to differ from production in whitespace and in which optional
//! fields the provider emits).
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use oxygenrouter_relay::sse::openai::to_claude_events;

#[test]
fn a_live_openai_stream_becomes_a_valid_anthropic_stream() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/live_openai_sse.txt"
    );
    let raw = std::fs::read_to_string(path).expect("live SSE fixture must exist");
    assert!(raw.contains("data: "), "fixture should be an SSE payload");

    let out = String::from_utf8_lossy(&to_claude_events(raw.as_bytes())).to_string();

    let events: Vec<&str> = out
        .lines()
        .filter_map(|l| l.strip_prefix("event: "))
        .collect();

    // The client must be able to drive a complete Anthropic stream.
    assert_eq!(events.first().copied(), Some("message_start"), "events: {events:?}");
    assert_eq!(events.last().copied(), Some("message_stop"), "events: {events:?}");
    assert!(events.contains(&"message_delta"), "events: {events:?}");

    // The live upstream always streams content (reasoning and/or text), so a
    // stream with no content block means the translator dropped it.
    assert!(
        events.iter().any(|e| *e == "content_block_start"),
        "no content block was produced from a live stream with content; events: {events:?}"
    );
    assert!(
        events.iter().any(|e| *e == "content_block_delta"),
        "no delta was produced; events: {events:?}"
    );

    // Every frame must be a well-formed SSE block with valid JSON.
    for block in out.split("\n\n").filter(|b| !b.trim().is_empty()) {
        assert!(block.starts_with("event: "), "bad frame: {block}");
        let data = block
            .split("\ndata: ")
            .nth(1)
            .unwrap_or_else(|| panic!("frame has no data line: {block}"));
        assert!(
            serde_json::from_str::<serde_json::Value>(data).is_ok(),
            "frame has invalid JSON: {data}"
        );
    }
}

#[test]
fn a_live_stream_preserves_the_reasoning_as_a_thinking_block() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/live_openai_sse.txt"
    );
    let raw = std::fs::read_to_string(path).expect("live SSE fixture must exist");
    // This particular capture streams `reasoning_content`, which must surface as
    // a thinking block rather than being silently dropped.
    assert!(raw.contains("reasoning_content"), "fixture should carry reasoning");
    let out = String::from_utf8_lossy(&to_claude_events(raw.as_bytes())).to_string();
    assert!(
        out.contains("\"thinking\""),
        "reasoning did not become a thinking block: {out}"
    );
}
