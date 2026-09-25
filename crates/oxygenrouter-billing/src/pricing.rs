//! Pricing tables: model -> expression, and the legacy model -> ratios.
//!
//! Mirrors NewAPI's two pricing sources:
//!
//! * `billing_setting.billing_expr` + `billing_setting.billing_mode` — the
//!   modern per-model expression map, where a model is priced by either a
//!   `tiered_expr` expression or the legacy `ratio` tables;
//! * `ModelRatio` / `CompletionRatio` / `CacheRatio` / … — the legacy tables.
//!
//! Resolution order matches `GetBillingExpr`: an operator-supplied expression
//! wins, then the built-in default (when the model's mode is `tiered_expr`).
//!
//! The shipped `default_pricing.json` is a **data** snapshot of public vendor
//! list prices (USD per million tokens). It is not derived from NewAPI's source;
//! see the file's `meta.provenance`.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::price::{PriceData, QUOTA_PER_UNIT};

/// How a model is priced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingMode {
    /// A per-model allowance expressed as a `tiered_expr` expression.
    TieredExpr,
    /// The classic multiplier tables.
    Ratio,
}

impl Default for BillingMode {
    fn default() -> Self {
        Self::Ratio
    }
}

/// The expression and legacy tables, as loaded from JSON.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PricingTables {
    #[serde(default)]
    pub billing_mode: HashMap<String, BillingMode>,
    #[serde(default)]
    pub billing_expr: HashMap<String, String>,
    #[serde(default)]
    pub model_ratio: HashMap<String, f64>,
    #[serde(default)]
    pub completion_ratio: HashMap<String, f64>,
    #[serde(default)]
    pub cache_ratio: HashMap<String, f64>,
    #[serde(default)]
    pub cache_creation_ratio: HashMap<String, f64>,
    #[serde(default)]
    pub image_ratio: HashMap<String, f64>,
    #[serde(default)]
    pub audio_ratio: HashMap<String, f64>,
    #[serde(default)]
    pub audio_completion_ratio: HashMap<String, f64>,
    #[serde(default)]
    pub model_price: HashMap<String, f64>,
    #[serde(default)]
    pub group_ratio: HashMap<String, f64>,
}

/// The shipped pricing document: metadata plus the tables.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PricingDocument {
    #[serde(default)]
    pub meta: PricingMeta,
    pub pricing: PricingTables,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PricingMeta {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub provenance: String,
    #[serde(default)]
    pub captured: String,
}

/// The default group ratio when a group is unknown, matching NewAPI.
pub const DEFAULT_GROUP_RATIO: f64 = 1.0;

/// A pricing table set with an operator-override layer on top.
///
/// Overrides come from the running instance's own option values; the shipped
/// document is only the fallback, exactly as `builtinBillingExpr` is upstream.
#[derive(Debug, Clone)]
pub struct Pricing {
    defaults: PricingDocument,
    /// Operator overrides for `billing_expr` (model -> expression).
    expr_overrides: HashMap<String, String>,
    /// Operator overrides for `billing_mode` (model -> mode).
    mode_overrides: HashMap<String, BillingMode>,
    /// Operator overrides for the legacy ratio tables.
    ratio_overrides: PricingTables,
    /// Operator override for `QuotaPerUnit`.
    quota_per_unit: Option<f64>,
}

impl Pricing {
    /// Load the built-in pack, then apply operator overrides.
    pub fn new(defaults: PricingDocument) -> Self {
        Self {
            defaults,
            expr_overrides: HashMap::new(),
            mode_overrides: HashMap::new(),
            ratio_overrides: PricingTables::default(),
            quota_per_unit: None,
        }
    }

    /// Load from the embedded pack shipped in this crate.
    pub fn from_embedded() -> Result<Self, PricingLoadError> {
        let raw = include_str!("../pricing/default_pricing.json");
        let doc: PricingDocument =
            serde_json::from_str(raw).map_err(PricingLoadError::Parse)?;
        Ok(Self::new(doc))
    }

    pub fn quota_per_unit(&self) -> f64 {
        self.quota_per_unit.unwrap_or(QUOTA_PER_UNIT)
    }

    pub fn set_quota_per_unit(&mut self, value: f64) {
        if value.is_finite() && value > 0.0 {
            self.quota_per_unit = Some(value);
        }
    }

    /// Install an operator `billing_expr` map (replaces, matching NewAPI's
    /// replace-not-merge semantics).
    pub fn set_expr_map(&mut self, map: HashMap<String, String>) {
        self.expr_overrides = map;
    }

    pub fn set_mode_map(&mut self, map: HashMap<String, BillingMode>) {
        self.mode_overrides = map;
    }

    pub fn set_ratio_tables(&mut self, tables: PricingTables) {
        self.ratio_overrides = tables;
    }

    pub fn set_group_ratio(&mut self, group: &str, ratio: f64) {
        if ratio.is_finite() {
            self.ratio_overrides
                .group_ratio
                .insert(group.to_string(), ratio);
        }
    }

