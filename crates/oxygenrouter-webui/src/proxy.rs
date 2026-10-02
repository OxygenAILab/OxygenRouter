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
        // --- Tasks and videos ----------------------------------------------
        // The video submit surface the reference publishes alongside the image
        // one (`router/video-router.go:15`), which is the same bridge under two
        // names: a video request is a task submission.
        .route("/v1/videos", any(videos_submit))
        .route("/v1/video/generations", any(videos_submit))
        .route("/v1/videos/:video_id/remix", any(videos_remix))
        .route("/v1/videos/:task_id/content", any(videos_content))
        .route("/v1/tasks/:key/artifacts", any(task_artifacts))
        .route(
            "/v1/tasks/:key/artifacts/:artifact_key/content",
            any(task_artifact_content),
        )
        // --- Tasks ---------------------------------------------------------
        // The read surfaces a task plugin needs: the generic task id, and the
        // video and response aliases the reference publishes for the same rows
        // (`router/task-router.go:24`, `router/video-router.go:30`,
        // `controller/plugin_protocol.go:948`).
        .route("/v1/tasks/:key", any(task_read))
        .route("/v1/videos/:task_id", any(task_read))
        .route("/v1/video/generations/:task_id", any(task_read))
        .route("/v1/responses/:response_id", any(responses_read))
        .with_state(state)
}

/// A task read by its public id.
async fn task_read(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(task_id): axum::extract::Path<String>,
) -> Response {
    task_read_response(&state, &task_id)
}

/// A response read, which is the same task under the Responses API's own id.
///
/// The reference publishes a `resp_` prefix and stores the task under `task_`, so
/// the translation is a prefix swap and nothing else
/// (`controller/plugin_protocol.go:959`). An id without the prefix is refused
/// rather than guessed at, because guessing would turn a typo into a lookup for
/// whatever task happens to share the rest of the string.
async fn responses_read(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(response_id): axum::extract::Path<String>,
) -> Response {
    let Some(rest) = response_id.strip_prefix("resp_") else {
        return not_found_response(
            "bad_prefix",
            &format!("a response id must start with \"resp_\"; got {response_id:?}"),
        );
    };
    task_read_response(&state, &format!("task_{rest}"))
}

/// Submit a video task, which is the same bridge the image endpoints use.
///
/// The difference from an image submission is only that the model must claim the
/// video protocol, which the endpoint index already enforces: the path is part of
/// the binding key, so a plugin that claims only images is not reachable here.
async fn videos_submit(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    task_submit(state, request, "/v1/videos").await
}

/// Remix an existing task: a submission that names the task it derives from.
///
/// The reference routes it to the same controller (`router/video-router.go:31`),
/// so it is the same bridge; the origin id reaches the plugin through the request
/// body, which is where a plugin looks for it.
async fn videos_remix(
    State(state): State<std::sync::Arc<AppState>>,
    request: Request,
) -> Response {
    task_submit(state, request, "/v1/videos/:video_id/remix").await
}

/// The shared submission path for a task-shaped endpoint.
async fn task_submit(
    state: std::sync::Arc<AppState>,
    request: Request,
    client_path: &str,
) -> Response {
    let (headers, body_bytes) = read_body(request).await;
    let model = request_body_model(&headers, &body_bytes).await;
    let Some(binding) = model
        .as_deref()
        .and_then(|model| plugin_for_endpoint(&state, "POST", client_path, model))
    else {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!(
                "no enabled plugin serves {client_path} for this model; an operator has to install and enable one, or a channel has to serve the model directly"
            ),
        );
    };
    plugin_bridge(
        state,
        PluginInvocation {
            binding: &binding,
            headers: &headers,
            body: &body_bytes,
        },
        client_path,
    )
    .await
}

/// List the artifacts a finished task produced.
///
/// A plugin that declares no `listArtifacts` produces none, which is an empty
/// list rather than an error: not every task has artifacts, and the reference
/// answers the same way (`adaptor.go:896`).
async fn task_artifacts(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(task_id): axum::extract::Path<String>,
) -> Response {
    let Some(task) = state.db.get_task(&task_id).ok().flatten() else {
        return task_artifact_error(StatusCode::NOT_FOUND, "artifact_not_found");
    };
    let Some(context) = artifact_context(&state, &task) else {
        return task_artifact_error(StatusCode::NOT_FOUND, "artifact_not_found");
    };
    match state
        .plugins
        .call_hook_args(
            &task.platform,
            oxygenrouter_plugin::HOOK_LIST_ARTIFACTS,
            &[serde_json::to_value(&context).unwrap_or(serde_json::Value::Null)],
            PLUGIN_TIMEOUT,
        )
        .await
    {
        Ok(value) => match oxygenrouter_plugin::validate_task_artifacts(&value) {
            Ok(artifacts) => (StatusCode::OK, axum::Json(artifacts)).into_response(),
            Err(reason) => {
                eprintln!("[OxygenRouter] task {task_id} artifact listing refused: {reason}");
                task_artifact_error(StatusCode::INTERNAL_SERVER_ERROR, "artifact_plugin_error")
            }
        },
        // A plugin without the hook lists nothing, which is not a failure.
        Err(oxygenrouter_plugin::PluginError::Hook { .. }) => {
            (StatusCode::OK, axum::Json(Vec::<oxygenrouter_plugin::TaskArtifact>::new()))
                .into_response()
        }
        Err(error) => {
            eprintln!("[OxygenRouter] task {task_id} artifact listing failed: {error}");
            task_artifact_error(StatusCode::INTERNAL_SERVER_ERROR, "artifact_plugin_error")
        }
    }
}

/// Serve one artifact's content.
///
/// Three refusals before the plugin is even asked, all of them the reference's
/// (`controller/task.go`'s `TaskArtifactContent`): an unreadable artifact key, a
/// task whose result was discarded, and a task that has not finished -- the last
/// as `409` rather than `404`, because the artifact is expected to exist later and
/// a client should retry rather than give up.
async fn task_artifact_content(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path((task_id, artifact_key)): axum::extract::Path<(String, String)>,
    request: Request,
) -> Response {
    let method = request.method().as_str().to_string();
    let client_headers: Vec<(String, String)> = request
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_string(), value.to_string()))
        })
        .collect();
    serve_artifact(&state, &task_id, &artifact_key, &method, client_headers).await
}

/// Serve an artifact through the video path alias, which is the same task row.
async fn videos_content(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Path(task_id): axum::extract::Path<String>,
    request: Request,
) -> Response {
    let method = request.method().as_str().to_string();
    let client_headers: Vec<(String, String)> = request
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_string(), value.to_string()))
        })
        .collect();
    serve_artifact(&state, &task_id, "video", &method, client_headers).await
}

async fn serve_artifact(
    state: &AppState,
    task_id: &str,
    artifact_key: &str,
    method: &str,
    client_headers: Vec<(String, String)>,
) -> Response {
    if !oxygenrouter_plugin::valid_artifact_key(artifact_key.trim()) {
        return task_artifact_error(StatusCode::NOT_FOUND, "artifact_not_found");
    }
    let Some(task) = state.db.get_task(task_id).ok().flatten() else {
        return task_artifact_error(StatusCode::NOT_FOUND, "artifact_not_found");
    };
    if task.status != oxygenrouter_plugin::STATUS_SUCCESS {
        return task_artifact_error(StatusCode::CONFLICT, "artifact_not_ready");
    }
    let Some(context) = artifact_context(state, &task) else {
        return task_artifact_error(StatusCode::NOT_FOUND, "artifact_not_found");
    };
    let listing = match state
        .plugins
        .call_hook_args(
            &task.platform,
            oxygenrouter_plugin::HOOK_LIST_ARTIFACTS,
            &[serde_json::to_value(&context).unwrap_or(serde_json::Value::Null)],
            PLUGIN_TIMEOUT,
        )
        .await
        .ok()
        .and_then(|value| oxygenrouter_plugin::validate_task_artifacts(&value).ok())
    {
        Some(listing) => listing,
        None => {
            return task_artifact_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "artifact_plugin_unavailable",
            )
        }
    };
    if !listing.iter().any(|artifact| artifact.key == artifact_key) {
        return task_artifact_error(StatusCode::NOT_FOUND, "artifact_not_found");
    }

    // What the plugin wants fetched, and the guard on it. A credentialless
    // request is checked against nothing but its own shape, because it carries no
    // credential to misdirect.
    // The context a plugin builds a content URL from: the artifact's identity, the
    // channel's base URL, and -- unless the plugin asked to go without it -- the
    // credential, in the same shape a submission gets
    // (`relay/channel/task/jsplugin/adaptor.go:924-930`).
    let channel = state.db.get_channel(&task.channel_id).ok().flatten();
    let mut build = serde_json::to_value(&context).unwrap_or(serde_json::Value::Null);
    if let Some(object) = build.as_object_mut() {
        object.insert(
            "upstreamTaskId".to_string(),
            serde_json::json!(task.private.upstream_task_id),
        );
        object.insert("artifactKey".to_string(), serde_json::json!(artifact_key));
        object.insert(
            "baseUrl".to_string(),
            serde_json::json!(channel
                .as_ref()
                .map(|channel| channel.base_url.clone())
                .unwrap_or_default()),
        );
        object.insert(
            "clientRequest".to_string(),
            serde_json::json!({ "method": method, "headers": client_headers }),
        );
        // The stored credential is what this task was submitted with, so a poll
        // and an artifact fetch authenticate the same way.
        let credential = task
            .private
            .credential
            .trim()
            .to_string()
            .is_empty()
            .then(|| {
                channel
                    .as_ref()
                    .and_then(|channel| task_authorization(channel))
            })
            .flatten()
            .or_else(|| {
                (!task.private.credential.trim().is_empty())
                    .then(|| task.private.credential.clone())
            });
        object.insert("auth".to_string(), serde_json::json!({ "authHeader": credential }));
        object.insert("authHeader".to_string(), serde_json::json!(credential));
    }
    let descriptor = match state
        .plugins
        .call_hook_args(
            &task.platform,
            oxygenrouter_plugin::HOOK_BUILD_CONTENT_REQUEST,
            &[build],
            PLUGIN_TIMEOUT,
        )
        .await
    {
        Ok(value) => serde_json::from_value::<oxygenrouter_plugin::RequestDescriptor>(value)
            .map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };
    let descriptor = match descriptor {
        Ok(descriptor) => descriptor,
        Err(reason) => {
            eprintln!("[OxygenRouter] task {task_id} artifact request failed: {reason}");
            return task_artifact_error(StatusCode::INTERNAL_SERVER_ERROR, "artifact_plugin_error");
        }
    };
    let client = oxygenrouter_plugin::ClientRequest {
        method: method.to_string(),
        headers: client_headers,
    };
    let content = match oxygenrouter_plugin::validate_content_request(&descriptor, &client) {
        Ok(content) => content,
        Err(reason) => {
            eprintln!("[OxygenRouter] task {task_id} artifact request refused: {reason}");
            return task_artifact_error(StatusCode::INTERNAL_SERVER_ERROR, "artifact_plugin_error");
        }
    };
    // A credentialed artifact request still has to stay on a host the operator
    // allowed, exactly like a submission.
    if !content.credentialless {
        let base_url = state
            .db
            .get_channel(&task.channel_id)
            .ok()
            .flatten()
            .map(|channel| channel.base_url)
            .unwrap_or_default();
        if let Err(reason) = oxygenrouter_plugin::validate_request_url(
            &content.url,
            &base_url,
            &allowed_hosts_of(state, &task.platform),
        ) {
            eprintln!("[OxygenRouter] task {task_id} artifact host refused: {reason}");
            return task_artifact_error(StatusCode::INTERNAL_SERVER_ERROR, "artifact_plugin_error");
        }
    }

    let Ok(transport) = task_transport(state) else {
        return task_artifact_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "artifact_plugin_unavailable",
        );
    };
    let authorization = if content.credentialless {
        None
    } else {
        task
            .private
            .credential
            .trim()
            .is_empty()
            .then(|| {
                state
                    .db
                    .get_channel(&task.channel_id)
                    .ok()
                    .flatten()
                    .and_then(|channel| task_authorization(&channel))
            })
            .flatten()
    };
    use oxygenrouter_plugin::TaskTransport;
    let outcome = match transport
        .execute(oxygenrouter_plugin::OutboundRequest {
            method: content.method.clone(),
            url: content.url.clone(),
            headers: content.headers.clone(),
            body: content.body.clone().map(|bytes| oxygenrouter_plugin::OutboundBody {
                content_type: "application/octet-stream".to_string(),
                bytes,
            }),
            authorization,
        })
        .await
    {
        Ok(outcome) => outcome,
        Err(reason) => {
            eprintln!("[OxygenRouter] task {task_id} artifact fetch failed: {reason}");
            return task_artifact_error(StatusCode::BAD_GATEWAY, "artifact_fetch_error");
        }
    };
    if !outcome.is_success() {
        return task_artifact_error(
            StatusCode::from_u16(outcome.status).unwrap_or(StatusCode::BAD_GATEWAY),
            "artifact_fetch_error",
        );
    }

    // The upstream's own content type and length travel, and a HEAD carries no
    // body -- which is the whole point of the method.
    let mut builder = Response::builder().status(StatusCode::OK);
    let is_head = content.method == "HEAD";
    for (name, value) in &outcome.headers {
        if name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("connection")
        {
            continue;
        }
        // A HEAD must report the headers a GET would, length included, while
        // carrying no body -- that is the whole reason the method exists, and a
        // HEAD that dropped the length would tell a client nothing it could not
        // learn by downloading. For every other method the length comes from the
        // body this response actually has, so the upstream's is dropped to avoid
        // contradicting it.
        if name.eq_ignore_ascii_case("content-length") && !is_head {
            continue;
        }
        builder = builder.header(name.as_str(), value.as_str());
    }
    let body = if content.method == "HEAD" {
        Body::empty()
    } else {
        Body::from(outcome.body)
    };
    builder.body(body).unwrap_or_else(|_| error_500())
}

