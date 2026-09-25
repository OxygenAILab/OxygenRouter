//! Adaptor-level stream shaping.
//!
//! Exercises `Adaptor::convert_response`, not just the converter function: the
//! bug class this catches is a correct converter that the adaptor never calls,
//! or calls with the wrong relay format.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use oxygenrouter_relay::adaptor::UpstreamResponse;
use oxygenrouter_relay::adapters::openai::OpenAiAdaptor;
use oxygenrouter_relay::value::{RelayFormat, RelayInfo};
use oxygenrouter_relay::Adaptor;

fn live_sse() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/live_openai_sse.txt"
    );
    std::fs::read(path).expect("live SSE fixture must exist")
}

fn info(format: RelayFormat) -> RelayInfo {
    RelayInfo {
        relay_format: format,
        is_stream: true,
        origin_model: "glm-5.3-flash".to_string(),
        upstream_model: "glm-5.3-flash".to_string(),
        ..Default::default()
    }
}

fn upstream(body: Vec<u8>) -> UpstreamResponse {
    UpstreamResponse {
        status: 200,
        headers: reqwest::header::HeaderMap::new(),
        body: body.into(),
        is_stream: true,
    }
}

#[test]
fn an_anthropic_client_gets_anthropic_events_from_an_openai_upstream() {
    let adaptor = OpenAiAdaptor;
    let out = adaptor
        .convert_response(&info(RelayFormat::Claude), &upstream(live_sse()))
        .expect("conversion must succeed");

    let text = String::from_utf8_lossy(&out.body).to_string();
    let events: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("event: "))
        .collect();

    assert_eq!(events.first().copied(), Some("message_start"), "{events:?}");
    assert_eq!(events.last().copied(), Some("message_stop"), "{events:?}");
    assert!(
        events.iter().any(|e| *e == "content_block_start"),
        "the adaptor produced no content block; events: {events:?}"
    );
    assert!(
        text.contains("glm-5.3-flash"),
        "the model name must survive; got: {text}"
    );
    // Usage must be extracted even when the body is being reshaped.
    assert!(
        out.usage.total_tokens > 0,
        "usage should be non-zero for a live stream"
    );
}

#[test]
fn an_openai_client_still_gets_openai_chunks() {
    // The same upstream body must stay OpenAI for an OpenAI client.
    let adaptor = OpenAiAdaptor;
    let out = adaptor
        .convert_response(&info(RelayFormat::OpenAiChat), &upstream(live_sse()))
        .expect("conversion must succeed");
    let text = String::from_utf8_lossy(&out.body).to_string();
    assert!(text.contains("chat.completion.chunk"), "not OpenAI chunks");
    assert!(!text.contains("content_block_delta"), "must not be Anthropic");
}
