//! Ratio sync: compare this instance's price tables with another instance's.
//!
//! Mirrors the reference's `/api/ratio_sync` group (`controller/ratio_sync.go`):
//! `GET /channels` lists the sources an operator can sync from, and
//! `POST /fetch` reads each source's public price payload and reports what
//! differs from here.
//!
//! The reference's handler is a preview: it returns differences and leaves
//! applying them to the versioned pricing editor. This implementation keeps the
//! same boundary, and reads the same source shapes: the two NewAPI price
//! payloads, OpenRouter `/v1/models`, and models.dev `/api.json`.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use futures_util::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::state::AppState;
use oxygenrouter_core::ApiResponse;

const DEFAULT_ENDPOINT: &str = "/api/pricing";
const DEFAULT_TIMEOUT_SECONDS: u64 = 10;
const MAX_UPSTREAMS: usize = 20;
const MAX_CONCURRENT_FETCHES: usize = 8;
const NEARLY_EQUAL: f64 = 1e-9;
const OFFICIAL_ID: &str = "official";
const OFFICIAL_NAME: &str = "Official preset";
const OFFICIAL_BASE_URL: &str = "https://basellm.github.io";
const OFFICIAL_ENDPOINT: &str = "/llm-metadata/api/newapi/ratio_config-v1-base.json";
const MODELS_DEV_ID: &str = "models.dev";
const MODELS_DEV_NAME: &str = "models.dev preset";
const MODELS_DEV_BASE_URL: &str = "https://models.dev";
const MODELS_DEV_ENDPOINT: &str = "https://models.dev/api.json";
const OPENROUTER_ENDPOINT: &str = "openrouter";

const SYNC_FIELDS: [&str; 10] = [
    "model_ratio",
    "completion_ratio",
    "cache_ratio",
    "create_cache_ratio",
    "image_ratio",
    "audio_ratio",
    "audio_completion_ratio",
    "model_price",
    "billing_mode",
    "billing_expr",
];

