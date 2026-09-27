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
use oxygenrouter_billing::{BillingUsage, Charge, EvalContext};

/// The set of OpenAI/Anthropic-style endpoints this proxy implements.
/// Returned in 404 responses to help SDKs auto-discover.
const SUPPORTED_ENDPOINTS: &[&str] = &[
    // OpenAI text and multimodal
    "POST /v1/chat/completions",
    "POST /v1/completions",
    "POST /v1/embeddings",
    "POST /v1/moderations",
    "POST /v1/edits",
    "POST /v1/rerank",
    "POST /v1/engines/:model/embeddings",
    // Responses and Anthropic
    "POST /v1/responses",
    "POST /v1/responses/compact",
    "POST /v1/alpha/search",
    "POST /v1/messages",
    "POST /v1/messages/count_tokens",
    // Gemini native inbound
    "POST /v1beta/models/*path",
    // Audio
    "POST /v1/audio/speech",
    "POST /v1/audio/transcriptions",
    "POST /v1/audio/translations",
    // Images
    "POST /v1/images/generations",
    "POST /v1/images/edits",
    "POST /v1/images/variations",
    // Fine-tuning
    "GET  /v1/fine-tunes",
    "POST /v1/fine-tunes",
    "GET  /v1/fine-tunes/:id",
    "POST /v1/fine-tunes/:id/cancel",
    "GET  /v1/fine-tunes/:id/events",
    // Files
    "GET  /v1/files",
    "POST /v1/files",
    "GET  /v1/files/:id",
    "DELETE /v1/files/:id",
    "GET  /v1/files/:id/content",
    // Batches
    "GET  /v1/batches",
    "POST /v1/batches",
    "GET  /v1/batches/:id",
    "POST /v1/batches/:id/cancel",
    // Models
    "GET  /v1/models",
    "GET  /v1/models/:model",
    "DELETE /v1/models/:model",
    "GET  /v1beta/models",
    "GET  /v1beta/openai/models",
];

pub fn router(state: std::sync::Arc<AppState>) -> Router {
    Router::new()
        // --- OpenAI text and multimodal ------------------------------------
        .route("/v1/chat/completions", any(chat_completions))
        .route("/v1/completions", any(text_completions))
        .route("/v1/embeddings", any(embeddings))
        .route("/v1/moderations", any(moderations_endpoint))
        .route("/v1/edits", any(edits_endpoint))
        .route("/v1/rerank", any(rerank_endpoint))
        // Legacy engine-shaped embedding path some SDKs still call.
        .route("/v1/engines/:model/embeddings", any(engines_embeddings))
        // --- Responses, Anthropic and the Responses-adjacent helpers -------
        .route("/v1/responses", any(responses_endpoint))
        .route("/v1/responses/compact", any(responses_compact_endpoint))
        .route("/v1/alpha/search", any(alpha_search_endpoint))
        .route("/v1/messages", any(messages_endpoint))
        .route("/v1/messages/count_tokens", any(count_tokens_endpoint))
        // --- Google Gemini native inbound ---------------------------------
        // A client that speaks Gemini posts to /v1beta/models/<model>:<verb>.
        .route("/v1beta/models/*path", any(gemini_inbound))
        // Service discovery: the Gemini client asks for the catalogue here, so
        // without this route a Gemini SDK cannot list models at all.
        .route("/v1beta/models", any(list_models_gemini))
        // The OpenAI-compatible alias the reference also serves.
        .route("/v1beta/openai/models", any(list_models))
        // --- Audio ---------------------------------------------------------
        .route("/v1/audio/speech", any(audio_speech))
        .route("/v1/audio/transcriptions", any(audio_transcription))
        .route("/v1/audio/translations", any(audio_translation))
        // --- Images --------------------------------------------------------
        .route("/v1/images/generations", any(image_generations))
        .route("/v1/images/edits", any(image_edits))
        .route("/v1/images/variations", any(image_variations))
        // --- Fine-tuning ---------------------------------------------------
        .route("/v1/fine-tunes", any(fine_tunes_list_or_create))
        .route("/v1/fine-tunes/:id", any(fine_tunes_get))
        .route("/v1/fine-tunes/:id/cancel", any(fine_tunes_cancel))
        .route("/v1/fine-tunes/:id/events", any(fine_tunes_events))
        // --- Files ---------------------------------------------------------
        .route("/v1/files", any(files_list_or_upload))
        .route("/v1/files/:id", any(files_get_or_delete))
        .route("/v1/files/:id/content", any(files_content))
        // --- Batches -------------------------------------------------------
        .route("/v1/batches", any(batches_list_or_create))
        .route("/v1/batches/:id", any(batches_get))
        .route("/v1/batches/:id/cancel", any(batches_cancel))
        // --- Models --------------------------------------------------------
        .route("/v1/models", any(list_models))
        .route("/v1/models/:model", any(models_get_or_delete))
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
    // An explicit `stream: false` is the most specific signal a client can give,
    // so it wins over a broad `Accept` header. Without this an SDK that always
    // sends `Accept: text/event-stream` but asked for a buffered reply would be
    // marked streaming, and its JSON body would then be fed to the SSE parser.
    match body.get("stream").and_then(|v| v.as_bool()) {
        Some(false) => return false,
        Some(true) => return true,
        None => {}
    }
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("text/event-stream"))
        .unwrap_or(false)
}

/// Whether a pass-through request asked for a streamed reply.
///
/// Three signals, because the endpoints differ: an OpenAI-family body says
/// `stream: true`, an `Accept: text/event-stream` header covers clients that do
/// not put it in the body, and Gemini selects streaming purely through the
/// `:streamGenerateContent` method name, which appears in neither.
fn passthrough_wants_stream(
    headers: &axum::http::HeaderMap,
    body: &serde_json::Value,
    upstream_path: &str,
) -> bool {
    // Gemini names the streaming method in the URL -- `:streamGenerateContent`
    // versus `:generateContent` -- and says nothing in the body or the headers.
    // That verb is authoritative, so it decides on its own: falling through to
    // the `Accept` header here would turn a client's ordinary
    // `Accept: text/event-stream` into a streamed request for a method that has
    // no streaming form. The verb is matched by name rather than by position so
    // an unrelated colon elsewhere in the path cannot be mistaken for one.
    if let Some((_, verb)) = upstream_path.rsplit('/').next().and_then(|leaf| leaf.split_once(':')) {
        if verb.starts_with("streamGenerateContent") || verb.starts_with("generateContent") {
            return verb.starts_with("streamGenerateContent");
        }
    }
    is_stream_request(headers, body)
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
                    &headers,
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
                &headers,
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
    headers: &axum::http::HeaderMap,
    method: &str,
    path: &str,
    model: Option<String>,
    channel_id: Option<String>,
    api_key_id: Option<String>,
    status_code: Option<u16>,
    error: Option<String>,
    duration_ms: i64,
) {
    log_request_with_usage(
        state,
        &headers,
        method,
        path,
        model,
        channel_id,
        api_key_id,
        status_code,
        error,
        duration_ms,
        None,
    );
}

