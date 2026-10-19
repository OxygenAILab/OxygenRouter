//! Relay performance by model, group and hour.
//!
//! Mirrors the reference's `pkg/perf_metrics`: a per-model series of hourly
//! buckets, a request-weighted summary, and a per-model overview. The counters
//! themselves are folded in memory (`state.perf`) and written by the same timer
//! that flushes the usage summary, so a busy gateway pays no transaction per
//! request for a table only charts read.
//!
//! Two things are deliberately absent, and named here rather than left to be
//! discovered from a zero:
//!
//! * **Time to first token.** This instance buffers a streamed reply before
//!   translating it, so there is no first-token timestamp to record. The
//!   reference reports `avg_ttft_ms`; this payload omits the field instead of
//!   reporting a measured zero.
//! * **A cap on the window.** The reference clamps `hours` to 24x30; this does
//!   the same, because a reader asking for ten years would scan the table.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::BTreeMap;

use serde::Serialize;

use crate::state::AppState;

/// Longest window a reader may ask for, matching the reference's clamp.
const MAX_HOURS: i64 = 24 * 30;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Counters {
    requests: i64,
    successes: i64,
    total_latency_ms: i64,
    output_tokens: i64,
    generation_ms: i64,
}

impl Counters {
    fn add(&mut self, other: Counters) {
        self.requests += other.requests;
        self.successes += other.successes;
        self.total_latency_ms += other.total_latency_ms;
        self.output_tokens += other.output_tokens;
        self.generation_ms += other.generation_ms;
    }

    fn merge(&mut self, row: &oxygenrouter_core::PerfMetric) {
        self.add(Counters {
            requests: row.request_count,
            successes: row.success_count,
            total_latency_ms: row.total_latency_ms,
            output_tokens: row.output_tokens,
            generation_ms: row.generation_ms,
        });
    }

    fn avg_latency_ms(&self) -> i64 {
        if self.requests <= 0 {
            return 0;
        }
        self.total_latency_ms / self.requests
    }

    fn success_rate(&self) -> f64 {
        if self.requests <= 0 {
            return 0.0;
        }
        round2(self.successes as f64 / self.requests as f64 * 100.0)
    }

    /// Output tokens per second over the time spent producing them.
    fn avg_tps(&self) -> f64 {
        if self.generation_ms <= 0 {
            return 0.0;
        }
        round2(self.output_tokens as f64 / (self.generation_ms as f64 / 1000.0))
    }
}

/// Two decimal places, the precision the reference reports.
fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// One hour, as a client renders it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BucketPoint {
    pub ts: i64,
    pub avg_latency_ms: i64,
    pub success_rate: f64,
    pub avg_tps: f64,
    pub request_count: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Summary {
    pub avg_latency_ms: i64,
    pub success_rate: f64,
    pub avg_tps: f64,
    pub request_count: i64,
}

