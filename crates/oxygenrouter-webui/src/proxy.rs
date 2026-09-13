//! OpenAI-compatible proxy endpoints
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use chrono::Utc;
use std::time::Instant;
use uuid::Uuid;

use crate::state::AppState;
use oxygenrouter_core::{ApiKey, ApiKeyResolutionError, RequestLog};
use oxygenrouter_proxy::{ModelRouter, ProxyRequest};

/// The set of OpenAI/Anthropic-style endpoints this proxy implements.
/// Returned in 404 responses to help SDKs auto-discover.
const SUPPORTED_ENDPOINTS: &[&str] = &[
    "POST /v1/chat/completions",
    "POST /v1/completions",
    "POST /v1/embeddings",
    "POST /v1/responses",
    "POST /v1/messages",
    "POST /v1/rerank",
    "POST /v1/audio/speech",
    "POST /v1/audio/transcriptions",
    "POST /v1/audio/translations",
    "POST /v1/images/generations",
    "POST /v1/images/edits",
    "POST /v1/images/variations",
    "GET  /v1/models",
];

pub fn router(state: std::sync::Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/chat/completions", any(chat_completions))
        .route("/v1/completions", any(text_completions))
        .route("/v1/embeddings", any(embeddings))
        .route("/v1/responses", any(responses_endpoint))
        .route("/v1/messages", any(messages_endpoint))
        .route("/v1/rerank", any(rerank_endpoint))
        .route("/v1/audio/speech", any(audio_speech))
        .route("/v1/audio/transcriptions", any(audio_transcription))
        .route("/v1/audio/translations", any(audio_translation))
        .route("/v1/models", any(list_models))
        .route("/v1/images/generations", any(image_generations))
        .route("/v1/images/edits", any(image_edits))
        .route("/v1/images/variations", any(image_variations))
        .with_state(state)
}

pub async fn fallback_handler(request: Request) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let body = serde_json::json!({
        "error": {
            "message": format!(
                "endpoint not found: {method} {path}. See `known_endpoints` for the list of OpenAI-compatible routes this proxy implements."
            ),
            "type": "not_found_error",
            "param": path,
            "code": "endpoint_not_found",
        },
        "known_endpoints": SUPPORTED_ENDPOINTS,
    });
    (
        StatusCode::NOT_FOUND,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_default(),
    )
        .into_response()
}

fn is_stream_request(headers: &axum::http::HeaderMap, body: &serde_json::Value) -> bool {
    if headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("text/event-stream"))
        .unwrap_or(false)
    {
        return true;
    }
    body.get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    json_policy_error(status, "upstream_error", msg)
}
fn json_policy_error(status: StatusCode, code: &str, msg: &str) -> Response {
    let body = serde_json::json!({"error": {"message": msg, "type": "invalid_request_error", "code": code}});
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap_or_default()))
        .unwrap_or_else(|_| error_500())
}

fn remote_ip(headers: &axum::http::HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .unwrap_or("")
        .to_string()
}
fn policy_error(error: ApiKeyResolutionError) -> (StatusCode, &'static str, &'static str) {
    match error {
        ApiKeyResolutionError::Invalid | ApiKeyResolutionError::Disabled => (
            StatusCode::UNAUTHORIZED,
            "invalid_api_key",
            "invalid or disabled API key",
        ),
        ApiKeyResolutionError::Expired => (
            StatusCode::UNAUTHORIZED,
            "api_key_expired",
            "API key has expired",
        ),
        ApiKeyResolutionError::ModelDisallowed => (
            StatusCode::FORBIDDEN,
            "model_disallowed",
            "API key is not allowed to use this model",
        ),
        ApiKeyResolutionError::IpDisallowed => (
            StatusCode::FORBIDDEN,
            "ip_disallowed",
            "API key is not allowed from this IP address",
        ),
        ApiKeyResolutionError::QuotaExceeded => (
            StatusCode::TOO_MANY_REQUESTS,
            "quota_exceeded",
            "API key quota has been exceeded",
        ),
    }
}
fn authorize(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    model: &str,
    path: &str,
    start: Instant,
) -> Result<Option<ApiKey>, Response> {
    if !state.db.has_active_api_keys().unwrap_or(false) {
        return Ok(None);
    }
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        });
    let result = token
        .ok_or(ApiKeyResolutionError::Invalid)
        .and_then(|token| state.db.resolve_api_key(token, model, &remote_ip(headers)));
    match result {
        Ok(key) => match state.db.record_api_key_usage(&key.id, 0) {
            Ok(()) => Ok(Some(key)),
            Err(error) => {
                let (status, code, message) = policy_error(error);
                log_request(
                    state,
                    "POST",
                    path,
                    Some(model.to_string()),
                    None,
                    None,
                    Some(status.as_u16()),
                    Some(code.to_string()),
                    start.elapsed().as_millis() as i64,
                );
                Err(json_policy_error(status, code, message))
            }
        },
        Err(error) => {
            let (status, code, message) = policy_error(error);
            log_request(
                state,
                "POST",
                path,
                Some(model.to_string()),
                None,
                None,
                Some(status.as_u16()),
                Some(code.to_string()),
                start.elapsed().as_millis() as i64,
            );
            Err(json_policy_error(status, code, message))
        }
    }
}

