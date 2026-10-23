//! The public price catalogue: `GET /api/pricing`.
//!
//! Mirrors the reference's `controller/pricing.go` `GetPricing`, whose payload is
//! deliberately *not* wrapped in `data`: the model list sits in `data` and the
//! instance's group ratios, vendors, usable groups and endpoint map are siblings
//! of it. A client written against that shape reads the siblings, so this
//! endpoint serde-serialises the same structure rather than flattening it into
//! this project's `ApiResponse`.
//!
//! Two differences are deliberate and recorded here rather than hidden:
//!
//! * `supported_endpoint` is keyed by **path** (`/v1/chat/completions`), because
//!   that is what this instance actually routes — its 404 body advertises the
//!   same list. The reference keys the same field by its internal endpoint type.
//! * A channel that declares `*` (this project's wildcard, which the reference
//!   does not have) does not add model names: the catalogue lists what channels
//!   name explicitly, so a wildcard deployment shows the models it declared
//!   rather than an invented list.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::state::AppState;

/// One model, in the reference's field names.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PublicPricing {
    pub model_name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub icon: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tags: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    /// 0 when the model is billed per token, 1 when it is billed per call —
    /// the reference's `quota_type` convention.
    pub quota_type: i64,
    pub model_ratio: f64,
    pub model_price: f64,
    pub completion_ratio: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_cache_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_ratio: Option<f64>,
    /// USD per million input tokens, so a client does not have to know
    /// `QuotaPerUnit` to render a price.
    pub input_price_per_million: f64,
    pub output_price_per_million: f64,
    /// The groups this instance prices the model for.
    pub enable_groups: Vec<String>,
    /// The channel providers that serve the model, e.g. `openai`, `anthropic`.
    pub supported_endpoint_types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_expr: Option<String>,
    pub pricing_version: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PublicVendor {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub icon: String,
}

/// One relay route, as the catalogue advertises it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PublicEndpoint {
    pub method: String,
    pub path: String,
}

/// The whole payload, siblings included.
#[derive(Debug, Clone, Serialize)]
pub struct PricingCatalogue {
    pub success: bool,
    pub data: Vec<PublicPricing>,
    pub vendors: Vec<PublicVendor>,
    pub group_ratio: BTreeMap<String, f64>,
    pub usable_group: BTreeMap<String, String>,
    pub supported_endpoint: BTreeMap<String, PublicEndpoint>,
    /// This instance has no auto-group routing, so the list is empty rather than
    /// absent: a client that filters on it sees a well-formed answer.
    pub auto_groups: Vec<String>,
    pub pricing_version: String,
}

/// Every relay route this proxy serves, keyed by path.
fn endpoint_map() -> BTreeMap<String, PublicEndpoint> {
    let mut map = BTreeMap::new();
    for entry in crate::proxy::SUPPORTED_ENDPOINTS {
        // The list is written as "METHOD /path", the same form the 404 body uses.
        if let Some((method, path)) = entry.split_once(' ') {
            map.insert(
                path.to_string(),
                PublicEndpoint {
                    method: method.to_string(),
                    path: path.to_string(),
                },
            );
        }
    }
    map
}

/// The models enabled channels actually declare, with the providers serving them.
fn served_models(state: &AppState) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let channels = state.db.list_channels().map_err(|error| error.to_string())?;
    let mut served: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for channel in channels.into_iter().filter(|channel| channel.enabled) {
        for model in &channel.model_list {
            let name = model.trim();
            // `*` is this project's wildcard; it names no model to list.
            if name.is_empty() || name == "*" {
                continue;
            }
            served
                .entry(name.to_string())
                .or_default()
                .insert(channel.provider.clone());
        }
    }
    Ok(served)
}

/// Build the catalogue the public pricing page renders.
pub fn build(state: &AppState) -> Result<PricingCatalogue, String> {
    let maps = crate::pricing_config::read_maps(state)?;
    let pricing_version = crate::pricing_config::pricing_version(&maps);

    let metadata = state
        .db
        .list_model_metadata()
        .map_err(|error| error.to_string())?;
    let vendors = state
        .db
        .list_vendors()
        .map_err(|error| error.to_string())?;
    let usable_group = usable_groups(state);

    let pricing = state.billing.pricing();
    let guard = pricing.read();
    let default_group = state.default_group();
    // The instance's own default group is always listed: it is what an
    // unqualified request bills against, and the console's group endpoint adds
    // it the same way, so a client can always resolve the group it will be
    // charged in.
    let mut groups = guard.group_names();
    if !groups.iter().any(|existing| existing == &default_group) {
        groups.push(default_group.clone());
        groups.sort();
    }
    let group_ratio = groups
        .iter()
        .map(|group| (group.clone(), guard.group_ratio(group)))
        .collect::<BTreeMap<_, _>>();
    let quota_per_unit = guard.quota_per_unit();

    let mut data = Vec::new();
    for (model, providers) in served_models(state)? {
        let data_for_model = guard.price_data(&model, &default_group);
        let per_million = |ratio: f64| ratio * 1_000_000.0 / quota_per_unit;
        let entry_vendor = metadata
            .iter()
            .find(|meta| meta.model_name == model)
            .map(|meta| meta.vendor.clone())
            .filter(|vendor| !vendor.trim().is_empty());
        let meta = metadata.iter().find(|meta| meta.model_name == model);
        data.push(PublicPricing {
            model_name: model.clone(),
            description: meta.map(|meta| meta.description.clone()).unwrap_or_default(),
            icon: meta.map(|meta| meta.icon.clone()).unwrap_or_default(),
            tags: meta.map(|meta| meta.tags.clone()).unwrap_or_default(),
            vendor: entry_vendor,
            quota_type: if data_for_model.use_price { 1 } else { 0 },
            model_ratio: data_for_model.model_ratio,
            model_price: data_for_model.model_price,
            completion_ratio: data_for_model.completion_ratio,
            cache_ratio: (data_for_model.cache_ratio != 1.0).then_some(data_for_model.cache_ratio),
            create_cache_ratio: (data_for_model.cache_creation_ratio != 1.0)
                .then_some(data_for_model.cache_creation_ratio),
            image_ratio: (data_for_model.image_ratio != 1.0).then_some(data_for_model.image_ratio),
            input_price_per_million: per_million(data_for_model.model_ratio),
            output_price_per_million: per_million(
                data_for_model.model_ratio * data_for_model.completion_ratio,
            ),
            enable_groups: groups.clone(),
            supported_endpoint_types: providers.into_iter().collect(),
            billing_mode: Some(
                match guard.mode(&model) {
                    oxygenrouter_billing::BillingMode::TieredExpr => "tiered_expr",
                    oxygenrouter_billing::BillingMode::Ratio => "ratio",
                }
                .to_string(),
            ),
            billing_expr: guard.expression(&model).map(str::to_string),
            pricing_version: pricing_version.clone(),
        });
    }

    Ok(PricingCatalogue {
        success: true,
        data,
        vendors: vendors
            .into_iter()
            .map(|vendor| PublicVendor {
                id: vendor.id,
                name: vendor.name,
                description: vendor.description,
                icon: vendor.icon,
            })
            .collect(),
        group_ratio,
        usable_group,
        supported_endpoint: endpoint_map(),
        auto_groups: Vec::new(),
        pricing_version,
    })
}