fn summarize(total: &Counters) -> Summary {
    Summary {
        avg_latency_ms: total.avg_latency_ms(),
        success_rate: total.success_rate(),
        avg_tps: total.avg_tps(),
        request_count: total.requests,
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GroupSeries {
    pub group: String,
    #[serde(flatten)]
    pub summary: Summary,
    pub series: Vec<BucketPoint>,
}

/// One model over the window: its buckets, its summary, and the same figures per
/// group, because a slow group is the usual reason a model looks slow.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelQuery {
    pub model_name: String,
    pub window_start: i64,
    pub window_end: i64,
    pub summary: Summary,
    pub series: Vec<BucketPoint>,
    pub groups: Vec<GroupSeries>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SuccessRatePoint {
    pub ts: i64,
    pub success_rate: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelSummary {
    pub model_name: String,
    #[serde(flatten)]
    pub summary: Summary,
    pub recent_success_series: Vec<SuccessRatePoint>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SummaryAll {
    pub window_start: i64,
    pub window_end: i64,
    pub summary: Summary,
    pub models: Vec<ModelSummary>,
}

/// The window a reader gets: the current partial hour plus the preceding
/// complete ones, so every caller shares the same bucket boundaries.
fn query_window(now: i64, hours: i64) -> (i64, i64) {
    let hours = hours.clamp(1, MAX_HOURS);
    let end = now;
    let start = end - end.rem_euclid(3600) - (hours - 1) * 3600;
    (start, end)
}

fn points_from(buckets: &BTreeMap<i64, Counters>) -> Vec<BucketPoint> {
    buckets
        .iter()
        .filter(|(_, counters)| counters.requests > 0)
        .map(|(ts, counters)| BucketPoint {
            ts: *ts,
            avg_latency_ms: counters.avg_latency_ms(),
            success_rate: counters.success_rate(),
            avg_tps: counters.avg_tps(),
            request_count: counters.requests,
        })
        .collect()
}

/// One model's series over a window.
pub fn query(
    state: &AppState,
    model: &str,
    group: Option<&str>,
    hours: i64,
) -> Result<ModelQuery, String> {
    let (start, end) = query_window(chrono::Utc::now().timestamp(), hours);
    let rows = state
        .db
        .list_perf_metrics(model, group, start, end)
        .map_err(|error| error.to_string())?;

    let mut overall: BTreeMap<i64, Counters> = BTreeMap::new();
    let mut by_group: BTreeMap<String, BTreeMap<i64, Counters>> = BTreeMap::new();
    for row in &rows {
        overall.entry(row.bucket_ts).or_default().merge(row);
        by_group
            .entry(row.group_name.clone())
            .or_default()
            .entry(row.bucket_ts)
            .or_default()
            .merge(row);
    }

    let mut total = Counters::default();
    for counters in overall.values() {
        total.add(*counters);
    }
    let groups = by_group
        .into_iter()
        .map(|(name, buckets)| {
            let mut group_total = Counters::default();
            for counters in buckets.values() {
                group_total.add(*counters);
            }
            GroupSeries {
                group: name,
                summary: summarize(&group_total),
                series: points_from(&buckets),
            }
        })
        .collect();

    Ok(ModelQuery {
        model_name: model.to_string(),
        window_start: start,
        window_end: end,
        summary: summarize(&total),
        series: points_from(&overall),
        groups,
    })
}

/// Every model's figures over a window, busiest first.
pub fn summary_all(state: &AppState, hours: i64) -> Result<SummaryAll, String> {
    let (start, end) = query_window(chrono::Utc::now().timestamp(), hours);
    let rows = state
        .db
        .list_perf_metric_buckets(start, end)
        .map_err(|error| error.to_string())?;

    let mut per_model: BTreeMap<String, (BTreeMap<i64, Counters>, Counters)> = BTreeMap::new();
    let mut overall = Counters::default();
    for row in &rows {
        let entry = per_model.entry(row.model_name.clone()).or_default();
        entry.0.entry(row.bucket_ts).or_default().merge(row);
        entry.1.merge(row);
        overall.merge(row);
    }

    let mut models: Vec<ModelSummary> = per_model
        .into_iter()
        .filter(|(_, (_, total))| total.requests > 0)
        .map(|(name, (buckets, total))| ModelSummary {
            model_name: name,
            summary: summarize(&total),
            recent_success_series: buckets
                .iter()
                .filter(|(_, counters)| counters.requests > 0)
                .map(|(ts, counters)| SuccessRatePoint {
                    ts: *ts,
                    success_rate: counters.success_rate(),
                })
                .collect(),
        })
        .collect();
    models.sort_by(|a, b| {
        b.summary
            .request_count
            .cmp(&a.summary.request_count)
            .then_with(|| a.model_name.cmp(&b.model_name))
    });

    Ok(SummaryAll {
        window_start: start,
        window_end: end,
        summary: summarize(&overall),
        models,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_is_whole_hours_and_clamped() {
        let now = 1_700_000_000; // not on an hour boundary
        let (start, end) = query_window(now, 24);
        assert_eq!(end, now);
        assert_eq!(start % 3600, 0, "the window starts on an hour boundary");
        assert_eq!(end - start, 23 * 3600 + (end % 3600));

        // A reader asking for a decade gets the reference's cap, not a scan.
        let (clamped, _) = query_window(now, 24 * 365 * 10);
        assert_eq!(clamped, now - now % 3600 - (MAX_HOURS - 1) * 3600);
    }

    #[test]
    fn averages_are_request_weighted() {
        let mut counters = Counters::default();
        counters.add(Counters {
            requests: 3,
            successes: 2,
            total_latency_ms: 300,
            output_tokens: 90,
            generation_ms: 3000,
        });
        assert_eq!(counters.avg_latency_ms(), 100);
        assert_eq!(counters.success_rate(), 66.67);
        assert_eq!(counters.avg_tps(), 30.0);
    }

    #[test]
    fn an_empty_bucket_reads_as_zero_not_a_division_panic() {
        let counters = Counters::default();
        assert_eq!(counters.avg_latency_ms(), 0);
        assert_eq!(counters.success_rate(), 0.0);
        assert_eq!(counters.avg_tps(), 0.0);
    }
}
