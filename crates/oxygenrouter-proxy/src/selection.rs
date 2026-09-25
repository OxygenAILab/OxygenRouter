//! Priority-tiered channel selection with smoothing-weighted random pick.
//!
//! Mirrors NewAPI's `GetRandomSatisfiedChannel` (`model/channel_cache.go`):
//!
//! * Eligible channels are grouped by priority. The `retry` counter selects a
//!   tier — retry 0 takes the highest priority, retry 1 the next, and so on —
//!   which is how failover walks from preferred channels to backups.
//! * Within a tier the pick is weighted random, with NewAPI's smoothing rules so
//!   that small or zero weights still spread load instead of collapsing onto one
//!   channel.
//!
//! Kept free of I/O so the policy is directly testable.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use oxygenrouter_core::{Channel, ModelMap};

/// The outcome of a selection attempt.
///
/// `PartialEq` is not derived because `Channel` is not comparable; compare
/// `Selection::Picked` results by channel id instead.
#[derive(Debug, Clone)]
pub enum Selection {
    /// A channel was chosen.
    Picked(Channel),
    /// No channel matches the group/model at all.
    NoCandidate,
    /// Candidates exist but every priority tier has been walked through.
    TiersExhausted,
}

/// Weights at or below this average are amplified, matching NewAPI: a set of
/// tiny weights would otherwise make the random draw effectively uniform only
/// by accident, and zero weights would break it entirely.
const SMOOTHING_AVERAGE_THRESHOLD: i64 = 10;
/// Effective weight given to every channel when all weights are zero.
const ZERO_WEIGHT_SMOOTHING: i64 = 100;

/// Distinct priorities among `channels`, highest first.
pub fn priority_tiers(channels: &[Channel]) -> Vec<i32> {
    let mut tiers: Vec<i32> = channels.iter().map(|c| c.priority).collect();
    tiers.sort_unstable_by(|a, b| b.cmp(a));
    tiers.dedup();
    tiers
}

/// Pick a channel from one priority tier using smoothing-weighted randomness.
///
/// `roll` is a value in `[0, total_weight)`; it is a parameter rather than an
/// internal call so the distribution is deterministic under test.
fn pick_within_tier(tier: &[Channel], roll: i64) -> Option<Channel> {
    if tier.is_empty() {
        return None;
    }
    if tier.len() == 1 {
        return tier.first().cloned();
    }

    let sum_weight: i64 = tier.iter().map(|c| c.weight.max(0) as i64).sum();

    let (unit_weight, adjustment) = if sum_weight == 0 {
        // All weights zero: give everyone an equal, non-zero weight.
        (ZERO_WEIGHT_SMOOTHING, ZERO_WEIGHT_SMOOTHING)
    } else if (sum_weight / tier.len() as i64) < SMOOTHING_AVERAGE_THRESHOLD {
        (100, 0)
    } else {
        (1, 0)
    };

    // Effective weight per channel = weight * unit_weight + adjustment.
    let total: i64 = tier
        .iter()
        .map(|c| c.weight.max(0) as i64 * unit_weight + adjustment)
        .sum();
    if total <= 0 {
        return tier.first().cloned();
    }
    let mut remaining = roll.rem_euclid(total);
    for channel in tier {
        let effective = channel.weight.max(0) as i64 * unit_weight + adjustment;
        remaining -= effective;
        if remaining < 0 {
            return Some(channel.clone());
        }
    }
    tier.last().cloned()
}

/// Select a channel for the given retry number.
///
/// `retry` indexes the priority tiers of `all`; it is clamped to the last tier
/// so a caller that retries past the available tiers keeps getting the
/// lowest-priority fallback rather than failing outright.
///
/// `exclude` holds channels already attempted *in this request*. Exclusions are
/// applied inside each tier, and a tier whose only members are excluded falls
/// through to the next tier instead of aborting the request. Deducting the
/// exclusions from the tier count instead would make a two-channel deployment
/// give up after one failure, which is exactly the failover case this exists
/// to serve.
pub fn select_tiered_excluding(
    all: &[Channel],
    exclude: &[String],
    retry: u32,
    roll: i64,
) -> Selection {
    if all.is_empty() {
        return Selection::NoCandidate;
    }
    let tiers = priority_tiers(all);
    if tiers.is_empty() {
        return Selection::NoCandidate;
    }
    let start = (retry as usize).min(tiers.len() - 1);
    for priority in tiers.into_iter().skip(start) {
        let tier: Vec<Channel> = all
            .iter()
            .filter(|c| c.priority == priority)
            .filter(|c| !exclude.iter().any(|id| id == &c.id))
            .cloned()
            .collect();
        if let Some(channel) = pick_within_tier(&tier, roll) {
            return Selection::Picked(channel);
        }
    }
    Selection::TiersExhausted
}

