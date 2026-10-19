//! Per-model pricing configuration, the console's editor for the price tables.
//!
//! Mirrors the reference's `controller/model_pricing_config.go` +
//! `model/model_pricing_config.go`: a snapshot of every model's configured and
//! effective price, a preview of a draft, a conversion preview that turns a
//! legacy ratio into a `tiered_expr` expression, and a versioned batch update.
//!
//! Two properties are load-bearing:
//!
//! * **`effective` is the engine's own resolution, not a second implementation.**
//!   The preview builds a throwaway [`Pricing`] from the shipped pack, applies
//!   the instance's stored maps, applies the draft, and then reads the result
//!   back through `mode` / `expression` / `price_data`. A hand-written
//!   "defaults" table would drift from the engine the moment either changed, and
//!   the drift would be invisible until a bill disagreed with the console.
//! * **A write is version-checked.** Each entry carries a hash of the model's
//!   stored values; a change must name the version it was computed against, and
//!   a mismatch answers `409` rather than overwriting an edit made in another
//!   tab. The reference does the same with its `expected_version`.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::state::AppState;

/// The option keys this module reads and writes.
///
/// They are the instance's own option names, not the reference's
/// (`billing_setting.billing_expr` there). Serving the reference's spelling
/// while storing under a different key would make the API a description of
/// something the instance does not have.
pub const PRICE_KEYS: [&str; 3] = ["ModelRatio", "billing_expr", "billing_mode"];