    /// Resolve the billing mode for a model: override, then default, then `ratio`.
    pub fn mode(&self, model: &str) -> BillingMode {
        if let Some(mode) = self.mode_overrides.get(model) {
            return *mode;
        }
        if let Some(mode) = self.defaults.pricing.billing_mode.get(model) {
            return *mode;
        }
        BillingMode::Ratio
    }

    /// Resolve the pricing expression for a model, if it has one.
    ///
    /// Mirrors `GetBillingExpr`: an operator expression wins; otherwise the
    /// built-in expression applies only when the model's mode is `tiered_expr`.
    pub fn expression(&self, model: &str) -> Option<&str> {
        if let Some(expr) = self.expr_overrides.get(model) {
            return Some(expr.as_str());
        }
        if self.mode(model) != BillingMode::TieredExpr {
            return None;
        }
        self.defaults
            .pricing
            .billing_expr
            .get(model)
            .map(String::as_str)
    }

    /// The group ratio for a group name.
    pub fn group_ratio(&self, group: &str) -> f64 {
        self.ratio_overrides
            .group_ratio
            .get(group)
            .or_else(|| self.defaults.pricing.group_ratio.get(group))
            .copied()
            .unwrap_or(DEFAULT_GROUP_RATIO)
    }

    /// True when the group is explicitly configured (so a caller can tell an
    /// intentional `1.0` from a missing entry).
    pub fn has_group(&self, group: &str) -> bool {
        self.ratio_overrides.group_ratio.contains_key(group)
            || self.defaults.pricing.group_ratio.contains_key(group)
    }

    /// Build the legacy `PriceData` snapshot for a model in a given group.
    pub fn price_data(&self, model: &str, group: &str) -> PriceData {
        let table = |overrides: &HashMap<String, f64>,
                     defaults: &HashMap<String, f64>,
                     model: &str,
                     fallback: f64| {
            overrides
                .get(model)
                .or_else(|| defaults.get(model))
                .copied()
                .filter(|v| v.is_finite())
                .unwrap_or(fallback)
        };

        let model_price = table(
            &self.ratio_overrides.model_price,
            &self.defaults.pricing.model_price,
            model,
            0.0,
        );

        PriceData {
            model_ratio: table(
                &self.ratio_overrides.model_ratio,
                &self.defaults.pricing.model_ratio,
                model,
                1.0,
            ),
            completion_ratio: table(
                &self.ratio_overrides.completion_ratio,
                &self.defaults.pricing.completion_ratio,
                model,
                1.0,
            ),
            cache_ratio: table(
                &self.ratio_overrides.cache_ratio,
                &self.defaults.pricing.cache_ratio,
                model,
                1.0,
            ),
            cache_creation_ratio: table(
                &self.ratio_overrides.cache_creation_ratio,
                &self.defaults.pricing.cache_creation_ratio,
                model,
                1.0,
            ),
            cache_creation_5m_ratio: 1.0,
            cache_creation_1h_ratio: 1.0,
            image_ratio: table(
                &self.ratio_overrides.image_ratio,
                &self.defaults.pricing.image_ratio,
                model,
                1.0,
            ),
            group_ratio: self.group_ratio(group),
            model_price,
            use_price: model_price > 0.0,
            other_ratios: Vec::new(),
        }
    }

    /// True when this model has no price at all, so it should be free rather
    /// than falling back to a default ratio of 1.
    ///
    /// A per-call price (`model_price`) counts as priced even when no token
    /// ratio is present — `dall-e-3` has a price and no ratio, and treating it
    /// as unpriced would make image generation free.
    pub fn is_unpriced(&self, model: &str) -> bool {
        if self.expression(model).is_some() {
            return false;
        }
        let per_call = self
            .ratio_overrides
            .model_price
            .get(model)
            .or_else(|| self.defaults.pricing.model_price.get(model))
            .copied()
            .unwrap_or(0.0);
        if per_call > 0.0 {
            return false;
        }
        !self.ratio_overrides.model_ratio.contains_key(model)
            && !self.defaults.pricing.model_ratio.contains_key(model)
    }

