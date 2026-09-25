//! A Gemini client served by an OpenAI channel, end to end through the adaptor.
//!
//! The parallel of `openai_channel_serves_claude_clients`: a client posting
//! Gemini shape to `/v1beta/models/<m>:generateContent` must be routed to
//! `{base}/v1/chat/completions` with a translated body, and the OpenAI reply must
//! come back as Gemini shape **with its content**. The forward path is the part
//! that did not exist at all -- there was no `gemini_to_openai` request converter,
//! so a Gemini client on an OpenAI-compatible channel (vLLM, SGLang, DeepSeek)
//! simply could not work.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use oxygenrouter_relay::adaptor::UpstreamResponse;
use oxygenrouter_relay::adapters::openai::OpenAiAdaptor;
use oxygenrouter_relay::value::{RelayFormat, RelayInfo};
use oxygenrouter_relay::Adaptor;

fn gemini_client_info() -> RelayInfo {
    RelayInfo {
        base_url: "http://127.0.0.1:3000/v1".to_string(),
        request_path: "/v1beta/models/gemini-2.5-flash:generateContent".to_string(),
        relay_format: RelayFormat::Gemini,
        origin_model: "gemini-2.5-flash".to_string(),
        upstream_model: "glm-5.3-flash".to_string(),
        ..Default::default()
    }
}

#[test]
fn a_gemini_client_is_routed_to_the_openai_chat_route() {
    let adaptor = OpenAiAdaptor;
    let url = adaptor.request_url(&gemini_client_info()).expect("url builds");
    // `/v1beta/...` does not exist on an OpenAI-compatible server.
    assert_eq!(url, "http://127.0.0.1:3000/v1/chat/completions");
    assert!(!url.contains("v1beta"), "must not target the Gemini path: {url}");
}

#[test]
fn a_gemini_body_is_translated_into_an_openai_chat_body() {
    let adaptor = OpenAiAdaptor;
    let inbound = serde_json::json!({
        "systemInstruction": {"parts": [{"text": "be brief"}]},
        "contents": [
            {"role": "user", "parts": [{"text": "Reply with exactly: PING"}]},
            {"role": "model", "parts": [{"text": "PING"}]}
        ],
        "generationConfig": {"temperature": 0.2, "maxOutputTokens": 32}
    });
    let sent = adaptor
        .convert_request(&gemini_client_info(), &inbound)
        .expect("conversion must succeed")
        .into_raw();

    assert_eq!(sent["model"], "glm-5.3-flash");
    assert_eq!(sent["temperature"], 0.2);
    assert_eq!(sent["max_tokens"], 32);
    assert_eq!(sent["messages"][0]["role"], "system");
    assert_eq!(sent["messages"][0]["content"], "be brief");
    assert_eq!(sent["messages"][1]["role"], "user");
    assert_eq!(sent["messages"][1]["content"], "Reply with exactly: PING");
    // `model` on the Gemini side is the assistant role on the OpenAI side.
    assert_eq!(sent["messages"][2]["role"], "assistant");
    assert_eq!(sent["messages"][2]["content"], "PING");
    assert!(sent.get("contents").is_none(), "Gemini field must be gone");
}

#[test]
fn an_openai_reply_comes_back_as_gemini_with_its_text() {
    let adaptor = OpenAiAdaptor;
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
            &gemini_client_info(),
            &UpstreamResponse {
                status: 200,
                headers: reqwest::header::HeaderMap::new(),
                body: reply.into_bytes().into(),
                is_stream: false,
            },
        )
        .expect("conversion must succeed");

    let body: serde_json::Value = serde_json::from_slice(&out.body).expect("valid JSON");
    let candidate = &body["candidates"][0];
    assert_eq!(candidate["content"]["role"], "model");
    // The load-bearing assertion: the text survived the round trip.
    assert_eq!(candidate["content"]["parts"][0]["text"], "PING");
    assert_eq!(candidate["finishReason"], "STOP");
    assert_eq!(body["usageMetadata"]["promptTokenCount"], 10);
    assert_eq!(body["usageMetadata"]["candidatesTokenCount"], 2);
    assert_eq!(body["usageMetadata"]["totalTokenCount"], 12);
    // Billing still sees the upstream's numbers.
    assert_eq!(out.usage.total_tokens, 12);
}

#[test]
fn a_reasoning_reply_comes_back_as_a_thought_part() {
    let adaptor = OpenAiAdaptor;
    let reply = serde_json::json!({
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "42",
                        "reasoning_content": "let me think"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 5, "completion_tokens": 4, "total_tokens": 9,
                  "completion_tokens_details": {"reasoning_tokens": 2}}
    })
    .to_string();

    let out = adaptor
        .convert_response(
            &gemini_client_info(),
            &UpstreamResponse {
                status: 200,
                headers: reqwest::header::HeaderMap::new(),
                body: reply.into_bytes().into(),
                is_stream: false,
            },
        )
        .unwrap();

    let body: serde_json::Value = serde_json::from_slice(&out.body).unwrap();
    let parts = body["candidates"][0]["content"]["parts"].as_array().unwrap();
    // Reasoning first, flagged as a thought, then the answer.
    assert_eq!(parts[0]["thought"], true);
    assert_eq!(parts[0]["text"], "let me think");
    assert_eq!(parts[1]["text"], "42");
    // Gemini reports reasoning separately, so it is taken out of the candidate
    // token count rather than double-counted.
    assert_eq!(body["usageMetadata"]["thoughtsTokenCount"], 2);
    assert_eq!(body["usageMetadata"]["candidatesTokenCount"], 2);
}

#[test]
fn a_streamed_openai_reply_becomes_gemini_sse() {
    let adaptor = OpenAiAdaptor;
    let sse = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],",
        "\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2,\"total_tokens\":6}}\n\n",
        "data: [DONE]\n\n"
    );

    let mut info = gemini_client_info();
    info.is_stream = true;
    let out = adaptor
        .convert_response(
            &info,
            &UpstreamResponse {
                status: 200,
                headers: reqwest::header::HeaderMap::new(),
                body: sse.as_bytes().to_vec().into(),
                is_stream: true,
            },
        )
        .unwrap();

    let text = String::from_utf8_lossy(&out.body).to_string();
    let events: Vec<serde_json::Value> = text
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter(|d| *d != "[DONE]")
        .filter_map(|d| serde_json::from_str(d).ok())
        .collect();

    // The role-only opening delta carries neither content nor a finish reason,
    // so it is dropped rather than emitted as an empty candidate.
    assert_eq!(events.len(), 3, "events: {text}");

    let first = &events[0]["candidates"][0];
    assert_eq!(first["content"]["role"], "model");
    assert_eq!(first["content"]["parts"][0]["text"], "Hel");
    assert_eq!(first["finishReason"], serde_json::Value::Null);

    let last = &events[2]["candidates"][0];
    assert_eq!(last["finishReason"], "STOP");
    assert_eq!(events[2]["usageMetadata"]["promptTokenCount"], 4);
    assert_eq!(events[2]["usageMetadata"]["totalTokenCount"], 6);
    // Billing sees the upstream numbers, not zeros.
    assert_eq!(out.usage.total_tokens, 6);
    assert_eq!(out.usage.prompt_tokens, 4);
}

#[test]
fn an_openai_client_on_the_same_channel_is_untouched() {
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
}