/// One model's editable price, keyed by option name.
///
/// A key that is absent inherits the engine's default; an explicit `0` is a free
/// model. That distinction is why this is a map rather than a struct with
/// `Option` fields: it round-trips exactly what the operator wrote.
pub type PricingValues = Map<String, Value>;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelPricingEntry {
    pub model_name: String,
    /// Hash of `configured`; what a later change must present as
    /// `expected_version`.
    pub version: String,
    /// What the operator has stored for this model, and nothing else.
    pub configured: PricingValues,
    /// What the engine will actually charge, defaults included.
    pub effective: PricingValues,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelPricingSnapshot {
    pub entries: Vec<ModelPricingEntry>,
    /// The raw stored blobs, so an editor can show the whole table it is
    /// editing rather than only the models it happened to request.
    pub options: BTreeMap<String, String>,
    /// The version of a draft with no keys set, which is what a model that has
    /// never been priced reports.
    pub empty_version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelPricingChange {
    pub model_name: String,
    #[serde(default)]
    pub expected_version: Option<String>,
    #[serde(default)]
    pub pricing: Option<PricingValues>,
    /// Drop every stored key for this model, returning it to the shipped
    /// defaults. Named `reset` because that is what the reference calls it.
    #[serde(default)]
    pub reset: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BillingDetails {
    pub mode: String,
    /// USD per million input tokens, when the model is token-priced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_price_per_million: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_price_per_million: Option<f64>,
    /// USD per call, when the model is priced per request instead of per token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_call_price: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelPricingPreview {
    pub effective: PricingValues,
    pub billing_details: BillingDetails,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelPricingConversion {
    /// The expression the draft converts to. Absent when it cannot be
    /// converted, in which case `unsupported_reason` says why.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsupported_reason: Option<String>,
    pub effective: PricingValues,
}

/// Why a batch was refused. The HTTP status follows from this rather than from
/// the message text, so a reworded error cannot silently become a `400`.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteError {
    /// One change named a version that is no longer current.
    Conflict(String),
    /// The draft itself is not usable, or the stored tables are unreadable.
    Invalid(String),
}

impl WriteError {
    pub fn message(&self) -> &str {
        match self {
            WriteError::Conflict(message) | WriteError::Invalid(message) => message,
        }
    }
}

/// The stored option blobs, parsed.
#[derive(Debug, Clone, Default)]
pub struct PricingMaps {
    ratio: Map<String, Value>,
    expr: Map<String, Value>,
    mode: Map<String, Value>,
    /// The raw strings, kept so a write can rewrite exactly the three keys it
    /// owns without re-serialising (and reformatting) anything else.
    raw: BTreeMap<String, String>,
}

impl PricingMaps {
    /// The models the operator has set a ratio for.
    ///
    /// The three tables stay private so nothing outside this module can rewrite
    /// them without going through `apply`; this is the one read another module
    /// needs, for the price-table comparison.
    pub fn ratio_names(&self) -> Vec<String> {
        self.ratio.keys().cloned().collect()
    }
}

/// Read the three price tables from the option store.
///
/// A blob that is not a JSON object is an error rather than a silent empty
/// table: the reference refuses it too (`readModelPricingMaps`), because
/// treating a typo as "no prices configured" would report every model as
/// unpriced while the engine still charged the shipped default.
pub fn read_maps(state: &AppState) -> Result<PricingMaps, String> {
    let mut maps = PricingMaps::default();
    for key in PRICE_KEYS {
        let raw = state
            .db
            .get_setting(key)
            .map_err(|error| error.to_string())?
            .unwrap_or_else(|| "{}".to_string());
        let value: Value = serde_json::from_str(&raw)
            .map_err(|error| format!("{key}: {error}"))?;
        let object = value
            .as_object()
            .cloned()
            .ok_or_else(|| format!("{key} must be a JSON object"))?;
        match key {
            "ModelRatio" => maps.ratio = object,
            "billing_expr" => maps.expr = object,
            "billing_mode" => maps.mode = object,
            _ => unreachable!("PRICE_KEYS and this match are one list"),
        }
        maps.raw.insert(key.to_string(), raw);
    }
    Ok(maps)
}

/// The keys stored for one model, with absent keys omitted.
fn configured_for(maps: &PricingMaps, model: &str) -> PricingValues {
    let mut values = PricingValues::new();
    for (key, table) in [
        ("ModelRatio", &maps.ratio),
        ("billing_expr", &maps.expr),
        ("billing_mode", &maps.mode),
    ] {
        if let Some(value) = table.get(model) {
            values.insert(key.to_string(), value.clone());
        }
    }
    values
}

/// A stable hash of one model's stored values.
///
/// `serde_json`'s map is ordered, so the encoding is deterministic and the same
/// draft hashes identically in every process — which is what makes it usable as
/// a concurrency token.
pub fn version_of(values: &PricingValues) -> String {
    use sha2::{Digest, Sha256};
    let encoded = serde_json::to_string(values).unwrap_or_default();
    let digest = Sha256::digest(encoded.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A hash of the whole price table set, for clients that cache the catalogue.
///
/// The three tables are hashed in a fixed order and concatenated, so a change to
/// any of them changes the version while an unrelated option does not.
pub fn pricing_version(maps: &PricingMaps) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for table in [&maps.ratio, &maps.expr, &maps.mode] {
        let encoded = serde_json::to_string(table).unwrap_or_default();
        hasher.update(encoded.as_bytes());
        hasher.update(b"\n");
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Reject a draft the engine could not honour.
///
/// Checked before anything is stored, so a bad edit is refused by name instead
/// of being written and failing later at request time.
fn validate(model: &str, draft: &PricingValues) -> Result<(), String> {
    for key in draft.keys() {
        if !PRICE_KEYS.contains(&key.as_str()) {
            return Err(format!("{model}: unknown pricing key {key:?}"));
        }
    }
    if let Some(ratio) = draft.get("ModelRatio") {
        let ratio = ratio
            .as_f64()
            .ok_or_else(|| format!("{model}: ModelRatio must be a number"))?;
        if !ratio.is_finite() || ratio < 0.0 {
            return Err(format!("{model}: ModelRatio must be finite and not negative"));
        }
    }
    if let Some(expr) = draft.get("billing_expr") {
        let expr = expr
            .as_str()
            .ok_or_else(|| format!("{model}: billing_expr must be a string"))?;
        if expr.trim().is_empty() {
            return Err(format!("{model}: billing_expr must not be empty"));
        }
        // The engine has no parse-only entry point, so the draft is evaluated
        // once against a probe usage. That is a stronger check than parsing: an
        // unknown function or a term that divides by zero is refused here,
        // rather than at the first request that used the model.
        let probe = oxygenrouter_billing::expr::TokenParams {
            p: 1000.0,
            c: 1000.0,
            len: 2000.0,
            cr: 0.0,
            cc: 0.0,
            cc1h: 0.0,
            img: 0.0,
            img_cr: 0.0,
            img_o: 0.0,
            ai: 0.0,
            ao: 0.0,
        };
        let outcome = oxygenrouter_billing::expr::evaluate(expr, &probe)
            .map_err(|error| format!("{model}: {error}"))?;
        if !outcome.cost_usd.is_finite() || outcome.cost_usd < 0.0 {
            return Err(format!(
                "{model}: billing_expr evaluates to {}, which is not a price",
                outcome.cost_usd
            ));
        }
    }
    if let Some(mode) = draft.get("billing_mode") {
        let mode = mode
            .as_str()
            .ok_or_else(|| format!("{model}: billing_mode must be a string"))?;
        if mode != "ratio" && mode != "tiered_expr" {
            return Err(format!(
                "{model}: billing_mode must be ratio or tiered_expr, got {mode:?}"
            ));
        }
    }
    // An expression without the mode that selects it is inert in the engine, so
    // accepting it would store a price the operator cannot see taking effect.
    if draft.contains_key("billing_expr") && draft.get("billing_mode").is_none() {
        return Err(format!(
            "{model}: billing_expr needs billing_mode=tiered_expr to take effect"
        ));
    }
    Ok(())
}

/// Build a throwaway engine with the instance's tables and one draft applied.
///
/// The draft replaces the model's stored values rather than merging with them,
/// matching the reference: the editor submits the complete configuration it
/// rendered, and a merge would keep a key the operator had just deleted.
fn resolve_with_draft(
    maps: &PricingMaps,
    model: &str,
    draft: &PricingValues,
) -> Result<oxygenrouter_billing::Pricing, String> {
    let mut pricing = oxygenrouter_billing::Pricing::from_embedded()
        .map_err(|error| error.to_string())?;
    let mut expr: Map<String, Value> = maps.expr.clone();
    let mut mode: Map<String, Value> = maps.mode.clone();
    let mut ratio: Map<String, Value> = maps.ratio.clone();
    expr.remove(model);
    mode.remove(model);
    ratio.remove(model);
    if let Some(value) = draft.get("billing_expr") {
        expr.insert(model.to_string(), value.clone());
    }
    if let Some(value) = draft.get("billing_mode") {
        mode.insert(model.to_string(), value.clone());
    }
    if let Some(value) = draft.get("ModelRatio") {
        ratio.insert(model.to_string(), value.clone());
    }
    pricing.set_expr_map(string_map(&expr, model, "billing_expr")?);
    pricing.set_mode_map(mode_map(&mode, model)?);
    pricing.set_ratio_tables(ratio_tables(&ratio, model)?);
    Ok(pricing)
}

fn string_map(
    map: &Map<String, Value>,
    model: &str,
    key: &str,
) -> Result<std::collections::HashMap<String, String>, String> {
    let mut out = std::collections::HashMap::new();
    for (name, value) in map {
        let text = value
            .as_str()
            .ok_or_else(|| format!("{key}[{}] must be a string", display(name, model)))?;
        out.insert(name.clone(), text.to_string());
    }
    Ok(out)
}

fn mode_map(
    map: &Map<String, Value>,
    model: &str,
) -> Result<std::collections::HashMap<String, oxygenrouter_billing::BillingMode>, String> {
    let mut out = std::collections::HashMap::new();
    for (name, value) in map {
        let text = value
            .as_str()
            .ok_or_else(|| {
                format!("billing_mode[{}] must be a string", display(name, model))
            })?;
        let mode = match text {
            "ratio" => oxygenrouter_billing::BillingMode::Ratio,
            "tiered_expr" => oxygenrouter_billing::BillingMode::TieredExpr,
            other => {
                return Err(format!(
                    "billing_mode[{}] must be ratio or tiered_expr, got {other:?}",
                    display(name, model)
                ))
            }
        };
        out.insert(name.clone(), mode);
    }
    Ok(out)
}

/// The ratio table, as the engine wants it.
///
/// `ModelRatio` is the only legacy table the console edits, so it is the only
/// one rebuilt here; every other table keeps the shipped pack's values.
fn ratio_tables(
    map: &Map<String, Value>,
    model: &str,
) -> Result<oxygenrouter_billing::PricingTables, String> {
    let mut tables = oxygenrouter_billing::PricingTables::default();
    for (name, value) in map {
        let ratio = value
            .as_f64()
            .ok_or_else(|| format!("ModelRatio[{}] must be a number", display(name, model)))?;
        if !ratio.is_finite() || ratio < 0.0 {
            return Err(format!(
                "ModelRatio[{}] must be finite and not negative",
                display(name, model)
            ));
        }
        tables.model_ratio.insert(name.clone(), ratio);
    }
    Ok(tables)
}

fn display(name: &str, model: &str) -> String {
    if name == model {
        "this model".to_string()
    } else {
        name.to_string()
    }
}

/// The engine's own view of a model, as the console renders it.
fn effective_for(
    pricing: &oxygenrouter_billing::Pricing,
    model: &str,
    group: &str,
) -> PricingValues {
    let mut values = PricingValues::new();
    let mode = pricing.mode(model);
    values.insert(
        "billing_mode".to_string(),
        Value::String(
            match mode {
                oxygenrouter_billing::BillingMode::TieredExpr => "tiered_expr",
                oxygenrouter_billing::BillingMode::Ratio => "ratio",
            }
            .to_string(),
        ),
    );
    if let Some(expression) = pricing.expression(model) {
        values.insert(
            "billing_expr".to_string(),
            Value::String(expression.to_string()),
        );
    }
    let data = pricing.price_data(model, group);
    if data.use_price {
        values.insert("ModelPrice".to_string(), json_number(data.model_price));
    } else {
        values.insert("ModelRatio".to_string(), json_number(data.model_ratio));
        values.insert(
            "CompletionRatio".to_string(),
            json_number(data.completion_ratio),
        );
        if data.cache_ratio != 1.0 {
            values.insert("CacheRatio".to_string(), json_number(data.cache_ratio));
        }
        if data.cache_creation_ratio != 1.0 {
            values.insert(
                "CreateCacheRatio".to_string(),
                json_number(data.cache_creation_ratio),
            );
        }
        if data.image_ratio != 1.0 {
            values.insert("ImageRatio".to_string(), json_number(data.image_ratio));
        }
    }
    values
}

/// `serde_json::Number` for a float the engine produced.
fn json_number(value: f64) -> Value {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn billing_details(
    pricing: &oxygenrouter_billing::Pricing,
    model: &str,
    group: &str,
) -> BillingDetails {
    let data = pricing.price_data(model, group);
    let mode = match pricing.mode(model) {
        oxygenrouter_billing::BillingMode::TieredExpr => "tiered_expr",
        oxygenrouter_billing::BillingMode::Ratio => "ratio",
    }
    .to_string();
    // The ratio unit is "quota per token", and `QuotaPerUnit` quota is one unit
    // of currency, so one million tokens cost `ratio / QuotaPerUnit`.
    let per_ratio = |ratio: f64| ratio * 1_000_000.0 / pricing.quota_per_unit();
    let mut details = BillingDetails {
        mode,
        input_price_per_million: None,
        output_price_per_million: None,
        per_call_price: None,
    };
    if data.use_price {
        details.per_call_price = Some(data.model_price);
    } else {
        details.input_price_per_million = Some(per_ratio(data.model_ratio));
        details.output_price_per_million =
            Some(per_ratio(data.model_ratio * data.completion_ratio));
    }
    details
}

/// The models a snapshot covers when the caller named none.
///
/// Every model the instance has overridden, plus every model the shipped pack
/// prices with an expression — the same union the reference builds from its
/// stored maps and its builtin expression table (`model/model_pricing_config.go`).
/// The pack's ratio table is deliberately not walked: it holds thousands of
/// rows, and a caller that wants one of those names asks for it by name.
fn known_models(state: &AppState, maps: &PricingMaps) -> Vec<String> {
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for table in [&maps.ratio, &maps.expr, &maps.mode] {
        names.extend(table.keys().cloned());
    }
    let pricing = state.billing.pricing();
    let guard = pricing.read();
    for name in guard.expression_models() {
        names.insert(name);
    }
    names.into_iter().collect()
}

/// Build the snapshot the pricing page renders.
pub fn snapshot(state: &AppState, requested: &[String]) -> Result<ModelPricingSnapshot, String> {
    let maps = read_maps(state)?;
    let group = state.default_group();
    let names = if requested.is_empty() {
        known_models(state, &maps)
    } else {
        requested.to_vec()
    };
    let pricing = resolve_with_draft(&maps, "", &PricingValues::new())?;
    let mut entries = Vec::with_capacity(names.len());
    for name in names {
        let configured = configured_for(&maps, &name);
        entries.push(ModelPricingEntry {
            model_name: name.clone(),
            version: version_of(&configured),
            configured,
            effective: effective_for(&pricing, &name, &group),
        });
    }
    Ok(ModelPricingSnapshot {
        entries,
        options: maps.raw.clone(),
        empty_version: version_of(&PricingValues::new()),
    })
}

/// Resolve a draft without storing it.
pub fn preview(
    state: &AppState,
    model: &str,
    draft: &PricingValues,
) -> Result<ModelPricingPreview, String> {
    if model.trim().is_empty() {
        return Err("model name is required".to_string());
    }
    validate(model, draft)?;
    let maps = read_maps(state)?;
    let pricing = resolve_with_draft(&maps, model, draft)?;
    let group = state.default_group();
    Ok(ModelPricingPreview {
        effective: effective_for(&pricing, model, &group),
        billing_details: billing_details(&pricing, model, &group),
    })
}

/// Turn a legacy ratio draft into the equivalent `tiered_expr` expression.
///
/// The engine's expression variables are priced in **USD per million tokens**
/// while the ratio tables are priced in quota-per-token, so the conversion runs
/// through `QuotaPerUnit` once and then scales each component by the multiplier
/// the ratio path would have applied. `cr` / `cc` / `img` are only written when
/// their multiplier is not 1, because the engine already folds those tokens into
/// `p` when the term is absent.
pub fn convert(
    state: &AppState,
    model: &str,
    draft: &PricingValues,
) -> Result<ModelPricingConversion, String> {
    if model.trim().is_empty() {
        return Err("model name is required".to_string());
    }
    validate(model, draft)?;
    if draft.get("billing_mode").and_then(Value::as_str) == Some("tiered_expr") {
        return Ok(ModelPricingConversion {
            expression: None,
            unsupported_reason: Some("This model already uses an expression.".to_string()),
            effective: PricingValues::new(),
        });
    }
    let maps = read_maps(state)?;
    let pricing = resolve_with_draft(&maps, model, draft)?;
    let group = state.default_group();
    let effective = effective_for(&pricing, model, &group);
    if pricing.expression(model).is_some() {
        return Ok(ModelPricingConversion {
            expression: None,
            unsupported_reason: Some("This model already uses an expression.".to_string()),
            effective,
        });
    }
    let data = pricing.price_data(model, &group);
    if data.use_price {
        return Ok(ModelPricingConversion {
            expression: None,
            unsupported_reason: Some(
                "A per-call price is not a per-token expression; keep it on the ratio path."
                    .to_string(),
            ),
            effective,
        });
    }
    // An unpriced model resolves to the engine's bare 1.0 fallback, which is not
    // a price anyone chose; converting that would invent one. A ratio the draft
    // or the store actually sets counts, even when it is exactly 1.0.
    let configured_ratio = draft.contains_key("ModelRatio") || maps.ratio.contains_key(model);
    if !configured_ratio && data.model_ratio == 1.0 {
        return Ok(ModelPricingConversion {
            expression: None,
            unsupported_reason: Some(
                "This model has no ratio to convert; set one first.".to_string(),
            ),
            effective,
        });
    }
    let per_million = |ratio: f64| ratio * 1_000_000.0 / pricing.quota_per_unit();
    let input = per_million(data.model_ratio);
    let mut terms = vec![
        format!("p * {}", trim(input)),
        format!("c * {}", trim(per_million(data.model_ratio * data.completion_ratio))),
    ];
    let mut extra = |variable: &str, multiplier: f64| {
        if multiplier != 1.0 {
            terms.push(format!(
                "{variable} * {}",
                trim(per_million(data.model_ratio * multiplier))
            ));
        }
    };
    extra("cr", data.cache_ratio);
    extra("cc", data.cache_creation_ratio);
    extra("img", data.image_ratio);
    Ok(ModelPricingConversion {
        expression: Some(format!("tier(\"base\", {})", terms.join(" + "))),
        unsupported_reason: None,
        effective,
    })
}

/// Render a float without a trailing `.0` or exponent noise.
fn trim(value: f64) -> String {
    let mut text = format!("{value:.10}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text.is_empty() {
        "0".to_string()
    } else {
        text
    }
}

/// Apply a batch of changes, or refuse the whole batch.
///
/// All three tables are rewritten in one transaction: they are read together by
/// the engine, so a partial write could leave a model with an expression whose
/// mode says `ratio` — a state that prices the model somewhere the console
/// cannot show.
pub fn apply(state: &AppState, changes: &[ModelPricingChange]) -> Result<Vec<String>, WriteError> {
    if changes.is_empty() {
        return Err(WriteError::Invalid("no changes were submitted".to_string()));
    }
    let maps = read_maps(state).map_err(WriteError::Invalid)?;
    let mut ratio = maps.ratio.clone();
    let mut expr = maps.expr.clone();
    let mut mode = maps.mode.clone();
    let mut updated = Vec::with_capacity(changes.len());

    for change in changes {
        let model = change.model_name.trim();
        if model.is_empty() {
            return Err(WriteError::Invalid(
                "a change is missing its model name".to_string(),
            ));
        }
        let current = version_of(&configured_for(&maps, model));
        if let Some(expected) = change.expected_version.as_deref() {
            if expected != current {
                return Err(WriteError::Conflict(format!(
                    "{model}: pricing changed; reload before saving"
                )));
            }
        }
        if change.reset {
            ratio.remove(model);
            expr.remove(model);
            mode.remove(model);
            updated.push(model.to_string());
            continue;
        }
        let draft = change
            .pricing
            .clone()
            .ok_or_else(|| WriteError::Invalid(format!("{model}: pricing is required")))?;
        validate(model, &draft).map_err(WriteError::Invalid)?;
        for (key, value) in &draft {
            let table = match key.as_str() {
                "ModelRatio" => &mut ratio,
                "billing_expr" => &mut expr,
                "billing_mode" => &mut mode,
                other => {
                    return Err(WriteError::Invalid(format!(
                        "{model}: unknown pricing key {other:?}"
                    )))
                }
            };
            table.insert(model.to_string(), value.clone());
        }
        // A key the editor removed is a key the operator deleted, so the stored
        // table must lose it too.
        for (key, table) in [
            ("ModelRatio", &mut ratio),
            ("billing_expr", &mut expr),
            ("billing_mode", &mut mode),
        ] {
            if !draft.contains_key(key) {
                table.remove(model);
            }
        }
        updated.push(model.to_string());
    }

    let rows = [
        ("ModelRatio", Value::Object(ratio)),
        ("billing_expr", Value::Object(expr)),
        ("billing_mode", Value::Object(mode)),
    ]
    .iter()
    .map(|(key, value)| {
        (
            (*key).to_string(),
            serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string()),
        )
    })
    .collect::<Vec<_>>();
    state
        .db
        .set_settings_atomic(&rows)
        .map_err(|error| WriteError::Invalid(error.to_string()))?;
    // The engine caches these maps, so the write only takes effect once it is
    // reloaded; doing it here keeps the API's promise that a saved price is live.
    state.reload_pricing();
    Ok(updated)
}

/// Drop stored overrides so the shipped pack prices the named models again.
///
/// An empty list means every model the instance has overridden, which is the
/// reference's table-wide reset (`controller/pricing.go` `ResetModelRatio`).
/// Naming models is the narrower version of the same operation and shares this
/// code path, so there is one definition of "forget this model's price".
pub fn reset(state: &AppState, models: &[String]) -> Result<Vec<String>, WriteError> {
    let maps = read_maps(state).map_err(WriteError::Invalid)?;
    let targets: Vec<String> = if models.is_empty() {
        let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for table in [&maps.ratio, &maps.expr, &maps.mode] {
            names.extend(table.keys().cloned());
        }
        names.into_iter().collect()
    } else {
        models.iter().map(|name| name.trim().to_string()).collect()
    };
    let changes = targets
        .into_iter()
        .map(|model_name| ModelPricingChange {
            model_name,
            expected_version: None,
            pricing: None,
            reset: true,
        })
        .collect::<Vec<_>>();
    // Nothing overridden is a successful reset, not an empty submission: the
    // caller asked for a state, and that state already holds.
    if changes.is_empty() {
        return Ok(Vec::new());
    }
    apply(state, &changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A state backed by a throwaway database, so a test exercises the real
    /// option store and the real pricing engine rather than a stub of either.
    fn state() -> std::sync::Arc<AppState> {
        use oxygenrouter_core::ChannelSelector;
        use oxygenrouter_proxy::{ChannelScheduler, RelayClient};

        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("pricing-test.db");
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

    fn draft(value: Value) -> PricingValues {
        value.as_object().cloned().expect("an object")
    }

    /// A snapshot reports what the operator stored and what the engine will
    /// charge, and they differ exactly when a default fills a gap.
    #[test]
    fn a_snapshot_separates_configured_from_effective() {
        let state = state();
        state
            .db
            .set_setting("ModelRatio", r#"{"probe-a":2.0}"#)
            .expect("stored");

        let snapshot = snapshot(&state, &["probe-a".to_string(), "probe-b".to_string()])
            .expect("snapshot");
        let a = &snapshot.entries[0];
        assert_eq!(a.model_name, "probe-a");
        assert_eq!(a.configured.get("ModelRatio"), Some(&json!(2.0)));
        assert_eq!(a.effective.get("ModelRatio"), Some(&json!(2.0)));
        assert_ne!(a.version, snapshot.empty_version);

        // A model nobody priced has an empty configuration but still resolves,
        // because the engine has a default for it.
        let b = &snapshot.entries[1];
        assert!(b.configured.is_empty());
        assert_eq!(b.version, snapshot.empty_version);
        assert!(b.effective.contains_key("ModelRatio"));
    }

    /// The effective price is the engine's own arithmetic, including the
    /// currency conversion the console renders beside the editor.
    #[test]
    fn a_preview_resolves_the_draft_through_the_engine() {
        let state = state();
        let preview = preview(&state, "probe-preview", &draft(json!({"ModelRatio": 3.0})))
            .expect("preview");
        assert_eq!(preview.effective.get("ModelRatio"), Some(&json!(3.0)));
        // 3.0 ratio at 500_000 quota per unit is $6 per million tokens.
        assert_eq!(preview.billing_details.input_price_per_million, Some(6.0));
        assert_eq!(preview.billing_details.output_price_per_million, Some(6.0));
        assert_eq!(preview.billing_details.mode, "ratio");
    }

    /// An expression is inert without the mode that selects it, so a draft that
    /// omits the mode is refused rather than stored and ignored.
    #[test]
    fn an_expression_draft_requires_its_mode() {
        let state = state();
        let error = preview(
            &state,
            "probe-expr",
            &draft(json!({"billing_expr": "tier(\"base\", p * 1)"})),
        )
        .expect_err("a mode-less expression must be refused");
        assert!(error.contains("billing_mode"), "{error}");

        let preview = preview(
            &state,
            "probe-expr",
            &draft(json!({
                "billing_expr": "tier(\"base\", p * 1)",
                "billing_mode": "tiered_expr"
            })),
        )
        .expect("preview");
        assert_eq!(preview.billing_details.mode, "tiered_expr");
        assert_eq!(
            preview.effective.get("billing_expr"),
            Some(&json!("tier(\"base\", p * 1)"))
        );
    }

    /// A version is a concurrency token: a save computed against a stale one is
    /// refused instead of overwriting an edit made elsewhere.
    #[test]
    fn a_stale_version_is_refused_as_a_conflict() {
        let state = state();
        state
            .db
            .set_setting("ModelRatio", r#"{"probe-v":1.0}"#)
            .expect("stored");
        let change = |expected: Option<&str>, ratio: f64| ModelPricingChange {
            model_name: "probe-v".to_string(),
            expected_version: expected.map(String::from),
            pricing: Some(draft(json!({ "ModelRatio": ratio }))),
            reset: false,
        };

        let error = apply(&state, &[change(Some("stale-version"), 2.0)])
            .expect_err("a stale version must be refused");
        assert_eq!(
            error,
            WriteError::Conflict("probe-v: pricing changed; reload before saving".to_string())
        );

        let current = version_of(&configured_for(&read_maps(&state).unwrap(), "probe-v"));
        apply(&state, &[change(Some(&current), 2.0)]).expect("a current version saves");
        let stored = read_maps(&state).expect("maps");
        assert_eq!(stored.ratio.get("probe-v"), Some(&json!(2.0)));
    }

    /// The draft is the whole configuration, so a key the operator deleted is
    /// deleted from the store rather than left behind.
    #[test]
    fn omitting_a_key_removes_it() {
        let state = state();
        state
            .db
            .set_setting(
                "billing_expr",
                r#"{"probe-drop":"tier(\"base\", p * 1)"}"#,
            )
            .expect("stored");
        state
            .db
            .set_setting("billing_mode", r#"{"probe-drop":"tiered_expr"}"#)
            .expect("stored");

        apply(
            &state,
            &[ModelPricingChange {
                model_name: "probe-drop".to_string(),
                expected_version: None,
                pricing: Some(draft(json!({ "ModelRatio": 4.0 }))),
                reset: false,
            }],
        )
        .expect("applied");

        let stored = read_maps(&state).expect("maps");
        assert_eq!(stored.ratio.get("probe-drop"), Some(&json!(4.0)));
        assert!(!stored.expr.contains_key("probe-drop"));
        assert!(!stored.mode.contains_key("probe-drop"));
    }

    /// Reset forgets overrides; the model then resolves from the shipped pack.
    #[test]
    fn reset_forgets_one_model_or_all_of_them() {
        let state = state();
        state
            .db
            .set_setting("ModelRatio", r#"{"probe-r1":5.0,"probe-r2":6.0}"#)
            .expect("stored");

        reset(&state, &["probe-r1".to_string()]).expect("reset one");
        let stored = read_maps(&state).expect("maps");
        assert!(!stored.ratio.contains_key("probe-r1"));
        assert!(stored.ratio.contains_key("probe-r2"));

        reset(&state, &[]).expect("reset all");
        let stored = read_maps(&state).expect("maps");
        assert!(stored.ratio.is_empty());
        // Resetting an instance with nothing overridden is a no-op success.
        assert!(reset(&state, &[]).expect("second reset").is_empty());
    }

    /// A reset must reach the engine that prices requests, not only the store.
    ///
    /// This is the regression guard for a defect the live probe found: the
    /// reload merged the stored table into the engine entry by entry, so a
    /// deleted ratio kept charging until the process restarted. The store and
    /// the console both looked correct while every request was billed at the
    /// removed price.
    #[test]
    fn a_reset_reaches_the_running_engine() {
        let state = state();
        apply(
            &state,
            &[ModelPricingChange {
                model_name: "probe-engine".to_string(),
                expected_version: None,
                pricing: Some(draft(json!({ "ModelRatio": 5.0, "billing_mode": "ratio" }))),
                reset: false,
            }],
        )
        .expect("saved");
        {
            let pricing = state.billing.pricing();
            let guard = pricing.read();
            assert_eq!(guard.price_data("probe-engine", "default").model_ratio, 5.0);
        }

        reset(&state, &["probe-engine".to_string()]).expect("reset");
        let pricing = state.billing.pricing();
        let guard = pricing.read();
        assert_eq!(
            guard.price_data("probe-engine", "default").model_ratio,
            1.0,
            "the engine must forget the deleted ratio"
        );
    }

    /// The conversion preview turns a ratio into the equivalent expression,
    /// priced in USD per million tokens.
    #[test]
    fn a_ratio_converts_to_an_equivalent_expression() {
        let state = state();
        let conversion = convert(&state, "probe-conv", &draft(json!({"ModelRatio": 1.0})))
            .expect("conversion");
        assert_eq!(
            conversion.expression.as_deref(),
            Some("tier(\"base\", p * 2 + c * 2)")
        );
        assert!(conversion.unsupported_reason.is_none());
    }

    /// A model already on the expression path has nothing to convert, and the
    /// caller is told so by reason rather than by an empty field.
    #[test]
    fn a_model_already_on_expressions_is_not_converted() {
        let state = state();
        let conversion = convert(
            &state,
            "probe-expr-conv",
            &draft(json!({
                "billing_expr": "tier(\"base\", p * 1)",
                "billing_mode": "tiered_expr"
            })),
        )
        .expect("conversion");
        assert!(conversion.expression.is_none());
        assert_eq!(
            conversion.unsupported_reason.as_deref(),
            Some("This model already uses an expression.")
        );
    }

    /// A draft the engine could not honour is refused by name.
    #[test]
    fn an_unusable_draft_is_refused() {
        let state = state();
        let negative = preview(&state, "probe-bad", &draft(json!({"ModelRatio": -1.0})))
            .expect_err("a negative ratio must be refused");
        assert!(negative.contains("not negative"), "{negative}");

        let mode = preview(
            &state,
            "probe-bad",
            &draft(json!({"billing_mode": "percent"})),
        )
        .expect_err("an unknown mode must be refused");
        assert!(mode.contains("ratio or tiered_expr"), "{mode}");

        let key = preview(&state, "probe-bad", &draft(json!({"FixPrice": 1.0})))
            .expect_err("an unknown key must be refused");
        assert!(key.contains("unknown pricing key"), "{key}");
    }

    /// A table that is not a JSON object is an error, not an empty table: the
    /// engine would still charge the shipped price, so reporting "no prices"
    /// would describe a state the instance is not in.
    #[test]
    fn a_malformed_table_is_an_error() {
        let state = state();
        state.db.set_setting("ModelRatio", "[]").expect("stored");
        let error = snapshot(&state, &["probe-a".to_string()]).expect_err("must fail");
        assert!(error.contains("must be a JSON object"), "{error}");
    }

    /// Naming no models lists the catalogue: the overrides plus every model the
    /// pack prices with an expression. A stored override must not shrink that
    /// list, which is what the second half of this test pins.
    #[test]
    fn an_empty_request_lists_the_catalogue() {
        let state = state();
        let first = snapshot(&state, &[]).expect("snapshot");
        assert!(
            first.entries.len() > 10,
            "expected the shipped pack's models, got {}",
            first.entries.len()
        );

        state
            .db
            .set_setting("ModelRatio", r#"{"probe-only":7.0}"#)
            .expect("stored");
        let with_override = snapshot(&state, &[]).expect("snapshot");
        assert!(
            with_override.entries.len() > first.entries.len(),
            "an override should be added to the catalogue, not replace it"
        );
        assert!(with_override
            .entries
            .iter()
            .any(|entry| entry.model_name == "probe-only"));
    }
}