/// Variant that records the upstream's token usage.
///
/// Kept separate so the dozen call sites that have no usage to report stay
/// unchanged, and so the one path that does have it is explicit.
#[allow(clippy::too_many_arguments)]
fn log_request_with_usage(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    method: &str,
    path: &str,
    model: Option<String>,
    channel_id: Option<String>,
    api_key_id: Option<String>,
    status_code: Option<u16>,
    error: Option<String>,
    duration_ms: i64,
    tokens_used: Option<i64>,
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
        tokens_used,
        duration_ms,
        created_at: Utc::now(),
        client_ip: None,
    };
    // `RecordIpLog` (off by default) decides whether the address is captured at
    // all; the row is still broadcast so the live console view keeps working, but
    // a row that was never permitted to hold an address never has one to leak.
    let client_ip = state.record_ip_log().then(|| remote_ip(headers));
    // `RequestLogEnabled` (on by default) is the master switch for persistence.
    // Analytics and the log tables are built from `request_logs`, so turning this
    // off stops history — which is what the option says it does. The broadcast
    // still fires either way, so the live view degrades instead of breaking.
    if state.request_log_enabled() {
        let _ = state
            .db
            .insert_request_log_with_ip(&log, client_ip.as_deref());
    }
    let _ = state.log_broadcast.send(log);
}

#[axum::debug_handler(state = std::sync::Arc<AppState>)]
async fn chat_completions(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;

    // Parse only to reject a malformed body early and with a clean message;
    // `dispatch_openai` re-parses it and detects streaming itself.
    if let Err(_) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
        return json_error(StatusCode::BAD_REQUEST, "invalid JSON body");
    }
    dispatch_openai(
        state,
        "/v1/chat/completions",
        body_bytes,
        "gpt-3.5-turbo",
        headers,
    )
    .await
}

async fn text_completions(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    dispatch_openai(state, "/v1/completions", body_bytes, "gpt-3.5-turbo", headers).await
}

async fn embeddings(State(state): State<std::sync::Arc<AppState>>, request: Request) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    dispatch_openai(
        state,
        "/v1/embeddings",
        body_bytes,
        "text-embedding-3-small",
        headers,
    )
    .await
}

async fn image_generations(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    dispatch_openai(state, "/v1/images/generations", body_bytes, "dall-e-3", headers).await
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

/// `GET /v1/models` — built from the local database rather than a hard-coded
/// list, so adding a channel immediately makes its models visible to clients.
///
/// Sources, in precedence order:
/// 1. every model declared by an enabled channel (`channels.model_list`)
/// 2. every model known to the registry (`model_metadata`)
///
/// `owned_by` is the channel's provider for source 1, and the registry vendor
/// (falling back to the provider) for source 2. The synthetic `auto` entry is
/// always present because the router accepts it.
/// Collect the catalogue, keeping the endpoint families each model supports.
///
/// Channels are consulted first so a callable model wins over catalogued
/// metadata; metadata then fills in models no enabled channel serves, which is
/// what lets the console show a catalogue before any channel is configured.
fn model_catalogue(state: &AppState) -> Vec<crate::model_list::ModelEntry> {
    use crate::model_list::ModelEntry;
    use std::collections::BTreeMap;

    // The reference stamps every model with the same fixed epoch
    // (`controller.ListModels`); matching it keeps this field stable across
    // listings rather than churning per request.
    const CATALOGUE_EPOCH: i64 = 1626777600;

    let mut entries: BTreeMap<String, ModelEntry> = BTreeMap::new();

    if let Ok(channels) = state.db.get_enabled_channels() {
        for channel in channels {
            let provider = if channel.provider.trim().is_empty() {
                "openai".to_string()
            } else {
                channel.provider.clone()
            };
            for model in channel.model_list {
                let model = model.trim();
                if model.is_empty() {
                    continue;
                }
                entries.entry(model.to_string()).or_insert_with(|| ModelEntry {
                    id: model.to_string(),
                    owned_by: provider.clone(),
                    created: CATALOGUE_EPOCH,
                    display_name: model.to_string(),
                    description: None,
                    endpoints: Vec::new(),
                });
            }
        }
    }

    if let Ok(metadata) = state.db.list_model_metadata() {
        for entry in metadata {
            if entry.id.trim().is_empty() {
                continue;
            }
            let vendor = if entry.vendor.trim().is_empty() {
                "system".to_string()
            } else {
                entry.vendor.clone()
            };
            let display_name = if entry.model_name.trim().is_empty() {
                entry.id.clone()
            } else {
                entry.model_name.clone()
            };
            let description = entry
                .description
                .trim()
                .is_empty()
                .then_some(())
                .map_or(Some(entry.description.clone()), |_| None);
            entries
                .entry(entry.id.clone())
                .and_modify(|existing| {
                    // A channel already claims it, but the registry may still
                    // carry the richer endpoint list and description.
                    if !entry.endpoints.is_empty() {
                        existing.endpoints = entry.endpoints.clone();
                    }
                    if existing.description.is_none() {
                        existing.description = description.clone();
                    }
                })
                .or_insert_with(|| ModelEntry {
                    id: entry.id.clone(),
                    owned_by: vendor,
                    created: CATALOGUE_EPOCH,
                    display_name,
                    description,
                    endpoints: entry.endpoints.clone(),
                });
        }
    }

    // `auto` is our routing heuristic, not a model anyone serves, so it is
    // advertised alongside the real catalogue.
    entries
        .entry("auto".to_string())
        .or_insert_with(|| ModelEntry {
            id: "auto".to_string(),
            owned_by: "system".to_string(),
            created: CATALOGUE_EPOCH,
            display_name: "auto".to_string(),
            description: Some("Automatic model routing".to_string()),
            endpoints: vec!["openai".to_string()],
        });

    entries.into_values().collect()
}

/// `GET /v1/models`, in the dialect the client's credentials imply.
async fn list_models(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let query = request.uri().query().unwrap_or("").to_string();
    let query_key = query.split('&').find_map(|pair| {
        pair.split_once('=')
            .filter(|(k, _)| *k == "key")
            .map(|(_, v)| v.to_string())
    });
    let dialect =
        crate::model_list::dialect_from_headers(request.headers(), query_key.as_deref());

    let catalogue = model_catalogue(&state);
    let payload = crate::model_list::render(&catalogue, dialect);
    json_ok(payload)
}

/// A 200 JSON response.
fn json_ok(payload: serde_json::Value) -> Response {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        serde_json::to_string(&payload).unwrap_or_default(),
    )
        .into_response()
}