/// The context an artifact hook reads, or `None` when the plugin is gone.
fn artifact_context(
    state: &AppState,
    task: &oxygenrouter_core::TaskRecord,
) -> Option<oxygenrouter_plugin::ArtifactContext> {
    let manifest = state.db.plugin_manifest(&task.platform).ok().flatten()?;
    Some(oxygenrouter_plugin::ArtifactContext {
        task_id: task.task_id.clone(),
        status: task.status.clone(),
        action: task.action.clone(),
        data: task.data.clone(),
        state: task.private.plugin_state.clone(),
        producer_version: manifest
            .get("version")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

/// An artifact failure in the reference's error shape
/// (`controller/task.go`'s `writeTaskArtifactError`).
fn task_artifact_error(status: StatusCode, code: &str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({
            "error": {
                "code": code,
                "message": match status {
                    StatusCode::NOT_FOUND => "Task or artifact not found",
                    StatusCode::CONFLICT => "Task artifacts are not ready",
                    StatusCode::INTERNAL_SERVER_ERROR => "Artifact content plugin failed",
                    StatusCode::SERVICE_UNAVAILABLE => "Artifact content plugin is unavailable",
                    _ => "Failed to fetch artifact content",
                },
                "type": "invalid_request_error",
            }
        })),
    )
        .into_response()
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
    let model = request_body_model(&headers, &body_bytes).await;
    if let Some(binding) = model
        .as_deref()
        .and_then(|model| plugin_for_endpoint(&state, "POST", "/v1/images/generations", model))
    {
        return plugin_bridge(
            state,
            PluginInvocation {
                binding: &binding,
                headers: &headers,
                body: &body_bytes,
            },
            "/v1/images/generations",
        )
        .await;
    }
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
            client_headers: client_headers(&headers),
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
        client_headers: client_headers(&headers),
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
            // A key with no group falls back to the configured `DefaultGroup`
            // rather than the literal, which is what that option says it does.
            let fallback_group = state.default_group();
            let group = api_key
                .as_ref()
                .map(|k| k.group_name.as_str())
                .filter(|g| !g.trim().is_empty())
                .unwrap_or(fallback_group.as_str());
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
                // The settlement is the one place a request's cost is known, so it
                // is also where the usage summary is fed. `DataExportEnabled` gates
                // it, and the counters are held in memory until the periodic flush.
                if state.data_export_enabled() {
                    state.usage.record(
                        user_id,
                        &r.channel_id,
                        api_key.as_ref().map(|k| k.id.as_str()).unwrap_or(""),
                        group,
                        &model,
                        charge.quota,
                        r.usage.total_tokens,
                    );
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

/// How long a plugin may take to decode or render, matching the reference's
/// `DefaultCallTimeout` (`pkg/jsplugin/engine.go:18`).
const PLUGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Every endpoint binding an enabled plugin claims, as a lookup index.
///
/// Built from the instance's stored claims rather than from the engine, because a
/// plugin that is merely installed must not affect traffic and a plugin with no
/// active build has nothing to call. The key is `method + path + model`, which is
/// the reference's (`pkg/jsplugin/routing.go:994`): one endpoint serves many
/// models, and two plugins may share it while declaring disjoint model sets, so a
/// path-only lookup would hand a model to a plugin that never claimed it.
fn plugin_endpoint_index(state: &AppState) -> oxygenrouter_plugin::EndpointIndex {
    let claims = state.db.list_enabled_plugin_claims().unwrap_or_default();
    // One claim per (plugin, protocol a manifest claims), which is the unit the
    // reference indexes: the plugin's own model list, narrowed by the claim's.
    let expanded: Vec<oxygenrouter_plugin::EndpointClaim<'_>> = claims
        .iter()
        .flat_map(|claim| {
            claim.protocols.iter().map(move |protocol| {
                oxygenrouter_plugin::EndpointClaim {
                    plugin_key: &claim.key,
                    protocol: &protocol.name,
                    claim_models: &protocol.models,
                    models: &claim.models,
                    supports: &protocol.supports,
                }
            })
        })
        .collect();
    oxygenrouter_plugin::EndpointIndex::build(expanded)
}

/// The plugin bound to one endpoint, if any.
fn plugin_for_endpoint(
    state: &AppState,
    method: &str,
    client_path: &str,
    model: &str,
) -> Option<oxygenrouter_plugin::ProtocolBinding> {
    plugin_endpoint_index(state).lookup(method, client_path, model)
}

/// The model a request names, from whichever encoding it used.
///
/// A task submission may be JSON, a URL-encoded form, or multipart, and the model
/// is the declared field top-level in all three (`pkg/jsplugin/routing.go:38`).
/// Reading it is the only body parsing the host does before a hook runs, which is
/// what keeps the host out of the vendor's dialect.
async fn request_body_model(headers: &axum::http::HeaderMap, body_bytes: &[u8]) -> Option<String> {
    let content_type = request_content_type(headers);
    if content_type == "multipart/form-data" {
        let boundary = multipart_boundary(headers)?;
        let mut multipart = multer::Multipart::new(body_stream(body_bytes), boundary);
        while let Some(field) = multipart.next_field().await.ok()? {
            if field.name() != Some("model") {
                continue;
            }
            let data = field.bytes().await.ok()?;
            let value = String::from_utf8(data.to_vec()).ok()?.trim().to_string();
            return (!value.is_empty()).then_some(value);
        }
        return None;
    }
    if content_type == "application/x-www-form-urlencoded" {
        let text = std::str::from_utf8(body_bytes).ok()?;
        return url::form_urlencoded::parse(text.as_bytes())
            .find(|(key, _)| key == "model")
            .map(|(_, value)| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }
    let parsed: serde_json::Value = serde_json::from_slice(body_bytes).ok()?;
    parsed
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from)
}

/// The one-shot byte stream a multipart parser reads.
fn body_stream(
    bytes: &[u8],
) -> impl futures_util::Stream<Item = Result<bytes::Bytes, std::io::Error>> + '_ {
    futures_util::stream::once(async move { Ok(bytes::Bytes::copy_from_slice(bytes)) })
}

/// The boundary of a multipart request, without its surrounding quotes.
fn multipart_boundary(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .split(';')
                .skip(1)
                .find_map(|part| part.trim().strip_prefix("boundary="))
        })
        .map(|value| value.trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
}

/// Every plugin bound to one endpoint, in binding order.
fn plugin_candidates_for_endpoint(
    state: &AppState,
    method: &str,
    client_path: &str,
    model: &str,
) -> Vec<oxygenrouter_plugin::ProtocolBinding> {
    plugin_endpoint_index(state).candidates(method, client_path, model)
}

/// Whether a request body asks for one of the forms the protocol defines.
fn body_wants(body: &serde_json::Value, key: &str) -> bool {
    body.as_object()
        .and_then(|_| body.get("body"))
        .and_then(|tagged| tagged.get("value"))
        .unwrap_or(body)
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// The content type of a request, with any parameters stripped.
fn request_content_type(headers: &axum::http::HeaderMap) -> String {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .unwrap_or_default()
}

/// The context a plugin's hooks read, built from what the client actually sent.
///
/// The reference's shape is `path`, `method`, `params`, `query`, `body`,
/// `protocol`, `operation`, `model`, `upstreamModel` and `stream`
/// (`pkg/jsplugin/routing.go:283,342`). `body` is a tagged union so a hook can
/// tell a JSON body from a form body from no body at all
/// (`middleware/task_plugin.go:793`).
#[allow(clippy::too_many_arguments)]
async fn build_protocol_context(
    headers: &axum::http::HeaderMap,
    body_bytes: &[u8],
    client_path: &str,
    protocol: &'static str,
    operation: &'static str,
    model: &str,
    upstream_model: &str,
) -> Result<(oxygenrouter_plugin::ProtocolContext, serde_json::Value), String> {
    let content_type = request_content_type(headers);
    let mut request = oxygenrouter_plugin::RequestContext::new(client_path, "POST");
    let mut stream = false;

    if content_type == "multipart/form-data" {
        let Some(boundary) = multipart_boundary(headers) else {
            return Err("multipart request is missing its boundary".to_string());
        };
        let mut multipart = multer::Multipart::new(body_stream(body_bytes), boundary);
        let mut fields = std::collections::BTreeMap::<String, Vec<String>>::new();
        let mut files = Vec::new();
        while let Some(field) = multipart
            .next_field()
            .await
            .map_err(|error| format!("multipart body could not be read: {error}"))?
        {
            let name = field.name().unwrap_or_default().to_string();
            if name.is_empty() {
                return Err("multipart part is missing a field name".to_string());
            }
            let filename = field.file_name().map(String::from);
            let mime_type = field.content_type().map(|m| m.to_string()).unwrap_or_default();
            let data = field
                .bytes()
                .await
                .map_err(|error| format!("multipart field {name:?} could not be read: {error}"))?;
            match filename {
                // The bytes stay host-owned: a plugin gets a reference and asks
                // for the content by name (`pkg/jsplugin/routing.go:248`).
                Some(filename) => {
                    let index = fields.get(&name).map(Vec::len).unwrap_or(0);
                    files.push(oxygenrouter_plugin::BodyFile {
                        reference: oxygenrouter_plugin::file_reference(&name, index),
                        field: name.clone(),
                        filename,
                        mime_type,
                        size: data.len() as u64,
                    });
                }
                None => {
                    let text = String::from_utf8(data.to_vec())
                        .map_err(|_| format!("multipart field {name:?} must be valid UTF-8"))?;
                    fields.entry(name).or_default().push(text);
                }
            }
        }
        // A multipart form carries its text fields in the tagged body alongside
        // the file references (`middleware/task_plugin.go:955`).
        // A multipart form carries its text fields in the tagged body
        // alongside the file references (`middleware/task_plugin.go:955`).
        request = request.with_multipart(fields, files);
    } else if content_type == "application/x-www-form-urlencoded" {
        let text = String::from_utf8(body_bytes.to_vec())
            .map_err(|_| "form body must be valid UTF-8".to_string())?;
        let mut fields = std::collections::BTreeMap::<String, Vec<String>>::new();
        for (key, value) in url::form_urlencoded::parse(text.as_bytes()) {
            fields.entry(key.into_owned()).or_default().push(value.into_owned());
        }
        request = request.with_form(fields);
    } else {
        // JSON is the default, as it is for every OpenAI-compatible endpoint. An
        // empty body is "none" rather than an empty object, so a hook can tell
        // the two apart.
        if body_bytes.is_empty() {
            request = oxygenrouter_plugin::RequestContext::new(client_path, "POST");
        } else {
            let parsed: serde_json::Value = serde_json::from_slice(body_bytes)
                .map_err(|error| format!("request body must be JSON: {error}"))?;
            stream = body_wants(&parsed, "stream");
            request = request.with_json(parsed);
        }
    }

    let context = oxygenrouter_plugin::ProtocolContext {
        request,
        protocol,
        operation,
        model: model.to_string(),
        upstream_model: upstream_model.to_string(),
        stream,
    };
    let value = context.js_value();
    Ok((context, value))
}

/// What a plugin-backed endpoint is called for, once a binding is known.
struct PluginInvocation<'a> {
    binding: &'a oxygenrouter_plugin::ProtocolBinding,
    headers: &'a axum::http::HeaderMap,
    body: &'a [u8],
}

/// The channel a task flow talks to, and the credential that rides with it.
///
/// A task is bound to one channel for its whole life, not re-picked per poll: the
/// upstream knows the task by an id it issued to *that* channel, so asking a
/// different one about it would be asking a stranger.
fn select_task_channel(state: &AppState, model: &str) -> Option<oxygenrouter_core::Channel> {
    state
        .db
        .get_enabled_channels()
        .ok()?
        .into_iter()
        .filter(|channel| {
            channel.model_list.is_empty()
                || channel
                    .model_list
                    .iter()
                    .any(|listed| listed == model || listed == "*")
        })
        .max_by_key(|channel| (channel.priority, channel.weight))
}

/// Run one plugin-backed request: decode, submit, and (when the upstream answers
/// at once) render.
///
/// This is what makes a plugin a *bridge* rather than a filter. The plugin states
/// the upstream request through `decodeRequest`; the host performs exactly that;
/// the plugin renders the result back into the shape the client asked for.
/// Nothing about the client's dialect is interpreted here, which is the point: it
/// is how the Responses API can be served with its own semantics instead of a
/// fixed adapter's approximation.
///
/// An upstream that answers *immediately* is rendered here and now, which is the
/// whole of the synchronous image protocol. An upstream that returns a task id is
/// persisted and answered with a task handle, because the work outlives the
/// request: a client that gave up must not cancel billable work, and the task is
/// polled to a terminal state by `poll_tasks_once` rather than inside this call.
async fn plugin_bridge(
    state: std::sync::Arc<AppState>,
    invocation: PluginInvocation<'_>,
    client_path: &str,
) -> Response {
    let binding = invocation.binding.clone();
    let protocol = binding.protocol;
    let plugin_key = binding.plugin_key.clone();

    let (protocol_context, context_value) = match build_protocol_context(
        invocation.headers,
        invocation.body,
        client_path,
        protocol,
        binding.operation.name,
        &binding.model,
        "",
    )
    .await
    {
        Ok(pair) => pair,
        Err(reason) => return json_error(StatusCode::BAD_REQUEST, &reason),
    };

    // Which request form this request needs, and whether the plugin that owns the
    // endpoint declared it. A plugin that never claimed `stream` must not be
    // handed a streaming request: it would return a non-streaming payload that the
    // host then has no way to frame (`middleware/task_plugin.go:382`).
    let wants_stream = context_value
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let wants_background = body_wants(&context_value, "background");
    // Only a protocol that *defines* request forms can be checked against them.
    // A mode-less protocol (the synchronous image API) has nothing to declare, so
    // demanding `sync` of it would refuse every request it exists to serve
    // (`middleware/task_plugin.go:382`).
    let protocol_has_modes = oxygenrouter_plugin::host_protocol(protocol)
        .map(oxygenrouter_plugin::protocol_has_modes)
        .unwrap_or(false);
    for mode in oxygenrouter_plugin::required_modes(wants_stream, wants_background)
        .into_iter()
        .filter(|_| protocol_has_modes)
    {
        if !binding.supports_mode(mode) {
            let candidates = plugin_candidates_for_endpoint(
                &state,
                "POST",
                client_path,
                &binding.model,
            );
            return json_error(
                StatusCode::BAD_REQUEST,
                &oxygenrouter_plugin::unsupported_form_message(
                    &candidates,
                    protocol,
                    wants_stream,
                    wants_background,
                ),
            );
        }
    }

    // Decode first: the plugin decides what goes upstream, and a decode that
    // fails is reported as the plugin's fault rather than the upstream's.
    let decoded = match state
        .plugins
        .decode_request(protocol, context_value.clone(), PLUGIN_TIMEOUT)
        .await
    {
        Ok(value) => value,
        Err(error) => {
            return json_error(
                StatusCode::BAD_GATEWAY,
                &format!("plugin {plugin_key:?} could not decode the request: {error}"),
            )
        }
    };

    // Two refusals the reference makes and this host keeps, both because the
    // decode step runs inside the plugin and the host must be able to trust what
    // comes back. A decoder that changes the model would silently route the
    // request to a different upstream than the caller asked for and was quoted
    // for; a decoder that returns a renderer would choose the *response* shape
    // from the request side, where the caller can influence it
    // (`relay/channel/task/jsplugin/adaptor.go:103,106`).
    // The decoder must name the model it resolved, and it must be the one this
    // endpoint serves. A decoder that stays silent about the model would let the
    // host dispatch a request whose quota was quoted for a different one; a
    // decoder that renames it would route the request somewhere the caller never
    // asked for (`middleware/task_plugin.go:640,643`).
    let decoded_model = decoded
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if decoded_model.is_empty() {
        return json_error(
            StatusCode::BAD_REQUEST,
            &format!("plugin {plugin_key:?} decoded request is missing a model"),
        );
    }
    if decoded_model != binding.model {
        return json_error(
            StatusCode::BAD_REQUEST,
            &format!(
                "plugin {plugin_key:?} decoded model {decoded_model:?}, but the endpoint serves {:?}",
                binding.model
            ),
        );
    }
    if decoded.get("renderer").is_some() {
        return json_error(
            StatusCode::BAD_REQUEST,
            &format!("plugin {plugin_key:?} decodeRequest must not return a renderer"),
        );
    }

    let kind = decoded.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    if kind != "submit" {
        return json_error(
            StatusCode::BAD_REQUEST,
            &format!(
                "plugin {plugin_key:?} decodeRequest must return kind \"submit\"; it returned {kind:?}"
            ),
        );
    }

    // The channel owns the upstream and the credential, so a task without one has
    // nowhere to go. This is the same refusal the relay path makes, worded for a
    // task so an operator knows it is a channel gap and not a plugin bug.
    let Some(channel) = select_task_channel(&state, &binding.model) else {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!(
                "no enabled channel serves {:?}, so plugin {plugin_key:?} has no upstream to submit to",
                binding.model
            ),
        );
    };

    // The task's quota is reserved *now*, and the facts settlement will need travel
    // with the task. A task settles long after this request is gone, from a loop
    // that has no access to the key or the group; the reference persists the same
    // snapshot for the same reason (`model/task.go`'s `BillingContext`).
    let task_billing = match reserve_task_quota(
        &state,
        invocation.headers,
        &binding.model,
        invocation.body,
    ) {
        Ok(billing) => billing,
        Err(response) => return response,
    };

    let flow_context = oxygenrouter_plugin::TaskFlowContext {
        plugin_key: plugin_key.clone(),
        model: binding.model.clone(),
        base_url: channel.base_url.clone(),
        authorization: task_authorization(&channel),
        allowed_hosts: allowed_hosts_of(&state, &plugin_key),
        submit_response_types: submit_response_types_of(&state, &plugin_key),
        required_capabilities: manifest_strings(&state, &plugin_key, "requiredCapabilities"),
        files: match resolve_request_files(invocation.headers, invocation.body).await {
            Ok(files) => files,
            Err(reason) => return json_error(StatusCode::BAD_REQUEST, &reason),
        },
        max_inline_bytes: oxygenrouter_plugin::DEFAULT_MAX_INLINE_FILE_BYTES,
        timeout: PLUGIN_TIMEOUT,
        // The request as the client sent it: `response_format` is the caller's
        // instruction, not part of any vendor's dialect.
        request_body: decoded
            .get("requestBody")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        created_at: Utc::now().timestamp(),
    };

    // The descriptor is what the plugin wants sent; the guard checks it before a
    // socket exists, so a plugin cannot point the channel credential anywhere the
    // operator did not allow.
    let descriptor = match build_submit_descriptor(
        &state,
        &plugin_key,
        &flow_context,
        &binding.model,
        &decoded,
    )
    .await
    {
        Ok(descriptor) => descriptor,
        Err(error) => {
            return json_error(
                if error.retryable {
                    StatusCode::BAD_GATEWAY
                } else {
                    StatusCode::BAD_REQUEST
                },
                &error.message,
            )
        }
    };

    let transport = match task_transport(&state) {
        Ok(transport) => transport,
        Err(reason) => return json_error(StatusCode::BAD_GATEWAY, &reason),
    };
    let request = match oxygenrouter_plugin::build_outbound_request(&descriptor, &flow_context) {
        Ok(request) => request,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, &error.message),
    };
    let requested_at = Instant::now();
    let mut outcome = match oxygenrouter_plugin::send_submit(&transport, request).await {
        Ok(outcome) => outcome,
        Err(error) => {
            log_request(
                state.as_ref(),
                invocation.headers,
                "POST",
                client_path,
                Some(binding.model.clone()),
                Some(channel.id.clone()),
                None,
                Some(StatusCode::BAD_GATEWAY.as_u16()),
                Some(error.message.clone()),
                requested_at.elapsed().as_millis() as i64,
            );
            return json_error(StatusCode::BAD_GATEWAY, &error.message);
        }
    };

    match oxygenrouter_plugin::interpret_submit(
        &state.plugins,
        &descriptor,
        &flow_context,
        context_value,
        &mut outcome,
    )
    .await
    {
        Ok(oxygenrouter_plugin::SubmitAnswer::Immediate { result, body }) => {
            let response = render_immediate(
                &state,
                &binding,
                &protocol_context,
                &flow_context,
                &result,
                body,
            )
            .await;
            // A synchronous answer is already terminal, so its reservation settles
            // here rather than waiting for a poll that will never come.
            settle_immediate_task(&state, task_billing.as_ref(), &result, response)
        }
        Ok(oxygenrouter_plugin::SubmitAnswer::Pending(submission)) => {
            persist_task(
                &state,
                &binding,
                &channel,
                &flow_context,
                &submission,
                task_billing,
            )
        }
        Err(error) => json_error(
            if error.retryable {
                StatusCode::BAD_GATEWAY
            } else {
                StatusCode::BAD_REQUEST
            },
            &error.message,
        ),
    }
}

