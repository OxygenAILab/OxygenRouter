//! Channel affinity: keep related requests on the same upstream.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover
//!
//! # What this is, and why it is not a boolean
//!
//! The console has carried `ChannelAffinityEnabled` with the description "Prefer
//! the last healthy channel for a token" and nothing implemented it. Reading the
//! reference settled what the feature actually is: a **rule-driven session
//! stickiness system** (`setting/operation_setting/channel_affinity_setting.go`).
//! A rule matches a request by model, path and user agent, extracts one or more
//! identity values from its headers or body, and remembers which channel served
//! that identity for a while. A follow-up request carrying the same identity goes
//! back to the same channel.
//!
//! The reason is not fairness but **prefix cache**: a conversation that keeps
//! landing on different upstreams pays for its prompt every time, and an agent
//! session that switches providers mid-thread is visibly worse. So the identity
//! is usually a session or thread id the client already sends.
//!
//! Recorded as one structured setting rather than the boolean the console
//! advertised, because a switch cannot express which requests are sticky, on what
//! identity, or for how long — and an option that cannot express its feature is
//! how the inert boolean happened in the first place.
//!
//! # Deliberate deviations from the reference
//!
//! * Model and path patterns are **globs**, not regexes, because that is what
//!   this codebase's model maps already use and two matching dialects in one
//!   product would be a papercut for whoever configures both. A glob covers the
//!   matching the feature needs (`gpt-4*`, `/v1/responses`).
//! * `param_override_template` is not implemented. It rewrites request
//!   parameters for the pinned channel, which belongs with the routing work
//!   already done for model maps rather than bolted onto stickiness.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use crate::upstream::ProxyRequest;

/// What to do when the remembered channel is no longer usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    /// Leave the option inert for this rule.
    Off,
    /// Use the remembered channel when it is still a candidate; otherwise pick
    /// normally and remember the new choice.
    Prefer,
    /// Use the remembered channel or fail. A session that must not move — an
    /// agent mid-thread — would rather see an error than be silently moved to an
    /// upstream whose cache it does not share.
    Strict,
}

impl Default for SessionMode {
    fn default() -> Self {
        Self::Prefer
    }
}

/// Where a rule reads its identity value from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum KeySource {
    /// A request header, matched case-insensitively.
    Header { name: String },
    /// A dotted path into the JSON body, e.g. `metadata.session_id` or
    /// `messages[0].role`.
    BodyPath { path: String },
}

impl KeySource {
    /// The value this source yields for a request, if any.
    pub fn read(&self, req: &ProxyRequest) -> Option<String> {
        match self {
            Self::Header { name } => req
                .client_headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.trim().to_string())
                .filter(|v| !v.is_empty()),
            Self::BodyPath { path } => {
                let body = req.body.as_deref()?;
                let json: serde_json::Value = serde_json::from_slice(body).ok()?;
                walk_path(&json, path)
            }
        }
    }
}

/// One stickiness rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffinityRule {
    pub name: String,
    /// Globs the request's model must match; empty matches every model.
    #[serde(default)]
    pub model_patterns: Vec<String>,
    /// Globs the request's path must match; empty matches every path.
    #[serde(default)]
    pub path_patterns: Vec<String>,
    /// Fragments, any of which must appear in the user agent; empty matches all.
    #[serde(default)]
    pub user_agent_includes: Vec<String>,
    /// Values read to form the identity. Every source must yield a value, or the
    /// request is not considered part of a session: a rule that matched on half
    /// an identity would pin unrelated requests together.
    pub key_sources: Vec<KeySource>,
    /// How long a remembered choice lasts.
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
    /// Empty inherits the setting's default mode.
    #[serde(default)]
    pub session_mode: Option<SessionMode>,
    /// Fold the model name into the identity, so two models in one session do
    /// not pin to a channel chosen for the other.
    #[serde(default)]
    pub include_model_name: bool,
    /// Fold the rule name in, so two rules cannot collide on the same value.
    #[serde(default)]
    pub include_rule_name: bool,
    /// Fold the request's group in.
    #[serde(default)]
    pub include_using_group: bool,
}