/// `GET /v1beta/models` — the Gemini client's service-discovery endpoint.
///
/// Gemini shape is implied by the route itself, so the credential headers are not
/// inspected: a client that reaches this path wants Gemini shape even if it
/// authenticated with a bearer token.
async fn list_models_gemini(State(state): State<std::sync::Arc<AppState>>) -> Response {
    let catalogue = model_catalogue(&state);
    json_ok(crate::model_list::render(
        &catalogue,
        crate::model_list::ModelListDialect::Gemini,
    ))
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

    // Streaming is detected here rather than passed in, because a caller that
    // forgot to forward `stream: true` would send a non-streaming upstream
    // request whose SSE body the response converter then fails to parse.
    let stream = is_stream_request(&headers, &parsed);

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
            stream,
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
        stream,
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
    let stream = proxy_req.stream;

    // --- rate limit and concurrency gate ----------------------------------
    //
    // Both are checked before any upstream work, so a rejected request costs
    // nothing. The permit is held for the whole dispatch and released on drop,
    // including on every error path below.
    if state.relay_rate_limit.is_enabled() {
        let scope = format!("relay:{}", client_scope(&_headers));
        if let Err(error) = state.rate_limiter.check(&scope, state.relay_rate_limit) {
            log_request(
                &state,
                &_headers,
                "POST",
                path,
                Some(model.clone()),
                None,
                api_key.as_ref().map(|k| k.id.clone()),
                Some(error.status_code()),
                Some("rate limited".to_string()),
                start.elapsed().as_millis() as i64,
            );
            return limit_error_response(error);
        }
    }
    let _permit = match state.concurrency.try_acquire() {
        Ok(permit) => permit,
        Err(error) => {
            log_request(
                &state,
                &_headers,
                "POST",
                path,
                Some(model.clone()),
                None,
                api_key.as_ref().map(|k| k.id.clone()),
                Some(error.status_code()),
                Some("concurrency ceiling reached".to_string()),
                start.elapsed().as_millis() as i64,
            );
            return limit_error_response(error);
        }
    };

    // --- reserve before spending an upstream call -------------------------
    //
    // The reservation is deliberately keyed on the *client-supplied* body, not
    // the translated one, because the estimate only needs to be conservative.
    // A key with no owning user is not wallet-backed, so it is not billed here
    // (channel-only deployments keep working unchanged).
    let reservation = match billing_target(&state, &api_key) {
        Some((key_id, user_id)) => {
            let body: serde_json::Value = proxy_req
                .body
                .as_deref()
                .and_then(|b| serde_json::from_slice(b).ok())
                .unwrap_or_else(|| serde_json::json!({}));
            let group = api_key
                .as_ref()
                .map(|k| k.group_name.as_str())
                .unwrap_or("default");
            let amount = state
                .billing
                .reservation(&model, group, &body, path);
            // Prefer a subscription's pool when one can fund this request, and
            // fall back to the wallet otherwise. The reference makes the same
            // choice through its `FundingSource` abstraction
            // (`service/funding_source.go`); a subscription's quota is only
            // spendable while it is active, which the lookup enforces.
            let funding = match state
                .db
                .subscription_funding_source(&user_id, amount)
            {
                Ok(Some(subscription)) => {
                    oxygenrouter_billing::FundingSource::Subscription {
                        user_id: user_id.clone(),
                        subscription_id: subscription.id,
                    }
                }
                // No usable pool: the wallet pays, as it always has.
                Ok(None) => oxygenrouter_billing::FundingSource::Wallet {
                    user_id: user_id.clone(),
                },
                Err(error) => {
                    // A lookup failure must not silently change who pays, but it
                    // also must not block the request: fall back to the wallet,
                    // which is the pre-existing behaviour.
                    eprintln!(
                        "[OxygenRouter] subscription lookup failed for user {user_id} ({error}); billing the wallet"
                    );
                    oxygenrouter_billing::FundingSource::Wallet {
                        user_id: user_id.clone(),
                    }
                }
            };
            match state.billing.begin(
                state.billing_store.as_ref(),
                &key_id,
                &funding,
                false,
                amount,
            ) {
                Ok((session, reserved)) => {
                    Some((session, reserved, key_id, user_id, group.to_string(), funding))
                }
                Err(error) => {
                    // A refused reservation is the client's condition, not a
                    // server fault. NewAPI answers insufficient quota with
                    // 403 Forbidden (billing_session.go), not 429 -- 429 is
                    // reserved for rate limiting.
                    let status = StatusCode::FORBIDDEN;
                    log_request(
                        &state,
                        &_headers,
                        "POST",
                        path,
                        Some(model.clone()),
                        None,
                        Some(key_id),
                        Some(status.as_u16()),
                        Some(error.to_string()),
                        start.elapsed().as_millis() as i64,
                    );
                    return json_policy_error(status, "insufficient_quota", &error.to_string());
                }
            }
        }
        None => None,
    };

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
            let total_tokens = r.usage.total_tokens;
            let tokens_used = if total_tokens > 0 {
                Some(total_tokens)
            } else {
                None
            };

            // Settle at what the upstream actually reported. An upstream that
            // reports nothing is billed on the reservation estimate, matching
            // NewAPI's "keep the estimate" behaviour rather than charging zero.
            if let Some((session, reserved, key_id, user_id, group, funding)) = &reservation {
                let usage = billing_usage_from(&r.usage, &model, path);
                let ctx = EvalContext::default();
                let charge = state.billing.charge(&model, group, &usage, &ctx);
                let charge = if charge.quota == 0 && *reserved > 0 && !usage.has_billable_tokens() {
                    // No usage reported: hold the reservation.
                    Charge {
                        quota: *reserved,
                        ..charge
                    }
                } else {
                    charge
                };
                let description = format!(
                    "{} via {} ({})",
                    model,
                    r.adaptor,
                    charge.path.as_str()
                );
                if let Err(error) = state.billing.settle(
                    state.billing_store.as_ref(),
                    session,
                    key_id,
                    user_id,
                    &charge,
                    &description,
                    None,
                    funding,
                ) {
                    eprintln!("[OxygenRouter] billing settle failed: {error}");
                }
            }

            // A failover is recorded on the log row so a retried request is
            // auditable; the column holds the winning channel and this holds
            // the trail that preceded it.
            let failover_note = if r.failed_channels.is_empty() {
                None
            } else {
                Some(format!(
                    "failed over: {}",
                    r.failed_channels.join(" -> ")
                ))
            };
            log_request_with_usage(
                &state,
                &_headers,
                "POST",
                path,
                Some(model),
                Some(r.channel_id.clone()),
                api_key.as_ref().map(|k| k.id.clone()),
                Some(r.status),
                failover_note,
                duration_ms,
                tokens_used,
            );
            if stream {
                build_streaming_response(r.body.to_vec(), r.status, &r.headers)
            } else {
                build_buffered_response(r.body.to_vec(), r.status, "application/json", &r.headers)
            }
        }
        Err(e) => {
            let status = e.status();
            // A failed request must cost nothing: return the reservation.
            if let Some((session, _reserved, key_id, user_id, _group, funding)) = &reservation {
                if let Err(error) =
                    state
                        .billing
                        .refund(state.billing_store.as_ref(), session, key_id, user_id, funding)
                {
                    eprintln!("[OxygenRouter] billing refund failed: {error}");
                }
            }
            log_request(
                &state,
                &_headers,
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

/// Identify the caller for rate-limit scoping.
///
/// Prefers the bearer token so one client cannot exhaust another's budget behind
/// a shared NAT, and falls back to the forwarded address.
fn client_scope(headers: &axum::http::HeaderMap) -> String {
    if let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
    {
        // Truncated: the scope must distinguish callers without storing the
        // credential itself.
        //
        // Counted in characters, not bytes. `HeaderValue::to_str` only yields
        // visible ASCII, so today byte 16 can never land mid-character and this
        // is not a reachable panic — it is the same trap written up in
        // `api::chars_prefix`, removed here so a later change to how the header
        // is read cannot turn it into one. A token that does not parse as ASCII
        // still falls through to address scoping below.
        return format!(
            "token:{}",
            token.chars().take(16).collect::<String>()
        );
    }
    let ip = remote_ip(headers);
    if ip.is_empty() {
        "anonymous".to_string()
    } else {
        format!("ip:{ip}")
    }
}

/// The caller's address, for scoping guards that are keyed on origin rather
/// than on a credential.
///
/// Exposed so the console's login lock and the relay's rate limit derive the
/// address the same way — two implementations of "where did this come from"
/// would eventually disagree, and the disagreement would be a bypass.
pub fn caller_address(headers: &axum::http::HeaderMap) -> String {
    let ip = remote_ip(headers);
    if ip.is_empty() {
        "anonymous".to_string()
    } else {
        ip
    }
}

/// Render a limit rejection in the OpenAI error shape.
fn limit_error_response(error: oxygenrouter_proxy::limits::LimitError) -> Response {
    let message = match error {
        oxygenrouter_proxy::limits::LimitError::RateLimited { retry_after_secs } => {
            format!("rate limit exceeded; retry after {retry_after_secs}s")
        }
        oxygenrouter_proxy::limits::LimitError::ConcurrencyExceeded => {
            "server is at its concurrent request ceiling; retry shortly".to_string()
        }
    };
    let mut response = json_policy_error(
        StatusCode::from_u16(error.status_code()).unwrap_or(StatusCode::TOO_MANY_REQUESTS),
        "rate_limit_exceeded",
        &message,
    );
    if let Some(retry) = error.retry_after_secs() {
        response.headers_mut().insert(
            "retry-after",
            axum::http::HeaderValue::from_str(&retry.to_string())
                .unwrap_or_else(|_| axum::http::HeaderValue::from_static("60")),
        );
    }
    response
}
/// Which accounts pay for a request, if any.
///
/// Returns `None` when there is no key, or the key has no owning user, so
/// unauthenticated and channel-only deployments are not billed.
fn billing_target(state: &AppState, api_key: &Option<ApiKey>) -> Option<(String, String)> {
    let key = api_key.as_ref()?;
    if key.user_id.trim().is_empty() {
        return None;
    }
    // A user row must exist or the wallet updates would silently affect nothing.
    match state.db.get_user(&key.user_id) {
        Ok(Some(_)) => Some((key.id.clone(), key.user_id.clone())),
        Ok(None) => {
            eprintln!(
                "[OxygenRouter] key {} references missing user {}; not billing",
                key.id, key.user_id
            );
            None
        }
        Err(e) => {
            eprintln!("[OxygenRouter] billing target lookup failed: {e}");
            None
        }
    }
}

/// Convert the adaptor's `Usage` into the billing engine's shape.
fn billing_usage_from(
    usage: &oxygenrouter_proxy::Usage,
    model: &str,
    path: &str,
) -> BillingUsage {
    use oxygenrouter_billing::UsageSemantic as Sem;
    let semantic = match usage.semantic.as_str() {
        "anthropic" => Sem::Anthropic,
        "gemini" => Sem::Gemini,
        _ => {
            // The dialect also depends on what the client asked for: a client
            // posting to /v1/messages expects Anthropic accounting even when the
            // upstream reported OpenAI-shaped numbers.
            if path.starts_with("/v1/messages") {
                Sem::Anthropic
            } else {
                Sem::OpenAi
            }
        }
    };
    let _ = model;
    BillingUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        cached_tokens: usage.cached_tokens,
        cache_creation_tokens: usage.cache_creation_tokens,
        cache_creation_5m_tokens: usage.cache_creation_5m_tokens,
        cache_creation_1h_tokens: usage.cache_creation_1h_tokens,
        image_tokens: usage.image_tokens,
        audio_tokens: usage.audio_tokens,
        semantic,
    }
}

// ---------------------------------------------------------------------------
// P4 protocol superset
//
// The endpoints below complete the protocol surface the roadmap calls for.
// Most of them share one shape: forward the client's own body and method to the
// selected upstream, then return the upstream's response verbatim, because the
// wire formats already agree (a client that speaks OpenAI's Files API is talking
// to a provider that does too). Only the paths that need translation are special.
// ---------------------------------------------------------------------------

/// Forward one request to the selected upstream without reshaping it.
///
/// `override_path` replaces the client-visible path, which matters for the
/// legacy engine embedding route (`/v1/engines/:model/embeddings` must reach
/// the provider as `/v1/embeddings`), and `raw_body` lets a handler that has
/// already rewritten the JSON pass the rewritten bytes on.
async fn relay_passthrough(
    state: std::sync::Arc<AppState>,
    method: &str,
    client_path: &str,
    upstream_path: &str,
    body_bytes: Vec<u8>,
    headers: axum::http::HeaderMap,
    default_model: &str,
) -> Response {
    let start = Instant::now();
    let parsed: serde_json::Value =
        serde_json::from_slice(&body_bytes).unwrap_or_else(|_| serde_json::json!({}));

    // Multipart uploads (audio, files, image edits) carry no JSON model field,
    // so the caller's default is used and no model rewriting happens.
    let model = parsed
        .get("model")
        .and_then(|v| v.as_str())
        .filter(|m| !m.trim().is_empty() && !m.eq_ignore_ascii_case("auto"))
        .map(str::to_string)
        .unwrap_or_else(|| default_model.to_string());

    let key = match authorize(&state, &headers, &model, client_path, start) {
        Ok(key) => key,
        Err(response) => return response,
    };

    // A streamed pass-through needs the upstream to actually stream. This was
    // hard-coded `false`, so a client asking for SSE got a buffered JSON body and
    // the Gemini client gave up; the same defect class as the hard-coded
    // `stream: false` on the chat completions path.
    let stream = passthrough_wants_stream(&headers, &parsed, upstream_path);

    let proxy_req = ProxyRequest {
        method: method.to_string(),
        path: upstream_path.to_string(),
        headers: forwarded_headers(&headers),
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
            log_request_with_usage(
                &state,
                &headers,
                method,
                client_path,
                Some(model),
                Some(r.channel_id.clone()),
                key.as_ref().map(|k| k.id.clone()),
                Some(r.status),
                None,
                duration_ms,
                if r.usage.total_tokens > 0 {
                    Some(r.usage.total_tokens)
                } else {
                    None
                },
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
                &headers,
                method,
                client_path,
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

/// Headers worth passing upstream.
///
/// The client's own credential must not leak: the channel's key is applied by
/// the adaptor. Content negotiation and provider-specific beta flags do matter,
/// so `anthropic-beta` and `openai-beta` are forwarded.
fn forwarded_headers(headers: &axum::http::HeaderMap) -> Vec<(String, String)> {
    const FORWARD: &[&str] = &[
        "anthropic-beta",
        "anthropic-version",
        "openai-beta",
        "openai-organization",
        "openai-project",
    ];
    FORWARD
        .iter()
        .filter_map(|name| {
            headers
                .get(*name)
                .and_then(|v| v.to_str().ok())
                .map(|v| ((*name).to_string(), v.to_string()))
        })
        .collect()
}

async fn moderations_endpoint(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        "POST",
        "/v1/moderations",
        "/v1/moderations",
        body_bytes,
        headers,
        "text-moderation-latest",
    )
    .await
}

async fn edits_endpoint(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        "POST",
        "/v1/edits",
        "/v1/edits",
        body_bytes,
        headers,
        "gpt-3.5-turbo-instruct",
    )
    .await
}

/// `/v1/engines/:model/embeddings` is the legacy spelling of `/v1/embeddings`.
/// The engine id is folded into the body's `model` so the provider sees the
/// modern shape.
async fn engines_embeddings(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(model): axum::extract::Path<String>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    let mut parsed: serde_json::Value =
        serde_json::from_slice(&body_bytes).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(obj) = parsed.as_object_mut() {
        obj.insert("model".to_string(), serde_json::Value::String(model));
    }
    let body = serde_json::to_vec(&parsed).unwrap_or(body_bytes);
    relay_passthrough(
        state,
        "POST",
        "/v1/engines/:model/embeddings",
        "/v1/embeddings",
        body,
        headers,
        "text-embedding-3-small",
    )
    .await
}

/// Count tokens for a Claude-shaped request.
///
/// Clients call this to size a prompt before sending it. NewAPI forwards it so a
/// provider that implements the endpoint answers authoritatively; the relay has
/// no local counter for this path.
async fn count_tokens_endpoint(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        "POST",
        "/v1/messages/count_tokens",
        "/v1/messages/count_tokens",
        body_bytes,
        headers,
        "claude-3-haiku",
    )
    .await
}

/// `POST /v1/responses/compact` — Responses-API context compaction.
///
/// The reference forwards only the documented compaction fields and drops the
/// Codex-parity extras (`tools`, `reasoning`, `text`) on the way upstream
/// (`relay/responses_handler.go:23-39`, `dto/openai_responses_compaction_request.go`).
/// It also keeps the request's own `input` verbatim, which is what makes the
/// endpoint usable for compacting an existing context.
async fn responses_compact_endpoint(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        "POST",
        "/v1/responses/compact",
        "/v1/responses/compact",
        compact_body(body_bytes),
        headers,
        "gpt-4o-mini",
    )
    .await
}