/// The group labels the console shows, from `UserUsableGroups` with the group's
/// own name as the fallback.
fn usable_groups(state: &AppState) -> BTreeMap<String, String> {
    let raw = state
        .db
        .typed_setting("UserUsableGroups", "{}".to_string());
    let parsed: BTreeMap<String, String> = serde_json::from_str(&raw).unwrap_or_default();
    let pricing = state.billing.pricing();
    let guard = pricing.read();
    let mut groups = BTreeMap::new();
    let mut names = guard.group_names();
    let default_group = state.default_group();
    if !names.iter().any(|existing| existing == &default_group) {
        names.push(default_group);
    }
    for name in names {
        let label = parsed
            .get(&name)
            .cloned()
            .unwrap_or_else(|| name.clone());
        groups.insert(name, label);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxygenrouter_core::{Channel, ChannelSelector};
    use oxygenrouter_proxy::{ChannelScheduler, RelayClient};

    fn state() -> std::sync::Arc<AppState> {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("catalogue-test.db");
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

    fn channel(id: &str, provider: &str, models: &[&str], enabled: bool) -> Channel {
        Channel {
            id: id.to_string(),
            name: id.to_string(),
            provider: provider.to_string(),
            base_url: "http://127.0.0.1:9".to_string(),
            api_key: "k".to_string(),
            priority: 1,
            weight: 1,
            enabled,
            test_model: "gpt-3.5-turbo".to_string(),
            auto_ban: true,
            auto_disabled: false,
            status_reason: String::new(),
            status_time: None,
            group_name: "default".to_string(),
            tags: Vec::new(),
            model_list: models.iter().map(|model| model.to_string()).collect(),
            response_headers: serde_json::Value::Null,
            status_code_mapping: serde_json::Value::Null,
            override_parameters: serde_json::Value::Null,
            balance_micros: 0,
            last_test_at: None,
            info: Default::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    /// Only enabled channels contribute, a wildcard names nothing, and two
    /// providers serving one model are both reported.
    #[test]
    fn the_catalogue_lists_what_enabled_channels_declare() {
        let state = state();
        state
            .db
            .upsert_channel(&channel("a", "openai", &["alpha", "shared"], true))
            .expect("channel");
        state
            .db
            .upsert_channel(&channel("b", "anthropic", &["shared", "*"], true))
            .expect("channel");
        state
            .db
            .upsert_channel(&channel("c", "openai", &["hidden"], false))
            .expect("channel");

        let catalogue = build(&state).expect("catalogue");
        let names = catalogue
            .data
            .iter()
            .map(|entry| entry.model_name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["alpha", "shared"]);
        let shared = catalogue
            .data
            .iter()
            .find(|entry| entry.model_name == "shared")
            .expect("shared");
        assert_eq!(shared.supported_endpoint_types, vec!["anthropic", "openai"]);
        assert!(catalogue.success);
        assert!(!catalogue.pricing_version.is_empty());
        // Every relay route the proxy serves is advertised.
        assert!(catalogue
            .supported_endpoint
            .contains_key("/v1/chat/completions"));
    }

    /// A per-call price is reported as `quota_type: 1` with its price, and a
    /// ratio model's prices are converted to USD per million tokens.
    #[test]
    fn a_price_is_reported_in_the_units_a_client_renders() {
        let state = state();
        state
            .db
            .upsert_channel(&channel("a", "openai", &["alpha"], true))
            .expect("channel");
        state
            .db
            .set_setting("ModelRatio", r#"{"alpha":3.0}"#)
            .expect("stored");
        state.reload_pricing();

        let catalogue = build(&state).expect("catalogue");
        let alpha = catalogue
            .data
            .iter()
            .find(|entry| entry.model_name == "alpha")
            .expect("alpha");
        assert_eq!(alpha.quota_type, 0);
        assert_eq!(alpha.input_price_per_million, 6.0);
        assert_eq!(alpha.output_price_per_million, 6.0);
        assert_eq!(alpha.model_ratio, 3.0);
    }
}