fn is_numeric_field(field: &str) -> bool {
    matches!(
        field,
        "model_ratio"
            | "completion_ratio"
            | "cache_ratio"
            | "create_cache_ratio"
            | "image_ratio"
            | "audio_ratio"
            | "audio_completion_ratio"
            | "model_price"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceKind {
    Standard,
    OpenRouter,
    ModelsDev,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SyncableChannel {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub endpoint: String,
    /// `channel`, `official`, or `models_dev`.
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpstreamTarget {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Used only for an explicit OpenRouter source. Channel-backed sources read
    /// the credential from the channel row instead.
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FetchRequest {
    #[serde(default)]
    pub channel_ids: Vec<String>,
    #[serde(default)]
    pub upstreams: Vec<UpstreamTarget>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FieldDifference {
    pub field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelDifference {
    pub model_name: String,
    pub differences: Vec<FieldDifference>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UpstreamReport {
    pub name: String,
    pub base_url: String,
    pub endpoint: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    pub upstream_models: usize,
    pub models: Vec<ModelDifference>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FetchReport {
    pub upstreams: Vec<UpstreamReport>,
    pub new_models: Vec<String>,
}

type SyncValues = BTreeMap<String, Value>;
type SyncModels = BTreeMap<String, SyncValues>;

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn as_number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|raw| raw.trim().parse::<f64>().ok()))
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn as_text(value: &Value) -> Option<String> {
    value.as_str().map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

fn insert_value(models: &mut SyncModels, model: &str, field: &str, value: Value) {
    if model.trim().is_empty() {
        return;
    }
    if is_numeric_field(field) && as_number(&value).is_none() {
        return;
    }
    models
        .entry(model.to_string())
        .or_default()
        .insert(field.to_string(), value);
}

/// Parse the two payload shapes the reference accepts:
/// `data: {field: {model: value}}` and `data: [{model_name, ...}]`.
fn parse_standard_payload(body: &str) -> Result<SyncModels, String> {
    let document: Value =
        serde_json::from_str(body).map_err(|error| format!("the source's payload is not JSON: {error}"))?;

    let payload = document.get("data").unwrap_or(&document);
    if let Some(array) = payload.as_array() {
        return parse_pricing_array(array);
    }
    if let Some(object) = payload.as_object() {
        if SYNC_FIELDS.iter().any(|field| object.contains_key(*field)) {
            return parse_ratio_maps(object);
        }
    }
    Err("the source's payload is neither a ratio map nor a pricing list".to_string())
}

fn parse_ratio_maps(object: &serde_json::Map<String, Value>) -> Result<SyncModels, String> {
    let mut models = SyncModels::new();
    for field in SYNC_FIELDS {
        let Some(entries) = object.get(field).and_then(Value::as_object) else {
            continue;
        };
        for (model, value) in entries {
            if is_numeric_field(field) {
                if let Some(number) = as_number(value) {
                    insert_value(&mut models, model, field, json!(number));
                }
            } else if let Some(text) = as_text(value) {
                insert_value(&mut models, model, field, json!(text));
            }
        }
    }
    if models.is_empty() {
        Err("the source's ratio maps contained no usable models".to_string())
    } else {
        Ok(models)
    }
}

fn parse_pricing_array(items: &[Value]) -> Result<SyncModels, String> {
    let mut models = SyncModels::new();
    for item in items {
        let Some(object) = item.as_object() else {
            continue;
        };
        let Some(model) = object.get("model_name").and_then(Value::as_str) else {
            continue;
        };
        if as_text(object.get("billing_mode").unwrap_or(&Value::Null)).as_deref()
            == Some("tiered_expr")
        {
            insert_value(&mut models, model, "billing_mode", json!("tiered_expr"));
            if let Some(expression) = object.get("billing_expr").and_then(Value::as_str) {
                if !expression.trim().is_empty() {
                    insert_value(&mut models, model, "billing_expr", json!(expression));
                }
            }
            continue;
        }
        let quota_type = object.get("quota_type").and_then(Value::as_i64).unwrap_or(0);
        if quota_type == 1 {
            if let Some(price) = object.get("model_price").and_then(as_number) {
                insert_value(&mut models, model, "model_price", json!(price));
            }
        } else {
            if let Some(value) = object.get("model_ratio").and_then(as_number) {
                insert_value(&mut models, model, "model_ratio", json!(value));
            }
            if let Some(value) = object.get("completion_ratio").and_then(as_number) {
                insert_value(&mut models, model, "completion_ratio", json!(value));
            }
        }
        for (field, json_key) in [
            ("cache_ratio", "cache_ratio"),
            ("create_cache_ratio", "create_cache_ratio"),
            ("image_ratio", "image_ratio"),
            ("audio_ratio", "audio_ratio"),
            ("audio_completion_ratio", "audio_completion_ratio"),
        ] {
            if let Some(value) = object.get(json_key).and_then(as_number) {
                insert_value(&mut models, model, field, json!(value));
            }
        }
    }
    if models.is_empty() {
        Err("the source's pricing list contained no usable models".to_string())
    } else {
        Ok(models)
    }
}

fn parse_openrouter_payload(body: &str) -> Result<SyncModels, String> {
    let document: Value =
        serde_json::from_str(body).map_err(|error| format!("OpenRouter's payload is not JSON: {error}"))?;
    let entries = document
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "OpenRouter's payload has no data list".to_string())?;
    let mut models = SyncModels::new();
    for entry in entries {
        let Some(model) = entry.get("id").and_then(Value::as_str) else {
            continue;
        };
        let pricing = entry.get("pricing").and_then(Value::as_object);
        let prompt = pricing
            .and_then(|p| p.get("prompt"))
            .and_then(as_number);
        let completion = pricing
            .and_then(|p| p.get("completion"))
            .and_then(as_number);
        let (Some(prompt), Some(completion)) = (prompt, completion) else {
            continue;
        };
        if prompt < 0.0 || completion < 0.0 {
            continue;
        }
        if prompt == 0.0 && completion == 0.0 {
            insert_value(&mut models, model, "model_ratio", json!(0.0));
            continue;
        }
        if prompt <= 0.0 {
            continue;
        }
        insert_value(
            &mut models,
            model,
            "model_ratio",
            json!(round6(prompt * 500_000.0)),
        );
        insert_value(
            &mut models,
            model,
            "completion_ratio",
            json!(round6(completion / prompt)),
        );
        if let Some(cache) = pricing
            .and_then(|p| p.get("input_cache_read"))
            .and_then(as_number)
        {
            insert_value(
                &mut models,
                model,
                "cache_ratio",
                json!(round6(cache / prompt)),
            );
        }
    }
    if models.is_empty() {
        Err("OpenRouter's payload contained no usable models".to_string())
    } else {
        Ok(models)
    }
}

#[derive(Debug, Clone)]
struct ModelsDevCandidate {
    provider: String,
    input: f64,
    output: Option<f64>,
    cache_read: Option<f64>,
}

fn parse_models_dev_payload(body: &str) -> Result<SyncModels, String> {
    let document: Value =
        serde_json::from_str(body).map_err(|error| format!("models.dev's payload is not JSON: {error}"))?;
    let providers = document
        .as_object()
        .ok_or_else(|| "models.dev's payload is not a provider map".to_string())?;
    let mut selected: BTreeMap<String, ModelsDevCandidate> = BTreeMap::new();
    for (provider, provider_value) in providers {
        let Some(models) = provider_value.get("models").and_then(Value::as_object) else {
            continue;
        };
        for (model, model_value) in models {
            let Some(cost) = model_value.get("cost").and_then(Value::as_object) else {
                continue;
            };
            let Some(input) = cost.get("input").and_then(as_number) else {
                continue;
            };
            if input < 0.0 {
                continue;
            }
            let output = cost.get("output").and_then(as_number);
            let cache_read = cost.get("cache_read").and_then(as_number);
            if input == 0.0 && output.unwrap_or(0.0) > 0.0 {
                continue;
            }
            let candidate = ModelsDevCandidate {
                provider: provider.clone(),
                input,
                output,
                cache_read,
            };
            let replace = match selected.get(model) {
                None => true,
                Some(current) => {
                    (current.input == 0.0 && candidate.input > 0.0)
                        || (current.input > 0.0
                            && candidate.input > 0.0
                            && (candidate.input, &candidate.provider)
                                < (current.input, &current.provider))
                }
            };
            if replace {
                selected.insert(model.clone(), candidate);
            }
        }
    }
    let mut models = SyncModels::new();
    for (model, candidate) in selected {
        if candidate.input == 0.0 {
            insert_value(&mut models, &model, "model_ratio", json!(0.0));
            continue;
        }
        insert_value(
            &mut models,
            &model,
            "model_ratio",
            json!(round6(candidate.input / 2.0)),
        );
        if let Some(output) = candidate.output {
            insert_value(
                &mut models,
                &model,
                "completion_ratio",
                json!(round6(output / candidate.input)),
            );
        }
        if let Some(cache_read) = candidate.cache_read {
            insert_value(
                &mut models,
                &model,
                "cache_ratio",
                json!(round6(cache_read / candidate.input)),
            );
        }
    }
    if models.is_empty() {
        Err("models.dev's payload contained no usable pricing entries".to_string())
    } else {
        Ok(models)
    }
}

fn parse_source_payload(kind: SourceKind, body: &str) -> Result<SyncModels, String> {
    match kind {
        SourceKind::Standard => parse_standard_payload(body),
        SourceKind::OpenRouter => parse_openrouter_payload(body),
        SourceKind::ModelsDev => parse_models_dev_payload(body),
    }
}

fn detect_source_kind(base_url: &str, endpoint: &str) -> SourceKind {
    let full = if endpoint.starts_with("http") {
        endpoint.to_string()
    } else {
        format!("{}{}", base_url.trim_end_matches('/'), endpoint)
    };
    if endpoint.eq_ignore_ascii_case(OPENROUTER_ENDPOINT)
        || full.to_ascii_lowercase().contains("openrouter.ai")
    {
        SourceKind::OpenRouter
    } else if full.to_ascii_lowercase().contains("models.dev/api.json") {
        SourceKind::ModelsDev
    } else {
        SourceKind::Standard
    }
}

fn syncable_channel_from_db(channel: oxygenrouter_core::Channel) -> SyncableChannel {
    SyncableChannel {
        id: channel.id,
        name: channel.name,
        base_url: channel.base_url.trim().trim_end_matches('/').to_string(),
        endpoint: DEFAULT_ENDPOINT.to_string(),
        kind: "channel".to_string(),
    }
}

/// The channels and built-in presets an operator can sync from.
pub fn syncable_channels(state: &AppState) -> Result<Vec<SyncableChannel>, String> {
    let mut channels: Vec<SyncableChannel> = state
        .db
        .list_channels()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|channel| channel.enabled)
        .filter(|channel| {
            let base = channel.base_url.trim();
            base.starts_with("http://") || base.starts_with("https://")
        })
        .map(syncable_channel_from_db)
        .collect();
    channels.push(SyncableChannel {
        id: OFFICIAL_ID.to_string(),
        name: OFFICIAL_NAME.to_string(),
        base_url: OFFICIAL_BASE_URL.to_string(),
        endpoint: OFFICIAL_ENDPOINT.to_string(),
        kind: "official".to_string(),
    });
    channels.push(SyncableChannel {
        id: MODELS_DEV_ID.to_string(),
        name: MODELS_DEV_NAME.to_string(),
        base_url: MODELS_DEV_BASE_URL.to_string(),
        endpoint: MODELS_DEV_ENDPOINT.to_string(),
        kind: "models_dev".to_string(),
    });
    Ok(channels)
}

/// This instance's own ratios, in the same shape.
fn local_models(state: &AppState) -> SyncModels {
    let pricing = state.billing.pricing();
    let guard = pricing.read();
    guard
        .sync_models()
        .into_iter()
        .map(|model| {
            let mut values = SyncValues::new();
            for field in SYNC_FIELDS.iter().filter(|field| is_numeric_field(field)) {
                if let Some(value) = guard.sync_numeric(field, &model) {
                    values.insert((*field).to_string(), json!(value));
                }
            }
            if let Some(mode) = guard.sync_billing_mode(&model) {
                values.insert("billing_mode".to_string(), json!(mode));
            }
            if let Some(expression) = guard.expression(&model) {
                values.insert("billing_expr".to_string(), json!(expression));
            }
            (model, values)
        })
        .collect()
}

fn values_equal(left: &Value, right: &Value) -> bool {
    match (as_number(left), as_number(right)) {
        (Some(left), Some(right)) => (left - right).abs() < NEARLY_EQUAL,
        _ => left == right,
    }
}

fn expression_priority(local: &SyncValues, upstream: &SyncValues) -> bool {
    local.get("billing_mode").and_then(Value::as_str) == Some("tiered_expr")
        || upstream.get("billing_mode").and_then(Value::as_str) == Some("tiered_expr")
}

fn differences_for(local: &SyncModels, upstream: &SyncModels) -> (Vec<ModelDifference>, Vec<String>) {
    let mut models = Vec::new();
    let mut new_models = Vec::new();
    for (name, upstream_values) in upstream {
        let Some(local_values) = local.get(name) else {
            new_models.push(name.clone());
            continue;
        };
        let expression = expression_priority(local_values, upstream_values);
        let mut fields = Vec::new();
        for field in SYNC_FIELDS {
            if expression && is_numeric_field(field) {
                continue;
            }
            let local_value = local_values.get(field).cloned();
            let upstream_value = upstream_values.get(field).cloned();
            match (&local_value, &upstream_value) {
                (Some(left), Some(right)) if !values_equal(left, right) => {
                    fields.push(FieldDifference {
                        field: field.to_string(),
                        local: local_value,
                        upstream: upstream_value,
                    });
                }
                (None, Some(_)) => fields.push(FieldDifference {
                    field: field.to_string(),
                    local: None,
                    upstream: upstream_value,
                }),
                _ => {}
            }
        }
        if !fields.is_empty() {
            models.push(ModelDifference {
                model_name: name.clone(),
                differences: fields,
            });
        }
    }
    (models, new_models)
}

async fn fetch_one(
    client: &reqwest::Client,
    name: String,
    base_url: String,
    endpoint: String,
    kind: SourceKind,
    api_key: Option<String>,
    local: &SyncModels,
) -> (UpstreamReport, Vec<String>) {
    let url = if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        endpoint.clone()
    } else if kind == SourceKind::OpenRouter {
        format!("{}/v1/models", base_url.trim_end_matches('/'))
    } else {
        let path = if endpoint.starts_with('/') {
            endpoint.clone()
        } else {
            format!("/{endpoint}")
        };
        format!("{}{}", base_url.trim_end_matches('/'), path)
    };
    let mut report = UpstreamReport {
        name,
        base_url,
        endpoint,
        ok: false,
        error: String::new(),
        upstream_models: 0,
        models: Vec::new(),
    };
    let mut request = client.get(&url);
    if let Some(key) = api_key.as_deref().map(str::trim).filter(|key| !key.is_empty()) {
        request = request.bearer_auth(key);
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            report.error = format!("could not reach the source: {error}");
            return (report, Vec::new());
        }
    };
    if !response.status().is_success() {
        report.error = format!("the source answered HTTP {}", response.status());
        return (report, Vec::new());
    }
    let body = match response.text().await {
        Ok(body) => body,
        Err(error) => {
            report.error = format!("could not read the source's answer: {error}");
            return (report, Vec::new());
        }
    };
    let upstream = match parse_source_payload(kind, &body) {
        Ok(models) => models,
        Err(error) => {
            report.error = error;
            return (report, Vec::new());
        }
    };
    report.upstream_models = upstream.len();
    let (models, new_models) = differences_for(local, &upstream);
    report.models = models;
    report.ok = true;
    (report, new_models)
}

pub async fn fetch(state: &AppState, request: &FetchRequest) -> Result<FetchReport, String> {
    let local = local_models(state);
    let available = syncable_channels(state)?;
    let mut targets: Vec<(String, String, String, SourceKind, Option<String>)> = Vec::new();

    for target in &request.upstreams {
        let Some(base_url) = target
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
        else {
            continue;
        };
        let endpoint = target
            .endpoint
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .unwrap_or(DEFAULT_ENDPOINT)
            .to_string();
        let kind = detect_source_kind(base_url, &endpoint);
        targets.push((
            target
                .name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| base_url.to_string()),
            base_url.trim_end_matches('/').to_string(),
            endpoint,
            kind,
            target.api_key.clone(),
        ));
    }

    for id in &request.channel_ids {
        let Some(source) = available.iter().find(|source| &source.id == id) else {
            continue;
        };
        let api_key = if source.kind == "channel" {
            state
                .db
                .get_channel(&source.id)
                .ok()
                .flatten()
                .map(|channel| channel.api_key)
        } else {
            None
        };
        targets.push((
            source.name.clone(),
            source.base_url.clone(),
            source.endpoint.clone(),
            detect_source_kind(&source.base_url, &source.endpoint),
            api_key,
        ));
    }

    if targets.is_empty() {
        return Err("no usable upstream was given".to_string());
    }
    targets.truncate(MAX_UPSTREAMS);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(
            request
                .timeout_seconds
                .filter(|seconds| *seconds > 0)
                .unwrap_or(DEFAULT_TIMEOUT_SECONDS),
        ))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?;

    let results = stream::iter(targets.into_iter().map(
        |(name, base_url, endpoint, kind, api_key)| {
            let client = client.clone();
            let local = local.clone();
            async move { fetch_one(&client, name, base_url, endpoint, kind, api_key, &local).await }
        },
    ))
    .buffer_unordered(MAX_CONCURRENT_FETCHES)
    .collect::<Vec<_>>()
    .await;

    let mut new_models: Vec<String> = Vec::new();
    let mut upstreams = Vec::with_capacity(results.len());
    for (report, mut added) in results {
        new_models.append(&mut added);
        upstreams.push(report);
    }
    new_models.sort();
    new_models.dedup();

    Ok(FetchReport {
        upstreams,
        new_models,
    })
}

fn bad_request<T: Serialize>(message: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiResponse::<T>::err(message)),
    )
        .into_response()
}