/// The compaction fields the reference documents and forwards.
///
/// Everything else is a Codex-parity field it parses for client compatibility but
/// deliberately does not send upstream
/// (`dto/openai_responses_compaction_request.go:11-27`,
/// `relay/responses_handler.go:23-39`). Forwarding `tools` or `reasoning` would
/// hand the upstream fields its compaction endpoint does not define.
const COMPACTION_FORWARDED_FIELDS: &[&str] = &[
    "model",
    "input",
    "instructions",
    "previous_response_id",
    "parallel_tool_calls",
    "service_tier",
    "prompt_cache_key",
    "prompt_cache_options",
    "prompt_cache_retention",
];

/// Reduce a compaction request body to the documented field set.
///
/// A body that is not a JSON object is returned untouched: an unusual client
/// should get a real upstream error rather than a silent rewrite of its payload.
fn compact_body(body_bytes: Vec<u8>) -> Vec<u8> {
    match serde_json::from_slice::<serde_json::Value>(&body_bytes) {
        Ok(serde_json::Value::Object(obj)) => {
            let kept: serde_json::Map<String, serde_json::Value> = obj
                .into_iter()
                .filter(|(k, _)| COMPACTION_FORWARDED_FIELDS.contains(&k.as_str()))
                .collect();
            serde_json::to_vec(&serde_json::Value::Object(kept)).unwrap_or(body_bytes)
        }
        _ => body_bytes,
    }
}

