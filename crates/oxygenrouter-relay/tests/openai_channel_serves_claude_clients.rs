//! An OpenAI channel serving an Anthropic client, end to end through the adaptor.
//!
//! This is the regression guard for a bug that shape-only assertions cannot see.
//! A Claude client posts `/v1/messages`; the OpenAI adaptor used to append that
//! path verbatim, so the request landed on whichever endpoint answered
//! `/v1/messages` upstream with an Anthropic body, and the Anthropic reply was
//! then handed to the OpenAI->Claude converter. That converter found no `choices`
//! and returned a syntactically valid but *empty* content block -- a well-formed
//! `{"type":"message","content":[{"type":"text","text":""}]}` that every
//! shape-only assertion accepts.
//!
//! So these tests assert on the request URL and the returned text, never on the
//! envelope alone.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use oxygenrouter_relay::adapters::openai::OpenAiAdaptor;
use oxygenrouter_relay::adaptor::UpstreamResponse;
use oxygenrouter_relay::value::{RelayFormat, RelayInfo};
use oxygenrouter_relay::Adaptor;

fn claude_client_info(base_url: &str) -> RelayInfo {
    RelayInfo {
        base_url: base_url.to_string(),
        request_path: "/v1/messages".to_string(),
        relay_format: RelayFormat::Claude,
        origin_model: "claude-3-5-sonnet".to_string(),
        upstream_model: "glm-5.3-flash".to_string(),
        ..Default::default()
    }
}

#[test]
fn a_claude_client_is_routed_to_the_openai_chat_route() {
    let adaptor = OpenAiAdaptor;
    let url = adaptor
        .request_url(&claude_client_info("http://127.0.0.1:3000/v1"))
        .expect("url must build");
    // `/v1/messages` is not a route an OpenAI upstream implements, so forwarding
    // it would answer from the wrong handler.
    assert_eq!(url, "http://127.0.0.1:3000/v1/chat/completions");
    assert!(!url.contains("/messages"), "must not target the messages path: {url}");
}

#[test]
fn a_claude_body_is_translated_into_an_openai_chat_body() {
    let adaptor = OpenAiAdaptor;
    let inbound = serde_json::json!({
        "model": "claude-3-5-sonnet",
        "max_tokens": 32,
        "system": [{"type": "text", "text": "be brief"}],
        "messages": [{"role": "user", "content": "Reply with exactly: PING"}]
    });
    let sent = adaptor
        .convert_request(&claude_client_info("http://x/v1"), &inbound)
        .expect("conversion must succeed")
        .into_raw();

    // Anthropic fields must not survive into an OpenAI request.
    assert!(sent.get("max_tokens").is_some(), "max_tokens carries over");
    assert_eq!(sent["model"], "glm-5.3-flash");
    assert_eq!(sent["messages"][0]["role"], "system");
    assert_eq!(sent["messages"][0]["content"], "be brief");
    assert_eq!(sent["messages"][1]["role"], "user");
    assert_eq!(sent["messages"][1]["content"], "Reply with exactly: PING");
    // Anthropic uses a top-level `system`; OpenAI folds it into the messages.
    assert!(sent.get("system").is_none(), "system must be folded in: {sent}");
}

#[test]
fn an_openai_reply_carries_its_text_back_to_the_claude_client() {
    let adaptor = OpenAiAdaptor;
    // A real OpenAI-shaped reply, not a stub: this is what the upstream returns
    // once the request above is routed correctly.
    let reply = serde_json::json!({
        "id": "cmb-1",
        "object": "chat.completion",
        "model": "glm-5.3-flash",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "PING"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12}
    })
    .to_string();

    let out = adaptor
        .convert_response(
            &claude_client_info("http://x/v1"),
            &UpstreamResponse {
                status: 200,
                headers: reqwest::header::HeaderMap::new(),
                body: reply.into_bytes().into(),
                is_stream: false,
            },
        )
        .expect("conversion must succeed");

    let body: serde_json::Value = serde_json::from_slice(&out.body).expect("valid JSON");
    assert_eq!(body["type"], "message");
    assert_eq!(body["content"][0]["type"], "text");
    // The load-bearing assertion: the text actually survived. Asserting only
    // that `content` is a non-empty array passes on the empty-block bug.
    assert_eq!(body["content"][0]["text"], "PING", "the answer must survive");
    assert_eq!(body["stop_reason"], "end_turn");
    assert_eq!(body["usage"]["input_tokens"], 10);
    assert_eq!(body["usage"]["output_tokens"], 2);
}

#[test]
fn an_openai_client_on_the_same_channel_still_passes_through() {
    // The Claude handling must not disturb an OpenAI client on the same channel.
    let adaptor = OpenAiAdaptor;
    let info = RelayInfo {
        base_url: "http://127.0.0.1:3000/v1".to_string(),
        request_path: "/v1/chat/completions".to_string(),
        relay_format: RelayFormat::OpenAiChat,
        ..Default::default()
    };
    assert_eq!(
        adaptor.request_url(&info).unwrap(),
        "http://127.0.0.1:3000/v1/chat/completions"
    );

    let inbound = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "hi"}]
    });
    let sent = adaptor.convert_request(&info, &inbound).unwrap().into_raw();
    assert_eq!(sent["messages"][0]["content"], "hi");
    assert!(sent.get("max_tokens").is_none());
}