/// The credential a task's upstream request carries, as an `Authorization` value.
///
/// A channel key is a bearer token by convention. A channel that needs something
/// else states it in its own headers, which the descriptor may already carry.
fn task_authorization(channel: &oxygenrouter_core::Channel) -> Option<String> {
    let key = channel.api_key.trim();
    if key.is_empty() {
        None
    } else if key.to_ascii_lowercase().starts_with("bearer ") {
        Some(key.to_string())
    } else {
        Some(format!("Bearer {key}"))
    }
}

/// The hosts a plugin's descriptors may address, from its own stored manifest.
///
/// Read from the manifest rather than from the live host, because the manifest is
/// what the operator approved when they enabled the plugin.
fn allowed_hosts_of(state: &AppState, plugin_key: &str) -> Vec<String> {
    manifest_strings(state, plugin_key, "allowedHosts")
}

/// The submission encodings a plugin declared it can parse.
fn submit_response_types_of(state: &AppState, plugin_key: &str) -> Vec<String> {
    let declared = manifest_strings(state, plugin_key, "submitResponseTypes");
    if declared.is_empty() {
        vec!["json".to_string()]
    } else {
        declared
    }
}

/// A string list from a plugin's active manifest.
fn manifest_strings(state: &AppState, plugin_key: &str, field: &str) -> Vec<String> {
    let Some(manifest) = state.db.plugin_manifest(plugin_key).ok().flatten() else {
        return Vec::new();
    };
    manifest
        .get(field)
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// The uploaded files a request carried, resolved to their bytes.
///
/// A plugin addresses a file by the reference its context showed it, so the host
/// has to be able to hand back the content that reference names. The bytes cannot
/// be derived from the reference, so the body is read again here -- once, and only
/// for a multipart request.
async fn resolve_request_files(
    headers: &axum::http::HeaderMap,
    body: &[u8],
) -> Result<Vec<oxygenrouter_plugin::ResolvedFile>, String> {
    if request_content_type(headers) != "multipart/form-data" {
        return Ok(Vec::new());
    }
    let Some(boundary) = multipart_boundary(headers) else {
        return Ok(Vec::new());
    };
    let mut multipart = multer::Multipart::new(body_stream(body), boundary);
    let mut files: Vec<oxygenrouter_plugin::ResolvedFile> = Vec::new();
    let mut per_field: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| format!("multipart body could not be read: {error}"))?
    {
        let Some(filename) = field.file_name().map(String::from) else {
            // A text field's content is already in the context; the parser still
            // has to consume it to reach the next part.
            let _ = field.bytes().await;
            continue;
        };
        let name = field.name().unwrap_or_default().to_string();
        let mime_type = field
            .content_type()
            .map(|mime| mime.to_string())
            .unwrap_or_default();
        let index = {
            let slot = per_field.entry(name.clone()).or_insert(0);
            let index = *slot;
            *slot += 1;
            index
        };
        let reference = oxygenrouter_plugin::file_reference(&name, index);
        let bytes = field
            .bytes()
            .await
            .map_err(|error| format!("multipart field {name:?} could not be read: {error}"))?;
        files.push(oxygenrouter_plugin::ResolvedFile {
            reference,
            field: name,
            filename,
            mime_type,
            bytes: bytes.to_vec(),
        });
    }
    Ok(files)
}

