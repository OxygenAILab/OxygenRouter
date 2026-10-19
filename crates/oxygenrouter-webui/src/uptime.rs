//! Uptime-Kuma status board.
//!
//! The console's overview can show the health of the services the gateway
//! depends on, read from one or more Uptime Kuma status pages. This mirrors the
//! reference's `GET /api/uptime/status` (`controller/uptime_kuma.go`): for each
//! configured group the instance reads the public status-page API and flattens
//! the monitors into `{name, uptime, status, group}` rows.
//!
//! Two behaviours are deliberate rather than incidental:
//!
//! * A group that fails is reported with an empty monitor list instead of
//!   failing the request, which is what the reference does -- one unreachable
//!   status page must not blank the whole panel.
//! * The route is public (the reference serves it above `UserAuth`) but the
//!   fan-out only runs when an operator has switched the panel on, and the
//!   result is cached for [`CACHE_TTL`]. Without those two guards an anonymous
//!   caller could turn one cheap request into twenty outbound ones, which is an
//!   amplification path the reference leaves open. With the panel off there is
//!   nothing to render, so the refusal costs a reader nothing.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use crate::state::AppState;
use oxygenrouter_core::ApiResponse;

/// Longest a single Uptime Kuma instance is waited on, matching the reference's
/// per-request client timeout.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest the whole fan-out waits across every group.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
/// Most groups one settings blob may declare. A typo must not turn one console
/// page load into an unbounded scan of the network.
const MAX_GROUPS: usize = 20;
/// How long a fan-out result is reused. The overview polls, and the status pages
/// themselves refresh on the order of a minute, so this trades nothing visible
/// for a bounded outbound request rate.
pub const CACHE_TTL: Duration = Duration::from_secs(10);
/// The status-page API appends this to a monitor id in `uptimeList`.
const UPTIME_KEY_SUFFIX: &str = "_24";

/// One configured status page.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct UptimeGroup {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub slug: String,
    /// Rendered as the heading above the group's monitors. The reference stores
    /// it as `categoryName`, which is also what its settings form submits.
    #[serde(default, rename = "categoryName", alias = "category_name")]
    pub category_name: String,
}

/// One monitor row, shaped exactly as the reference emits it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Monitor {
    pub name: String,
    pub uptime: f64,
    pub status: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// A group's monitors. `category_name` is echoed even when the group could not
/// be reached, so the panel can show the heading it was configured with.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UptimeGroupResult {
    #[serde(rename = "categoryName")]
    pub category_name: String,
    pub monitors: Vec<Monitor>,
}

#[derive(Debug, Default, Deserialize)]
struct StatusPage {
    #[serde(default, rename = "publicGroupList")]
    public_group_list: Vec<PublicGroup>,
}

#[derive(Debug, Default, Deserialize)]
struct PublicGroup {
    #[serde(default)]
    name: String,
    #[serde(default, rename = "monitorList")]
    monitor_list: Vec<PublicMonitor>,
}

#[derive(Debug, Default, Deserialize)]
struct PublicMonitor {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    name: String,
}

#[derive(Debug, Default, Deserialize)]
struct HeartbeatPage {
    #[serde(default, rename = "heartbeatList")]
    heartbeat_list: HashMap<String, Vec<Heartbeat>>,
    #[serde(default, rename = "uptimeList")]
    uptime_list: HashMap<String, f64>,
}

#[derive(Debug, Default, Deserialize)]
struct Heartbeat {
    #[serde(default)]
    status: i64,
}

/// Parse the stored `UptimeKumaGroups` option.
///
/// A malformed blob yields no groups rather than an error: the panel is a
/// read-only convenience, and refusing the request would only hide the rest of
/// the overview. Entries that could not be fetched are dropped here, where the
/// reason is visible, instead of producing a request to `.../api/status-page/`.
pub fn parse_groups(raw: &str) -> Vec<UptimeGroup> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Vec::new();
    }
    let parsed: Vec<UptimeGroup> = serde_json::from_str(raw).unwrap_or_default();
    parsed
        .into_iter()
        .filter(|group| group_is_fetchable(group))
        .take(MAX_GROUPS)
        .collect()
}

/// A group is only usable when its base URL is http(s) and its slug is a single
/// path segment. The slug is appended to the request path, so accepting `../`
/// or a query string would let a typo address a different route on the same
/// host than the operator meant.
fn group_is_fetchable(group: &UptimeGroup) -> bool {
    let url = group.url.trim();
    let slug = group.slug.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return false;
    }
    if slug.is_empty() || slug.contains('/') || slug.contains('?') || slug.contains('#') {
        return false;
    }
    !url.contains(char::is_whitespace)
}