fn error_500() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "").into_response()
}

async fn read_body(req: Request) -> (axum::http::HeaderMap, Vec<u8>) {
    let headers = req.headers().clone();
    let body_bytes = axum::body::to_bytes(req.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap_or_default();
    (headers, body_bytes.to_vec())
}

fn log_request(
    state: &AppState,
    method: &str,
    path: &str,
    model: Option<String>,
    channel_id: Option<String>,
    api_key_id: Option<String>,
    status_code: Option<u16>,
    error: Option<String>,
    duration_ms: i64,
) {
    let log = RequestLog {
        id: Uuid::new_v4().to_string(),
        method: method.to_string(),
        path: path.to_string(),
        model,
        channel_id,
        api_key_id,
        status_code,
        error,
        tokens_used: None,
        duration_ms,
        created_at: Utc::now(),
    };
    let _ = state.db.insert_request_log(&log);
    let _ = state.log_broadcast.send(log);
}

#[axum::debug_handler(state = std::sync::Arc<AppState>)]
async fn chat_completions(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let start = Instant::now();
    let (headers, body_bytes) = read_body(request).await;

    let parsed: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(_) => return json_error(StatusCode::BAD_REQUEST, "invalid JSON body"),
    };

    let stream = is_stream_request(&headers, &parsed);

    let router = ModelRouter::new();
    let decision = router.resolve_auto(&parsed, "/v1/chat/completions");
    let key = match authorize(
        &state,
        &headers,
        &decision.resolved_model,
        "/v1/chat/completions",
        start,
    ) {
        Ok(key) => key,
        Err(response) => return response,
    };

    let proxy_req = ProxyRequest {
        method: "POST".to_string(),
        path: "/v1/chat/completions".to_string(),
        headers: vec![],
        body: Some(body_bytes),
        model: decision.resolved_model.clone(),
        stream,
    };

    let scheduler = state.scheduler.read().await;
    let result = scheduler
        .dispatch_for_group(
            &proxy_req,
            &decision.resolved_model,
            key.as_ref().map(|k| k.group_name.as_str()),
            key.as_ref().map(|k| k.cross_group_retry).unwrap_or(true),
        )
        .await;
    drop(scheduler);

    let duration_ms = start.elapsed().as_millis() as i64;
    match result {
        Ok(r) => {
            log_request(
                &state,
                "POST",
                "/v1/chat/completions",
                Some(decision.resolved_model.clone()),
                Some(r.channel_id.clone()),
                key.as_ref().map(|k| k.id.clone()),
                Some(r.status),
                None,
                duration_ms,
            );
            if stream {
                build_streaming_response(r.body.to_vec(), r.status, &r.headers)
            } else {
                build_buffered_response(r.body.to_vec(), r.status, "application/json", &r.headers)
            }
        }
        Err(e) => {
            let status = e.status();
            log_request(
                &state,
                "POST",
                "/v1/chat/completions",
                Some(decision.resolved_model.clone()),
                None,
                key.as_ref().map(|k| k.id.clone()),
                Some(status),
                Some(e.to_string()),
                duration_ms,
            );
            json_error(
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                &e.to_string(),
            )
        }
    }
}