/// Ask the plugin what to send, and check it before a socket exists.
///
/// The body the decoder produced is passed *into* `buildSubmitRequest` as
/// `requestBody`, which is the reference's contract (`adaptor.go:1302`): it is the
/// normalized request the plugin asked to send, and the hook decides how to shape
/// it. Replacing the descriptor's own body with it instead would discard whatever
/// routing the hook does -- the vendor's own envelope, an action, a mode -- and
/// send the client's raw payload where the upstream expects something else.
async fn build_submit_descriptor(
    state: &AppState,
    plugin_key: &str,
    flow_context: &oxygenrouter_plugin::TaskFlowContext,
    resolved_model: &str,
    decoded: &serde_json::Value,
) -> Result<oxygenrouter_plugin::RequestDescriptor, oxygenrouter_plugin::FlowError> {
    let request_body = decoded
        .get("requestBody")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let descriptor = call_build_submit(state, plugin_key, flow_context, &request_body).await?;
    oxygenrouter_plugin::validate_descriptor(&descriptor, resolved_model, flow_context)?;
    Ok(descriptor)
}

/// Call `buildSubmitRequest`, the hook that states the upstream request.
async fn call_build_submit(
    state: &AppState,
    plugin_key: &str,
    flow_context: &oxygenrouter_plugin::TaskFlowContext,
    request_body: &serde_json::Value,
) -> Result<oxygenrouter_plugin::RequestDescriptor, oxygenrouter_plugin::FlowError> {
    let value = state
        .plugins
        .call_hook_args(
            plugin_key,
            oxygenrouter_plugin::HOOK_BUILD_SUBMIT_REQUEST,
            &[serde_json::json!({
                "model": flow_context.model,
                "baseUrl": flow_context.base_url,
                // What the plugin's decoder normalized, which the hook shapes
                // into the vendor's own envelope (`adaptor.go:1302`).
                "requestBody": request_body,
            })],
            flow_context.timeout,
        )
        .await
        .map_err(|error| oxygenrouter_plugin::FlowError::fatal(error.to_string()))?;
    serde_json::from_value(value).map_err(|error| {
        oxygenrouter_plugin::FlowError::fatal(format!(
            "buildSubmitRequest returned an unusable shape: {error}"
        ))
    })
}

/// The transport a task flow sends through, built from the instance's own SSRF
/// policy so the network guard is the operator's rather than a default.
fn task_transport(state: &AppState) -> Result<crate::task_transport::ReqwestTaskTransport, String> {
    crate::task_transport::ReqwestTaskTransport::new(None, state.fetch_policy())
}