/// Select without exclusions. Kept for callers that have no attempt history.
pub fn select_tiered(candidates: &[Channel], retry: u32, roll: i64) -> Selection {
    select_tiered_excluding(candidates, &[], retry, roll)
}

/// True when no tier from `retry` onward still has an un-excluded channel.
pub fn tiers_exhausted_excluding(all: &[Channel], exclude: &[String], retry: u32) -> bool {
    if all.is_empty() {
        return true;
    }
    let tiers = priority_tiers(all);
    if tiers.is_empty() {
        return true;
    }
    let start = (retry as usize).min(tiers.len() - 1);
    tiers.into_iter().skip(start).all(|priority| {
        !all.iter().any(|c| {
            c.priority == priority && !exclude.iter().any(|id| id == &c.id)
        })
    })
}

/// True when `retry` has walked past every available tier.
pub fn tiers_exhausted(candidates: &[Channel], retry: u32) -> bool {
    let tiers = priority_tiers(candidates);
    !tiers.is_empty() && retry as usize >= tiers.len()
}

/// Rewrite a model name for a channel using that channel's model maps.
///
/// The first matching enabled map wins, so map order is significant and is
/// preserved from the database's ordering.
pub fn apply_model_map(maps: &[ModelMap], channel_id: &str, model: &str) -> String {
    for map in maps {
        if !map.enabled || map.channel_id != channel_id {
            continue;
        }
        if glob_match(&map.pattern, model) {
            return map.target_model.clone();
        }
    }
    model.to_string()
}