async fn text_completions(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let start = Instant::now();
    let (headers, body_bytes) = read_body(request).await;
    let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap_or_default();

    let router = ModelRouter::new();
    let decision = router.resolve_auto(&parsed, "/v1/completions");
    let model = decision.resolved_model.clone();
    let key = match authorize(&state, &headers, &model, "/v1/completions", start) {
        Ok(key) => key,
        Err(response) => return response,
    };

    let stream = is_stream_request(&headers, &parsed);
    let proxy_req = ProxyRequest {
        method: "POST".to_string(),
        path: "/v1/completions".to_string(),
        headers: vec![],
        body: Some(body_bytes),
        model: model.clone(),
        stream,
    };
    let scheduler = state.scheduler.read().await;
    let result = scheduler
        .dispatch_for_group(
            &proxy_req,
            &model,
            key.as_ref().map(|k| k.group_name.as_str()),
            key.as_ref().map(|k| k.cross_group_retry).unwrap_or(true),
        )
        .await;
    drop(scheduler);

    let duration_ms = start.elapsed().as_millis() as i64;
    match result {
        Ok(r) => {
            log_request(
                &state,
                "POST",
                "/v1/completions",
                Some(model),
                Some(r.channel_id.clone()),
                key.as_ref().map(|k| k.id.clone()),
                Some(r.status),
                None,
                duration_ms,
            );
            if stream {
                build_streaming_response(r.body.to_vec(), r.status, &r.headers)
            } else {
                build_buffered_response(r.body.to_vec(), r.status, "application/json", &r.headers)
            }
        }
        Err(e) => {
            let status = e.status();
            log_request(
                &state,
                "POST",
                "/v1/completions",
                Some(model),
                None,
                key.as_ref().map(|k| k.id.clone()),
                Some(status),
                Some(e.to_string()),
                duration_ms,
            );
            json_error(
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                &e.to_string(),
            )
        }
    }
}

async fn embeddings(State(state): State<std::sync::Arc<AppState>>, request: Request) -> Response {
    let start = Instant::now();
    let (headers, body_bytes) = read_body(request).await;
    let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap_or_default();

    let router = ModelRouter::new();
    let decision = router.resolve_auto(&parsed, "/v1/embeddings");
    let model = decision.resolved_model.clone();
    let key = match authorize(&state, &headers, &model, "/v1/embeddings", start) {
        Ok(key) => key,
        Err(response) => return response,
    };

    let proxy_req = ProxyRequest {
        method: "POST".to_string(),
        path: "/v1/embeddings".to_string(),
        headers: vec![],
        body: Some(body_bytes),
        model: model.clone(),
        stream: false,
    };
    let scheduler = state.scheduler.read().await;
    let result = scheduler
        .dispatch_for_group(
            &proxy_req,
            &model,
            key.as_ref().map(|k| k.group_name.as_str()),
            key.as_ref().map(|k| k.cross_group_retry).unwrap_or(true),
        )
        .await;
    drop(scheduler);

    let duration_ms = start.elapsed().as_millis() as i64;
    match result {
        Ok(r) => {
            log_request(
                &state,
                "POST",
                "/v1/embeddings",
                Some(model),
                Some(r.channel_id.clone()),
                key.as_ref().map(|k| k.id.clone()),
                Some(r.status),
                None,
                duration_ms,
            );
            build_buffered_response(r.body.to_vec(), r.status, "application/json", &r.headers)
        }
        Err(e) => {
            let status = e.status();
            log_request(
                &state,
                "POST",
                "/v1/embeddings",
                Some(model),
                None,
                key.as_ref().map(|k| k.id.clone()),
                Some(status),
                Some(e.to_string()),
                duration_ms,
            );
            json_error(
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                &e.to_string(),
            )
        }
    }
}

async fn image_generations(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let start = Instant::now();
    let (headers, body_bytes) = read_body(request).await;
    let parsed: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap_or_default();

    let router = ModelRouter::new();
    let decision = router.resolve_auto(&parsed, "/v1/images/generations");
    let model = decision.resolved_model.clone();
    let key = match authorize(&state, &headers, &model, "/v1/images/generations", start) {
        Ok(key) => key,
        Err(response) => return response,
    };

    let proxy_req = ProxyRequest {
        method: "POST".to_string(),
        path: "/v1/images/generations".to_string(),
        headers: vec![],
        body: Some(body_bytes),
        model: model.clone(),
        stream: false,
    };
    let scheduler = state.scheduler.read().await;
    let result = scheduler
        .dispatch_for_group(
            &proxy_req,
            &model,
            key.as_ref().map(|k| k.group_name.as_str()),
            key.as_ref().map(|k| k.cross_group_retry).unwrap_or(true),
        )
        .await;
    drop(scheduler);

    let duration_ms = start.elapsed().as_millis() as i64;
    match result {
        Ok(r) => {
            log_request(
                &state,
                "POST",
                "/v1/images/generations",
                Some(model),
                Some(r.channel_id.clone()),
                key.as_ref().map(|k| k.id.clone()),
                Some(r.status),
                None,
                duration_ms,
            );
            build_buffered_response(r.body.to_vec(), r.status, "application/json", &r.headers)
        }
        Err(e) => {
            let status = e.status();
            log_request(
                &state,
                "POST",
                "/v1/images/generations",
                Some(model),
                None,
                key.as_ref().map(|k| k.id.clone()),
                Some(status),
                Some(e.to_string()),
                duration_ms,
            );
            json_error(
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                &e.to_string(),
            )
        }
    }
}