/// `POST /v1/alpha/search` — Codex standalone web search.
///
/// Only some channel families implement it; the reference refuses the others so
/// the scheduler retries elsewhere (`relay/alpha_search_handler.go:24-35`). The
/// upstream returns no usage, so the reference bills a single
/// `web_search_preview` call rather than inventing a token count.
///
/// We forward the raw body (unknown fields preserved, matching
/// `buildAlphaSearchRequestBody`) and apply no charge: our billing engine is
/// driven by upstream-reported usage, and charging a synthesised amount would be
/// a guess rather than parity.
async fn alpha_search_endpoint(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        "POST",
        "/v1/alpha/search",
        "/v1/alpha/search",
        body_bytes,
        headers,
        "gpt-5.6-terra",
    )
    .await
}

/// Native Gemini inbound: `/v1beta/models/<model>:<verb>`.
///
/// The client already speaks Gemini, so the body needs no conversion; the model
/// still has to be resolved so the router can pick a channel and the scheduler
/// can apply a model map.
async fn gemini_inbound(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(path): axum::extract::Path<String>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    // `<model>:<verb>`; the verb decides streaming but not the wire format.
    let model = path
        .split(':')
        .next()
        .unwrap_or("");
    relay_passthrough(
        state,
        "POST",
        "/v1beta/models/:model",
        &format!("/v1beta/models/{path}"),
        body_bytes,
        headers,
        model,
    )
    .await
}

