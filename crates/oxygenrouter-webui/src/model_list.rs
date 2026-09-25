//! `GET /v1/models` in whichever dialect the client speaks.
//!
//! The reference resolves the client's dialect and answers accordingly
//! (`router/relay-router.go:25-43`): an OpenAI client gets `{object:list,data:[…]}`,
//! an Anthropic client gets the `first_id`/`has_more` envelope with `display_name`,
//! and a Gemini client gets `{models:[{name,…}]}`. It also serves
//! `GET /v1beta/models` for Gemini.
//!
//! We returned OpenAI shape to everyone, so an Anthropic or Gemini SDK could not
//! parse the list, and `/v1beta/models` 404'd outright. The shapes below are
//! pinned to what the live reference actually returned, not to the vendor docs.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use axum::http::HeaderMap;
use serde_json::{json, Value};

/// One model in the list, before it is shaped for a dialect.
#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub id: String,
    /// Who serves it: a channel provider, a metadata vendor, or `system`.
    pub owned_by: String,
    /// Unix seconds the entry was created.
    pub created: i64,
    pub display_name: String,
    pub description: Option<String>,
    /// Endpoint families the model can be called through. Empty means unknown.
    pub endpoints: Vec<String>,
}

/// The dialect a client speaks when it asks for the model list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelListDialect {
    OpenAi,
    Anthropic,
    Gemini,
}

/// The endpoint families we serve, used when no per-model list is recorded.
const DEFAULT_ENDPOINTS: &[&str] = &["openai", "openai-response", "anthropic", "gemini"];

/// Detect the client's dialect from its credential headers.
///
/// The reference keys off the same two signals (`relay-router.go:27-32`): an
/// Anthropic client sends `x-api-key` alongside `anthropic-version`, and a Gemini
/// client sends `x-goog-api-key` (or `?key=`). Anything else is OpenAI.
pub fn dialect_from_headers(headers: &HeaderMap, query_key: Option<&str>) -> ModelListDialect {
    let present = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    };

    if present("x-api-key") && present("anthropic-version") {
        return ModelListDialect::Anthropic;
    }
    if present("x-goog-api-key") || query_key.map(|k| !k.is_empty()).unwrap_or(false) {
        return ModelListDialect::Gemini;
    }
    ModelListDialect::OpenAi
}

/// Shape the model list for `dialect`.
pub fn render(models: &[ModelEntry], dialect: ModelListDialect) -> Value {
    match dialect {
        ModelListDialect::OpenAi => openai_shape(models),
        ModelListDialect::Anthropic => anthropic_shape(models),
        ModelListDialect::Gemini => gemini_shape(models),
    }
}

fn endpoint_types(model: &ModelEntry) -> Vec<String> {
    if model.endpoints.is_empty() {
        DEFAULT_ENDPOINTS.iter().map(|s| s.to_string()).collect()
    } else {
        model.endpoints.clone()
    }
}

fn openai_shape(models: &[ModelEntry]) -> Value {
    let data: Vec<Value> = models
        .iter()
        .map(|m| {
            // `supported_endpoint_types` is what the reference reports per model;
            // it is how an SDK learns it may send Claude or Gemini shape for this
            // id. Recording it keeps that discovery working.
            json!({
                "id": m.id,
                "object": "model",
                "created": m.created,
                "owned_by": m.owned_by,
                "supported_endpoint_types": endpoint_types(m),
            })
        })
        .collect();

    json!({"object": "list", "data": data, "success": true})
}

/// Anthropic's list envelope.
///
/// `first_id`/`last_id`/`has_more` are part of the shape, and `created_at` is an
/// RFC 3339 string rather than a unix integer -- a mismatch here is what makes an
/// Anthropic SDK reject the response.
fn anthropic_shape(models: &[ModelEntry]) -> Value {
    let data: Vec<Value> = models
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "created_at": rfc3339(m.created),
                "display_name": m.display_name,
                "type": "model",
            })
        })
        .collect();

    let first = models.first().map(|m| m.id.clone());
    let last = models.last().map(|m| m.id.clone());
    json!({
        "data": data,
        "first_id": first,
        "has_more": false,
        "last_id": last,
    })
}

/// Gemini's list envelope.
///
/// Every field is present even when null, which is how the reference emits it and
/// what the Gemini client libraries expect.
fn gemini_shape(models: &[ModelEntry]) -> Value {
    let list: Vec<Value> = models
        .iter()
        .map(|m| {
            json!({
                "name": m.id,
                "baseModelId": Value::Null,
                "version": Value::Null,
                "displayName": m.display_name,
                "description": m.description,
                "inputTokenLimit": Value::Null,
                "outputTokenLimit": Value::Null,
                "supportedGenerationMethods": Value::Null,
                "thinking": Value::Null,
                "temperature": Value::Null,
                "maxTemperature": Value::Null,
                "topP": Value::Null,
                "topK": Value::Null,
            })
        })
        .collect();

    json!({"models": list, "nextPageToken": Value::Null})
}

/// Render unix seconds as RFC 3339 in UTC, matching the reference's `created_at`.
pub fn rfc3339(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0)
        .unwrap_or_else(|| chrono::DateTime::from_timestamp(0, 0).expect("epoch is valid"))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The single-model retrieve shape for the client's dialect.