/// Render an upstream's immediate answer.
///
/// An immediate answer is the synchronous case: there is nothing to poll, so the
/// plugin's render hook shapes the response now.
async fn render_immediate(
    state: &AppState,
    binding: &oxygenrouter_plugin::ProtocolBinding,
    protocol_context: &oxygenrouter_plugin::ProtocolContext,
    flow_context: &oxygenrouter_plugin::TaskFlowContext,
    result: &oxygenrouter_plugin::TaskResult,
    upstream_body: serde_json::Value,
) -> Response {
    let Some(hook) = binding.operation.render_hook else {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "this operation declares no render hook",
        );
    };
    let now = Utc::now().timestamp();
    // The view a renderer sees is deliberately narrow: the task's lifecycle and
    // the plugin's own data, never a credential (`service/task_plugin_view.go:11`).
    // For an immediate answer there is no stored task yet, so the "task" is made
    // from the upstream's own answer, which is what a synchronous plugin's renderer
    // reads (`controller/plugin_protocol_image.go:113,125`).
    let mut data = upstream_body;
    if let Some(object) = data.as_object_mut() {
        object
            .entry("url".to_string())
            .or_insert_with(|| serde_json::json!(result.url));
    }
    let view = oxygenrouter_plugin::TaskView::build(
        &result.task_id,
        &binding.plugin_key,
        if result.status.is_empty() {
            oxygenrouter_plugin::STATUS_SUCCESS
        } else {
            &result.status
        },
        &result.progress,
        &result.reason,
        now,
        now,
        now,
        data,
        "",
    );

    let mut rendered = match state
        .plugins
        .call_hook_args(
            &binding.plugin_key,
            hook,
            &[
                protocol_context.js_value(),
                serde_json::to_value(&view).unwrap_or(serde_json::Value::Null),
            ],
            PLUGIN_TIMEOUT,
        )
        .await
    {
        Ok(rendered) => rendered,
        Err(error) => {
            return json_error(
                StatusCode::BAD_GATEWAY,
                &format!(
                    "plugin {:?} could not render the response: {error}",
                    binding.plugin_key
                ),
            )
        }
    };

    // The host's own encoding decision, applied to what the renderer produced:
    // `created` is the host's clock, and `b64_json` is the host inlining whichever
    // URLs the renderer returned (`controller/plugin_protocol_image.go:138,147`).
    let response_format = flow_context
        .request_body
        .get("response_format")
        .and_then(|value| value.as_str())
        .map(String::from);
    let plan = oxygenrouter_plugin::plan_image_encoding(
        &mut rendered,
        response_format.as_deref(),
        flow_context.created_at,
    );
    if !plan.is_empty() {
        match task_transport(state) {
            Ok(transport) => {
                for inline in plan {
                    match fetch_image(&transport, &inline.url).await {
                        Ok((mime_type, bytes)) => {
                            let encoded = oxygenrouter_plugin::image_base64(&bytes);
                            oxygenrouter_plugin::apply_image_inline(
                                &mut rendered,
                                &inline,
                                &mime_type,
                                &encoded,
                            );
                        }
                        // The reference keeps the URL and carries on, so one
                        // unfetchable image does not fail the whole request
                        // (`controller/plugin_protocol_image.go:158`).
                        Err(reason) => eprintln!(
                            "[OxygenRouter] image inline skipped for {}: {reason}",
                            inline.url
                        ),
                    }
                }
            }
            Err(reason) => eprintln!("[OxygenRouter] image inline skipped: {reason}"),
        }
    }
    (StatusCode::OK, axum::Json(rendered)).into_response()
}

/// Fetch an image the host is about to inline.
///
/// The same transport, and therefore the same guards, as any other task request:
/// an upstream that answers with an internal URL must not be able to make the
/// gateway fetch that address on the caller's behalf.
async fn fetch_image(
    transport: &crate::task_transport::ReqwestTaskTransport,
    url: &str,
) -> Result<(String, Vec<u8>), String> {
    use oxygenrouter_plugin::TaskTransport;
    let outcome = transport
        .execute(oxygenrouter_plugin::OutboundRequest {
            method: "GET".to_string(),
            url: url.to_string(),
            headers: Vec::new(),
            body: None,
            // An image host is not the upstream that holds the channel
            // credential, so the credential stays behind.
            authorization: None,
        })
        .await?;
    if !outcome.is_success() {
        return Err(format!("image answered {}", outcome.status));
    }
    let mime_type = outcome
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| {
            value
                .split(';')
                .next()
                .unwrap_or("application/octet-stream")
                .trim()
                .to_string()
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "application/octet-stream".to_string());
    Ok((mime_type, outcome.body))
}

/// Store a task whose upstream work has just started, and answer with its handle.
///
/// The answer carries the public task id, because that is what a client polls and
/// what a renderer will be given. The private half -- the upstream id and the
/// plugin's own state -- is stored beside it and never leaves.
fn persist_task(
    state: &AppState,
    binding: &oxygenrouter_plugin::ProtocolBinding,
    channel: &oxygenrouter_core::Channel,
    flow_context: &oxygenrouter_plugin::TaskFlowContext,
    submission: &oxygenrouter_plugin::SubmitOutcome,
    billing: Option<oxygenrouter_core::TaskBilling>,
) -> Response {
    let now = Utc::now();
    let task_id = format!("task_{}", Uuid::new_v4().simple());
    let record = oxygenrouter_core::TaskRecord {
        id: Uuid::new_v4().to_string(),
        task_id: task_id.clone(),
        platform: binding.plugin_key.clone(),
        // Attributed to whoever paid, so the console's per-user view and the
        // billing record cannot disagree about whose task this is.
        user_id: billing
            .as_ref()
            .map(|billing| billing.user_id.clone())
            .unwrap_or_default(),
        channel_id: channel.id.clone(),
        api_key_id: billing
            .as_ref()
            .map(|billing| billing.key_id.clone())
            .unwrap_or_default(),
        action: String::new(),
        model: flow_context.model.clone(),
        upstream_model: flow_context.model.clone(),
        status: oxygenrouter_plugin::STATUS_SUBMITTED.to_string(),
        progress: oxygenrouter_plugin::PROGRESS_SUBMITTED.to_string(),
        fail_reason: String::new(),
        created_at: now,
        updated_at: now,
        submit_time: Some(now),
        start_time: None,
        finish_time: None,
        data: submission.task_data.clone(),
        private: oxygenrouter_core::TaskPrivate {
            upstream_task_id: submission.task_id.clone(),
            plugin_state: submission.state.clone(),
            // The credential is stored so a later poll can authenticate without
            // the client being present, and it lives only in the private half.
            credential: flow_context.authorization.clone().unwrap_or_default(),
            request_snapshot: serde_json::json!({
                "model": flow_context.model,
                "baseUrl": flow_context.base_url,
            }),
            // The reservation's facts, so the poll loop can settle without knowing
            // anything about the request that created the task.
            billing,
        },
    };
    if let Err(error) = state.db.upsert_task(&record) {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("the task could not be stored: {error}"),
        );
    }
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "taskId": task_id,
            "status": oxygenrouter_plugin::STATUS_SUBMITTED,
            "upstreamTaskId": submission.task_id,
        })),
    )
        .into_response()
}

/// How often a streamed response is observed, before jitter.
///
/// The reference's default (`controller/plugin_protocol.go`'s
/// `defaultPluginProtocolBridgeDeps`): two seconds, jittered per task so a fleet
/// of simultaneous submissions does not reach the upstream in lockstep.
pub const RESPONSES_TICK_MS: u64 = 2_000;
/// The jitter added to each tick, from the same default (zero).
pub const RESPONSES_TICK_JITTER_MS: u64 = 0;
/// How long a streamed response may run before it is ended as incomplete
/// (the reference's ten-minute default).
pub const RESPONSES_OBSERVATION_SECS: u64 = 600;

/// The delay before one tick, jittered by the task id.
///
/// Ported from `pluginProtocolTickDelay`: an FNV-1a hash of the task id and the
/// tick number, reduced into the jitter window. Two tasks submitted in the same
/// millisecond therefore drift apart instead of arriving together, which matters
/// because a plugin's upstream is usually the same host for all of them.
pub fn protocol_tick_delay(task_id: &str, tick: u64, base_ms: u64, jitter_ms: u64) -> std::time::Duration {
    if jitter_ms == 0 {
        return std::time::Duration::from_millis(base_ms);
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in task_id.as_bytes().iter().chain(b":").chain(tick.to_string().as_bytes()) {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    std::time::Duration::from_millis(base_ms + hash % (jitter_ms + 1))
}

/// Whether a set of stream events ended the response.
///
/// Ported from `taskPluginProtocolEventsTerminal`: the three endings are the only
/// types that stop the observation loop.
pub fn events_terminal(events: &[oxygenrouter_plugin::StreamEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event.event_type.as_str(),
            "response.completed" | "response.failed" | "response.incomplete"
        )
    })
}

/// Stream a plugin-backed Responses request.
///
/// The stream is the observation loop the reference runs: `created`, then a
/// `renderEvents` tick per interval until the task is terminal, ending as
/// completed, failed or incomplete. Two things are deliberate.
///
/// The task is *submitted first, outside this call*, so the work does not depend on
/// this connection staying open -- a client that gives up stops watching, not the
/// job. And every event a client sees is built by the host from semantic events, so
/// nothing here can be influenced by a plugin beyond the text it produced.
async fn responses_stream(
    state: std::sync::Arc<AppState>,
    task_id: String,
) -> Response {
    let started = std::time::Instant::now();
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);
    let state_for_task = state.clone();
    tokio::spawn(async move {
        let mut machine = match load_response_machine(&state_for_task, &task_id) {
            Some(machine) => machine,
            None => {
                let _ = tx
                    .send(sse_frame_of_error(
                        "task_protocol_error",
                        "the task could not be read back",
                    ))
                    .await;
                return;
            }
        };
        match machine.created() {
            Ok(created) => {
                if tx.send(oxygenrouter_plugin::sse_frame(&created)).await.is_err() {
                    return;
                }
            }
            Err(reason) => {
                let _ = tx.send(sse_frame_of_error("task_protocol_error", &reason)).await;
                return;
            }
        }

        let limits = oxygenrouter_plugin::EventLimits::default();
        let mut tick: u64 = 0;
        loop {
            if started.elapsed().as_secs() >= RESPONSES_OBSERVATION_SECS {
                if let Ok(event) = machine.timeout(None) {
                    let _ = tx.send(oxygenrouter_plugin::sse_frame(&event)).await;
                }
                return;
            }
            // The delay is computed before the work, so the interval is the gap
            // between ticks rather than the gap plus however long a tick took.
            let delay = protocol_tick_delay(
                &task_id,
                tick,
                RESPONSES_TICK_MS,
                RESPONSES_TICK_JITTER_MS,
            );
            tokio::time::sleep(delay).await;
            tick += 1;

            let Some(task) = state_for_task.db.get_task(&task_id).ok().flatten() else {
                if let Ok(event) = machine.failure(None) {
                    let _ = tx.send(oxygenrouter_plugin::sse_frame(&event)).await;
                }
                return;
            };

            // The events a plugin emits are read through its own hook; a plugin
            // that cannot render at all ends the stream rather than stalling it.
            // One ending for every way a tick can go wrong: a refusal, a missing
            // hook and a failed call all end the stream as failed rather than
            // leaving a client watching something that will never finish.
            let events = match render_events_once(&state_for_task, &task, &machine).await {
                Ok(Some(result)) => match machine.apply_tick(&result, &task.status) {
                    Ok(events) => events,
                    Err(reason) => {
                        eprintln!("[OxygenRouter] task {task_id} renderEvents refused: {reason}");
                        machine
                            .failure(Some(&task.status))
                            .map(|event| vec![event])
                            .unwrap_or_default()
                    }
                },
                // A plugin that no longer implements the hook ends the stream.
                Ok(None) => machine
                    .failure(Some(&task.status))
                    .map(|event| vec![event])
                    .unwrap_or_default(),
                Err(reason) => {
                    eprintln!("[OxygenRouter] task {task_id} renderEvents failed: {reason}");
                    machine
                        .failure(Some(&task.status))
                        .map(|event| vec![event])
                        .unwrap_or_default()
                }
            };
            if events.is_empty() {
                return;
            }
            for event in &events {
                if tx.send(oxygenrouter_plugin::sse_frame(event)).await.is_err() {
                    // The client went away. The task keeps running: it is already
                    // stored and the poll loop owns it.
                    return;
                }
            }
            if events_terminal(&events) {
                return;
            }
            let _ = limits;
        }
    });

    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|chunk| (Ok::<_, std::convert::Infallible>(chunk), rx))
    });
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .header("connection", "keep-alive")
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| error_500())
}