/// Flatten one status page with its heartbeat data.
///
/// The reference reads the first heartbeat to decide current state, and the
/// `<id>_24` entry of `uptimeList` for the 24-hour availability figure. A
/// monitor with neither is reported as `status: 0, uptime: 0` rather than being
/// dropped, so an operator sees the monitor exists but has no data.
fn assemble(status: &StatusPage, heartbeat: &HeartbeatPage) -> Vec<Monitor> {
    let mut monitors = Vec::new();
    for page in &status.public_group_list {
        if page.monitor_list.is_empty() {
            continue;
        }
        for monitor in &page.monitor_list {
            let id = monitor.id.to_string();
            let uptime = heartbeat
                .uptime_list
                .get(&format!("{id}{UPTIME_KEY_SUFFIX}"))
                .copied()
                .unwrap_or(0.0);
            let status = heartbeat
                .heartbeat_list
                .get(&id)
                .and_then(|beats| beats.first())
                .map(|beat| beat.status)
                .unwrap_or(0);
            monitors.push(Monitor {
                name: monitor.name.clone(),
                uptime,
                status,
                group: Some(page.name.clone()).filter(|name| !name.is_empty()),
            });
        }
    }
    monitors
}

/// Fetch one group's two status-page endpoints.
///
/// Both are read concurrently, and a failure of either leaves the monitor list
/// empty: a page without heartbeats would otherwise render every monitor as
/// down, which is worse than showing nothing.
async fn fetch_group(client: &reqwest::Client, group: &UptimeGroup) -> UptimeGroupResult {
    let mut result = UptimeGroupResult {
        category_name: group.category_name.clone(),
        monitors: Vec::new(),
    };
    let base = group.url.trim().trim_end_matches('/');
    let slug = group.slug.trim();
    let status_url = format!("{base}/api/status-page/{slug}");
    let heartbeat_url = format!("{base}/api/status-page/heartbeat/{slug}");

    let (status, heartbeat) = tokio::join!(
        fetch_json::<StatusPage>(client, &status_url),
        fetch_json::<HeartbeatPage>(client, &heartbeat_url),
    );
    if let (Ok(status), Ok(heartbeat)) = (status, heartbeat) {
        result.monitors = assemble(&status, &heartbeat);
    }
    result
}

/// GET a URL and decode it, refusing anything that is not a `200`.
async fn fetch_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
) -> Result<T, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("{url}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }
    let body = response
        .text()
        .await
        .map_err(|error| format!("{url}: {error}"))?;
    serde_json::from_str(&body).map_err(|error| format!("{url}: {error}"))
}

struct CacheEntry {
    /// The settings blob this result was built from. A cache keyed only by time
    /// would keep serving an old configuration after an operator fixes a typo.
    config: String,
    fetched_at: Instant,
    groups: Vec<UptimeGroupResult>,
}

static CACHE: once_cell::sync::Lazy<parking_lot::Mutex<Option<CacheEntry>>> =
    once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(None));

/// A cached fan-out, when one matches the current configuration and is fresh.
fn cached(config: &str, now: Instant) -> Option<Vec<UptimeGroupResult>> {
    let cache = CACHE.lock();
    let entry = cache.as_ref()?;
    if entry.config != config || now.duration_since(entry.fetched_at) >= CACHE_TTL {
        return None;
    }
    Some(entry.groups.clone())
}

fn store(config: &str, groups: Vec<UptimeGroupResult>) {
    *CACHE.lock() = Some(CacheEntry {
        config: config.to_string(),
        fetched_at: Instant::now(),
        groups,
    });
}

