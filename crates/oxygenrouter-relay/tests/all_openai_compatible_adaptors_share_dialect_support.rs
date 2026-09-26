//! Every OpenAI-compatible adaptor must serve all three client dialects.
//!
//! The original defect -- a native-dialect client whose own path was forwarded to
//! an OpenAI upstream -- was fixed in `OpenAiAdaptor`. That only covered one of
//! the channels able to hit it; `Ollama` and `AdvancedCustom` speak OpenAI too and
//! had the identical problem. The shared handling now lives in
//! `adapters/openai_compat.rs`, and this test holds every such adaptor to it.
//!
//! Driving the check off the registry rather than a hand-written list is the point:
//! a new OpenAI-compatible adaptor added later is covered the moment it is
//! registered, instead of silently repeating the bug.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use oxygenrouter_relay::adaptor::UpstreamResponse;
use oxygenrouter_relay::channel_type::ApiType;
use oxygenrouter_relay::get_adaptor;
use oxygenrouter_relay::value::{RelayFormat, RelayInfo};

/// The OpenAI-compatible adaptors, with the base URL each expects and the exact
/// URL a translated request must produce.
///
/// `AdvancedCustom` is template-driven, so it gets a template carrying both
/// placeholders. Its `{action}` expands from the same path the other adaptors
/// append, which is why the expected URLs agree on the trailing
/// `v1/chat/completions`.
fn openai_compatible_adaptors() -> Vec<(ApiType, &'static str, &'static str)> {
    vec![
        (
            ApiType::OpenAi,
            "http://127.0.0.1:3000/v1",
            "http://127.0.0.1:3000/v1/chat/completions",
        ),
        (
            ApiType::Ollama,
            "http://127.0.0.1:11434/v1",
            "http://127.0.0.1:11434/v1/chat/completions",
        ),
        (
            ApiType::AdvancedCustom,
            "http://127.0.0.1:8080/{action}?model={model}",
            "http://127.0.0.1:8080/v1/chat/completions?model=glm-5.3-flash",
        ),
    ]
}

fn claude_info(base_url: &str) -> RelayInfo {
    RelayInfo {
        base_url: base_url.to_string(),
        request_path: "/v1/messages".to_string(),
        relay_format: RelayFormat::Claude,
        origin_model: "claude-3-5-sonnet".to_string(),
        upstream_model: "glm-5.3-flash".to_string(),
        ..Default::default()
    }
}

fn gemini_info(base_url: &str) -> RelayInfo {
    RelayInfo {
        base_url: base_url.to_string(),
        request_path: "/v1beta/models/gemini-2.5-flash:generateContent".to_string(),
        relay_format: RelayFormat::Gemini,
        origin_model: "gemini-2.5-flash".to_string(),
        upstream_model: "glm-5.3-flash".to_string(),
        ..Default::default()
    }
}

#[test]
fn every_openai_compatible_adaptor_aims_a_native_client_at_the_chat_route() {
    for (api_type, base_url, expected_url) in openai_compatible_adaptors() {
        let adaptor = get_adaptor(api_type).expect("adaptor must exist");

        // Both native dialects must land on the chat completions route. Asserting
        // the exact URL (rather than merely the absence of `/v1/messages`) is what
        // proves the re-aiming produced a *usable* endpoint.
        for (dialect, url) in [
            ("Anthropic", adaptor.request_url(&claude_info(base_url)).unwrap()),
            ("Gemini", adaptor.request_url(&gemini_info(base_url)).unwrap()),
        ] {
            assert_eq!(
                url,
                expected_url,
                "{dialect} client on {} produced the wrong upstream URL",
                adaptor.name()
            );
        }

        // And the ordinary OpenAI client is left on its own path.
        let url = adaptor.request_url(&openai_info(base_url)).unwrap();
        assert_eq!(url, expected_url, "{} moved an OpenAI client", adaptor.name());
    }
}

fn openai_info(base_url: &str) -> RelayInfo {
    RelayInfo {
        base_url: base_url.to_string(),
        request_path: "/v1/chat/completions".to_string(),
        relay_format: RelayFormat::OpenAiChat,
        upstream_model: "glm-5.3-flash".to_string(),
        ..Default::default()
    }
}