/// The machine for a stored task, or `None` when it cannot be read.
fn load_response_machine(
    state: &AppState,
    task_id: &str,
) -> Option<oxygenrouter_plugin::ResponsesMachine> {
    let task = state.db.get_task(task_id).ok().flatten()?;
    Some(oxygenrouter_plugin::ResponsesMachine::new(
        &task.task_id,
        &task.model,
        task.created_at.timestamp(),
        oxygenrouter_plugin::EventLimits::default(),
    ))
}

/// One `renderEvents` observation.
///
/// `None` means the plugin does not implement the hook, which ends the stream; an
/// `Err` is a failure of the call itself. A plugin error event is *not* treated as
/// a host failure: it is decoded and handed to the machine, which decides what a
/// client may be told.
async fn render_events_once(
    state: &AppState,
    task: &oxygenrouter_core::TaskRecord,
    machine: &oxygenrouter_plugin::ResponsesMachine,
) -> Result<Option<oxygenrouter_plugin::EventResult>, String> {
    let Some(channel) = state.db.get_channel(&task.channel_id).ok().flatten() else {
        return Ok(None);
    };
    let view = oxygenrouter_plugin::TaskView::build(
        &task.task_id,
        &task.platform,
        &task.status,
        &task.progress,
        &task.fail_reason,
        task.created_at.timestamp(),
        task.updated_at.timestamp(),
        task.finish_time.map(|at| at.timestamp()).unwrap_or(0),
        task.data.clone(),
        &task.private.upstream_task_id,
    );
    // The renderer's context is the request's own view plus what a renderer needs
    // to name the task it is narrating, which is what the reference passes
    // (`controller/plugin_protocol.go:915`).
    let mut context = serde_json::json!({
        "taskId": task.task_id,
        "responseId": machine.response_id(),
        "model": task.model,
        "baseUrl": channel.base_url,
        "metadata": machine.metadata(),
    });
    if let Some(object) = context.as_object_mut() {
        object.insert(
            "upstreamTaskId".to_string(),
            serde_json::json!(task.private.upstream_task_id),
        );
    }
    let view_value = serde_json::to_value(&view).unwrap_or(serde_json::Value::Null);
    match state
        .plugins
        .call_hook_args(
            &task.platform,
            "renderEvents",
            &[context, view_value],
            PLUGIN_TIMEOUT,
        )
        .await
    {
        Ok(value) => oxygenrouter_plugin::decode_event_result(
            &value,
            &oxygenrouter_plugin::EventLimits::default(),
        )
        .map(Some)
        .map_err(|reason| reason),
        Err(oxygenrouter_plugin::PluginError::NoSuchHook { .. }) => Ok(None),
        Err(oxygenrouter_plugin::PluginError::Hook { message, .. })
            if message.contains("no protocol member") =>
        {
            Ok(None)
        }
        Err(error) => Err(error.to_string()),
    }
}

/// An error frame for a stream that could not start.
fn sse_frame_of_error(code: &str, message: &str) -> String {
    format!(
        "event: error\ndata: {}\n\n",
        serde_json::json!({
            "type": "error",
            "error": { "code": code, "message": message },
        })
    )
}

/// How many tasks one poll tick examines.
///
/// Bounded on purpose: a tick that walked an unbounded backlog would hold a
/// worker for as long as the upstreams are slow, and the next tick would be late.
/// A backlog larger than this drains over successive ticks, oldest first, which is
/// the order `list_in_flight_tasks` returns.
pub const TASK_POLL_BATCH: usize = 32;

/// One poll tick: every in-flight task gets exactly one poll attempt.
///
/// The decisions belong to `poll_once`; this is where their verdicts reach the
/// store. Completion is a compare-and-set against the status that was read, so a
/// client's synchronous wait and this loop cannot both settle one task, and a
/// task that changed underneath is simply left for the next tick.
pub async fn poll_tasks_once(state: &std::sync::Arc<AppState>, now: i64) -> (usize, usize) {
    let Ok(tasks) = state.db.list_in_flight_tasks(TASK_POLL_BATCH) else {
        return (0, 0);
    };
    if tasks.is_empty() {
        return (0, 0);
    }
    let timeout_secs = task_timeout_secs();
    let mut polled = 0;
    let mut advanced = 0;

    for task in tasks {
        // A task whose channel is gone cannot be polled at all, and failing it
        // would blame the upstream for an operator's edit. It is left alone; the
        // timeout above is what eventually reclaims it.
        let Some(channel) = state
            .db
            .get_channel(&task.channel_id)
            .ok()
            .flatten()
        else {
            continue;
        };
        let Ok(transport) = task_transport(state) else {
            continue;
        };
        let flow_context = oxygenrouter_plugin::TaskFlowContext {
            plugin_key: task.platform.clone(),
            model: task.model.clone(),
            base_url: channel.base_url.clone(),
            // The credential stored with the task, because the client that
            // submitted it is not present to supply one.
            authorization: None,
            allowed_hosts: allowed_hosts_of(state, &task.platform),
            submit_response_types: submit_response_types_of(state, &task.platform),
            required_capabilities: manifest_strings(state, &task.platform, "requiredCapabilities"),
            files: Vec::new(),
            max_inline_bytes: 0,
            timeout: PLUGIN_TIMEOUT,
            request_body: serde_json::Value::Null,
            created_at: task.created_at.timestamp(),
        };
        let authorization = task_authorization(&channel);
        let flow_context = oxygenrouter_plugin::TaskFlowContext {
            authorization: task
                .private
                .credential
                .trim()
                .is_empty()
                .then_some(authorization)
                .flatten(),
            ..flow_context
        };

        let poll_task = oxygenrouter_plugin::PollTask {
            task_id: task.task_id.clone(),
            status: task.status.clone(),
            upstream_task_id: task.private.upstream_task_id.clone(),
            action: task.action.clone(),
            model: task.model.clone(),
            upstream_model: task.upstream_model.clone(),
            created_at: task.created_at.timestamp(),
            data: task.data.clone(),
            state: task.private.plugin_state.clone(),
        };
        let settlement = oxygenrouter_plugin::poll_once(
            &state.plugins,
            &transport,
            &flow_context,
            &poll_task,
            now,
            timeout_secs,
            false,
        )
        .await;
        polled += 1;

        if settlement.round == oxygenrouter_plugin::PollRound::Retried
            || settlement.round == oxygenrouter_plugin::PollRound::Refused
        {
            if let Some(reason) = &settlement.reason {
                eprintln!(
                    "[OxygenRouter] task {} poll {}: {reason}",
                    task.task_id,
                    match settlement.round {
                        oxygenrouter_plugin::PollRound::Retried => "failed",
                        _ => "was refused",
                    }
                );
            }
            continue;
        }

        let finish_time = matches!(
            settlement.round,
            oxygenrouter_plugin::PollRound::Settled | oxygenrouter_plugin::PollRound::Failed
        )
        .then(Utc::now);
        let update = oxygenrouter_core::TaskUpdate {
            status: settlement.status.clone(),
            progress: settlement.progress.clone(),
            fail_reason: settlement.reason.clone(),
            start_time: None,
            finish_time,
            data: None,
            plugin_state: None,
        };
        match state
            .db
            .complete_task(&task.task_id, &settlement.expected_status, &update)
        {
            Ok(true) => advanced += 1,
            // Somebody else got there first: the client's own wait, or the
            // previous tick. Either way this loop must not settle it twice.
            Ok(false) => continue,
            Err(error) => {
                eprintln!("[OxygenRouter] task {} could not be updated: {error}", task.task_id);
                continue;
            }
        }

        // The plugin's opaque poll state is merged rather than written as a
        // column, because the private half also holds the upstream id and the
        // credential -- replacing it wholesale would strand the next poll.
        if let Some(state_value) = &settlement.plugin_state {
            if let Err(error) = state
                .db
                .merge_task_plugin_state(&task.task_id, state_value)
            {
                eprintln!(
                    "[OxygenRouter] task {} poll state could not be stored: {error}",
                    task.task_id
                );
            }
        }

        if let Some(plan) = settlement.settle {
            settle_polled_task(state, &task, plan);
        }
    }
    (polled, advanced)
}