// --- Files ----------------------------------------------------------------

async fn files_list_or_upload(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let method = request.method().as_str().to_string();
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        &method,
        "/v1/files",
        "/v1/files",
        body_bytes,
        headers,
        "",
    )
    .await
}

async fn files_get_or_delete(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    request: Request,
) -> Response {
    let method = request.method().as_str().to_string();
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        &method,
        "/v1/files/:id",
        &format!("/v1/files/{id}"),
        body_bytes,
        headers,
        "",
    )
    .await
}

async fn files_content(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        "GET",
        "/v1/files/:id/content",
        &format!("/v1/files/{id}/content"),
        body_bytes,
        headers,
        "",
    )
    .await
}

// --- Batches --------------------------------------------------------------

async fn batches_list_or_create(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let method = request.method().as_str().to_string();
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        &method,
        "/v1/batches",
        "/v1/batches",
        body_bytes,
        headers,
        "",
    )
    .await
}

async fn batches_get(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        "GET",
        "/v1/batches/:id",
        &format!("/v1/batches/{id}"),
        body_bytes,
        headers,
        "",
    )
    .await
}

async fn batches_cancel(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        "POST",
        "/v1/batches/:id/cancel",
        &format!("/v1/batches/{id}/cancel"),
        body_bytes,
        headers,
        "",
    )
    .await
}

// --- Fine-tuning ----------------------------------------------------------

async fn fine_tunes_list_or_create(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let method = request.method().as_str().to_string();
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        &method,
        "/v1/fine-tunes",
        "/v1/fine-tunes",
        body_bytes,
        headers,
        "",
    )
    .await
}

async fn fine_tunes_get(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        "GET",
        "/v1/fine-tunes/:id",
        &format!("/v1/fine-tunes/{id}"),
        body_bytes,
        headers,
        "",
    )
    .await
}

async fn fine_tunes_cancel(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        "POST",
        "/v1/fine-tunes/:id/cancel",
        &format!("/v1/fine-tunes/{id}/cancel"),
        body_bytes,
        headers,
        "",
    )
    .await
}

async fn fine_tunes_events(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    request: Request,
) -> Response {
    let method = request.method().as_str().to_string();
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        &method,
        "/v1/fine-tunes/:id/events",
        &format!("/v1/fine-tunes/{id}/events"),
        body_bytes,
        headers,
        "",
    )
    .await
}

// --- Models ---------------------------------------------------------------

/// `/v1/models/:model` — retrieve or delete a single model upstream.
async fn models_get_or_delete(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(model): axum::extract::Path<String>,
    request: Request,
) -> Response {
    let method = request.method().as_str().to_string();
    let (headers, body_bytes) = read_body(request).await;
    relay_passthrough(
        state,
        &method,
        "/v1/models/:model",
        &format!("/v1/models/{model}"),
        body_bytes,
        headers,
        &model,
    )
    .await
}

async fn responses_endpoint(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    dispatch_openai(
        state,
        "/v1/responses",
        body_bytes,
        "gpt-4o-mini",
        headers,
    )
    .await
}

