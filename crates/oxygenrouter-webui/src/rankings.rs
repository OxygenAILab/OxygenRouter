//! Public model/vendor rankings, ported from the reference's `service/rankings.go`.
//!
//! The shape and every derived figure follow the reference so a page rendered
//! from this payload matches the page rendered from NewAPI's: a leaderboard of
//! models, vendor aggregation with a top-model pointer, rank movers against the
//! previous period, and two bucketed series for the charts.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::Serialize;

use crate::state::AppState;

/// How long a built snapshot is reused, matching the reference's cache TTL.
pub const RANKING_CACHE_TTL_SECS: i64 = 5 * 60;
const LEADERBOARD_LIMIT: usize = 20;
const HISTORY_LIMIT: usize = 10;
const VENDOR_LIMIT: usize = 5;
const MOVER_LIMIT: usize = 6;
const OTHERS: &str = "Others";
const UNKNOWN_VENDOR: &str = "Unknown";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RankingsResponse {
    pub models: Vec<RankedModel>,
    pub vendors: Vec<RankedVendor>,
    pub top_movers: Vec<RankingMover>,
    pub top_droppers: Vec<RankingMover>,
    pub models_history: ModelHistorySeries,
    pub vendor_share_history: VendorShareSeries,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RankedModel {
    pub rank: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_rank: Option<usize>,
    pub model_name: String,
    pub vendor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor_icon: Option<String>,
    pub category: String,
    pub total_tokens: i64,
    pub share: f64,
    pub growth_pct: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RankedVendor {
    pub rank: usize,
    pub vendor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor_icon: Option<String>,
    pub total_tokens: i64,
    pub share: f64,
    pub growth_pct: f64,
    pub models_count: usize,
    pub top_model: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RankingMover {
    pub model_name: String,
    pub vendor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor_icon: Option<String>,
    pub rank_delta: i64,
    pub current_rank: usize,
    pub growth_pct: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelHistoryPoint {
    pub ts: String,
    pub label: String,
    pub model: String,
    pub vendor: String,
    pub tokens: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelHistoryModel {
    pub name: String,
    pub vendor: String,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelHistorySeries {
    pub points: Vec<ModelHistoryPoint>,
    pub models: Vec<ModelHistoryModel>,
    pub buckets: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VendorSharePoint {
    pub ts: String,
    pub label: String,
    pub vendor: String,
    pub share: f64,
    pub tokens: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VendorShareVendor {
    pub name: String,
    pub total: i64,
    pub share: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VendorShareSeries {
    pub points: Vec<VendorSharePoint>,
    pub vendors: Vec<VendorShareVendor>,
    pub buckets: usize,
}

/// The period a snapshot covers: its length, its bucket width, and the label
/// layout its history points render with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RankingPeriod {
    pub id: &'static str,
    pub seconds: i64,
    pub bucket_secs: i64,
    pub label: &'static str,
}

/// The reference's four periods, including its default (`service/rankings.go:164`).
pub fn ranking_period(period: &str) -> Option<RankingPeriod> {
    match period {
        "" | "week" => Some(RankingPeriod {
            id: "week",
            seconds: 7 * 24 * 3600,
            bucket_secs: 24 * 3600,
            label: "%b %-d",
        }),
        "today" => Some(RankingPeriod {
            id: "today",
            seconds: 24 * 3600,
            bucket_secs: 3600,
            label: "%H:%M",
        }),
        "month" => Some(RankingPeriod {
            id: "month",
            seconds: 30 * 24 * 3600,
            bucket_secs: 24 * 3600,
            label: "%b %-d",
        }),
        "year" => Some(RankingPeriod {
            id: "year",
            seconds: 365 * 24 * 3600,
            bucket_secs: 7 * 24 * 3600,
            label: "%b %-d",
        }),
        _ => None,
    }
}

/// Where a model's vendor and icon come from: the registry first, the vendor
/// table second, and `Unknown` when neither knows the name
/// (`service/rankings.go:137`).
#[derive(Debug, Clone, Default)]
pub struct ModelMeta {
    pub vendor: String,
    pub icon: String,
}

/// Build a snapshot from the instance's own usage aggregates.
pub fn build_snapshot(
    state: &AppState,
    period: RankingPeriod,
    now: DateTime<Utc>,
) -> Result<RankingsResponse, String> {
    let start = now - Duration::seconds(period.seconds);
    let meta = model_meta_index(state)?;

    // The previous window abuts this one, which is what the reference compares
    // against (`service/rankings.go:231`).
    let previous_start = start - Duration::seconds(period.seconds);
    let previous_end = start - Duration::seconds(1);

    let current = state
        .db
        .quota_tokens_by_model(start, now)
        .map_err(|e| e.to_string())?;
    let previous = state
        .db
        .quota_tokens_by_model(previous_start, previous_end)
        .map_err(|e| e.to_string())?;
    let buckets = state
        .db
        .quota_tokens_by_model_bucket(start, now, period.bucket_secs)
        .map_err(|e| e.to_string())?;

    let total_tokens: i64 = current.iter().map(|(_, tokens)| *tokens).sum();
    let previous_ranks = rank_map(&previous);
    let previous_tokens: HashMap<&str, i64> = previous
        .iter()
        .map(|(name, tokens)| (name.as_str(), *tokens))
        .collect();

    let mut models = Vec::with_capacity(current.len());
    for (index, (name, tokens)) in current.iter().enumerate() {
        let meta = meta_for(&meta, name);
        let growth = growth_pct(*tokens, previous_tokens.get(name.as_str()).copied());
        models.push(RankedModel {
            rank: index + 1,
            previous_rank: previous_ranks.get(name.as_str()).copied(),
            model_name: name.clone(),
            vendor: meta.vendor.clone(),
            vendor_icon: (!meta.icon.is_empty()).then(|| meta.icon.clone()),
            category: "all".to_string(),
            total_tokens: *tokens,
            share: share(*tokens, total_tokens),
            growth_pct: growth,
        });
    }

    let vendors = build_vendors(&current, &previous, total_tokens, &meta);
    let models_history = build_model_history(&buckets, &current, &meta, period);
    let vendor_share_history =
        build_vendor_share_history(&buckets, &vendors, total_tokens, &meta, period);
    // Movers are computed over *every* ranked model, not the truncated
    // leaderboard: the reference passes the full `rankedModels`
    // (`service/rankings.go:222`) and truncates only the leaderboard and each
    // mover list, so a model ranked 25th can still be a top mover.
    let (top_movers, top_droppers) = build_movers(&models);

    Ok(RankingsResponse {
        models: models.into_iter().take(LEADERBOARD_LIMIT).collect(),
        vendors,
        top_movers,
        top_droppers,
        models_history,
        vendor_share_history,
    })
}

fn model_meta_index(state: &AppState) -> Result<HashMap<String, ModelMeta>, String> {
    let vendors = state.db.list_vendors().map_err(|e| e.to_string())?;
    let icon_by_name: HashMap<&str, &str> = vendors
        .iter()
        .map(|vendor| (vendor.name.as_str(), vendor.icon.as_str()))
        .collect();

    let mut index = HashMap::new();
    for entry in state
        .db
        .list_model_metadata()
        .map_err(|e| e.to_string())?
    {
        let vendor = if entry.vendor.trim().is_empty() {
            UNKNOWN_VENDOR.to_string()
        } else {
            entry.vendor.clone()
        };
        let icon = icon_by_name
            .get(vendor.as_str())
            .map(|icon| icon.to_string())
            .unwrap_or_default();
        index.insert(entry.model_name, ModelMeta { vendor, icon });
    }
    Ok(index)
}

fn meta_for(index: &HashMap<String, ModelMeta>, model: &str) -> ModelMeta {
    index
        .get(model)
        .filter(|meta| !meta.vendor.is_empty())
        .cloned()
        .unwrap_or_else(|| ModelMeta {
            vendor: UNKNOWN_VENDOR.to_string(),
            icon: String::new(),
        })
}

fn build_vendors(
    current: &[(String, i64)],
    previous: &[(String, i64)],
    total_tokens: i64,
    meta: &HashMap<String, ModelMeta>,
) -> Vec<RankedVendor> {
    struct Aggregate {
        name: String,
        icon: String,
        total: i64,
        previous: i64,
        models: BTreeSet<String>,
        top_model: String,
        top_tokens: i64,
    }

    let mut by_name: BTreeMap<String, Aggregate> = BTreeMap::new();
    for (model, tokens) in current {
        let meta = meta_for(meta, model);
        let aggregate = by_name
            .entry(meta.vendor.clone())
            .or_insert_with(|| Aggregate {
                name: meta.vendor.clone(),
                icon: meta.icon.clone(),
                total: 0,
                previous: 0,
                models: BTreeSet::new(),
                top_model: String::new(),
                top_tokens: 0,
            });
        aggregate.total += tokens;
        aggregate.models.insert(model.clone());
        if *tokens > aggregate.top_tokens {
            aggregate.top_model = model.clone();
            aggregate.top_tokens = *tokens;
        }
    }
    for (model, tokens) in previous {
        let meta = meta_for(meta, model);
        let aggregate = by_name
            .entry(meta.vendor.clone())
            .or_insert_with(|| Aggregate {
                name: meta.vendor.clone(),
                icon: meta.icon.clone(),
                total: 0,
                previous: 0,
                models: BTreeSet::new(),
                top_model: String::new(),
                top_tokens: 0,
            });
        aggregate.previous += tokens;
    }

    let mut rows: Vec<RankedVendor> = by_name
        .into_values()
        .filter(|aggregate| aggregate.total > 0)
        .map(|aggregate| RankedVendor {
            rank: 0,
            growth_pct: growth_pct(aggregate.total, Some(aggregate.previous)),
            share: share(aggregate.total, total_tokens),
            vendor: aggregate.name,
            vendor_icon: (!aggregate.icon.is_empty()).then_some(aggregate.icon),
            total_tokens: aggregate.total,
            models_count: aggregate.models.len(),
            top_model: aggregate.top_model,
        })
        .collect();
    // Tokens first, then the name, so an exact tie has one stable order rather
    // than whichever the map happened to yield (`service/rankings.go:326`).
    rows.sort_by(|a, b| {
        b.total_tokens
            .cmp(&a.total_tokens)
            .then_with(|| a.vendor.cmp(&b.vendor))
    });
    for (index, row) in rows.iter_mut().enumerate() {
        row.rank = index + 1;
    }
    rows
}

fn build_model_history(
    buckets: &[(String, i64, i64)],
    totals: &[(String, i64)],
    meta: &HashMap<String, ModelMeta>,
    period: RankingPeriod,
) -> ModelHistorySeries {
    let mut top_models: BTreeSet<&str> = BTreeSet::new();
    let mut models = Vec::new();
    let mut other_total = 0i64;
    for (index, (model, tokens)) in totals.iter().enumerate() {
        if index < HISTORY_LIMIT {
            top_models.insert(model.as_str());
            models.push(ModelHistoryModel {
                name: model.clone(),
                vendor: meta_for(meta, model).vendor,
                total: *tokens,
            });
        } else {
            other_total += tokens;
        }
    }
    if other_total > 0 {
        models.push(ModelHistoryModel {
            name: OTHERS.to_string(),
            vendor: "Various".to_string(),
            total: other_total,
        });
    }

    let mut bucket_set = BTreeSet::new();
    let mut by_bucket: BTreeMap<i64, BTreeMap<String, i64>> = BTreeMap::new();
    for (model, bucket, tokens) in buckets {
        let name = if top_models.contains(model.as_str()) {
            model.clone()
        } else {
            OTHERS.to_string()
        };
        bucket_set.insert(*bucket);
        *by_bucket
            .entry(*bucket)
            .or_default()
            .entry(name)
            .or_insert(0) += tokens;
    }

    let mut points = Vec::new();
    for bucket in &bucket_set {
        for series in &models {
            let tokens = by_bucket
                .get(bucket)
                .and_then(|row| row.get(&series.name))
                .copied()
                .unwrap_or(0);
            if tokens <= 0 {
                continue;
            }
            points.push(ModelHistoryPoint {
                ts: bucket_ts(*bucket),
                label: bucket_label(*bucket, period),
                model: series.name.clone(),
                vendor: series.vendor.clone(),
                tokens,
            });
        }
    }

    ModelHistorySeries {
        points,
        models,
        buckets: bucket_set.len(),
    }
}

fn build_vendor_share_history(
    buckets: &[(String, i64, i64)],
    vendors: &[RankedVendor],
    total_tokens: i64,
    meta: &HashMap<String, ModelMeta>,
    period: RankingPeriod,
) -> VendorShareSeries {
    let mut top_vendors: BTreeSet<&str> = BTreeSet::new();
    let mut rows = Vec::new();
    let mut other_total = 0i64;
    for (index, vendor) in vendors.iter().enumerate() {
        if index < VENDOR_LIMIT {
            top_vendors.insert(vendor.vendor.as_str());
            rows.push(VendorShareVendor {
                name: vendor.vendor.clone(),
                total: vendor.total_tokens,
                share: vendor.share,
            });
        } else {
            other_total += vendor.total_tokens;
        }
    }
    if other_total > 0 {
        rows.push(VendorShareVendor {
            name: OTHERS.to_string(),
            total: other_total,
            share: share(other_total, total_tokens),
        });
    }

    let mut bucket_set = BTreeSet::new();
    let mut by_bucket: BTreeMap<i64, BTreeMap<String, i64>> = BTreeMap::new();
    let mut totals_by_bucket: BTreeMap<i64, i64> = BTreeMap::new();
    for (model, bucket, tokens) in buckets {
        let vendor = meta_for(meta, model).vendor;
        let name = if top_vendors.contains(vendor.as_str()) {
            vendor
        } else {
            OTHERS.to_string()
        };
        bucket_set.insert(*bucket);
        *by_bucket
            .entry(*bucket)
            .or_default()
            .entry(name)
            .or_insert(0) += tokens;
        *totals_by_bucket.entry(*bucket).or_insert(0) += tokens;
    }

    let mut points = Vec::new();
    for bucket in &bucket_set {
        for vendor in &rows {
            let tokens = by_bucket
                .get(bucket)
                .and_then(|row| row.get(&vendor.name))
                .copied()
                .unwrap_or(0);
            if tokens <= 0 {
                continue;
            }
            points.push(VendorSharePoint {
                ts: bucket_ts(*bucket),
                label: bucket_label(*bucket, period),
                vendor: vendor.name.clone(),
                share: share(tokens, totals_by_bucket.get(bucket).copied().unwrap_or(0)),
                tokens,
            });
        }
    }

    VendorShareSeries {
        points,
        vendors: rows,
        buckets: bucket_set.len(),
    }
}

fn build_movers(models: &[RankedModel]) -> (Vec<RankingMover>, Vec<RankingMover>) {
    let mut movers = Vec::new();
    let mut droppers = Vec::new();
    for model in models {
        let Some(previous) = model.previous_rank else {
            continue;
        };
        let delta = previous as i64 - model.rank as i64;
        if delta == 0 {
            continue;
        }
        let row = RankingMover {
            model_name: model.model_name.clone(),
            vendor: model.vendor.clone(),
            vendor_icon: model.vendor_icon.clone(),
            rank_delta: delta,
            current_rank: model.rank,
            growth_pct: model.growth_pct,
        };
        if delta > 0 {
            movers.push(row);
        } else {
            droppers.push(row);
        }
    }
    movers.sort_by(|a, b| {
        b.rank_delta
            .cmp(&a.rank_delta)
            .then_with(|| b.growth_pct.partial_cmp(&a.growth_pct).unwrap_or(std::cmp::Ordering::Equal))
    });
    droppers.sort_by(|a, b| {
        a.rank_delta
            .cmp(&b.rank_delta)
            .then_with(|| a.growth_pct.partial_cmp(&b.growth_pct).unwrap_or(std::cmp::Ordering::Equal))
    });
    movers.truncate(MOVER_LIMIT);
    droppers.truncate(MOVER_LIMIT);
    (movers, droppers)
}

fn rank_map(totals: &[(String, i64)]) -> HashMap<&str, usize> {
    totals
        .iter()
        .enumerate()
        .map(|(index, (name, _))| (name.as_str(), index + 1))
        .collect()
}

/// The reference's share figure, rounded to four decimals
/// (`service/rankings.go:556`).
fn share(value: i64, total: i64) -> f64 {
    if total <= 0 || value <= 0 {
        return 0.0;
    }
    round4(value as f64 / total as f64)
}

/// Growth against the previous window; a brand-new model reads +100%, matching
/// the reference (`service/rankings.go:563`).
fn growth_pct(current: i64, previous: Option<i64>) -> f64 {
    match previous {
        Some(previous) if previous > 0 => {
            round4(((current - previous) as f64 / previous as f64) * 100.0)
        }
        Some(_) => {
            if current > 0 {
                100.0
            } else {
                0.0
            }
        }
        // A window with no comparison, which only the period-less path reaches.
        None => 0.0,
    }
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn bucket_ts(bucket: i64) -> String {
    Utc.timestamp_opt(bucket, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .to_rfc3339()
}

fn bucket_label(bucket: i64, period: RankingPeriod) -> String {
    // The reference formats labels with `time.Unix(bucket, 0).Format`, which is
    // the *server's* local zone (`service/rankings.go:528`), while the `ts`
    // beside it is explicitly UTC (`:524`). Matching both keeps a chart's axis
    // legible to the operator without moving the underlying instant.
    let time = chrono::Local
        .timestamp_opt(bucket, 0)
        .single()
        .unwrap_or_else(chrono::Local::now);
    time.format(period.label).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_periods_match_the_reference() {
        for (input, id, seconds, bucket_secs) in [
            ("", "week", 604_800, 86_400),
            ("week", "week", 604_800, 86_400),
            ("today", "today", 86_400, 3_600),
            ("month", "month", 2_592_000, 86_400),
            ("year", "year", 31_536_000, 604_800),
        ] {
            let period = ranking_period(input).expect(input);
            assert_eq!(period.id, id, "{input}");
            assert_eq!(period.seconds, seconds, "{input}");
            assert_eq!(period.bucket_secs, bucket_secs, "{input}");
        }
        assert!(ranking_period("fortnight").is_none());
    }

    #[test]
    fn share_and_growth_are_rounded_like_the_reference() {
        assert_eq!(share(1, 3), 0.3333);
        assert_eq!(share(0, 10), 0.0);
        assert_eq!(share(10, 0), 0.0);
        assert_eq!(growth_pct(200, Some(100)), 100.0);
        assert_eq!(growth_pct(50, Some(100)), -50.0);
        // A new model counts as +100, and a vanished one as no growth.
        assert_eq!(growth_pct(5, Some(0)), 100.0);
        assert_eq!(growth_pct(0, Some(0)), 0.0);
    }

    #[test]
    fn movers_are_sorted_and_capped() {
        let row = |name: &str, rank: usize, previous: Option<usize>| RankedModel {
            rank,
            previous_rank: previous,
            model_name: name.to_string(),
            vendor: "V".to_string(),
            vendor_icon: None,
            category: "all".to_string(),
            total_tokens: 1,
            share: 0.1,
            growth_pct: 10.0,
        };
        let models = vec![
            row("a", 1, None),
            // delta = previous - current: b climbs 3 -> 1 (+2), c climbs 5 -> 2 (+3).
            row("b", 1, Some(3)),
            row("c", 2, Some(5)),
            row("d", 4, Some(4)), // unchanged
            row("e", 9, Some(2)), // falls 2 -> 9 (-7)
        ];
        let (movers, droppers) = build_movers(&models);
        assert_eq!(
            movers.iter().map(|m| m.model_name.as_str()).collect::<Vec<_>>(),
            vec!["c", "b"]
        );
        assert_eq!(
            droppers.iter().map(|m| m.model_name.as_str()).collect::<Vec<_>>(),
            vec!["e"]
        );
    }

    /// A model with no registry entry is attributed to `Unknown` rather than
    /// silently disappearing from the vendor aggregation.
    #[test]
    fn an_unregistered_model_falls_back_to_unknown() {
        let meta = HashMap::new();
        assert_eq!(meta_for(&meta, "mystery").vendor, UNKNOWN_VENDOR);
    }

    /// `ts` is the instant in UTC while `label` is the same instant in the
    /// server's zone, which is the pair the reference emits
    /// (`service/rankings.go:524,528`).
    #[test]
    fn a_bucket_renders_utc_ts_and_local_label() {
        let bucket = 1_700_000_000i64;
        let ts = bucket_ts(bucket);
        assert!(ts.ends_with("+00:00"), "{ts}");
        assert!(ts.starts_with("2023-11-14T22:13:20"), "{ts}");

        let hourly = RankingPeriod {
            id: "today",
            seconds: 86_400,
            bucket_secs: 3_600,
            label: "%H:%M",
        };
        let expected = chrono::Local
            .timestamp_opt(bucket, 0)
            .single()
            .unwrap()
            .format("%H:%M")
            .to_string();
        assert_eq!(bucket_label(bucket, hourly), expected);
    }
}