/// `GET /api/uptime/status` — the console's Uptime-Kuma panel.
///
/// Public, matching the reference, because the panel renders on the overview for
/// signed-out visitors too. A disabled panel answers with an empty list rather
/// than an error: the field is a display switch, and the console hides the panel
/// when it is off (`SystemStatus::uptime_kuma_enabled`).
pub async fn get_uptime_status(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<UptimeGroupResult>>> {
    if !state.db.typed_setting("UptimeKumaEnabled", false) {
        return Json(ApiResponse::ok(Vec::new()));
    }
    let config = state
        .db
        .typed_setting("UptimeKumaGroups", "[]".to_string());
    if let Some(groups) = cached(&config, Instant::now()) {
        return Json(ApiResponse::ok(groups));
    }
    let groups = parse_groups(&config);
    if groups.is_empty() {
        return Json(ApiResponse::ok(Vec::new()));
    }

    let client = match reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        // Redirects are not followed: an operator points this at a status page
        // they run, and a redirect would silently move the request somewhere
        // else on every poll.
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            eprintln!("[OxygenRouter] uptime client unusable: {error}");
            return Json(ApiResponse::ok(Vec::new()));
        }
    };

    let fan_out = async {
        let futures = groups.iter().map(|group| fetch_group(&client, group));
        futures_util::future::join_all(futures).await
    };
    let results = match tokio::time::timeout(TOTAL_TIMEOUT, fan_out).await {
        Ok(results) => results,
        // A group that never answers is reported as unreachable rather than
        // dropping the groups that did answer; the reference's context timeout
        // has the same effect because the per-group function returns early.
        Err(_) => groups
            .iter()
            .map(|group| UptimeGroupResult {
                category_name: group.category_name.clone(),
                monitors: Vec::new(),
            })
            .collect(),
    };
    store(&config, results.clone());
    Json(ApiResponse::ok(results))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(url: &str, slug: &str) -> UptimeGroup {
        UptimeGroup {
            url: url.to_string(),
            slug: slug.to_string(),
            category_name: "Probes".to_string(),
        }
    }

    /// The reference's settings form submits `categoryName`, so that spelling
    /// must round-trip; a blob written with `category_name` is accepted too
    /// because the option is hand-editable.
    #[test]
    fn a_group_reads_both_category_spellings() {
        let groups = parse_groups(
            r#"[{"url":"https://kuma.example.com","slug":"main","categoryName":"Public"},
                {"url":"https://kuma.example.com","slug":"lan","category_name":"LAN"}]"#,
        );
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].category_name, "Public");
        assert_eq!(groups[1].category_name, "LAN");
    }

    /// A malformed blob, a non-http scheme and a slug that is not one path
    /// segment are all refused without panicking; the rest of the list survives.
    #[test]
    fn unusable_groups_are_dropped() {
        assert!(parse_groups("").is_empty());
        assert!(parse_groups("not json").is_empty());
        assert!(parse_groups("{}").is_empty());

        let groups = parse_groups(
            r#"[{"url":"file:///etc/passwd","slug":"main"},
                {"url":"https://kuma.example.com","slug":""},
                {"url":"https://kuma.example.com","slug":"a/../b"},
                {"url":"https://kuma.example.com","slug":"main?x=1"},
                {"url":"https://kuma.example.com","slug":"ok"}]"#,
        );
        let mut ok = group("https://kuma.example.com", "ok");
        // The entry carries no `categoryName`, so the heading is empty rather
        // than the fixture's placeholder.
        ok.category_name = String::new();
        assert_eq!(groups, vec![ok]);
    }

    /// More groups than the cap are truncated rather than all being fetched.
    #[test]
    fn the_group_count_is_bounded() {
        let blob = (0..MAX_GROUPS + 5)
            .map(|i| format!(r#"{{"url":"https://kuma.example.com","slug":"s{i}"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(parse_groups(&format!("[{blob}]")).len(), MAX_GROUPS);
    }

    /// A monitor's figures come from its own id: uptime from `<id>_24`, state
    /// from the first heartbeat. A second monitor without data is still listed.
    #[test]
    fn monitors_are_flattened_with_their_heartbeats() {
        let status: StatusPage = serde_json::from_str(
            r#"{"publicGroupList":[
                 {"id":1,"name":"Public","monitorList":[{"id":7,"name":"gateway"},{"id":9,"name":"db"}]},
                 {"id":2,"name":"","monitorList":[]},
                 {"id":3,"name":"Empty","monitorList":[]}]}"#,
        )
        .expect("status page fixture");
        let heartbeat: HeartbeatPage = serde_json::from_str(
            r#"{"heartbeatList":{"7":[{"status":1}]},"uptimeList":{"7_24":0.987,"9_24":0.5}}"#,
        )
        .expect("heartbeat fixture");

        let monitors = assemble(&status, &heartbeat);
        assert_eq!(monitors.len(), 2);
        assert_eq!(monitors[0].name, "gateway");
        assert_eq!(monitors[0].uptime, 0.987);
        assert_eq!(monitors[0].status, 1);
        assert_eq!(monitors[0].group.as_deref(), Some("Public"));
        // No heartbeat for monitor 9: it is reported as down with no uptime
        // rather than dropped, so the row count still matches the status page.
        assert_eq!(monitors[1].name, "db");
        assert_eq!(monitors[1].uptime, 0.5);
        assert_eq!(monitors[1].status, 0);
    }

    /// A group with an empty name omits the field, matching the reference's
    /// `group,omitempty`; an empty monitor list carries no rows at all.
    #[test]
    fn an_unnamed_group_omits_its_name() {
        let status: StatusPage = serde_json::from_str(
            r#"{"publicGroupList":[{"name":"","monitorList":[{"id":1,"name":"only"}]}]}"#,
        )
        .expect("status page fixture");
        let monitors = assemble(&status, &HeartbeatPage::default());
        assert_eq!(monitors.len(), 1);
        assert!(monitors[0].group.is_none());
        assert_eq!(monitors[0].uptime, 0.0);
    }

    /// The cache is keyed on the configuration, so an operator's edit is visible
    /// immediately instead of after the TTL, while a repeat poll is served from
    /// memory.
    #[test]
    fn the_cache_follows_the_configuration_and_expires() {
        let now = Instant::now();
        let built = vec![UptimeGroupResult {
            category_name: "Public".to_string(),
            monitors: vec![Monitor {
                name: "gateway".to_string(),
                uptime: 1.0,
                status: 1,
                group: None,
            }],
        }];
        store("[{\"url\":\"https://kuma.example.com\",\"slug\":\"main\"}]", built.clone());

        let config = "[{\"url\":\"https://kuma.example.com\",\"slug\":\"main\"}]";
        assert_eq!(cached(config, now), Some(built.clone()));
        assert_eq!(cached("[{\"slug\":\"other\"}]", now), None);
        // `store` stamped the entry a moment after `now`, so expiry is asserted
        // past the deadline rather than exactly on it.
        assert_eq!(cached(config, now + CACHE_TTL + Duration::from_secs(1)), None);

        // Leave the process-wide cache clean for other tests.
        *CACHE.lock() = None;
    }
}