/// Reserve a task's quota, and describe it for the settlement that comes later.
///
/// `Ok(None)` when nothing is billed: a channel-only deployment has no tokens, and
/// a key with no owning user is not wallet-backed. Same shape as the relay path, so
/// a task and a chat call agree about who pays.
fn reserve_task_quota(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    model: &str,
    client_body: &[u8],
) -> Result<Option<oxygenrouter_core::TaskBilling>, Response> {
    let Some(api_key) = authorize_for_task(state, headers, model) else {
        return Ok(None);
    };
    let Some((key_id, user_id)) = billing_target(state, &Some(api_key.clone())) else {
        return Ok(None);
    };
    let fallback_group = state.default_group();
    let group = if api_key.group_name.trim().is_empty() {
        fallback_group
    } else {
        api_key.group_name.clone()
    };
    // The reservation estimates from *what the caller sent*: the estimator counts
    // the prompt in the body, so an empty body reserves nothing and a task would
    // ride free. A task's exact price is what the plugin's usage hooks exist to
    // report (`adaptor.go:155`); until those are wired the estimate is the honest
    // ceiling rather than an invented number.
    let body: serde_json::Value = serde_json::from_slice(client_body)
        .unwrap_or_else(|_| serde_json::json!({}));
    let amount = state
        .billing
        .reservation(model, &group, &body, "/v1/tasks");
    let funding = match state.db.subscription_funding_source(&user_id, amount) {
        Ok(Some(subscription)) => oxygenrouter_billing::FundingSource::Subscription {
            user_id: user_id.clone(),
            subscription_id: subscription.id.clone(),
        },
        Ok(None) => oxygenrouter_billing::FundingSource::Wallet {
            user_id: user_id.clone(),
        },
        Err(error) => {
            eprintln!(
                "[OxygenRouter] subscription lookup failed for user {user_id} ({error}); billing the wallet"
            );
            oxygenrouter_billing::FundingSource::Wallet {
                user_id: user_id.clone(),
            }
        }
    };
    match state
        .billing
        .begin(state.billing_store.as_ref(), &key_id, &funding, false, amount)
    {
        Ok((_session, reserved)) => {
            let subscription_id = match &funding {
                oxygenrouter_billing::FundingSource::Subscription {
                    subscription_id, ..
                } => subscription_id.clone(),
                oxygenrouter_billing::FundingSource::Wallet { .. } => String::new(),
            };
            Ok(Some(oxygenrouter_core::TaskBilling {
                key_id,
                user_id,
                group,
                model: model.to_string(),
                reserved_micros: reserved,
                from_subscription: !subscription_id.is_empty(),
                subscription_id,
                settled: false,
            }))
        }
        // A refused reservation is the client's condition: NewAPI answers
        // insufficient quota with 403, not 429.
        Err(error) => Err(json_policy_error(
            StatusCode::FORBIDDEN,
            "insufficient_quota",
            &error.to_string(),
        )),
    }
}

/// Resolve the caller's key for a task without recording a usage tick first.
///
/// The relay path's `authorize` bumps the key's last-used stamp, which is right for
/// a call that may or may not spend; a task's reservation is what decides whether
/// this request is billed at all, so the stamp is written once the key resolves.
fn authorize_for_task(state: &AppState, headers: &axum::http::HeaderMap, model: &str) -> Option<ApiKey> {
    if !state.db.has_active_api_keys().unwrap_or(false) {
        return None;
    }
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
        })?;
    let key = state
        .db
        .resolve_api_key(token, model, &remote_ip(headers))
        .ok()?;
    let _ = state.db.record_api_key_usage(&key.id, 0);
    Some(key)
}

/// Settle a synchronously-answered task's reservation.
fn settle_immediate_task(
    state: &AppState,
    billing: Option<&oxygenrouter_core::TaskBilling>,
    result: &oxygenrouter_plugin::TaskResult,
    response: Response,
) -> Response {
    let Some(billing) = billing else {
        return response;
    };
    // A request that already failed has nothing to settle: its reservation is
    // returned here, and doing it twice would pay the caller twice.
    if !response.status().is_success() {
        refund_task_quota(state, billing);
        return response;
    }
    let has_usage = result.total_tokens > 0.0 || result.completion_tokens > 0.0;
    match oxygenrouter_plugin::settle_plan(&result.status, has_usage, false) {
        oxygenrouter_plugin::SettlePlan::Refund => refund_task_quota(state, billing),
        oxygenrouter_plugin::SettlePlan::KeepReservation => {}
        // The usage-fact settlement is a separate piece of work. Saying so is the
        // difference between a known gap and a silent one; the reservation stands
        // meanwhile, so a caller is never under-charged by omission.
        oxygenrouter_plugin::SettlePlan::SettleWithUsage => eprintln!(
            "[OxygenRouter] a synchronous task reported usage, but task usage settlement is \
             not wired; the reservation of {} micros stands",
            billing.reserved_micros
        ),
    }
    response
}

/// Return a task's reservation in full.
fn refund_task_quota(state: &AppState, billing: &oxygenrouter_core::TaskBilling) {
    if billing.reserved_micros <= 0 || billing.key_id.trim().is_empty() {
        return;
    }
    let funding = task_funding(billing);
    match state.billing.refund(
        state.billing_store.as_ref(),
        &task_session(billing),
        &billing.key_id,
        &billing.user_id,
        &funding,
    ) {
        Ok(()) => println!(
            "[OxygenRouter] task {} refunded {} micros to key {}",
            billing.model, billing.reserved_micros, billing.key_id
        ),
        Err(error) => eprintln!(
            "[OxygenRouter] task refund for key {} failed: {error}",
            billing.key_id
        ),
    }
}

/// Rebuild the session a stored task's reservation belongs to.
///
/// The reservation outlives the request session that made it, so a later refund
/// or settle has to carry the reserved amount back into the session. A fresh
/// session has no memory of the pre-consume, and its `refund` would hand back
/// zero -- a failed task keeping the caller's money.
fn task_session(billing: &oxygenrouter_core::TaskBilling) -> oxygenrouter_billing::BillingSession {
    oxygenrouter_billing::BillingSession::restored(
        billing.key_id.clone(),
        billing.user_id.clone(),
        false,
        billing.reserved_micros,
    )
}

/// The funding source a stored task settles against.
fn task_funding(billing: &oxygenrouter_core::TaskBilling) -> oxygenrouter_billing::FundingSource {
    if billing.from_subscription && !billing.subscription_id.is_empty() {
        oxygenrouter_billing::FundingSource::Subscription {
            user_id: billing.user_id.clone(),
            subscription_id: billing.subscription_id.clone(),
        }
    } else {
        oxygenrouter_billing::FundingSource::Wallet {
            user_id: billing.user_id.clone(),
        }
    }
}

/// Settle a terminal task's reservation, once.
///
/// The idempotence guard is the stored record's own `settled` flag: the
/// compare-and-set that got here already made this the single writer for this
/// transition, and the flag is what stops a second finisher from moving money
/// again. A task with no record was never reserved (a channel-only deployment),
/// which is why "no record" is a no-op rather than an error.
fn settle_polled_task(
    state: &AppState,
    task: &oxygenrouter_core::TaskRecord,
    plan: oxygenrouter_plugin::SettlePlan,
) {
    let Some(billing) = task.private.billing.as_ref() else {
        return;
    };
    if billing.settled {
        return;
    }
    match plan {
        // A task that failed costs nothing.
        oxygenrouter_plugin::SettlePlan::Refund => {
            refund_task_quota(state, billing);
            mark_settled(state, task);
        }
        // The price was the contract: a per-call task keeps what it reserved.
        oxygenrouter_plugin::SettlePlan::KeepReservation => mark_settled(state, task),
        oxygenrouter_plugin::SettlePlan::SettleWithUsage => eprintln!(
            "[OxygenRouter] task {} reported usage, but task usage settlement is not wired; \
             the reservation of {} micros stands",
            task.task_id, billing.reserved_micros
        ),
    }
}

/// Record that a task's quota has moved, so a second finisher is a no-op.
fn mark_settled(state: &AppState, task: &oxygenrouter_core::TaskRecord) {
    if let Err(error) = state.db.mark_task_billing_settled(&task.task_id) {
        eprintln!(
            "[OxygenRouter] task {} could not record its settlement: {error}",
            task.task_id
        );
    }
}

/// How long a task may live before it is failed and refunded.
///
/// The reference keeps this as a deployment constant rather than a per-instance
/// option -- `constant.TaskTimeoutMinutes`, read from `TASK_TIMEOUT_MINUTES` with
/// a one-day default (`common/init.go:203`), and it skips the sweep entirely when
/// the value is not positive (`service/task_polling.go:71`). Mirroring that matters
/// more than the convenience of a console field: an instance whose timeout lives in
/// a setting the reference does not have is an instance whose behaviour cannot be
/// compared with it.
pub fn task_timeout_secs() -> i64 {
    let minutes: i64 = std::env::var("TASK_TIMEOUT_MINUTES")
        .ok()
        .and_then(|raw| raw.trim().parse().ok())
        .unwrap_or(1440);
    if minutes <= 0 {
        0
    } else {
        minutes.saturating_mul(60)
    }
}

/// Answer a task read: the stored task's public shape, or a reason it is absent.
///
/// The reference distinguishes five reasons a read fails
/// (`controller/task.go:167,253`: a malformed id, a task that does not exist, a
/// task whose plugin is no longer installed, a task whose plugin no longer claims
/// the protocol, and a task whose result was deliberately discarded). A caller
/// debugging a 404 needs to know which one it hit, so they are not collapsed.
pub fn task_read_response(state: &AppState, task_id: &str) -> Response {
    let Some(task) = state.db.get_task(task_id).ok().flatten() else {
        return not_found_response(
            "missing",
            &format!("no task {task_id:?} on this instance"),
        );
    };
    // The plugin has to still be able to render this task; a plugin that was
    // removed or disabled leaves a task that nothing can shape.
    if state.db.plugin_manifest(&task.platform).ok().flatten().is_none() {
        return not_found_response(
            "no_plugin",
            &format!(
                "task {task_id:?} belongs to plugin {:?}, which is no longer installed or enabled",
                task.platform
            ),
        );
    }
    let view = oxygenrouter_plugin::TaskView::build(
        &task.task_id,
        &task.platform,
        &task.status,
        &task.progress,
        &task.fail_reason,
        task.created_at.timestamp(),
        task.updated_at.timestamp(),
        task.finish_time.map(|at| at.timestamp()).unwrap_or(0),
        task.data.clone(),
        &task.private.upstream_task_id,
    );
    (StatusCode::OK, axum::Json(view)).into_response()
}