    /// Number of models with an expression, for diagnostics.
    pub fn expression_count(&self) -> usize {
        self.defaults.pricing.billing_expr.len().max(self.expr_overrides.len())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PricingLoadError {
    #[error("pricing document is not valid JSON: {0}")]
    Parse(serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn embedded() -> Pricing {
        Pricing::from_embedded().expect("embedded pricing pack must parse")
    }

    #[test]
    fn embedded_pack_loads() {
        let pricing = embedded();
        assert!(pricing.expression_count() > 300, "expected a broad pack");
        assert_eq!(pricing.quota_per_unit(), 500_000.0);
    }

    #[test]
    fn tiered_models_resolve_to_their_expression() {
        let pricing = embedded();
        // Models the live instance prices with expressions.
        let mut found = 0;
        for model in ["DeepSeek-V4.1-Flash", "MiniMax-M2", "Ling-1T"] {
            if let Some(expr) = pricing.expression(model) {
                assert_eq!(pricing.mode(model), BillingMode::TieredExpr);
                assert!(expr.contains("tier("), "{model} -> {expr}");
                found += 1;
            }
        }
        assert!(found > 0, "expected at least one known tiered model");
    }

    #[test]
    fn expression_is_none_for_ratio_models() {
        let pricing = embedded();
        // dall-e-3 has no expression and no mode entry, so it falls to the
        // legacy tables (it is priced per call).
        assert_eq!(pricing.mode("dall-e-3"), BillingMode::Ratio);
        assert!(pricing.expression("dall-e-3").is_none());
        // A completely unknown model also has no expression.
        assert!(pricing.expression("no-such-model-abc").is_none());
    }

    #[test]
    fn modern_models_really_are_expression_priced() {
        // gpt-4o is `tiered_expr` in the live pack, not ratio. Pinning this
        // guards against anyone assuming the legacy path for it.
        let pricing = embedded();
        assert_eq!(pricing.mode("gpt-4o"), BillingMode::TieredExpr);
        assert_eq!(
            pricing.expression("gpt-4o"),
            Some(r#"tier("standard", p * 2.5 + cr * 1.25 + c * 10)"#)
        );
    }

    #[test]
    fn operator_expression_override_wins() {
        let mut pricing = embedded();
        pricing.set_expr_map(HashMap::from([(
            "custom-model".to_string(),
            r#"tier("only", p * 99)"#.to_string(),
        )]));
        assert_eq!(pricing.expression("custom-model"), Some(r#"tier("only", p * 99)"#));
    }

    #[test]
    fn operator_expression_overrides_a_ratio_model() {
        let mut pricing = embedded();
        // gpt-4o is ratio mode by default; an explicit expression must win,
        // matching GetBillingExpr.
        pricing.set_expr_map(HashMap::from([(
            "gpt-4o".to_string(),
            r#"tier("x", p * 1)"#.to_string(),
        )]));
        assert_eq!(pricing.expression("gpt-4o"), Some(r#"tier("x", p * 1)"#));
    }

    #[test]
    fn legacy_ratios_resolve_with_defaults() {
        let pricing = embedded();
        let data = pricing.price_data("gpt-4o", "default");
        assert!(data.model_ratio > 0.0);
        assert!(data.completion_ratio > 0.0);
        assert!(!data.use_price, "gpt-4o is token-priced");
    }

    #[test]
    fn per_call_models_use_price() {
        let pricing = embedded();
        let data = pricing.price_data("dall-e-3", "default");
        assert!(data.use_price, "dall-e-3 should be per-call priced");
        assert!(data.model_price > 0.0);
    }

    #[test]
    fn group_ratio_lookup_and_default() {
        let mut pricing = embedded();
        assert_eq!(pricing.group_ratio("no-such-group"), DEFAULT_GROUP_RATIO);
        assert!(!pricing.has_group("no-such-group"));
        pricing.set_group_ratio("vip", 0.5);
        assert_eq!(pricing.group_ratio("vip"), 0.5);
        assert!(pricing.has_group("vip"));
    }

    #[test]
    fn unknown_model_is_reported_as_unpriced() {
        let pricing = embedded();
        assert!(pricing.is_unpriced("definitely-not-a-real-model-xyz"));
    }

    #[test]
    fn a_per_call_model_is_priced_even_without_a_token_ratio() {
        // Regression: dall-e-3 has model_price but no model_ratio. Treating it
        // as unpriced would silently serve image generation for free.
        let pricing = embedded();
        assert!(
            !pricing.is_unpriced("dall-e-3"),
            "a per-call price must count as priced"
        );
        let data = pricing.price_data("dall-e-3", "default");
        assert!(data.use_price);
        assert!(data.model_price > 0.0);
    }

    #[test]
    fn quota_per_unit_override_is_validated() {
        let mut pricing = embedded();
        pricing.set_quota_per_unit(-1.0);
        assert_eq!(pricing.quota_per_unit(), QUOTA_PER_UNIT);
        pricing.set_quota_per_unit(f64::NAN);
        assert_eq!(pricing.quota_per_unit(), QUOTA_PER_UNIT);
        pricing.set_quota_per_unit(1_000_000.0);
        assert_eq!(pricing.quota_per_unit(), 1_000_000.0);
    }

    #[test]
    fn every_embedded_expression_parses_with_our_engine() {
        // The pack ships expressions written by operators; none may be
        // unparseable or settlement would fail at runtime.
        let pricing = embedded();
        let mut checked = 0;
        for (model, expr) in &pricing.defaults.pricing.billing_expr {
            let used = crate::expr::used_vars(expr);
            let params = crate::expr::TokenParams {
                p: 1000.0,
                c: 100.0,
                len: 1000.0,
                ..Default::default()
            };
            match crate::expr::evaluate(expr, &params) {
                Ok(_) => checked += 1,
                Err(e) => panic!("model {model} has an unparseable expression: {e}\n  {expr}"),
            }
            let _ = used;
        }
        assert!(checked > 300, "only checked {checked} expressions");
    }
}