/// The whole setting, as one JSON document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffinitySetting {
    #[serde(default)]
    pub enabled: bool,
    /// Mode for rules that do not state one. Empty means `prefer`.
    #[serde(default)]
    pub session_mode: Option<SessionMode>,
    /// Upper bound on remembered sessions, so a high-cardinality identity cannot
    /// grow the map without limit.
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
    #[serde(default = "default_ttl")]
    pub default_ttl_seconds: u64,
    #[serde(default)]
    pub rules: Vec<AffinityRule>,
}

fn default_max_entries() -> usize {
    4096
}
fn default_ttl() -> u64 {
    900
}

impl Default for AffinitySetting {
    fn default() -> Self {
        Self {
            enabled: false,
            session_mode: None,
            max_entries: default_max_entries(),
            default_ttl_seconds: default_ttl(),
            rules: Vec::new(),
        }
    }
}

/// A remembered choice.
#[derive(Debug, Clone)]
struct Pinned {
    channel_id: String,
    expires_at: Instant,
}

/// What a request resolved to, for the scheduler to act on.
#[derive(Debug, Clone)]
pub struct AffinityHit {
    /// Identity for this request, to be remembered after a success.
    pub key: String,
    pub mode: SessionMode,
    /// The channel this session used last, when it is still remembered.
    pub pinned_channel: Option<String>,
    pub ttl: Duration,
}

/// The affinity store: settings plus the session map.
pub struct AffinityStore {
    setting: RwLock<AffinitySetting>,
    sessions: Mutex<HashMap<String, Pinned>>,
}