async fn messages_endpoint(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    dispatch_openai(
        state,
        "/v1/messages",
        body_bytes,
        "claude-3-haiku",
        headers,
    )
    .await
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
    dispatch_openai(
        state,
        "/v1/audio/speech",
        body_bytes,
        "tts-1",
        headers,
    )
    .await
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
    dispatch_openai(
        state,
        "/v1/images/edits",
        body_bytes,
        "dall-e-3",
        headers,
    )
    .await
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

#[cfg(test)]
mod tests {
    use super::*;
    use oxygenrouter_proxy::Usage;

    fn usage(semantic: &str, prompt: i64, completion: i64, cached: i64) -> Usage {
        Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
            semantic: semantic.to_string(),
            cached_tokens: cached,
            ..Default::default()
        }
    }

    #[test]
    fn usage_semantic_follows_the_upstream_dialect() {
        let u = usage("anthropic", 100, 20, 50);
        let b = billing_usage_from(&u, "claude-sonnet-4-20250514", "/v1/chat/completions");
        assert_eq!(b.semantic, oxygenrouter_billing::UsageSemantic::Anthropic);
        assert_eq!(b.prompt_tokens, 100);
        assert_eq!(b.cached_tokens, 50);
    }

    #[test]
    fn gemini_dialect_is_preserved() {
        let u = usage("gemini", 10, 5, 0);
        let b = billing_usage_from(&u, "gemini-2.0-flash", "/v1/chat/completions");
        assert_eq!(b.semantic, oxygenrouter_billing::UsageSemantic::Gemini);
    }

    #[test]
    fn native_messages_clients_get_anthropic_accounting() {
        // A client posting to /v1/messages expects Anthropic semantics even when
        // the upstream reported OpenAI-shaped numbers: the two dialects differ in
        // whether cached tokens are a subset of the prompt total.
        let u = usage("openai", 100, 20, 40);
        let b = billing_usage_from(&u, "glm-5.3-flash", "/v1/messages");
        assert_eq!(b.semantic, oxygenrouter_billing::UsageSemantic::Anthropic);
    }

    #[test]
    fn cache_creation_tiers_survive_the_mapping() {
        let mut u = usage("anthropic", 100, 20, 10);
        u.cache_creation_tokens = 30;
        u.cache_creation_5m_tokens = 20;
        u.cache_creation_1h_tokens = 10;
        let b = billing_usage_from(&u, "claude-sonnet-4-20250514", "/v1/chat/completions");
        assert_eq!(b.cache_creation_tokens, 30);
        assert_eq!(b.cache_creation_5m_tokens, 20);
        assert_eq!(b.cache_creation_1h_tokens, 10);
        // The split is authoritative for cache-write billing.
        assert_eq!(b.cache_write_tokens(), 30);
    }

    #[test]
    fn zero_usage_is_reported_as_not_billable() {
        let u = usage("openai", 0, 0, 0);
        let b = billing_usage_from(&u, "x", "/v1/chat/completions");
        assert!(!b.has_billable_tokens());
    }
#[test]
    fn every_supported_endpoint_is_registered_in_the_router() {
        // Regression guard: a handler can exist, compile and be listed in
        // SUPPORTED_ENDPOINTS while its route was never added, in which case the
        // request falls through to the 404 fallback. Asserting against the
        // advertised list only proves the two agree, not that either is right, so
        // this drives real requests through the real router and checks that none
        // of them reach the fallback.
        //
        // A 404 body carrying `known_endpoints` is the fallback's signature.
        let app = router(test_state());
        let runtime = test_runtime();

        for entry in SUPPORTED_ENDPOINTS {
            let (method, path) = entry
                .split_once(' ')
                .expect("SUPPORTED_ENDPOINTS entries are `METHOD /path`");
            let path = path.trim().replace(":model", "x").replace(":id", "x");

            let (status, text) = call(&app, &runtime, method.trim(), &path);
            assert!(
                !text.contains("known_endpoints"),
                "{entry} fell through to the 404 fallback (status {status})"
            );
        }
    }

    #[test]
    fn the_model_listing_routes_answer_their_own_dialect() {
        // A route can exist while resolving to the *wrong* handler. `/v1beta/models`
        // sits beside the `/v1beta/models/*path` wildcard, and the wildcard happily
        // swallows `/v1beta/models` too -- so a missing exact route is invisible
        // from the status code alone (the wildcard answers 200 with an error body).
        // The observable difference is the shape, so the shape is what is asserted.
        let app = router(test_state());
        let runtime = test_runtime();

        let (status, body) = call(&app, &runtime, "GET", "/v1beta/models");
        assert_eq!(status, 200, "Gemini catalogue route: {body}");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        assert!(
            parsed.get("models").is_some(),
            "`/v1beta/models` must answer the Gemini envelope, not the wildcard's body: {body}"
        );
        assert!(parsed["models"].is_array());

        let (status, body) = call(&app, &runtime, "GET", "/v1beta/openai/models");
        assert_eq!(status, 200, "Gemini-compatible OpenAI route: {body}");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        assert_eq!(
            parsed["object"], "list",
            "`/v1beta/openai/models` must answer OpenAI shape: {body}"
        );
    }

    #[test]
    fn the_model_list_dialect_follows_the_client_credentials() {
        // `/v1/models` is one route serving three shapes, chosen by the
        // credential headers. Asserting through the router (rather than calling
        // the detector directly) is what proves the handler actually consults
        // them: a handler that hard-coded OpenAI shape would pass a detector-only
        // test and still break every Anthropic and Gemini SDK.
        let app = router(test_state());
        let runtime = test_runtime();

        let (status, body) = call_with(
            &app,
            &runtime,
            "GET",
            "/v1/models",
            &[
                ("x-api-key", "test-token"),
                ("anthropic-version", "2023-06-01"),
            ],
        );
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        assert_eq!(
            parsed["data"][0]["type"], "model",
            "an Anthropic credential must yield Anthropic shape: {body}"
        );
        assert!(
            parsed.get("object").is_none(),
            "the OpenAI envelope must not leak into the Anthropic shape: {body}"
        );
        assert!(parsed.get("first_id").is_some());

        let (status, body) = call_with(
            &app,
            &runtime,
            "GET",
            "/v1/models",
            &[("x-goog-api-key", "test-token")],
        );
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        assert!(
            parsed["models"].is_array(),
            "a Gemini credential must yield the Gemini envelope: {body}"
        );

        // The default client still gets OpenAI shape.
        let (status, body) = call(&app, &runtime, "GET", "/v1/models");
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        assert_eq!(parsed["object"], "list");
        assert!(parsed["data"][0].get("object").is_some());
    }

    #[test]
    fn compaction_forwards_only_the_documented_fields() {
        // Codex sends `tools`/`reasoning`/`text` for compatibility, but the
        // reference drops them before the upstream call. Forwarding them would
        // hand the compaction endpoint fields it does not define.
        let body = serde_json::to_vec(&serde_json::json!({
            "model": "gpt-5-codex",
            "input": "context to compact",
            "instructions": "be terse",
            "previous_response_id": "resp_1",
            "parallel_tool_calls": true,
            "service_tier": "auto",
            "prompt_cache_key": "k",
            "prompt_cache_options": {"mode": "explicit"},
            "prompt_cache_retention": "24h",
            // Codex-parity fields, deliberately not forwarded:
            "tools": [{"type": "web_search"}],
            "reasoning": {"effort": "high"},
            "text": {"format": {"type": "text"}},
        }))
        .unwrap();

        let out: serde_json::Value =
            serde_json::from_slice(&compact_body(body)).expect("valid JSON");
        for kept in COMPACTION_FORWARDED_FIELDS {
            assert!(out.get(*kept).is_some(), "{kept} must be forwarded");
        }
        for dropped in ["tools", "reasoning", "text"] {
            assert!(
                out.get(dropped).is_none(),
                "{dropped} must not reach the compaction endpoint"
            );
        }
    }

    #[test]
    fn compaction_leaves_a_non_object_body_untouched() {
        // An unusual client should see the upstream's own error rather than a
        // silently rewritten payload.
        let raw = b"not json at all".to_vec();
        assert_eq!(compact_body(raw.clone()), raw);
        let array = b"[1,2,3]".to_vec();
        assert_eq!(compact_body(array.clone()), array);
    }

    /// Drive one request through the real router and return (status, body).
    fn call(app: &Router, runtime: &tokio::runtime::Runtime, method: &str, path: &str) -> (u16, String) {
        call_with(app, runtime, method, path, &[])
    }

    /// As [`call`], but with extra request headers. An empty name/value pair is
    /// the empty list; `authorization` is always sent.
    fn call_with(
        app: &Router,
        runtime: &tokio::runtime::Runtime,
        method: &str,
        path: &str,
        extra: &[(&str, &str)],
    ) -> (u16, String) {
        let app = app.clone();
        let (method, path) = (method.to_string(), path.to_string());
        let extra: Vec<(String, String)> = extra
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let response = runtime.block_on(async move {
            use tower::ServiceExt;
            let mut builder = axum::http::Request::builder()
                .method(method.as_str())
                .uri(&path)
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json");
            for (k, v) in &extra {
                builder = builder.header(k.as_str(), v.as_str());
            }
            let request = builder
                .body(axum::body::Body::from("{}"))
                .expect("request builds");
            app.oneshot(request).await.expect("router responds")
        });
        let status = response.status().as_u16();
        let body = runtime.block_on(async {
            axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap_or_default()
        });
        (status, String::from_utf8_lossy(&body).to_string())
    }

    fn test_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
    }

    /// A real `AppState` backed by a temporary database, for router tests.
    fn test_state() -> std::sync::Arc<AppState> {
        use oxygenrouter_core::ChannelSelector;
        use oxygenrouter_proxy::{ChannelScheduler, RelayClient};

        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("router-test.db");
        // The directory is leaked deliberately: `AppState` holds only the path,
        // and the file must outlive this function for the router to serve reads.
        let config_path = dir.path().join("config.json");
        std::mem::forget(dir);

        let db = std::sync::Arc::new(
            oxygenrouter_core::Database::new(&db_path).expect("database opens"),
        );
        let selector = ChannelSelector::new(std::sync::Arc::clone(&db));
        let relay = RelayClient::new("OxygenRouter-test".to_string(), 5_000);
        let scheduler = ChannelScheduler::new(selector, relay);
        std::sync::Arc::new(AppState::new(
            db,
            db_path,
            scheduler,
            "test-token".to_string(),
            config_path,
        ))
    }

    #[test]
    fn only_expected_headers_are_forwarded_upstream() {
        // The client's own credential must never reach the upstream: the
        // channel's key is applied by the adaptor instead.
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer client-secret"),
        );
        headers.insert(
            "anthropic-beta",
            axum::http::HeaderValue::from_static("prompt-caching-2024-07-31"),
        );
        headers.insert("cookie", axum::http::HeaderValue::from_static("session=1"));

        let forwarded = forwarded_headers(&headers);
        let names: Vec<&str> = forwarded.iter().map(|(n, _)| n.as_str()).collect();
        assert!(!names.contains(&"authorization"));
        assert!(!names.contains(&"cookie"));
        assert!(names.contains(&"anthropic-beta"));
    }

    #[test]
    fn forwarded_headers_carry_the_value_through() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("openai-beta", axum::http::HeaderValue::from_static("assistants=v2"));
        let forwarded = forwarded_headers(&headers);
        assert_eq!(
            forwarded
                .iter()
                .find(|(n, _)| n == "openai-beta")
                .map(|(_, v)| v.as_str()),
            Some("assistants=v2")
        );
    }

    #[test]
    fn an_empty_header_map_forwards_nothing() {
        let headers = axum::http::HeaderMap::new();
        assert!(forwarded_headers(&headers).is_empty());
    }

    fn headers_with_accept(value: &str) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::ACCEPT,
            axum::http::HeaderValue::from_str(value).unwrap(),
        );
        headers
    }

    #[test]
    fn an_explicit_stream_false_beats_a_broad_accept_header() {
        // An SDK that always sends the SSE accept header but asked for a buffered
        // reply must not be marked streaming, or its JSON body reaches the SSE
        // parser.
        let headers = headers_with_accept("text/event-stream");
        assert!(!is_stream_request(&headers, &serde_json::json!({"stream": false})));
        assert!(is_stream_request(&headers, &serde_json::json!({"stream": true})));
    }

    #[test]
    fn the_accept_header_still_selects_streaming_without_a_body_flag() {
        let headers = headers_with_accept("text/event-stream");
        assert!(is_stream_request(&headers, &serde_json::json!({})));
        assert!(!is_stream_request(&axum::http::HeaderMap::new(), &serde_json::json!({})));
    }

    #[test]
    fn the_gemini_verb_decides_passthrough_streaming() {
        let json = axum::http::HeaderMap::new();
        let body = serde_json::json!({});
        assert!(passthrough_wants_stream(
            &json,
            &body,
            "/v1beta/models/gemini-2.5-flash:streamGenerateContent"
        ));
        // The buffered verb must not stream even when the client sends the SSE
        // accept header, because `:generateContent` has no streaming form.
        assert!(!passthrough_wants_stream(
            &headers_with_accept("text/event-stream"),
            &body,
            "/v1beta/models/gemini-2.5-flash:generateContent"
        ));
    }

    #[test]
    fn a_non_gemini_passthrough_falls_back_to_the_body_flag() {
        let body = serde_json::json!({"stream": true});
        assert!(passthrough_wants_stream(
            &axum::http::HeaderMap::new(),
            &body,
            "/v1/files"
        ));
        let buffered = serde_json::json!({"stream": false});
        assert!(!passthrough_wants_stream(
            &headers_with_accept("text/event-stream"),
            &buffered,
            "/v1/files"
        ));
    }
}
