//! Validating the usage facts a plugin reports, and the token counts they imply.
//!
//! Ported from `relay/channel/task/jsplugin/adaptor.go:1486,1535` and
//! `pkg/jsplugin/registry.go`. A fact may only influence billing after it has
//! been checked against the shape the plugin declared for that model: an
//! undeclared numeric fact stays extensible but is bounded by the largest
//! canonical task limit, a declared fact must match its type/unit/enum, and the
//! three host-owned token fields are clamped to the quota domain.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::{UsageFieldSchema, MAX_IMAGE_N, MAX_TASK_DURATION_SECONDS};

/// A usage fact's declared shape is the only thing that decides how it is
/// validated.
pub type UsageSchema = BTreeMap<String, UsageFieldSchema>;

/// Validate what `extractUsageOnComplete` returned, and answer with the
/// normalized facts. Unknown keys are refused only when they claim to be one of
/// the host's own counters; everything else is carried through as declared.
pub fn validate_completion_facts(facts: &Value, schema: &UsageSchema) -> Result<Value, String> {
    let Some(object) = facts.as_object() else {
        return Err("plugin usage hook must return an object".to_string());
    };
    let mut validated = Map::new();
    for (key, value) in object {
        validated.insert(key.clone(), value.clone());
        if let Some(field) = schema.get(key) {
            let number = validate_usage_value(value, field)?;
            if field.kind == "number" {
                validated.insert(key.clone(), number_value(number));
            }
            continue;
        }
        if let Some(limit) = canonical_usage_limit(key) {
            let Some(number) = usage_number(value) else {
                return Err("plugin usage value must be a number".to_string());
            };
            validate_usage_number_limit(number, limit)?;
            validated.insert(key.clone(), number_value(number));
            continue;
        }
        match key.as_str() {
            "upstreamUnits" | "completionTokens" | "totalTokens" => {
                let Some(number) = usage_number(value) else {
                    return Err(
                        "plugin usage value must be a finite non-negative number".to_string()
                    );
                };
                if !number.is_finite() || number < 0.0 {
                    return Err(
                        "plugin usage value must be a finite non-negative number".to_string()
                    );
                }
                validated.insert(key.clone(), Value::Number((number as i64).into()));
            }
            _ => {
                if let Some(number) = usage_number(value) {
                    validated.insert(key.clone(), number_value(number));
                }
            }
        }
    }
    Ok(Value::Object(validated))
}

/// Apply validated facts to a task result, the way the reference does: the
/// host's `upstreamUnits` is the strongest signal, then explicit token counts
/// (`adaptor.go:827`).
pub fn apply_completion_usage(result: &mut crate::task::TaskResult, facts: &Value) {
    result.usage_facts = facts.clone();
    let Some(object) = facts.as_object() else {
        return;
    };
    let number = |key: &str| object.get(key).and_then(Value::as_f64);
    if let Some(units) = number("upstreamUnits").filter(|units| *units > 0.0) {
        result.completion_tokens = units;
        result.total_tokens = units;
        return;
    }
    if let Some(completion) = number("completionTokens") {
        result.completion_tokens = completion;
    }
    if let Some(total) = number("totalTokens") {
        result.total_tokens = total;
    }
}

/// The token count a settlement should use: total, else completion.
pub fn billable_tokens(result: &crate::task::TaskResult) -> i64 {
    let tokens = if result.total_tokens > 0.0 {
        result.total_tokens
    } else {
        result.completion_tokens
    };
    crate::task::TaskResult::positive_int(tokens)
}

/// One declared field's value (`adaptor.go:1535`).
fn validate_usage_value(value: &Value, field: &UsageFieldSchema) -> Result<f64, String> {
    if let Some(values) = &field.enum_values {
        let Some(text) = value.as_str() else {
            return Err("plugin usage enum must be a string".to_string());
        };
        if values.iter().any(|allowed| allowed == text) {
            return Ok(0.0);
        }
        return Err("plugin usage enum is not an allowed value".to_string());
    }
    if field.kind == "boolean" {
        if value.is_boolean() {
            return Ok(0.0);
        }
        return Err("plugin usage value must be a boolean".to_string());
    }
    let Some(number) = usage_number(value) else {
        return Err("plugin usage value must be a number".to_string());
    };
    if field.unit == "token" || field.unit == "credit" {
        if !number.is_finite() || number < 0.0 {
            return Err("plugin usage value must be a finite non-negative number".to_string());
        }
        // The reference saturates to the quota domain but keeps a fractional
        // credit fact like 3.5 intact.
        if number > i32::MAX as f64 {
            return Ok(i32::MAX as f64);
        }
        return Ok(number);
    }
    let limit = if field.unit == "count" {
        MAX_IMAGE_N
    } else {
        MAX_TASK_DURATION_SECONDS
    };
    validate_usage_number_limit(number, limit)?;
    Ok(number)
}

fn validate_usage_number_limit(number: f64, limit: f64) -> Result<(), String> {
    if !number.is_finite() || number < 0.0 {
        return Err("plugin usage value must be a finite non-negative number".to_string());
    }
    if number > limit {
        return Err("plugin usage value exceeds the host limit".to_string());
    }
    Ok(())
}