/// Glob matching supporting `*` anywhere, matching the existing project
/// convention (`model map uses simple glob patterns; no full regex`).
pub fn glob_match(pattern: &str, input: &str) -> bool {
    let pat = pattern.trim();
    if pat.is_empty() {
        return false;
    }
    if pat == "*" {
        return true;
    }
    // Split on '*' and require each fragment to appear in order.
    let fragments: Vec<&str> = pat.split('*').collect();
    let mut cursor = 0usize;
    for (index, fragment) in fragments.iter().enumerate() {
        if fragment.is_empty() {
            continue;
        }
        match input[cursor..].find(fragment) {
            Some(found) => {
                // A leading fragment must anchor at the start.
                if index == 0 && !pat.starts_with('*') && found != 0 {
                    return false;
                }
                cursor += found + fragment.len();
            }
            None => return false,
        }
    }
    // A trailing fragment must anchor at the end.
    if !pat.ends_with('*') {
        if let Some(last) = fragments.last() {
            if !last.is_empty() && !input.ends_with(last) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn channel(id: &str, priority: i32, weight: i32) -> Channel {
        Channel {
            id: id.to_string(),
            name: id.to_string(),
            provider: "openai".to_string(),
            base_url: "https://example.test".to_string(),
            api_key: "k".to_string(),
            priority,
            weight,
            enabled: true,
            test_model: "m".to_string(),
            group_name: "default".to_string(),
            tags: Vec::new(),
            model_list: Vec::new(),
            response_headers: serde_json::Value::Null,
            status_code_mapping: serde_json::Value::Null,
            override_parameters: serde_json::Value::Null,
            balance_micros: 0,
            last_test_at: None,
            info: Default::default(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn tiers_are_sorted_descending_and_unique() {
        let chans = vec![
            channel("a", 1, 1),
            channel("b", 10, 1),
            channel("c", 10, 1),
            channel("d", 5, 1),
        ];
        assert_eq!(priority_tiers(&chans), vec![10, 5, 1]);
    }

    #[test]
    fn retry_zero_takes_the_highest_priority_tier() {
        let chans = vec![channel("high", 10, 1), channel("low", 1, 1)];
        match select_tiered(&chans, 0, 0) {
            Selection::Picked(c) => assert_eq!(c.id, "high"),
            other => panic!("expected a pick, got {other:?}"),
        }
    }

    #[test]
    fn retry_walks_down_the_tiers() {
        let chans = vec![
            channel("primary", 10, 1),
            channel("secondary", 5, 1),
            channel("tertiary", 1, 1),
        ];
        let ids: Vec<String> = (0..3)
            .map(|retry| match select_tiered(&chans, retry, 0) {
                Selection::Picked(c) => c.id,
                other => panic!("retry {retry} failed: {other:?}"),
            })
            .collect();
        assert_eq!(ids, vec!["primary", "secondary", "tertiary"]);
    }

    #[test]
    fn retry_past_the_last_tier_clamps_to_the_fallback() {
        // A caller that retries more times than there are tiers must keep
        // getting the lowest-priority channel rather than failing.
        let chans = vec![channel("primary", 10, 1), channel("fallback", 1, 1)];
        match select_tiered(&chans, 99, 0) {
            Selection::Picked(c) => assert_eq!(c.id, "fallback"),
            other => panic!("expected the fallback, got {other:?}"),
        }
    }

    #[test]
    fn exclusion_falls_through_to_the_next_tier() {
        // The real failover shape: a dead primary must hand off to the backup
        // on the very next attempt, not abort the request.
        let chans = vec![channel("primary", 10, 1), channel("backup", 1, 1)];
        match select_tiered_excluding(&chans, &["primary".to_string()], 1, 0) {
            Selection::Picked(c) => assert_eq!(c.id, "backup"),
            other => panic!("expected the backup, got {other:?}"),
        }
    }

    #[test]
    fn a_tier_emptied_by_exclusion_falls_through() {
        // Both primaries tried, so the walk must continue to the next tier.
        let chans = vec![
            channel("p1", 10, 1),
            channel("p2", 10, 1),
            channel("backup", 1, 1),
        ];
        let exclude = vec!["p1".to_string(), "p2".to_string()];
        match select_tiered_excluding(&chans, &exclude, 0, 0) {
            Selection::Picked(c) => assert_eq!(c.id, "backup"),
            other => panic!("expected the backup, got {other:?}"),
        }
    }

    #[test]
    fn all_tiers_excluded_reports_exhaustion() {
        let chans = vec![channel("a", 10, 1), channel("b", 1, 1)];
        let exclude = vec!["a".to_string(), "b".to_string()];
        assert!(matches!(
            select_tiered_excluding(&chans, &exclude, 0, 0),
            Selection::TiersExhausted
        ));
        assert!(tiers_exhausted_excluding(&chans, &exclude, 0));
        // With nothing excluded the walk is not exhausted.
        assert!(!tiers_exhausted_excluding(&chans, &[], 0));
    }

    #[test]
    fn exhaustion_is_judged_on_the_full_candidate_set() {
        // Regression guard: judging exhaustion on the *filtered* set made a
        // two-channel deployment stop after one failure. The tier count must
        // come from every candidate, so retry 1 still has a tier to visit.
        let chans = vec![channel("primary", 10, 1), channel("backup", 1, 1)];
        let exclude = vec!["primary".to_string()];
        assert!(!tiers_exhausted_excluding(
            &chans,
            &exclude,
            1
        ));
    }
    #[test]
    fn empty_candidates_report_no_candidate() {
        assert!(matches!(
            select_tiered(&[], 0, 0),
            Selection::NoCandidate
        ));
    }

    #[test]
    fn single_channel_in_a_tier_is_always_picked() {
        let chans = vec![channel("only", 5, 1)];
        for roll in [0, 1, 999] {
            match select_tiered(&chans, 0, roll) {
                Selection::Picked(c) => assert_eq!(c.id, "only"),
                other => panic!("roll {roll}: {other:?}"),
            }
        }
    }

    #[test]
    fn weighting_is_respected_across_the_roll_range() {
        // Weights 9:1 average to 5, which is below the smoothing threshold, so
        // NewAPI amplifies every weight by 100. The roll therefore spans
        // [0, 1000) and the heavy channel owns the first 900.
        let chans = vec![channel("heavy", 1, 9), channel("light", 1, 1)];
        let mut heavy = 0;
        let mut light = 0;
        for roll in 0..1000 {
            match select_tiered(&chans, 0, roll) {
                Selection::Picked(c) if c.id == "heavy" => heavy += 1,
                Selection::Picked(_) => light += 1,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(heavy, 900, "9:1 weighting must hold after amplification");
        assert_eq!(light, 100);
    }

    #[test]
    fn large_weights_skip_the_smoothing_multiplier() {
        // Average weight 100 is at or above the threshold, so weights are used
        // as given and the roll spans [0, 200).
        let chans = vec![channel("heavy", 1, 100), channel("light", 1, 100)];
        let mut heavy = 0;
        let mut light = 0;
        for roll in 0..200 {
            match select_tiered(&chans, 0, roll) {
                Selection::Picked(c) if c.id == "heavy" => heavy += 1,
                Selection::Picked(_) => light += 1,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(heavy, 100);
        assert_eq!(light, 100);
    }

    #[test]
    fn zero_weights_spread_load_evenly() {
        // NewAPI's zero-weight rule: everyone gets an equal effective weight, so
        // a set of weight-0 channels still rotates instead of failing.
        let chans = vec![channel("a", 1, 0), channel("b", 1, 0), channel("c", 1, 0)];
        let mut seen = std::collections::BTreeSet::new();
        for roll in 0..300 {
            if let Selection::Picked(c) = select_tiered(&chans, 0, roll) {
                seen.insert(c.id);
            }
        }
        assert_eq!(seen.len(), 3, "all three channels must be reachable");
    }

    #[test]
    fn small_average_weights_are_amplified_not_collapsed() {
        // Average weight 2 (< 10) triggers the smoothing factor, so a weight-3
        // channel dominates a weight-1 channel rather than the draw degenerating.
        let chans = vec![channel("big", 1, 3), channel("small", 1, 1)];
        let mut picks = std::collections::BTreeMap::new();
        for roll in 0..400 {
            if let Selection::Picked(c) = select_tiered(&chans, 0, roll) {
                *picks.entry(c.id).or_insert(0) += 1;
            }
        }
        let big = picks.get("big").copied().unwrap_or(0);
        let small = picks.get("small").copied().unwrap_or(0);
        assert!(big > small, "big={big} small={small}");
        assert!(small > 0, "the lighter channel must remain reachable");
    }

    #[test]
    fn tiers_exhausted_reports_the_boundary() {
        let chans = vec![channel("a", 10, 1), channel("b", 1, 1)];
        assert!(!tiers_exhausted(&chans, 0));
        assert!(!tiers_exhausted(&chans, 1));
        assert!(tiers_exhausted(&chans, 2));
        assert!(!tiers_exhausted(&[], 0));
    }

    #[test]
    fn a_single_tier_is_reported_as_exhausted_after_one_retry() {
        let chans = vec![channel("a", 1, 1), channel("b", 1, 1)];
        assert!(!tiers_exhausted(&chans, 0));
        assert!(tiers_exhausted(&chans, 1));
    }

    #[test]
    fn model_map_rewrites_only_matching_channels() {
        let maps = vec![ModelMap {
            id: "m1".into(),
            channel_id: "ch-a".into(),
            pattern: "gpt-4*".into(),
            target_model: "gpt-4-turbo".into(),
            enabled: true,
            created_at: Utc::now(),
        }];
        assert_eq!(
            apply_model_map(&maps, "ch-a", "gpt-4o"),
            "gpt-4-turbo".to_string()
        );
        // A different channel is untouched.
        assert_eq!(apply_model_map(&maps, "ch-b", "gpt-4o"), "gpt-4o");
        // A non-matching model is untouched.
        assert_eq!(apply_model_map(&maps, "ch-a", "claude-3"), "claude-3");
    }

    #[test]
    fn disabled_model_maps_are_ignored() {
        let maps = vec![ModelMap {
            id: "m1".into(),
            channel_id: "ch-a".into(),
            pattern: "*".into(),
            target_model: "rewritten".into(),
            enabled: false,
            created_at: Utc::now(),
        }];
        assert_eq!(apply_model_map(&maps, "ch-a", "anything"), "anything");
    }

    #[test]
    fn glob_matching_handles_anchors_and_middles() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("gpt-4*", "gpt-4o"));
        assert!(!glob_match("gpt-4*", "claude-4"));
        assert!(glob_match("*turbo", "gpt-4-turbo"));
        assert!(glob_match("gpt-*o", "gpt-4o"));
        assert!(glob_match("*4*", "gpt-4o"));
        assert!(glob_match("gpt-4o", "gpt-4o"));
        assert!(!glob_match("gpt-4o", "gpt-4o-mini"));
        assert!(!glob_match("", "anything"));
        // A leading anchor must really anchor.
        assert!(!glob_match("4o*", "gpt-4o"));
        // A trailing anchor must really anchor.
        assert!(!glob_match("*4o", "gpt-4o-mini"));
    }
}