fn add_response_headers(
    mut builder: axum::http::response::Builder,
    headers: &[(String, String)],
) -> axum::http::response::Builder {
    for (name, value) in headers {
        if !name.eq_ignore_ascii_case("content-length")
            && !name.eq_ignore_ascii_case("transfer-encoding")
        {
            builder = builder.header(name, value);
        }
    }
    builder
}

fn build_buffered_response(
    body: Vec<u8>,
    status: u16,
    content_type: &str,
    headers: &[(String, String)],
) -> Response {
    add_response_headers(
        Response::builder()
            .status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK))
            .header("content-type", content_type),
        headers,
    )
    .body(Body::from(body))
    .unwrap_or_else(|_| error_500())
}

fn build_streaming_response(body: Vec<u8>, status: u16, headers: &[(String, String)]) -> Response {
    let bytes_per_chunk = 4096;
    let chunks: Vec<Result<axum::body::Bytes, std::io::Error>> = body
        .chunks(bytes_per_chunk)
        .map(|c| Ok(axum::body::Bytes::copy_from_slice(c)))
        .collect();

    let stream = futures_util::stream::iter(chunks);
    add_response_headers(
        Response::builder()
            .status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK))
            .header("content-type", "text/event-stream; charset=utf-8")
            .header("cache-control", "no-cache")
            .header("x-accel-buffering", "no"),
        headers,
    )
    .body(Body::from_stream(stream))
    .unwrap_or_else(|_| error_500())
}

async fn list_models() -> Response {
    let models = serde_json::json!({
        "object": "list",
        "data": [
            {"id": "gpt-3.5-turbo", "object": "model", "created": 1677610602, "owned_by": "openai"},
            {"id": "gpt-3.5-turbo-16k", "object": "model", "created": 1683758182, "owned_by": "openai"},
            {"id": "gpt-4", "object": "model", "created": 1687882411, "owned_by": "openai"},
            {"id": "gpt-4-32k", "object": "model", "created": 1687882411, "owned_by": "openai"},
            {"id": "gpt-4o", "object": "model", "created": 1715721543, "owned_by": "openai"},
            {"id": "gpt-4o-mini", "object": "model", "created": 1720201543, "owned_by": "openai"},
            {"id": "o1-preview", "object": "model", "created": 1724710400, "owned_by": "openai"},
            {"id": "o1-mini", "object": "model", "created": 1724710400, "owned_by": "openai"},
            {"id": "dall-e-3", "object": "model", "created": 1698785189, "owned_by": "openai"},
            {"id": "tts-1", "object": "model", "created": 1699094024, "owned_by": "openai"},
            {"id": "tts-1-hd", "object": "model", "created": 1699094024, "owned_by": "openai"},
            {"id": "whisper-1", "object": "model", "created": 1677532384, "owned_by": "openai"},
            {"id": "text-embedding-3-small", "object": "model", "created": 1705949951, "owned_by": "openai"},
            {"id": "text-embedding-3-large", "object": "model", "created": 1705949951, "owned_by": "openai"},
            {"id": "claude-3-haiku", "object": "model", "created": 1708000000, "owned_by": "anthropic"},
            {"id": "claude-3-sonnet", "object": "model", "created": 1708000000, "owned_by": "anthropic"},
            {"id": "claude-3-opus", "object": "model", "created": 1708000000, "owned_by": "anthropic"},
            {"id": "auto", "object": "model", "created": 1700000000, "owned_by": "system"}
        ]
    });
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        serde_json::to_string(&models).unwrap_or_default(),
    )
        .into_response()
}