/// The keys whose limits the host owns, whatever the schema says
/// (`adaptor.go:1614`).
fn canonical_usage_limit(key: &str) -> Option<f64> {
    let normalized: String = key
        .to_ascii_lowercase()
        .chars()
        .filter(|character| *character != '_' && *character != '-')
        .collect();
    match normalized.as_str() {
        "duration" | "durationseconds" | "second" | "seconds" => Some(MAX_TASK_DURATION_SECONDS),
        "n" | "count" | "imagecount" | "samplecount" | "batchcount" | "numimages" => {
            Some(MAX_IMAGE_N)
        }
        _ => None,
    }
}

fn usage_number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|number| number.is_finite())
}

fn number_value(number: f64) -> Value {
    serde_json::Number::from_f64(number)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UsageExample, UsageProfile};

    fn schema(entries: &[(&str, UsageFieldSchema)]) -> UsageSchema {
        entries
            .iter()
            .map(|(name, field)| ((*name).to_string(), field.clone()))
            .collect()
    }

    #[test]
    fn declared_facts_are_checked_against_their_field() {
        let schema = schema(&[
            (
                "seconds",
                UsageFieldSchema {
                    kind: "number".to_string(),
                    unit: "second".to_string(),
                    ..Default::default()
                },
            ),
            (
                "mode",
                UsageFieldSchema {
                    enum_values: Some(vec!["std".to_string(), "pro".to_string()]),
                    ..Default::default()
                },
            ),
        ]);
        let facts = serde_json::json!({"seconds": 5, "mode": "pro"});
        assert_eq!(
            validate_completion_facts(&facts, &schema).expect("valid"),
            serde_json::json!({"seconds": 5.0, "mode": "pro"})
        );

        assert!(validate_completion_facts(&serde_json::json!({"seconds": "5"}), &schema).is_err());
        assert!(validate_completion_facts(&serde_json::json!({"mode": "nope"}), &schema).is_err());
        assert!(
            validate_completion_facts(&serde_json::json!({"seconds": 3601}), &schema).is_err()
        );
    }

    #[test]
    fn the_host_owned_token_fields_are_bounded_and_integral() {
        let facts = validate_completion_facts(
            &serde_json::json!({"upstreamUnits": 4.0, "totalTokens": 2.6}),
            &UsageSchema::new(),
        )
        .expect("valid");
        assert_eq!(facts["upstreamUnits"], serde_json::json!(4));
        assert_eq!(facts["totalTokens"], serde_json::json!(2));

        assert!(validate_completion_facts(
            &serde_json::json!({"totalTokens": -1}),
            &UsageSchema::new()
        )
        .is_err());
        assert!(validate_completion_facts(
            &serde_json::json!({"upstreamUnits": "3"}),
            &UsageSchema::new()
        )
        .is_err());
    }

    #[test]
    fn an_undeclared_numeric_fact_stays_extensible_but_canonical_keys_are_bounded() {
        // The completion path keeps undeclared numeric facts usable by an
        // expression (`adaptor.go:1526`); the ratio path's task-duration
        // fallback does not apply here.
        let facts =
            validate_completion_facts(&serde_json::json!({"custom_ratio": 3601}), &UsageSchema::new())
                .expect("valid");
        assert_eq!(facts["custom_ratio"].as_f64(), Some(3601.0));

        // A canonical key is the host's to bound, whatever the schema says.
        assert!(validate_completion_facts(
            &serde_json::json!({"duration": 3601}),
            &UsageSchema::new()
        )
        .is_err());
    }

    #[test]
    fn completion_usage_prefers_upstream_units() {
        let mut result = crate::task::TaskResult {
            status: crate::task::STATUS_SUCCESS.to_string(),
            usage_facts: serde_json::Value::Null,
            ..Default::default()
        };
        apply_completion_usage(
            &mut result,
            &serde_json::json!({"upstreamUnits": 7, "completionTokens": 3, "totalTokens": 3}),
        );
        assert_eq!(result.total_tokens, 7.0);
        assert_eq!(billable_tokens(&result), 7);

        let mut result = crate::task::TaskResult {
            status: crate::task::STATUS_SUCCESS.to_string(),
            usage_facts: serde_json::Value::Null,
            ..Default::default()
        };
        apply_completion_usage(
            &mut result,
            &serde_json::json!({"completionTokens": 3, "totalTokens": 5}),
        );
        assert_eq!(billable_tokens(&result), 5);
    }

    /// Profiles are lookup sugar: the first matching model decides, and the
    /// base schema is the fallback (`registry.go:146`).
    #[test]
    fn a_profile_shadows_the_base_schema_for_its_models() {
        let usage = crate::PluginUsage {
            schema: schema(&[(
                "units",
                UsageFieldSchema {
                    kind: "number".to_string(),
                    unit: "count".to_string(),
                    ..Default::default()
                },
            )]),
            profiles: vec![UsageProfile {
                models: vec!["special".to_string()],
                schema: Some(schema(&[(
                    "seconds",
                    UsageFieldSchema {
                        kind: "number".to_string(),
                        unit: "second".to_string(),
                        ..Default::default()
                    },
                )])),
                examples: Vec::<UsageExample>::new(),
            }],
        };
        assert!(usage.for_models(&["SPECIAL"]).contains_key("seconds"));
        assert!(usage.for_models(&["other"]).contains_key("units"));
    }
}
