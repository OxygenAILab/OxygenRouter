//! Adaptor auth, URL and credential-resolution contract tests.
//!
//! These pin the two things most likely to regress silently and hardest to
//! notice at runtime: the provider-specific auth header, and the URL shape.
//! A wrong auth header produces a 401 that looks like a bad key; a wrong URL
//! produces a 404 that looks like a bad deployment.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use reqwest::header::HeaderMap;

use oxygenrouter_relay::adapters::{azure, bedrock, gemini, openai, vertex};
use oxygenrouter_relay::value::{RelayFormat, RelayInfo};
use oxygenrouter_relay::Adaptor;

fn info(base: &str, path: &str, model: &str) -> RelayInfo {
    RelayInfo {
        base_url: base.to_string(),
        api_key: "SECRET".to_string(),
        credential_raw: "SECRET".to_string(),
        request_path: path.to_string(),
        upstream_model: model.to_string(),
        origin_model: model.to_string(),
        relay_format: RelayFormat::OpenAiChat,
        ..Default::default()
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

#[test]
fn openai_uses_bearer_and_joins_path() {
    let adaptor = openai::OpenAiAdaptor;
    let info = info("https://api.openai.com", "/v1/chat/completions", "gpt-4o");

    assert_eq!(
        adaptor.request_url(&info).unwrap(),
        "https://api.openai.com/v1/chat/completions"
    );

    let mut headers = HeaderMap::new();
    adaptor.setup_headers(&mut headers, &info).unwrap();
    assert_eq!(header(&headers, "authorization").as_deref(), Some("Bearer SECRET"));
}

#[test]
fn openai_base_already_ending_in_v1_is_not_duplicated() {
    // A very common operator mistake: base_url already carries `/v1`.
    let adaptor = openai::OpenAiAdaptor;
    let info = info("https://api.example.com/v1", "/v1/chat/completions", "gpt-4o");
    assert_eq!(
        adaptor.request_url(&info).unwrap(),
        "https://api.example.com/v1/chat/completions"
    );
}

#[test]
fn openai_without_key_omits_authorization() {
    // Self-hosted OpenAI-compatible servers frequently need no auth at all.
    let adaptor = openai::OpenAiAdaptor;
    let mut info = info("http://localhost:8000", "/v1/chat/completions", "local");
    info.api_key.clear();

    let mut headers = HeaderMap::new();
    adaptor.setup_headers(&mut headers, &info).unwrap();
    assert!(header(&headers, "authorization").is_none());
}

#[test]
fn anthropic_uses_x_api_key_not_bearer() {
    let adaptor = oxygenrouter_relay::adapters::anthropic::AnthropicAdaptor;
    let info = info("https://api.anthropic.com", "/v1/chat/completions", "claude-sonnet-4-20250514");

    assert_eq!(
        adaptor.request_url(&info).unwrap(),
        "https://api.anthropic.com/v1/messages"
    );

    let mut headers = HeaderMap::new();
    adaptor.setup_headers(&mut headers, &info).unwrap();
    assert_eq!(header(&headers, "x-api-key").as_deref(), Some("SECRET"));
    assert!(header(&headers, "authorization").is_none());
    assert_eq!(
        header(&headers, "anthropic-version").as_deref(),
        Some("2023-06-01")
    );
}

#[test]
fn gemini_uses_goog_key_and_model_in_path() {
    let adaptor = gemini::GeminiAdaptor;
    let info = info(
        "https://generativelanguage.googleapis.com",
        "/v1/chat/completions",
        "gemini-2.0-flash",
    );

    assert_eq!(
        adaptor.request_url(&info).unwrap(),
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent"
    );

    let streaming = RelayInfo {
        is_stream: true,
        ..info.clone()
    };
    assert!(adaptor
        .request_url(&streaming)
        .unwrap()
        .ends_with(":streamGenerateContent?alt=sse"));

    let mut headers = HeaderMap::new();
    adaptor.setup_headers(&mut headers, &info).unwrap();
    assert_eq!(header(&headers, "x-goog-api-key").as_deref(), Some("SECRET"));
}

#[test]
fn azure_builds_deployment_url_and_uses_api_key_header() {
    let adaptor = azure::AzureAdaptor;
    let info = info(
        "https://my-resource.openai.azure.com",
        "/v1/chat/completions",
        "my-gpt4o-deployment",
    );

    let url = adaptor.request_url(&info).unwrap();
    assert_eq!(
        url,
        "https://my-resource.openai.azure.com/openai/deployments/my-gpt4o-deployment/chat/completions?api-version=2024-10-21"
    );

    let mut headers = HeaderMap::new();
    adaptor.setup_headers(&mut headers, &info).unwrap();
    assert_eq!(header(&headers, "api-key").as_deref(), Some("SECRET"));
    assert!(header(&headers, "authorization").is_none());
}

#[test]
fn azure_honours_key_pinned_api_version() {
    let adaptor = azure::AzureAdaptor;
    let mut info = info(
        "https://r.openai.azure.com",
        "/v1/chat/completions",
        "dep",
    );
    info.credential_raw = "SECRET|2025-01-01-preview".to_string();
    assert!(adaptor
        .request_url(&info)
        .unwrap()
        .ends_with("api-version=2025-01-01-preview"));
}

#[test]
fn azure_respects_pinned_deployment_path() {
    // When the operator pins the deployment in base_url we must not add a second.
    let adaptor = azure::AzureAdaptor;
    let info = info(
        "https://r.openai.azure.com/openai/deployments/pinned",
        "/v1/chat/completions",
        "ignored",
    );
    let url = adaptor.request_url(&info).unwrap();
    assert_eq!(
        url,
        "https://r.openai.azure.com/openai/deployments/pinned/chat/completions?api-version=2024-10-21"
    );
}

#[test]
fn bedrock_parses_region_from_credential_and_endpoint() {
    let mut info = info(
        "https://bedrock-runtime.eu-west-1.amazonaws.com",
        "/v1/chat/completions",
        "claude-sonnet-4-20250514",
    );
    // Credential region wins over the endpoint.
    info.credential_raw = "AK|SK|ap-southeast-2".to_string();
    assert_eq!(bedrock::bedrock_region(&info), "ap-southeast-2");

    // Falls back to the endpoint when the credential has no region.
    info.credential_raw = "AK|SK".to_string();
    assert_eq!(bedrock::bedrock_region(&info), "eu-west-1");

    // Falls back to the default when neither carries one.
    info.base_url = "https://bedrock-runtime.example.com".to_string();
    assert_eq!(bedrock::bedrock_region(&info), "us-east-1");
}

#[test]
fn bedrock_builds_model_invoke_url() {
    let adaptor = bedrock::BedrockAdaptor;
    let mut info = info(
        "https://bedrock-runtime.us-east-1.amazonaws.com",
        "/v1/chat/completions",
        "claude-sonnet-4-20250514",
    );
    info.credential_raw = "AK|SK|us-east-1".to_string();

    let url = adaptor.request_url(&info).unwrap();
    assert!(
        url.contains("/model/us.anthropic.claude-sonnet-4-20250514:0/invoke"),
        "unexpected bedrock url: {url}"
    );

    info.is_stream = true;
    assert!(adaptor
        .request_url(&info)
        .unwrap()
        .ends_with("/invoke-with-response-stream"));
}

#[test]
fn bedrock_aksk_selects_sigv4_signing_path() {
    let mut info = info("https://bedrock-runtime.us-east-1.amazonaws.com", "/v1/chat/completions", "m");
    info.credential_raw = "AKID|SECRETKEY|us-east-1".to_string();

    match bedrock::parse_credential(&info.credential_raw, "us-east-1") {
        bedrock::BedrockCredential::AkSk(c) => {
            assert_eq!(c.access_key, "AKID");
            assert_eq!(c.secret_key, "SECRETKEY");
            assert_eq!(c.region, "us-east-1");
        }
        other => panic!("expected AkSk, got {other:?}"),
    }

    // Signing must mutate the headers, proving the SigV4 path is reachable.
    let adaptor = bedrock::BedrockAdaptor;
    let url = adaptor.request_url(&info).unwrap();
    let mut headers = HeaderMap::new();
    adaptor.setup_headers(&mut headers, &info).unwrap();
    assert!(header(&headers, "authorization").is_none());
    adaptor
        .sign_request(&info, &url, &mut headers, b"{}")
        .unwrap();
    assert!(header(&headers, "authorization")
        .unwrap()
        .starts_with("AWS4-HMAC-SHA256"));
    assert!(header(&headers, "x-amz-content-sha256").is_some());
}

#[test]
fn bedrock_api_key_mode_uses_bearer() {
    let adaptor = bedrock::BedrockAdaptor;
    let mut info = info("https://bedrock-runtime.us-east-1.amazonaws.com", "/v1/chat/completions", "m");
    info.credential_raw = "BEDROCK_TOKEN|us-east-1".to_string();

    let mut headers = HeaderMap::new();
    adaptor.setup_headers(&mut headers, &info).unwrap();
    assert_eq!(
        header(&headers, "authorization").as_deref(),
        Some("Bearer BEDROCK_TOKEN")
    );
}

#[test]
fn bedrock_rejects_missing_credential() {
    let adaptor = bedrock::BedrockAdaptor;
    let mut info = info("https://bedrock-runtime.us-east-1.amazonaws.com", "/v1/chat/completions", "m");
    // An empty credential is the only genuinely unusable case.
    info.credential_raw.clear();
    info.api_key.clear();
    let mut headers = HeaderMap::new();
    assert!(adaptor.setup_headers(&mut headers, &info).is_err());
}

#[test]
fn bedrock_bare_token_is_treated_as_api_key_mode() {
    // A single un-piped secret is a bearer token, not a malformed AKSK triple.
    match bedrock::parse_credential("ONLYTOKEN", "us-west-2") {
        bedrock::BedrockCredential::ApiKey { token, region } => {
            assert_eq!(token, "ONLYTOKEN");
            assert_eq!(region, "us-west-2");
        }
        other => panic!("expected ApiKey, got {other:?}"),
    }
}

#[test]
fn bedrock_two_part_credential_is_api_key_with_region() {
    match bedrock::parse_credential("TOKEN|eu-central-1", "us-east-1") {
        bedrock::BedrockCredential::ApiKey { token, region } => {
            assert_eq!(token, "TOKEN");
            assert_eq!(region, "eu-central-1");
        }
        other => panic!("expected ApiKey, got {other:?}"),
    }
}

#[test]
fn vertex_parses_compound_credential() {
    let cred = vertex::parse_credential("my-project|europe-west4|ya29.TOKEN");
    assert_eq!(cred.project, "my-project");
    assert_eq!(cred.location, "europe-west4");
    assert_eq!(cred.access_token, "ya29.TOKEN");
}

#[test]
fn vertex_builds_publisher_model_url_and_bearer() {
    let adaptor = vertex::VertexAdaptor;
    let mut info = info("", "/v1/chat/completions", "gemini-2.0-flash");
    info.credential_raw = "proj|us-central1|ya29.TOKEN".to_string();
    info.api_key = info.credential_raw.clone();

    assert_eq!(
        adaptor.request_url(&info).unwrap(),
        "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.0-flash:generateContent"
    );

    let mut headers = HeaderMap::new();
    adaptor.setup_headers(&mut headers, &info).unwrap();
    assert_eq!(
        header(&headers, "authorization").as_deref(),
        Some("Bearer ya29.TOKEN")
    );
}

#[test]
fn vertex_derives_regional_host_from_location() {
    let adaptor = vertex::VertexAdaptor;
    let mut info = info("", "/v1/chat/completions", "gemini-2.0-flash");
    info.credential_raw = "proj|asia-northeast1|tok".to_string();
    info.api_key = info.credential_raw.clone();
    assert!(adaptor
        .request_url(&info)
        .unwrap()
        .starts_with("https://asia-northeast1-aiplatform.googleapis.com/"));
}

#[test]
fn vertex_rejects_token_only_credential() {
    // A bare token cannot address a project; we must fail loudly, not guess.
    let adaptor = vertex::VertexAdaptor;
    let mut info = info("", "/v1/chat/completions", "gemini-2.0-flash");
    info.credential_raw = "just-a-token".to_string();
    info.api_key = info.credential_raw.clone();
    assert!(adaptor.request_url(&info).is_err());
}

#[test]
fn advanced_custom_interpolates_model_and_action() {
    let adaptor = oxygenrouter_relay::adapters::advanced_custom::AdvancedCustomAdaptor;
    let info = info(
        "https://host/{model}/v1/{action}",
        "/v1/chat/completions",
        "my-model",
    );
    assert_eq!(
        adaptor.request_url(&info).unwrap(),
        "https://host/my-model/v1/v1/chat/completions"
    );
}