/// Shared dispatch helper for the OpenAI-style proxy endpoints.
/// Picks a model from the request body (or uses `default_fallback_model`),
/// runs the upstream call, and writes a request log entry.
async fn dispatch_openai(
    state: std::sync::Arc<AppState>,
    path: &str,
    body_bytes: Vec<u8>,
    default_fallback_model: &str,
    headers: axum::http::HeaderMap,
) -> Response {
    let start = Instant::now();
    let parsed: serde_json::Value =
        serde_json::from_slice(&body_bytes).unwrap_or_else(|_| serde_json::json!({}));

    let router = ModelRouter::new();
    let decision = router.resolve_auto(&parsed, path);
    let model_from_body = parsed
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let model = match model_from_body {
        Some(value) if !value.trim().is_empty() && !value.eq_ignore_ascii_case("auto") => value,
        _ => decision.resolved_model.clone(),
    };
    if model.is_empty() {
        let fallback = default_fallback_model.to_string();
        let key = match authorize(&state, &headers, &fallback, path, start) {
            Ok(key) => key,
            Err(response) => return response,
        };
        let proxy_req = ProxyRequest {
            method: "POST".to_string(),
            path: path.to_string(),
            headers: vec![],
            body: Some(body_bytes),
            model: fallback.clone(),
            stream: false,
        };
        return dispatch(state, path, fallback, proxy_req, start, headers, key).await;
    }
    let key = match authorize(&state, &headers, &model, path, start) {
        Ok(key) => key,
        Err(response) => return response,
    };
    let proxy_req = ProxyRequest {
        method: "POST".to_string(),
        path: path.to_string(),
        headers: vec![],
        body: Some(body_bytes),
        model: model.clone(),
        stream: false,
    };
    dispatch(state, path, model, proxy_req, start, headers, key).await
}

async fn dispatch(
    state: std::sync::Arc<AppState>,
    path: &str,
    model: String,
    proxy_req: ProxyRequest,
    start: std::time::Instant,
    _headers: axum::http::HeaderMap,
    api_key: Option<ApiKey>,
) -> Response {
    let scheduler = state.scheduler.read().await;
    let result = scheduler
        .dispatch_for_group(
            &proxy_req,
            &model,
            api_key.as_ref().map(|k| k.group_name.as_str()),
            api_key
                .as_ref()
                .map(|k| k.cross_group_retry)
                .unwrap_or(true),
        )
        .await;
    drop(scheduler);
    let duration_ms = start.elapsed().as_millis() as i64;
    match result {
        Ok(r) => {
            log_request(
                &state,
                "POST",
                path,
                Some(model),
                Some(r.channel_id.clone()),
                api_key.as_ref().map(|k| k.id.clone()),
                Some(r.status),
                None,
                duration_ms,
            );
            build_buffered_response(r.body.to_vec(), r.status, "application/json", &r.headers)
        }
        Err(e) => {
            let status = e.status();
            log_request(
                &state,
                "POST",
                path,
                Some(model),
                None,
                api_key.as_ref().map(|k| k.id.clone()),
                Some(status),
                Some(e.to_string()),
                duration_ms,
            );
            json_error(
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                &e.to_string(),
            )
        }
    }
}

async fn responses_endpoint(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    dispatch_openai(state, "/v1/responses", body_bytes, "gpt-4o-mini", headers).await
}

async fn messages_endpoint(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    dispatch_openai(state, "/v1/messages", body_bytes, "claude-3-haiku", headers).await
}

async fn rerank_endpoint(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    dispatch_openai(
        state,
        "/v1/rerank",
        body_bytes,
        "rerank-english-v3.0",
        headers,
    )
    .await
}

async fn audio_speech(State(state): State<std::sync::Arc<AppState>>, request: Request) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    dispatch_openai(state, "/v1/audio/speech", body_bytes, "tts-1", headers).await
}

async fn audio_transcription(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    dispatch_openai(
        state,
        "/v1/audio/transcriptions",
        body_bytes,
        "whisper-1",
        headers,
    )
    .await
}

async fn audio_translation(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    dispatch_openai(
        state,
        "/v1/audio/translations",
        body_bytes,
        "whisper-1",
        headers,
    )
    .await
}

async fn image_edits(State(state): State<std::sync::Arc<AppState>>, request: Request) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    dispatch_openai(state, "/v1/images/edits", body_bytes, "dall-e-3", headers).await
}

async fn image_variations(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    dispatch_openai(
        state,
        "/v1/images/variations",
        body_bytes,
        "dall-e-3",
        headers,
    )
    .await
}
