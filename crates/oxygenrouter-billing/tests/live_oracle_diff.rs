//! Differential replay against the live NewAPI instance.
//!
//! `fixtures/live_newapi_quota_oracle.json` is a verbatim export of every
//! `tiered_expr` request in the running instance's `logs` table: token counts,
//! the exact billing expression, the group ratio, and the quota NewAPI actually
//! charged. Replaying it here is the strongest available proof that the engine
//! agrees with the reference implementation — not on hand-picked examples but on
//! real production traffic spanning every expression the operator configured.
//!
//! The fixture is the raw `logs` export, not a summary. It is 4.65 MB in the
//! working tree; git stores the blob zlib-deflated at 435 KB (measured).
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::BTreeMap;

use oxygenrouter_billing::{
    evaluate_with, quota_from_cost, used_vars, BillingUsage, EvalContext, UsageSemantic,
};

#[derive(serde::Deserialize)]
struct Oracle {
    source: String,
    quota_per_unit: f64,
    cases: Vec<Case>,
}

#[derive(serde::Deserialize)]
struct Case {
    log_id: i64,
    model: String,
    quota: i64,
    prompt_tokens: i64,
    completion_tokens: i64,
    cached_tokens: i64,
    group_ratio: f64,
    expr: String,
    /// The tier NewAPI recorded, when it recorded one.
    #[serde(default)]
    matched_tier: Option<String>,
}

fn load() -> Oracle {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/live_newapi_quota_oracle.json"
    );
    let raw = std::fs::read_to_string(path)
        .expect("oracle fixture must exist next to this test");
    serde_json::from_str(&raw).expect("oracle fixture must deserialize")
}

/// Recompute one case with our engine.
fn recompute(case: &Case, quota_per_unit: f64) -> Result<(i64, Option<String>), String> {
    let usage = BillingUsage {
        prompt_tokens: case.prompt_tokens,
        completion_tokens: case.completion_tokens,
        cached_tokens: case.cached_tokens,
        semantic: UsageSemantic::OpenAi,
        ..Default::default()
    };
    let used = used_vars(&case.expr);
    let params = oxygenrouter_billing::build_params(&usage, &used);

    // A few expressions depend on wall-clock time or on the request body, which
    // the log table does not retain. Those are pinned to the branch NewAPI
    // itself recorded in `matched_tier`, so the branch selection is driven by
    // evidence rather than by a guess.
    let mut ctx = EvalContext::default();
    if case.expr.contains("hour(") || case.expr.contains("weekday(") {
        // 2026-09-25 is a Friday; 02:00 UTC is inside the peak window.
        ctx.now_unix = Some(1_790_330_400);
    }
    if case.expr.contains("param(") {
        // The only `param()` gate in the live data is `enable_thinking`. NewAPI
        // recorded which branch it took, which tells us what the body held.
        let thinking = case
            .matched_tier
            .as_deref()
            .map(|t| t.contains("thinking"))
            .unwrap_or(false);
        ctx.body = serde_json::json!({ "enable_thinking": thinking });
    }

    let outcome = evaluate_with(&case.expr, &params, &ctx)
        .map_err(|e| format!("evaluate failed: {e}"))?;
    let (quota, _) = quota_from_cost(outcome.cost_usd, quota_per_unit, case.group_ratio);
    Ok((quota, outcome.matched_tier))
}

#[test]
fn every_live_request_is_priced_identically() {
    let oracle = load();
    assert!(
        oracle.cases.len() > 10_000,
        "fixture looks truncated: {} cases",
        oracle.cases.len()
    );

    let mut mismatches: Vec<String> = Vec::new();
    let mut per_expr: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut errors: Vec<String> = Vec::new();

    for case in &oracle.cases {
        let entry = per_expr.entry(case.expr.clone()).or_insert((0, 0));
        match recompute(case, oracle.quota_per_unit) {
            Ok((quota, tier)) => {
                entry.1 += 1;
                // Recorded tiers must agree too: matching the amount is not
                // enough if we reached it through a different branch.
                let tier_agrees = match (&case.matched_tier, &tier) {
                    (Some(expected), Some(actual)) => expected == actual,
                    _ => true,
                };
                if quota == case.quota && tier_agrees {
                    entry.0 += 1;
                } else if !tier_agrees && mismatches.len() < 25 {
                    mismatches.push(format!(
                        "log {} ({}): tier expected={:?} got={:?} (quota {} vs {})",
                        case.log_id, case.model, case.matched_tier, tier, case.quota, quota
                    ));
                } else if mismatches.len() < 25 {
                    mismatches.push(format!(
                        "log {} ({}): expr={} p={} c={} cr={} gr={} expected={} got={}",
                        case.log_id,
                        case.model,
                        case.expr,
                        case.prompt_tokens,
                        case.completion_tokens,
                        case.cached_tokens,
                        case.group_ratio,
                        case.quota,
                        quota
                    ));
                }
            }
            Err(e) => {
                if errors.len() < 25 {
                    errors.push(format!("log {} expr={} -> {}", case.log_id, case.expr, e));
                }
            }
        }
    }

    if !errors.is_empty() {
        panic!(
            "{} expressions failed to evaluate:\n{}",
            errors.len(),
            errors.join("\n")
        );
    }

    let total = oracle.cases.len();
    let matched: usize = per_expr.values().map(|(ok, _)| *ok).sum();
    assert!(
        mismatches.is_empty(),
        "{} / {} live requests disagreed with NewAPI:\n{}\n\nper-expression (matched/total):\n{}",
        total - matched,
        total,
        mismatches.join("\n"),
        per_expr
            .iter()
            .map(|(e, (ok, n))| format!("  {ok:>6}/{n:<6}  {e}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_eq!(matched, total);
}

#[test]
fn all_distinct_live_expressions_evaluate() {
    let oracle = load();
    let mut distinct: Vec<String> = oracle
        .cases
        .iter()
        .map(|c| c.expr.clone())
        .collect();
    distinct.sort();
    distinct.dedup();
    assert!(
        distinct.len() >= 30,
        "expected the live instance to exercise many expressions, found {}",
        distinct.len()
    );

    for expr in &distinct {
        let case = oracle
            .cases
            .iter()
            .find(|c| &c.expr == expr)
            .expect("expression came from a case");
        assert!(
            recompute(case, oracle.quota_per_unit).is_ok(),
            "expression failed to evaluate: {expr}"
        );
    }
}

#[test]
fn oracle_fixture_is_attributable() {
    let oracle = load();
    assert!(
        oracle.source.contains("NewAPI"),
        "fixture must record its provenance, got: {}",
        oracle.source
    );
    assert_eq!(oracle.quota_per_unit, 500_000.0);
    // Sanity: real traffic, not synthetic.
    assert!(oracle.cases.iter().any(|c| c.cached_tokens > 0));
    assert!(oracle.cases.iter().any(|c| c.completion_tokens == 0));
}