impl Default for AffinityStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AffinityStore {
    pub fn new() -> Self {
        Self {
            setting: RwLock::new(AffinitySetting::default()),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn set_setting(&self, setting: AffinitySetting) {
        *self.setting.write() = setting;
        // A rule change invalidates every remembered choice: the identities it
        // derived are no longer the ones it would derive now.
        self.sessions.lock().clear();
    }

    pub fn enabled(&self) -> bool {
        self.setting.read().enabled
    }

    /// Sessions currently remembered, for diagnostics and tests.
    pub fn len(&self) -> usize {
        self.sessions.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Resolve the affinity of a request, if any rule claims it.
    pub fn resolve(
        &self,
        req: &ProxyRequest,
        model: &str,
        group: Option<&str>,
    ) -> Option<AffinityHit> {
        let setting = self.setting.read();
        if !setting.enabled {
            return None;
        }
        let user_agent = header(req, "user-agent").unwrap_or_default();
        let rule = setting.rules.iter().find(|rule| {
            matches_any(&rule.model_patterns, model)
                && matches_any(&rule.path_patterns, &req.path)
                && (rule.user_agent_includes.is_empty()
                    || rule
                        .user_agent_includes
                        .iter()
                        .any(|f| user_agent.to_ascii_lowercase().contains(&f.to_ascii_lowercase())))
        })?;

        // Every source must contribute. A partial identity would pin requests
        // that merely resemble each other.
        let mut parts = Vec::with_capacity(rule.key_sources.len() + 3);
        for source in &rule.key_sources {
            parts.push(source.read(req)?);
        }
        if rule.include_rule_name {
            parts.push(rule.name.clone());
        }
        if rule.include_model_name {
            parts.push(model.to_string());
        }
        if rule.include_using_group {
            parts.push(group.unwrap_or("").to_string());
        }
        let key = parts.join("\u{1f}");

        let mode = rule
            .session_mode
            .or(setting.session_mode)
            .unwrap_or_default();
        if mode == SessionMode::Off {
            return None;
        }
        let ttl = Duration::from_secs(
            rule.ttl_seconds
                .unwrap_or(setting.default_ttl_seconds)
                .max(1),
        );
        let pinned_channel = self
            .sessions
            .lock()
            .get(&key)
            .filter(|entry| entry.expires_at > Instant::now())
            .map(|entry| entry.channel_id.clone());

        Some(AffinityHit {
            key,
            mode,
            pinned_channel,
            ttl,
        })
    }

    /// Remember that `channel_id` served `key`.
    pub fn remember(&self, key: &str, channel_id: &str, ttl: Duration) {
        let mut sessions = self.sessions.lock();
        let max = self.setting.read().max_entries.max(1);
        let now = Instant::now();
        sessions.retain(|_, entry| entry.expires_at > now);
        // At the ceiling, one entry is evicted. Choosing the nearest to expiry
        // keeps the map bounded without discarding the session most likely to
        // still be in use.
        if sessions.len() >= max && !sessions.contains_key(key) {
            if let Some(victim) = sessions
                .iter()
                .min_by_key(|(_, entry)| entry.expires_at)
                .map(|(k, _)| k.clone())
            {
                sessions.remove(&victim);
            }
        }
        sessions.insert(
            key.to_string(),
            Pinned {
                channel_id: channel_id.to_string(),
                expires_at: now + ttl,
            },
        );
    }

    /// Drop expired sessions. Called by the maintenance task.
    pub fn prune(&self) -> usize {
        let mut sessions = self.sessions.lock();
        let before = sessions.len();
        let now = Instant::now();
        sessions.retain(|_, entry| entry.expires_at > now);
        before - sessions.len()
    }
}

/// The value of a header, matched case-insensitively.
fn header(req: &ProxyRequest, name: &str) -> Option<String> {
    req.client_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

/// An empty pattern list matches everything; otherwise any pattern may match.
fn matches_any(patterns: &[String], value: &str) -> bool {
    patterns.is_empty()
        || patterns
            .iter()
            .any(|pattern| glob_match(pattern, value))
}

/// Walk `a.b[0].c` through a JSON document.
///
/// Deliberately small: dotted keys and array indices cover the identity fields a
/// client actually sends. Anything richer would be a query language, which is not
/// what this feature needs.
fn walk_path(value: &serde_json::Value, path: &str) -> Option<String> {
    let mut current = value;
    for segment in path.split('.') {
        let (name, indices) = split_indices(segment);
        if !name.is_empty() {
            current = current.get(name)?;
        }
        for index in indices {
            current = current.get(index)?;
        }
    }
    match current {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Split `messages[0][1]` into `("messages", [0, 1])`.
fn split_indices(segment: &str) -> (&str, Vec<usize>) {
    let mut indices = Vec::new();
    let name = match segment.find('[') {
        None => segment,
        Some(at) => {
            let mut rest = &segment[at..];
            while let Some(open) = rest.find('[') {
                let Some(close) = rest[open..].find(']') else {
                    break;
                };
                if let Ok(index) = rest[open + 1..open + close].parse::<usize>() {
                    indices.push(index);
                }
                rest = &rest[open + close..];
            }
            &segment[..at]
        }
    };
    (name, indices)
}

/// The same glob matching the model maps use (`*` is the only wildcard).
fn glob_match(pattern: &str, value: &str) -> bool {
    let (p, v) = (pattern.as_bytes(), value.as_bytes());
    let (mut pi, mut vi) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while vi < v.len() {
        if pi < p.len() && (p[pi] == v[vi]) {
            pi += 1;
            vi += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            mark = vi;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            vi = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}


#[cfg(test)]
mod tests {
    use super::*;

    fn request(path: &str, model: &str, headers: Vec<(&str, &str)>, body: serde_json::Value) -> ProxyRequest {
        ProxyRequest {
            method: "POST".into(),
            path: path.into(),
            headers: Vec::new(),
            client_headers: headers
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: Some(serde_json::to_vec(&body).unwrap()),
            model: model.into(),
            stream: false,
        }
    }

    fn session_rule() -> AffinityRule {
        AffinityRule {
            name: "codex".into(),
            model_patterns: vec!["gpt-5*".into()],
            path_patterns: vec!["/v1/responses".into()],
            user_agent_includes: vec!["codex".into()],
            key_sources: vec![
                KeySource::Header { name: "session_id".into() },
                KeySource::BodyPath { path: "metadata.thread_id".into() },
            ],
            ttl_seconds: Some(600),
            session_mode: None,
            include_model_name: true,
            include_rule_name: false,
            include_using_group: false,
        }
    }

    fn store_with(rules: Vec<AffinityRule>, mode: SessionMode, enabled: bool) -> AffinityStore {
        let store = AffinityStore::new();
        store.set_setting(AffinitySetting {
            enabled,
            session_mode: Some(mode),
            max_entries: 4,
            default_ttl_seconds: 600,
            rules,
        });
        store
    }

    #[test]
    fn a_rule_matches_on_model_path_and_user_agent() {
        let store = store_with(vec![session_rule()], SessionMode::Prefer, true);
        let req = request(
            "/v1/responses",
            "gpt-5.1-codex",
            vec![("Session_id", "s1"), ("User-Agent", "codex-cli/1.0")],
            serde_json::json!({"metadata": {"thread_id": "t1"}}),
        );
        assert!(store.resolve(&req, "gpt-5.1-codex", None).is_some());

        // Wrong model, wrong path, wrong agent: each on its own disqualifies it.
        assert!(store.resolve(&req, "gpt-4o", None).is_none());
        let other_path = request("/v1/chat/completions", "gpt-5.1-codex",
            vec![("Session_id", "s1"), ("User-Agent", "codex-cli/1.0")],
            serde_json::json!({"metadata": {"thread_id": "t1"}}));
        assert!(store.resolve(&other_path, "gpt-5.1-codex", None).is_none());
        let other_agent = request("/v1/responses", "gpt-5.1-codex",
            vec![("Session_id", "s1"), ("User-Agent", "curl/8")],
            serde_json::json!({"metadata": {"thread_id": "t1"}}));
        assert!(store.resolve(&other_agent, "gpt-5.1-codex", None).is_none());
    }

    /// Every source must contribute. One missing field must not yield a partial
    /// identity, which would pin unrelated requests together.
    #[test]
    fn a_partial_identity_is_not_a_session() {
        let store = store_with(vec![session_rule()], SessionMode::Prefer, true);
        let no_thread = request("/v1/responses", "gpt-5.1-codex",
            vec![("Session_id", "s1"), ("User-Agent", "codex-cli/1.0")],
            serde_json::json!({"metadata": {}}));
        assert!(store.resolve(&no_thread, "gpt-5.1-codex", None).is_none());
        let no_header = request("/v1/responses", "gpt-5.1-codex",
            vec![("User-Agent", "codex-cli/1.0")],
            serde_json::json!({"metadata": {"thread_id": "t1"}}));
        assert!(store.resolve(&no_header, "gpt-5.1-codex", None).is_none());
    }

    #[test]
    fn the_identity_folds_in_what_the_rule_asks_for() {
        let store = store_with(vec![session_rule()], SessionMode::Prefer, true);   // include_model_name
        let req = request("/v1/responses", "gpt-5.1-codex",
            vec![("Session_id", "s1"), ("User-Agent", "codex-cli/1.0")],
            serde_json::json!({"metadata": {"thread_id": "t1"}}));
        let a = store.resolve(&req, "gpt-5.1-codex", None).unwrap();
        let b = store.resolve(&req, "gpt-5.2-codex", None).unwrap();
        assert_ne!(a.key, b.key, "a different model must be a different session");
    }

    #[test]
    fn remembering_returns_the_same_channel_and_expires() {
        let store = store_with(vec![session_rule()], SessionMode::Prefer, true);
        let req = request("/v1/responses", "gpt-5.1-codex",
            vec![("Session_id", "s1"), ("User-Agent", "codex-cli/1.0")],
            serde_json::json!({"metadata": {"thread_id": "t1"}}));
        let hit = store.resolve(&req, "gpt-5.1-codex", None).unwrap();
        assert_eq!(hit.pinned_channel, None);
        store.remember(&hit.key, "channel-a", Duration::from_millis(1));
        assert_eq!(store.len(), 1);
        // Non-expiring in practice for the assertion below, but the expiry path
        // is exercised by letting a zero-length TTL lapse.
        std::thread::sleep(Duration::from_millis(5));
        let after = store.resolve(&req, "gpt-5.1-codex", None).unwrap();
        assert_eq!(after.pinned_channel, None, "an expired session must not pin");
        assert_eq!(store.prune(), 1, "prune drops it");
        assert_eq!(store.len(), 0);

        store.remember(&hit.key, "channel-a", Duration::from_secs(60));
        assert_eq!(
            store.resolve(&req, "gpt-5.1-codex", None).unwrap().pinned_channel.as_deref(),
            Some("channel-a")
        );
    }

    #[test]
    fn off_disables_a_rule_and_a_disabled_setting_does_nothing() {
        let req = request("/v1/responses", "gpt-5.1-codex",
            vec![("Session_id", "s1"), ("User-Agent", "codex-cli/1.0")],
            serde_json::json!({"metadata": {"thread_id": "t1"}}));
        assert!(store_with(vec![session_rule()], SessionMode::Off, true)
            .resolve(&req, "gpt-5.1-codex", None).is_none());
        assert!(store_with(vec![session_rule()], SessionMode::Prefer, false)
            .resolve(&req, "gpt-5.1-codex", None).is_none());
    }

    #[test]
    fn strict_and_prefer_are_reported_distinctly() {
        let req = request("/v1/responses", "gpt-5.1-codex",
            vec![("Session_id", "s1"), ("User-Agent", "codex-cli/1.0")],
            serde_json::json!({"metadata": {"thread_id": "t1"}}));
        assert_eq!(
            store_with(vec![session_rule()], SessionMode::Strict, true)
                .resolve(&req, "gpt-5.1-codex", None).unwrap().mode,
            SessionMode::Strict
        );
        assert_eq!(
            store_with(vec![session_rule()], SessionMode::Prefer, true)
                .resolve(&req, "gpt-5.1-codex", None).unwrap().mode,
            SessionMode::Prefer
        );
    }

    /// A high-cardinality identity must not grow the map without bound.
    #[test]
    fn the_session_map_is_bounded() {
        let store = store_with(vec![session_rule()], SessionMode::Prefer, true);
        for i in 0..20 {
            let req = request("/v1/responses", "gpt-5.1-codex",
                vec![("Session_id", &format!("s{i}")), ("User-Agent", "codex-cli/1.0")],
                serde_json::json!({"metadata": {"thread_id": "t"}}));
            let hit = store.resolve(&req, "gpt-5.1-codex", None).unwrap();
            store.remember(&hit.key, "channel-a", Duration::from_secs(60));
        }
        assert!(store.len() <= 4, "max_entries was 4, got {}", store.len());
    }

    #[test]
    fn a_body_path_walks_keys_and_indices() {
        let body = serde_json::json!({
            "metadata": {"session": {"id": "abc"}},
            "messages": [{"role": "system"}, {"role": "user"}],
            "n": 7
        });
        assert_eq!(walk_path(&body, "metadata.session.id").as_deref(), Some("abc"));
        assert_eq!(walk_path(&body, "messages[1].role").as_deref(), Some("user"));
        assert_eq!(walk_path(&body, "n").as_deref(), Some("7"));
        // Missing paths and non-scalar leaves yield nothing rather than a debug
        // rendering of a subtree.
        assert_eq!(walk_path(&body, "metadata.missing"), None);
        assert_eq!(walk_path(&body, "messages"), None);
    }

    #[test]
    fn glob_matching_covers_the_shapes_a_rule_needs() {
        assert!(glob_match("gpt-4*", "gpt-4o"));
        assert!(glob_match("*codex*", "gpt-5.1-codex"));
        assert!(glob_match("/v1/responses", "/v1/responses"));
        assert!(!glob_match("gpt-4*", "claude-3"));
        assert!(!glob_match("/v1/responses", "/v1/responses/x"));
        assert!(glob_match("*", "anything"));
    }
}