#[test]
fn every_openai_compatible_adaptor_translates_an_anthropic_body() {
    let inbound = serde_json::json!({
        "model": "claude-3-5-sonnet",
        "max_tokens": 32,
        "system": [{"type": "text", "text": "be brief"}],
        "messages": [{"role": "user", "content": "hi"}]
    });

    for (api_type, base_url, _) in openai_compatible_adaptors() {
        let adaptor = get_adaptor(api_type).expect("adaptor must exist");
        let sent = adaptor
            .convert_request(&claude_info(base_url), &inbound)
            .unwrap_or_else(|e| panic!("{} failed to translate: {e}", adaptor.name()))
            .into_raw();

        assert_eq!(sent["model"], "glm-5.3-flash", "{}", adaptor.name());
        // Anthropic's top-level `system` must have been folded into the messages,
        // which is the observable sign that the Anthropic converter ran rather
        // than the body being forwarded verbatim.
        assert!(
            sent.get("system").is_none(),
            "{} forwarded the Anthropic body unchanged: {sent}",
            adaptor.name()
        );
        assert_eq!(
            sent["messages"][0]["role"], "system",
            "{}",
            adaptor.name()
        );
        assert_eq!(sent["messages"][1]["content"], "hi", "{}", adaptor.name());
    }
}

#[test]
fn every_openai_compatible_adaptor_translates_a_gemini_body() {
    let inbound = serde_json::json!({
        "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
        "generationConfig": {"maxOutputTokens": 32}
    });

    for (api_type, base_url, _) in openai_compatible_adaptors() {
        let adaptor = get_adaptor(api_type).expect("adaptor must exist");
        let sent = adaptor
            .convert_request(&gemini_info(base_url), &inbound)
            .unwrap_or_else(|e| panic!("{} failed to translate: {e}", adaptor.name()))
            .into_raw();

        assert_eq!(sent["model"], "glm-5.3-flash", "{}", adaptor.name());
        assert!(
            sent.get("contents").is_none(),
            "{} forwarded the Gemini body unchanged: {sent}",
            adaptor.name()
        );
        assert_eq!(sent["max_tokens"], 32, "{}", adaptor.name());
        assert_eq!(sent["messages"][0]["content"], "hi", "{}", adaptor.name());
    }
}

#[test]
fn every_openai_compatible_adaptor_shapes_a_reply_for_the_client() {
    let openai_reply = serde_json::json!({
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

    for (api_type, base_url, _) in openai_compatible_adaptors() {
        let adaptor = get_adaptor(api_type).expect("adaptor must exist");
        let upstream = || UpstreamResponse {
            status: 200,
            headers: reqwest::header::HeaderMap::new(),
            body: openai_reply.clone().into_bytes().into(),
            is_stream: false,
        };

        // A Claude client gets Anthropic shape, with the text intact.
        let out = adaptor
            .convert_response(&claude_info(base_url), &upstream())
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&out.body).unwrap();
        assert_eq!(body["type"], "message", "{}", adaptor.name());
        assert_eq!(
            body["content"][0]["text"], "PING",
            "{} lost the text",
            adaptor.name()
        );

        // A Gemini client gets Gemini shape, with the text intact.
        let out = adaptor
            .convert_response(&gemini_info(base_url), &upstream())
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&out.body).unwrap();
        assert_eq!(
            body["candidates"][0]["content"]["parts"][0]["text"], "PING",
            "{} lost the text",
            adaptor.name()
        );
    }
}

#[test]
fn an_openai_client_is_still_passed_through_by_every_adaptor() {
    // The dialect handling must not disturb the ordinary case.
    let inbound = serde_json::json!({
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "hi"}]
    });

    for (api_type, base_url, _) in openai_compatible_adaptors() {
        let adaptor = get_adaptor(api_type).expect("adaptor must exist");
        let info = RelayInfo {
            base_url: base_url.to_string(),
            request_path: "/v1/chat/completions".to_string(),
            relay_format: RelayFormat::OpenAiChat,
            upstream_model: "gpt-4o".to_string(),
            ..Default::default()
        };
        let sent = adaptor.convert_request(&info, &inbound).unwrap().into_raw();
        assert_eq!(sent["messages"][0]["content"], "hi", "{}", adaptor.name());
        assert!(sent.get("system").is_none(), "{}", adaptor.name());
    }
}