/// `GET /api/ratio_sync/channels`
pub async fn list_syncable_channels(State(state): State<Arc<AppState>>) -> Response {
    match syncable_channels(&state) {
        Ok(channels) => Json(ApiResponse::ok(channels)).into_response(),
        Err(error) => bad_request::<Vec<SyncableChannel>>(error),
    }
}

/// `POST /api/ratio_sync/fetch`
pub async fn fetch_upstreams(
    State(state): State<Arc<AppState>>,
    Json(request): Json<FetchRequest>,
) -> Response {
    match fetch(&state, &request).await {
        Ok(report) => Json(ApiResponse::ok(report)).into_response(),
        Err(error) => bad_request::<FetchReport>(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models(entries: &[(&str, &str, Value)]) -> SyncModels {
        let mut out = SyncModels::new();
        for (name, field, value) in entries {
            insert_value(&mut out, name, field, value.clone());
        }
        out
    }

    #[test]
    fn only_real_differences_are_reported() {
        let local = models(&[
            ("same", "model_ratio", json!(1.0)),
            ("noisy", "model_ratio", json!(0.8)),
            ("changed", "model_ratio", json!(1.0)),
        ]);
        let upstream = models(&[
            ("same", "model_ratio", json!(1.0)),
            ("noisy", "model_ratio", json!(0.8000000001)),
            ("changed", "model_ratio", json!(2.0)),
        ]);
        let (differences, new_models) = differences_for(&local, &upstream);
        assert!(new_models.is_empty());
        assert_eq!(differences.len(), 1);
        assert_eq!(differences[0].model_name, "changed");
        assert_eq!(differences[0].differences.len(), 1);
        assert_eq!(differences[0].differences[0].local, Some(json!(1.0)));
        assert_eq!(differences[0].differences[0].upstream, Some(json!(2.0)));
    }

    #[test]
    fn an_unknown_model_is_listed_as_new() {
        let local = models(&[("known", "model_ratio", json!(1.0))]);
        let upstream = models(&[
            ("known", "model_ratio", json!(1.0)),
            ("brand-new", "model_ratio", json!(3.0)),
        ]);
        let (differences, new_models) = differences_for(&local, &upstream);
        assert!(differences.is_empty());
        assert_eq!(new_models, vec!["brand-new".to_string()]);
    }

    #[test]
    fn a_missing_field_is_not_treated_as_zero() {
        let local = SyncModels::new();
        let upstream = models(&[("only-upstream", "model_ratio", json!(0.5))]);
        let (differences, new_models) = differences_for(&local, &upstream);
        assert!(differences.is_empty());
        assert_eq!(new_models, vec!["only-upstream".to_string()]);
    }

    #[test]
    fn the_standard_payloads_are_parsed_and_expression_priority_is_kept() {
        let map = parse_standard_payload(
            r#"{"success":true,"data":{
                "model_ratio":{"m1":2.5},
                "completion_ratio":{"m1":3.0},
                "billing_mode":{"m2":"tiered_expr"},
                "billing_expr":{"m2":"tier(\"base\", p * 2)"}
            }}"#,
        )
        .expect("ratio map");
        assert_eq!(map["m1"]["model_ratio"], json!(2.5));
        assert_eq!(map["m2"]["billing_mode"], json!("tiered_expr"));

        let list = parse_standard_payload(
            r#"{"success":true,"data":[
                {"model_name":"legacy","quota_type":0,"model_ratio":1.25,"completion_ratio":4.0},
                {"model_name":"fixed","quota_type":1,"model_price":0.04},
                {"model_name":"tiered","billing_mode":"tiered_expr","billing_expr":"tier(\"x\", p * 1)"}
            ]}"#,
        )
        .expect("pricing list");
        assert_eq!(list["legacy"]["completion_ratio"], json!(4.0));
        assert_eq!(list["fixed"]["model_price"], json!(0.04));
        assert_eq!(list["tiered"]["billing_mode"], json!("tiered_expr"));
        assert!(!list["tiered"].contains_key("model_ratio"));
    }

    #[test]
    fn openrouter_and_models_dev_are_converted_to_local_ratios() {
        let openrouter = parse_openrouter_payload(
            r#"{"data":[{"id":"or-model","pricing":{"prompt":"0.000002","completion":"0.000004","input_cache_read":"0.000001"}}]}"#,
        )
        .expect("openrouter");
        assert_eq!(openrouter["or-model"]["model_ratio"], json!(1.0));
        assert_eq!(openrouter["or-model"]["completion_ratio"], json!(2.0));
        assert_eq!(openrouter["or-model"]["cache_ratio"], json!(0.5));

        let models_dev = parse_models_dev_payload(
            r#"{"provider-b":{"models":{"dev-model":{"cost":{"input":4.0,"output":8.0,"cache_read":1.0}}}},"provider-a":{"models":{"dev-model":{"cost":{"input":2.0,"output":6.0,"cache_read":0.5}}}}}"#,
        )
        .expect("models.dev");
        assert_eq!(models_dev["dev-model"]["model_ratio"], json!(1.0));
        assert_eq!(models_dev["dev-model"]["completion_ratio"], json!(3.0));
        assert_eq!(models_dev["dev-model"]["cache_ratio"], json!(0.25));
    }

    #[test]
    fn a_source_that_publishes_no_audio_does_not_create_a_zero_difference() {
        let local = models(&[
            ("m1", "model_ratio", json!(1.0)),
            ("m1", "audio_ratio", json!(2.0)),
        ]);
        let upstream = models(&[("m1", "model_ratio", json!(1.0))]);
        let (differences, new_models) = differences_for(&local, &upstream);
        assert!(differences.is_empty());
        assert!(new_models.is_empty());
    }
}
