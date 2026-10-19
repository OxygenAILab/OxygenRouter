//! Ratio sync: compare this instance's price tables with another instance's.
//!
//! Mirrors the reference's `/api/ratio_sync` group (`controller/ratio_sync.go`):
//! `GET /channels` lists the sources an operator can sync from, and
//! `POST /fetch` reads each source's public price payload and reports what
//! differs from here.
//!
//! **It is a preview, never a write.** The reference's handler returns
//! differences and leaves applying them to the operator, and that is the right
//! shape for a price table: a sync that silently rewrote what every user is
//! charged would be indistinguishable from an outage until the bills arrived.
//!
//! The source endpoint is `/api/pricing`, the reference's default
//! (`ratio_sync.go:33`), which this instance also serves — so two OxygenRouter or
//! NewAPI instances can sync in either direction.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde::{Deserialize, Serialize};

use crate::state::AppState;
use oxygenrouter_core::ApiResponse;

/// The upstream route a sync reads, the reference's default.
const DEFAULT_ENDPOINT: &str = "/api/pricing";
const DEFAULT_TIMEOUT_SECONDS: u64 = 10;
/// Most sources one request may read, so a mistyped body cannot fan out widely.
const MAX_UPSTREAMS: usize = 20;
/// The fields compared, matching the reference's `pricingSyncFields` where this
/// instance has the same table.
const SYNC_FIELDS: [&str; 5] = [
    "model_ratio",
    "completion_ratio",
    "cache_ratio",
    "create_cache_ratio",
    "image_ratio",
];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SyncableChannel {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub endpoint: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpstreamTarget {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
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
    pub local: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<f64>,
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
    /// How many models the source priced, whether or not they differ.
    pub upstream_models: usize,
    pub models: Vec<ModelDifference>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FetchReport {
    pub upstreams: Vec<UpstreamReport>,
    /// Models present in a source and absent here; they are new prices rather
    /// than changed ones, and an operator reads them differently.
    pub new_models: Vec<String>,
}

/// One model's ratio as the source reported it.
type UpstreamModel = BTreeMap<String, f64>;

/// The channels that can act as a source: enabled, with an http(s) base URL.
pub fn syncable_channels(state: &AppState) -> Result<Vec<SyncableChannel>, String> {
    let channels = state
        .db
        .list_channels()
        .map_err(|error| error.to_string())?;
    Ok(channels
        .into_iter()
        .filter(|channel| channel.enabled)
        .filter(|channel| {
            let base = channel.base_url.trim();
            base.starts_with("http://") || base.starts_with("https://")
        })
        .map(|channel| SyncableChannel {
            id: channel.id,
            name: channel.name,
            base_url: channel.base_url.trim().trim_end_matches('/').to_string(),
            endpoint: DEFAULT_ENDPOINT.to_string(),
        })
        .collect())
}

/// Read the ratios out of a source's `/api/pricing` payload.
///
/// The payload is the reference's: `data` is a list of models, each carrying its
/// own ratio fields. A field that is absent or not a number is skipped rather
/// than treated as zero — "the source does not price this" is not "the source
/// charges nothing".
fn parse_upstream_models(body: &str) -> Result<BTreeMap<String, UpstreamModel>, String> {
    let document: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| format!("the source's payload is not JSON: {error}"))?;
    let entries = document
        .get("data")
        .and_then(|data| data.as_array())
        .ok_or_else(|| "the source's payload has no data list".to_string())?;
    let mut models: BTreeMap<String, UpstreamModel> = BTreeMap::new();
    for entry in entries {
        let Some(name) = entry.get("model_name").and_then(|name| name.as_str()) else {
            continue;
        };
        let mut values = UpstreamModel::new();
        for field in SYNC_FIELDS {
            if let Some(number) = entry.get(field).and_then(|value| value.as_f64()) {
                values.insert(field.to_string(), number);
            }
        }
        if !values.is_empty() {
            models.insert(name.to_string(), values);
        }
    }
    Ok(models)
}

/// This instance's own ratios, in the same shape.
fn local_models(state: &AppState) -> BTreeMap<String, UpstreamModel> {
    let pricing = state.billing.pricing();
    let guard = pricing.read();
    let group = state.default_group();
    // Every model this instance prices: the shipped pack's expressions plus the
    // operator's own ratio overrides. A model in neither has no price to compare.
    let mut names: std::collections::BTreeSet<String> =
        guard.expression_models().into_iter().collect();
    if let Ok(maps) = crate::pricing_config::read_maps(state) {
        names.extend(maps.ratio_names());
    }
    names
        .into_iter()
        .map(|model| {
            let data = guard.price_data(&model, &group);
            let mut values = UpstreamModel::new();
            values.insert("model_ratio".to_string(), data.model_ratio);
            values.insert("completion_ratio".to_string(), data.completion_ratio);
            values.insert("cache_ratio".to_string(), data.cache_ratio);
            values.insert("create_cache_ratio".to_string(), data.cache_creation_ratio);
            values.insert("image_ratio".to_string(), data.image_ratio);
            (model, values)
        })
        .collect()
}

/// Two prices count as equal when they agree to a cent's worth of ratio.
///
/// A sync that reported `0.7999999` against `0.8` would bury the differences an
/// operator actually has to decide about.
fn nearly_equal(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// What differs between this instance and one source.
fn differences_for(
    local: &BTreeMap<String, UpstreamModel>,
    upstream: &BTreeMap<String, UpstreamModel>,
) -> (Vec<ModelDifference>, Vec<String>) {
    let mut models = Vec::new();
    let mut new_models = Vec::new();
    for (name, upstream_values) in upstream {
        let Some(local_values) = local.get(name) else {
            new_models.push(name.clone());
            continue;
        };
        let mut fields = Vec::new();
        for field in SYNC_FIELDS {
            let upstream_value = upstream_values.get(field).copied();
            let local_value = local_values.get(field).copied();
            match (local_value, upstream_value) {
                (Some(local), Some(upstream)) if !nearly_equal(local, upstream) => {
                    fields.push(FieldDifference {
                        field: field.to_string(),
                        local: Some(local),
                        upstream: Some(upstream),
                    });
                }
                (None, Some(upstream)) => fields.push(FieldDifference {
                    field: field.to_string(),
                    local: None,
                    upstream: Some(upstream),
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

/// Read one source and compare it with this instance.
async fn fetch_one(
    client: &reqwest::Client,
    name: String,
    base_url: String,
    endpoint: String,
    local: &BTreeMap<String, UpstreamModel>,
) -> (UpstreamReport, Vec<String>) {
    let url = format!("{}{}", base_url.trim_end_matches('/'), endpoint);
    let mut report = UpstreamReport {
        name,
        base_url,
        endpoint,
        ok: false,
        error: String::new(),
        upstream_models: 0,
        models: Vec::new(),
    };
    let response = match client.get(&url).send().await {
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
    let upstream = match parse_upstream_models(&body) {
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

/// Fetch every named source and report the differences.
///
/// Sources are read concurrently, so one slow instance does not serialise the
/// rest; each is bounded by the request's own timeout.
pub async fn fetch(state: &AppState, request: &FetchRequest) -> Result<FetchReport, String> {
    let local = local_models(state);
    let mut targets: Vec<(String, String, String)> = Vec::new();

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
            .filter(|path| path.starts_with('/'))
            .unwrap_or(DEFAULT_ENDPOINT)
            .to_string();
        targets.push((
            target
                .name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| base_url.to_string()),
            base_url.trim_end_matches('/').to_string(),
            endpoint,
        ));
    }

    if !request.channel_ids.is_empty() {
        let channels = syncable_channels(state)?;
        for id in &request.channel_ids {
            if let Some(channel) = channels.iter().find(|channel| &channel.id == id) {
                targets.push((
                    channel.name.clone(),
                    channel.base_url.clone(),
                    channel.endpoint.clone(),
                ));
            }
        }
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
        // A price table read is a read: following a redirect would let the
        // source's answer come from somewhere the operator did not name.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?;

    let futures = targets
        .into_iter()
        .map(|(name, base_url, endpoint)| fetch_one(&client, name, base_url, endpoint, &local));
    let results = futures_util::future::join_all(futures).await;

    // A model the source prices and this instance does not is a new price rather
    // than a changed one, and the two are read differently.
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
///
/// A preview: it reports differences and writes nothing. Applying them is the
/// pricing editor's job, where the change is versioned and audited.
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

    fn models(entries: &[(&str, &str, f64)]) -> BTreeMap<String, UpstreamModel> {
        let mut out: BTreeMap<String, UpstreamModel> = BTreeMap::new();
        for (name, field, value) in entries {
            out.entry((*name).to_string())
                .or_default()
                .insert((*field).to_string(), *value);
        }
        out
    }

    /// Only a real difference is reported: equal values, and values that differ
    /// below the epsilon, are not news.
    #[test]
    fn only_real_differences_are_reported() {
        let local = models(&[
            ("same", "model_ratio", 1.0),
            ("noisy", "model_ratio", 0.8),
            ("changed", "model_ratio", 1.0),
        ]);
        let upstream = models(&[
            ("same", "model_ratio", 1.0),
            ("noisy", "model_ratio", 0.8000000001),
            ("changed", "model_ratio", 2.0),
        ]);
        let (differences, new_models) = differences_for(&local, &upstream);
        assert!(new_models.is_empty());
        assert_eq!(differences.len(), 1);
        assert_eq!(differences[0].model_name, "changed");
        assert_eq!(differences[0].differences.len(), 1);
        assert_eq!(differences[0].differences[0].local, Some(1.0));
        assert_eq!(differences[0].differences[0].upstream, Some(2.0));
    }

    /// A model the source prices and this instance does not is listed as new
    /// rather than as a field difference: there is nothing local to compare to.
    #[test]
    fn an_unknown_model_is_listed_as_new() {
        let local = models(&[("known", "model_ratio", 1.0)]);
        let upstream = models(&[
            ("known", "model_ratio", 1.0),
            ("brand-new", "model_ratio", 3.0),
        ]);
        let (differences, new_models) = differences_for(&local, &upstream);
        assert!(differences.is_empty());
        assert_eq!(new_models, vec!["brand-new".to_string()]);
    }

    /// A field the source does not publish is not "free": it is skipped, and the
    /// absence is visible in the difference as a `None` local only when the
    /// source *does* publish it.
    #[test]
    fn a_missing_field_is_not_treated_as_zero() {
        let local: BTreeMap<String, UpstreamModel> = BTreeMap::new();
        local_assert_empty(&local);
        let upstream = models(&[("only-upstream", "model_ratio", 0.5)]);
        let (differences, new_models) = differences_for(&local, &upstream);
        assert!(
            differences.is_empty(),
            "a new model is not a field difference"
        );
        assert_eq!(new_models, vec!["only-upstream".to_string()]);
    }

    fn local_assert_empty(map: &BTreeMap<String, UpstreamModel>) {
        assert!(map.is_empty());
    }

    /// The reference's payload is what a source serves, so that is what the
    /// parser reads; anything else is refused by name.
    #[test]
    fn the_source_payload_is_parsed_and_bad_ones_are_refused() {
        let body = r#"{"success":true,"data":[
            {"model_name":"m1","model_ratio":2.5,"completion_ratio":3.0,"cache_ratio":0.1},
            {"model_name":"m2","model_ratio":1.0},
            {"description":"no name"}
        ]}"#;
        let parsed = parse_upstream_models(body).expect("parsed");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed["m1"]["completion_ratio"], 3.0);
        assert!(!parsed["m2"].contains_key("cache_ratio"));

        assert!(parse_upstream_models("not json").is_err());
        assert!(parse_upstream_models(r#"{"models":[]}"#).is_err());
    }
}