///
/// Anthropic wraps the model in the same field names as its list; Gemini has no
/// retrieve route on the reference, and OpenAI returns the bare model object.
pub fn render_one(model: &ModelEntry, dialect: ModelListDialect) -> Value {
    match dialect {
        ModelListDialect::Anthropic => json!({
            "id": model.id,
            "created_at": rfc3339(model.created),
            "display_name": model.display_name,
            "type": "model",
        }),
        _ => json!({
            "id": model.id,
            "object": "model",
            "created": model.created,
            "owned_by": model.owned_by,
            "supported_endpoint_types": endpoint_types(model),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> ModelEntry {
        ModelEntry {
            id: id.to_string(),
            owned_by: "openai".to_string(),
            created: 1626777600,
            display_name: id.to_string(),
            description: None,
            endpoints: Vec::new(),
        }
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                axum::http::HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    #[test]
    fn an_anthropic_credential_selects_the_anthropic_dialect() {
        let h = headers(&[("x-api-key", "k"), ("anthropic-version", "2023-06-01")]);
        assert_eq!(dialect_from_headers(&h, None), ModelListDialect::Anthropic);
    }

    #[test]
    fn x_api_key_alone_is_not_anthropic() {
        // `x-api-key` without the version header is not enough to be Anthropic.
        let h = headers(&[("x-api-key", "k")]);
        assert_eq!(dialect_from_headers(&h, None), ModelListDialect::OpenAi);
    }

    #[test]
    fn a_gemini_key_header_or_query_selects_the_gemini_dialect() {
        assert_eq!(
            dialect_from_headers(&headers(&[("x-goog-api-key", "k")]), None),
            ModelListDialect::Gemini
        );
        assert_eq!(
            dialect_from_headers(&HeaderMap::new(), Some("k")),
            ModelListDialect::Gemini
        );
        assert_eq!(
            dialect_from_headers(&HeaderMap::new(), Some("")),
            ModelListDialect::OpenAi
        );
    }

    #[test]
    fn a_bearer_token_is_openai() {
        let h = headers(&[("authorization", "Bearer k")]);
        assert_eq!(dialect_from_headers(&h, None), ModelListDialect::OpenAi);
    }

    #[test]
    fn the_openai_shape_keeps_object_list_and_declares_endpoints() {
        let out = render(&[entry("gpt-4o")], ModelListDialect::OpenAi);
        assert_eq!(out["object"], "list");
        assert_eq!(out["data"][0]["object"], "model");
        assert_eq!(out["data"][0]["owned_by"], "openai");
        assert_eq!(out["data"][0]["created"], 1626777600);
        // Endpoint families must be advertised, or an SDK cannot tell it may
        // send Claude/Gemini shape for this id.
        let endpoints = out["data"][0]["supported_endpoint_types"].as_array().unwrap();
        assert!(endpoints.iter().any(|e| e == "anthropic"));
        assert!(endpoints.iter().any(|e| e == "gemini"));
    }

    #[test]
    fn the_anthropic_shape_uses_its_own_envelope_and_rfc3339() {
        let out = render(&[entry("claude-3-5-sonnet")], ModelListDialect::Anthropic);
        assert_eq!(out["data"][0]["type"], "model");
        assert_eq!(out["data"][0]["display_name"], "claude-3-5-sonnet");
        // RFC 3339 with a Z, which is what an Anthropic SDK parses.
        assert_eq!(out["data"][0]["created_at"], "2021-07-20T10:40:00Z");
        assert_eq!(out["first_id"], "claude-3-5-sonnet");
        assert_eq!(out["last_id"], "claude-3-5-sonnet");
        assert_eq!(out["has_more"], false);
        // The OpenAI-only envelope must not leak into this shape.
        assert!(out.get("object").is_none());
        assert!(out["data"][0].get("object").is_none());
    }

    #[test]
    fn the_gemini_shape_uses_name_and_speaks_for_every_nullable_field() {
        let out = render(&[entry("gemini-2.5-flash")], ModelListDialect::Gemini);
        assert_eq!(out["models"][0]["name"], "gemini-2.5-flash");
        assert_eq!(out["models"][0]["displayName"], "gemini-2.5-flash");
        // The client libraries read these keys, so they must be present.
        for key in [
            "baseModelId",
            "version",
            "description",
            "inputTokenLimit",
            "outputTokenLimit",
            "supportedGenerationMethods",
        ] {
            assert!(
                out["models"][0].as_object().unwrap().contains_key(key),
                "missing key {key}"
            );
        }
        assert!(out.get("nextPageToken").is_some());
        assert!(out["nextPageToken"].is_null());
    }

    #[test]
    fn an_empty_catalogue_still_produces_a_valid_envelope() {
        for dialect in [
            ModelListDialect::OpenAi,
            ModelListDialect::Anthropic,
            ModelListDialect::Gemini,
        ] {
            let out = render(&[], dialect);
            assert!(out.is_object(), "empty list must still be an object");
        }
    }

    #[test]
    fn recorded_endpoints_win_over_the_default_set() {
        let mut m = entry("m");
        m.endpoints = vec!["openai".to_string()];
        let out = render(&[m], ModelListDialect::OpenAi);
        assert_eq!(out["data"][0]["supported_endpoint_types"], json!(["openai"]));
    }

    #[test]
    fn the_retrieve_shape_follows_the_dialect() {
        let m = entry("claude-3-5-sonnet");
        let anthropic = render_one(&m, ModelListDialect::Anthropic);
        assert_eq!(anthropic["type"], "model");
        assert_eq!(anthropic["created_at"], "2021-07-20T10:40:00Z");

        let openai = render_one(&m, ModelListDialect::OpenAi);
        assert_eq!(openai["object"], "model");
        assert_eq!(openai["created"], 1626777600);
    }
}