/// A read failure in the reference's OpenAI error shape
/// (`controller/plugin_protocol.go`'s `writeTaskPluginResponseNotFound`).
fn not_found_response(code: &str, message: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({
            "error": {
                "code": code,
                "message": message,
                "type": "invalid_request_error",
            }
        })),
    )
        .into_response()
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
        client_headers: client_headers(&headers),
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

/// Every header the caller sent, for policy that inspects the request.
///
/// Not forwarded anywhere: `forwarded_headers` decides what the upstream sees,
/// and this exists so a rule can read what the client actually said. Names are
/// lower-cased so a rule does not have to guess the client's spelling.
fn client_headers(headers: &axum::http::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_ascii_lowercase(), v.to_string()))
        })
        .collect()
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
    // A plugin bound to this endpoint and model takes the request, because only a
    // plugin can render the Responses API's own semantics; without one the
    // ordinary OpenAI-compatible dispatch answers, so the endpoint works either
    // way.
    let model = request_body_model(&headers, &body_bytes).await;
    if let Some(binding) = model
        .as_deref()
        .and_then(|model| plugin_for_endpoint(&state, "POST", "/v1/responses", model))
    {
        // A streaming request goes through the observation loop; the submission
        // itself is the same bridge either way, so a plugin that cannot stream is
        // still reachable by a client that did not ask to.
        let wants_stream = serde_json::from_slice::<serde_json::Value>(&body_bytes)
            .ok()
            .map(|body| {
                body.get("stream").and_then(|value| value.as_bool()).unwrap_or(false)
            })
            .unwrap_or(false);
        if wants_stream && binding.supports_mode("stream") {
            let submitted = plugin_bridge(
                state.clone(),
                PluginInvocation {
                    binding: &binding,
                    headers: &headers,
                    body: &body_bytes,
                },
                "/v1/responses",
            )
            .await;
            // A submission that did not produce a task is passed through as-is: an
            // immediate answer has nothing to stream, and a refusal is a refusal.
            let (parts, body) = submitted.into_parts();
            if !parts.status.is_success() {
                return Response::from_parts(parts, body);
            }
            let bytes = match axum::body::to_bytes(body, 1 << 20).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    return json_error(
                        StatusCode::BAD_GATEWAY,
                        &format!("the submission answer could not be read: {error}"),
                    )
                }
            };
            let Some(task_id) = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|value| value.get("taskId").and_then(|id| id.as_str()).map(String::from))
            else {
                return (
                    StatusCode::from_u16(parts.status.as_u16()).unwrap_or(StatusCode::OK),
                    bytes,
                )
                    .into_response();
            };
            return responses_stream(state, task_id).await;
        }
        return plugin_bridge(
            state,
            PluginInvocation {
                binding: &binding,
                headers: &headers,
                body: &body_bytes,
            },
            "/v1/responses",
        )
        .await;
    }
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
    // An edit may be multipart, in which case the model is a form field rather
    // than a JSON one; `request_body_model` reads all three encodings.
    let model = request_body_model(&headers, &body_bytes).await;
    if let Some(binding) = model
        .as_deref()
        .and_then(|model| plugin_for_endpoint(&state, "POST", "/v1/images/edits", model))
    {
        return plugin_bridge(
            state,
            PluginInvocation {
                binding: &binding,
                headers: &headers,
                body: &body_bytes,
            },
            "/v1/images/edits",
        )
        .await;
    }
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

    /// Which endpoint a plugin owns, and for which model.
    ///
    /// Matched on the host's protocol table with the `:param` segments treated as
    /// wildcards, because that table is the contract and a second hand-written
    /// mapping would drift from it. The model matters as much as the path: two
    /// plugins may share an endpoint while owning different models
    /// (`pkg/jsplugin/routing.go:994`).
    #[test]
    fn a_binding_is_looked_up_by_method_path_and_model() {
        use oxygenrouter_plugin::{EndpointClaim, EndpointIndex};

        let models: Vec<String> = vec!["acme-large".into()];
        let index = EndpointIndex::build([EndpointClaim {
            plugin_key: "acme",
            protocol: "openai_responses",
            claim_models: &[],
            models: &models,
            supports: &[],
        }]);

        assert!(index.lookup("POST", "/v1/responses", "acme-large").is_some());
        // The path alone chooses nothing: the model has to be the one bound.
        assert!(index.lookup("POST", "/v1/responses", "someone-elses-model").is_none());
        // A different method, and a different endpoint, both miss.
        assert!(index.lookup("GET", "/v1/responses", "acme-large").is_none());
        assert!(index.lookup("POST", "/v1/chat/completions", "acme-large").is_none());
    }

    /// A video *submission* is bound to a plugin, but the video *retrieve* and
    /// *content* operations are not: the host answers those from its task store,
    /// which is why they declare no model field (`pkg/jsplugin/routing.go:94,989`).
    #[test]
    fn host_answered_operations_are_not_bound_to_a_plugin() {
        use oxygenrouter_plugin::{EndpointClaim, EndpointIndex};

        let models: Vec<String> = vec!["wan-video".into()];
        let index = EndpointIndex::build([EndpointClaim {
            plugin_key: "alibaba",
            protocol: "openai_video",
            claim_models: &[],
            models: &models,
            supports: &[],
        }]);

        assert!(index.lookup("POST", "/v1/videos", "wan-video").is_some());
        assert!(index.lookup("GET", "/v1/videos/task_1", "wan-video").is_none());
        assert!(
            index
                .lookup("GET", "/v1/videos/task_1/content", "wan-video")
                .is_none()
        );
    }

    /// The context a plugin's `decodeRequest` reads is the reference's shape, and
    /// the body is tagged so a hook can tell encodings apart
    /// (`pkg/jsplugin/routing.go:293`, `middleware/task_plugin.go:793`).
    #[tokio::test]
    async fn the_protocol_context_is_built_from_what_the_client_sent() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        );
        let body = br#"{"model":"acme-large","input":"hi","stream":true}"#;
        let (context, value) = build_protocol_context(
            &headers,
            body,
            "/v1/responses",
            "openai_responses",
            "create",
            "acme-large",
            "",
        )
        .await
        .expect("context");

        assert_eq!(value["protocol"], "openai_responses");
        assert_eq!(value["operation"], "create");
        assert_eq!(value["model"], "acme-large");
        assert_eq!(value["stream"], true);
        assert_eq!(value["body"]["kind"], "json");
        assert_eq!(value["body"]["value"]["input"], "hi");
        assert!(context.stream);

        // A form body declares itself and keeps its fields.
        let mut form_headers = axum::http::HeaderMap::new();
        form_headers.insert(
            header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        let (_, form) = build_protocol_context(
            &form_headers,
            b"model=acme-large&prompt=a+cat",
            "/v1/images/edits",
            "openai_image",
            "edit",
            "acme-large",
            "",
        )
        .await
        .expect("form context");
        assert_eq!(form["body"]["kind"], "form");
        assert_eq!(form["body"]["fields"]["prompt"][0], "a cat");
        assert_eq!(form["stream"], false);

        // An empty body is "none", not an empty object.
        let (_, empty) = build_protocol_context(
            &headers,
            b"",
            "/v1/images/generations",
            "openai_image",
            "generate",
            "acme-large",
            "",
        )
        .await
        .expect("empty context");
        assert_eq!(empty["body"]["kind"], "none");

        // Malformed JSON is refused rather than silently treated as empty.
        assert!(build_protocol_context(
            &headers,
            b"{not json",
            "/v1/responses",
            "openai_responses",
            "create",
            "acme-large",
            "",
        )
        .await
        .is_err());
    }

    /// A multipart body yields text fields and file *references*; the bytes stay
    /// host-owned (`pkg/jsplugin/routing.go:248`).
    #[tokio::test]
    async fn a_multipart_body_carries_file_references_not_bytes() {
        let boundary = "XBOUNDARYX";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nacme-large\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"prompt\"\r\n\r\na cat\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"image[]\"; filename=\"cat.png\"\r\n\
             Content-Type: image/png\r\n\r\nPNGBYTES\r\n--{boundary}--\r\n"
        );
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            axum::http::HeaderValue::from_str(&format!("multipart/form-data; boundary={boundary}"))
                .expect("header"),
        );

        let (_, value) = build_protocol_context(
            &headers,
            body.as_bytes(),
            "/v1/images/edits",
            "openai_image",
            "edit",
            "acme-large",
            "",
        )
        .await
        .expect("multipart context");

        assert_eq!(value["body"]["kind"], "multipart");
        assert_eq!(value["body"]["fields"]["model"][0], "acme-large");
        assert_eq!(value["body"]["files"][0]["ref"], "request_file:image[]");
        assert_eq!(value["body"]["files"][0]["filename"], "cat.png");
        assert_eq!(value["body"]["files"][0]["mimeType"], "image/png");
        // The bytes are not in the context at all.
        let rendered = value.to_string();
        assert!(!rendered.contains("PNGBYTES"), "{rendered}");

        // The model is readable from the multipart body, which is how the host
        // finds the binding before any hook runs.
        assert_eq!(
            request_body_model(&headers, body.as_bytes()).await.as_deref(),
            Some("acme-large")
        );
    }

    /// A plugin bound to an endpoint must have claimed the request form the
    /// request needs, and the refusal names both sides
    /// (`middleware/task_plugin.go:382,412`).
    #[tokio::test]
    async fn a_request_form_the_plugin_did_not_claim_is_refused_by_name() {
        let mode_less = oxygenrouter_plugin::required_modes(false, false);
        assert_eq!(mode_less, vec!["sync"]);
        assert_eq!(
            oxygenrouter_plugin::required_modes(true, false),
            vec!["stream"]
        );

        let models: Vec<String> = vec!["acme".into()];
        let index = oxygenrouter_plugin::EndpointIndex::build([
            oxygenrouter_plugin::EndpointClaim {
                plugin_key: "streamer",
                protocol: "openai_responses",
                claim_models: &[],
                models: &models,
                supports: &["stream".to_string()],
            },
        ]);
        let candidates = index.candidates("POST", "/v1/responses", "acme");
        assert!(candidates[0].supports_mode("stream"));
        assert!(!candidates[0].supports_mode("sync"));

        let message =
            oxygenrouter_plugin::unsupported_form_message(&candidates, "openai_responses", false, false);
        assert!(message.contains("synchronous"), "{message}");
        assert!(message.contains("\"stream\""), "{message}");
    }
}
