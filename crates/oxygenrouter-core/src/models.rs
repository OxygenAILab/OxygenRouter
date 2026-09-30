//! Data models for OxygenRouter
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default = "default_provider")]
    pub provider: String,
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_weight")]
    pub weight: i32,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_test_model")]
    pub test_model: String,
    #[serde(default = "default_group_name")]
    pub group_name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub model_list: Vec<String>,
    #[serde(default)]
    pub response_headers: serde_json::Value,
    #[serde(default)]
    pub status_code_mapping: serde_json::Value,
    #[serde(default)]
    pub override_parameters: serde_json::Value,
    #[serde(default)]
    pub balance_micros: i64,
    #[serde(default)]
    pub last_test_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub info: ChannelInfo,
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
}

/// Multi-key bookkeeping, mirrors NewAPI's ChannelInfo JSON column.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelInfo {
    #[serde(default)]
    pub is_multi_key: bool,
    #[serde(default)]
    pub multi_key_size: usize,
    /// key index -> status (1 enabled, 2 manually disabled, 3 auto disabled)
    #[serde(default)]
    pub multi_key_status_list: std::collections::BTreeMap<usize, i32>,
    #[serde(default)]
    pub multi_key_disabled_reason: std::collections::BTreeMap<usize, String>,
    #[serde(default)]
    pub multi_key_disabled_time: std::collections::BTreeMap<usize, i64>,
    #[serde(default)]
    pub multi_key_polling_index: usize,
    /// "random" | "polling"
    #[serde(default = "default_multi_key_mode")]
    pub multi_key_mode: String,
}

fn default_multi_key_mode() -> String {
    "random".to_string()
}

impl Channel {
    /// All keys of this channel (newline separated, blanks removed).
    pub fn keys(&self) -> Vec<String> {
        self.api_key
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// Status of a key index (missing entry means enabled).
    pub fn key_status(&self, index: usize) -> i32 {
        self.info.multi_key_status_list.get(&index).copied().unwrap_or(1)
    }

    /// Next enabled key index (random or polling), mirrors NewAPI semantics.
    pub fn next_enabled_key(&self) -> Option<(String, usize)> {
        let keys = self.keys();
        if keys.is_empty() {
            return None;
        }
        if !self.info.is_multi_key || keys.len() == 1 {
            return Some((keys[0].clone(), 0));
        }
        let enabled: Vec<usize> = (0..keys.len()).filter(|index| self.key_status(*index) == 1).collect();
        if enabled.is_empty() {
            return None;
        }
        let pick = if self.info.multi_key_mode == "polling" {
            let start = self.info.multi_key_polling_index % keys.len();
            enabled
                .iter()
                .copied()
                .find(|index| *index >= start)
                .unwrap_or(enabled[0])
        } else {
            // deterministic spread using current UTC nanoseconds as the entropy source
            let seed = Utc::now().timestamp_nanos_opt().unwrap_or(0) as usize;
            enabled[seed % enabled.len()]
        };
        Some((keys[pick].clone(), pick))
    }

    pub fn enabled_key_count(&self) -> usize {
        let keys = self.keys();
        (0..keys.len()).filter(|index| self.key_status(*index) == 1).count()
    }
}

fn default_provider() -> String {
    "openai".to_string()
}

fn default_weight() -> i32 {
    1
}
fn default_true() -> bool {
    true
}
fn default_test_model() -> String {
    "gpt-3.5-turbo".to_string()
}
fn default_group_name() -> String {
    "default".to_string()
}

impl Channel {
    pub fn new(
        name: String,
        base_url: String,
        api_key: String,
        priority: i32,
        weight: i32,
        test_model: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            provider: "openai".to_string(),
            base_url,
            api_key,
            priority,
            weight,
            enabled: true,
            test_model,
            group_name: default_group_name(),
            tags: Vec::new(),
            model_list: Vec::new(),
            response_headers: serde_json::Value::Object(Default::default()),
            status_code_mapping: serde_json::Value::Object(Default::default()),
            override_parameters: serde_json::Value::Object(Default::default()),
            balance_micros: 0,
            last_test_at: None,
            info: ChannelInfo::default(),
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMap {
    #[serde(default)]
    pub id: String,
    pub channel_id: String,
    pub pattern: String,
    pub target_model: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
}

impl ModelMap {
    pub fn new(channel_id: String, pattern: String, target_model: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            channel_id,
            pattern,
            target_model,
            enabled: true,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRule {
    pub id: String,
    pub name: String,
    pub rule_type: RouteType,
    pub priority: i32,
    pub enabled: bool,
    pub config: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RouteType {
    AutoHeuristic,
    KeywordMatch,
    TypeRouting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub priority: i32,
    /// Owning user, whose wallet pays for requests made with this key.
    ///
    /// Empty when the key is not wallet-backed.
    #[serde(default)]
    pub user_id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub quota_micros: i64,
    #[serde(default)]
    pub used_micros: i64,
    #[serde(default)]
    pub allowed_models: Vec<String>,
    #[serde(default)]
    pub ip_allowlist: Vec<String>,
    #[serde(default = "default_group_name")]
    pub group_name: String,
    #[serde(default = "default_true")]
    pub cross_group_retry: bool,
}

impl ApiKey {
    pub fn new(key: String, name: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            key,
            name,
            priority: 0,
            enabled: true,
            created_at: Utc::now(),
            expires_at: None,
            quota_micros: 0,
            used_micros: 0,
            allowed_models: Vec::new(),
            ip_allowlist: Vec::new(),
            group_name: default_group_name(),
            cross_group_retry: true,
            user_id: String::new(),
        }
    }

    /// Bind this key to an owning user, whose wallet pays for its requests.
    pub fn owned_by(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = user_id.into();
        self
    }

    /// Set the key's spend ceiling in micros. `0` means unlimited, matching
    /// NewAPI's semantics for a token without a quota ceiling.
    pub fn with_quota(mut self, quota_micros: i64) -> Self {
        self.quota_micros = quota_micros.max(0);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLog {
    pub id: String,
    pub method: String,
    pub path: String,
    pub model: Option<String>,
    pub channel_id: Option<String>,
    pub api_key_id: Option<String>,
    pub status_code: Option<u16>,
    pub error: Option<String>,
    pub tokens_used: Option<i64>,
    pub duration_ms: i64,
    pub created_at: DateTime<Utc>,
    /// The calling address, when the instance is configured to keep it.
    ///
    /// `None` unless `RecordIpLog` is on. Captured per request so a later change
    /// to that option does not retroactively relabel old rows: a row either
    /// recorded an address at the time, or it did not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_ip: Option<String>,
}

/// One stored build of a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginVersion {
    pub key: String,
    pub version: String,
    /// The plugin's JavaScript, as uploaded.
    pub source: String,
    /// The manifest the source declared, serialised. Stored rather than
    /// re-derived so a listing does not have to run untrusted code to describe
    /// itself.
    pub manifest: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Which build of a plugin is active, and whether it runs at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginState {
    pub key: String,
    pub active_version: Option<String>,
    pub enabled: bool,
    pub updated_at: DateTime<Utc>,
}

/// A plugin as the console sees it: its state plus the manifest of the active
/// build, with every version it has on file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSummary {
    pub key: String,
    pub name: String,
    pub version: Option<String>,
    pub description: String,
    pub enabled: bool,
    pub active_version: Option<String>,
    pub versions: Vec<String>,
    /// Hooks the active build implements, so a page can show what it will do
    /// without running it.
    pub hooks: Vec<String>,
    /// Protocols the active build claims. The routing path needs these to decide
    /// which plugin serves an endpoint, and reading them here keeps that decision
    /// out of the manifest parsing on the request path.
    #[serde(default)]
    pub protocols: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

/// One administrative action, recorded for accountability.
///
/// The console has always offered `AuditLogEnabled` with the description
/// "Record administrative operations", and nothing implemented it — there was
/// no table at all. The reference keeps these in the same table as traffic,
/// distinguished by a type column (`model/log.go:88`); a separate table is used
/// here so traffic retention can be decided independently of the audit trail,
/// which is the point of an audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    pub id: String,
    /// The acting account, when the action was authenticated.
    pub actor_id: Option<String>,
    /// The acting account's name, denormalised: an audit row must stay readable
    /// after the account it names is deleted.
    pub actor_name: String,
    /// The actor's role at the time, for the same reason.
    pub actor_role: String,
    /// `POST` / `PUT` / `DELETE` / `PATCH`.
    pub method: String,
    /// The console path that was called.
    pub path: String,
    /// HTTP status of the response.
    pub status_code: Option<u16>,
    /// The request body, with credential-bearing fields replaced. `None` when
    /// the action had no body.
    pub detail: Option<String>,
    pub client_ip: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// One hour of usage, summed across requests.
///
/// `DataExportEnabled` was advertised as "Aggregate usage into quota_data for
/// analytics" with no table and no writer. The reason the reference keeps this
/// alongside the raw log is retention: `request_logs` is pruned on
/// `LogRetentionDays`, so a long-range chart built by scanning it silently loses
/// everything older than the window. This table is the durable summary.
///
/// The grain and the columns mirror the reference's `model.QuotaData`
/// (`model/usedata.go:13`): one row per hour per dimension tuple, with `count`,
/// `quota` and `token_used` accumulated into it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaData {
    pub id: String,
    /// Start of the hour this row covers, as a UTC timestamp.
    pub bucket_at: DateTime<Utc>,
    pub user_id: String,
    pub username: String,
    pub model_name: String,
    pub group_name: String,
    pub channel_id: String,
    pub token_id: String,
    /// Billed requests counted in this hour.
    ///
    /// Not the same figure as `request_logs`: the relay has a pass-through path
    /// that authorises and forwards without settling a charge, and the reference
    /// likewise exports only its consume records. A request that cost nothing is
    /// absent here rather than counted at zero, so this table reads as "spend",
    /// not "traffic".
    pub count: i64,
    /// Quota micros spent in this hour.
    pub quota: i64,
    /// Prompt plus completion tokens in this hour.
    pub token_used: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_listen_host")]
    pub listen_host: String,
    #[serde(default = "default_listen_port")]
    pub listen_port: u16,
    #[serde(default = "default_local_api_token")]
    pub local_api_token: String,
    #[serde(default = "default_true")]
    pub open_browser_on_start: bool,
    #[serde(default = "default_db_path")]
    pub db_path: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_max_retries")]
    pub max_retries: i32,
    #[serde(default = "default_retry_delay_ms")]
    pub retry_delay_ms: i64,
    #[serde(default = "default_retry_backoff")]
    pub retry_backoff: String,
    #[serde(default = "default_upstream_timeout_ms")]
    pub upstream_timeout_ms: u64,
    #[serde(default = "default_user_agent")]
    pub user_agent: String,
    #[serde(default = "default_max_concurrent_requests")]
    pub max_concurrent_requests: i32,
    #[serde(default = "default_request_log_retention_days")]
    pub request_log_retention_days: i32,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_language")]
    pub language: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            listen_host: default_listen_host(),
            listen_port: default_listen_port(),
            local_api_token: default_local_api_token(),
            open_browser_on_start: default_true(),
            db_path: default_db_path(),
            log_level: default_log_level(),
            max_retries: default_max_retries(),
            retry_delay_ms: default_retry_delay_ms(),
            retry_backoff: default_retry_backoff(),
            upstream_timeout_ms: default_upstream_timeout_ms(),
            user_agent: default_user_agent(),
            max_concurrent_requests: default_max_concurrent_requests(),
            request_log_retention_days: default_request_log_retention_days(),
            theme: default_theme(),
            language: default_language(),
        }
    }
}

// Per-field defaults, shared by `Default` and the serde `#[serde(default = ...)]`
// hooks. Without these, a hand-written `config.json` that omits a field fails to
// deserialize, and `main` then silently overwrites the operator's file with
// stock defaults — losing settings such as a custom listen port.
fn default_listen_host() -> String {
    "127.0.0.1".to_string()
}
fn default_listen_port() -> u16 {
    3001
}
fn default_local_api_token() -> String {
    Uuid::new_v4().to_string()
}
fn default_db_path() -> String {
    "oxygenrouter.db".to_string()
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_max_retries() -> i32 {
    3
}
fn default_retry_delay_ms() -> i64 {
    500
}
fn default_retry_backoff() -> String {
    "exponential".to_string()
}
fn default_upstream_timeout_ms() -> u64 {
    120_000
}
fn default_user_agent() -> String {
    "OxygenRouter/0.1.0".to_string()
}
fn default_max_concurrent_requests() -> i32 {
    64
}
fn default_request_log_retention_days() -> i32 {
    30
}
fn default_theme() -> String {
    "dark".to_string()
}
fn default_language() -> String {
    "auto".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub model: String,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelTestResult {
    pub channel_id: String,
    pub success: bool,
    pub latency_ms: Option<i64>,
    pub error: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStatus {
    /// Public: the WebUI landing page shows this before anyone signs in, and
    /// `/api/status` is a public route.
    ///
    /// Deliberately **not** here: the local API token and the bind address. Those
    /// were part of this payload and were served to anonymous callers, which made
    /// `GET /api/status` — the readiness probe, reachable by anyone who can open
    /// the port — a credential disclosure for the whole gateway. An operator who
    /// needs them signs in; `GET /api/system/info` is root-only and returns them.
    pub version: String,
    pub uptime_seconds: u64,
    pub total_channels: i64,
    pub enabled_channels: i64,
    pub total_requests: i64,
    pub active_requests: i64,
    /// The bind address, filled in only for an authenticated caller.
    ///
    /// Absent from the anonymous payload because it describes the instance's
    /// deployment, which is not a signed-out visitor's business; `None` means
    /// "not disclosed to you" rather than "not configured".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen_port: Option<u16>,
    /// The configured display currency, so a signed-out pricing page formats
    /// amounts the way the operator set them rather than assuming USD.
    ///
    /// Public on purpose: it is a display preference, not a credential, and the
    /// plan catalogue is served before sign-in.
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub range_start: String,
    pub range_end: String,
    pub bucket_seconds: i64,
    pub total_requests: i64,
    pub successful_requests: i64,
    pub failed_requests: i64,
    pub success_rate: f64,
    pub average_latency_ms: f64,
    pub today_requests: i64,
    pub today_errors: i64,
    pub total_tokens: i64,
    pub active_channels: i64,
    pub model_breakdown: Vec<DashboardBreakdown>,
    pub channel_breakdown: Vec<DashboardBreakdown>,
    pub api_key_breakdown: Vec<DashboardBreakdown>,
    pub recent_requests: Vec<RequestLog>,
    pub time_series: Vec<TimeBucket>,
    pub model_time_series: Vec<ModelTimeBucket>,
    pub api_key_time_series: Vec<ModelTimeBucket>,
    pub channel_perf: Vec<ChannelPerf>,
    pub time_range: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeBucket {
    pub label: String,
    pub requests: i64,
    pub errors: i64,
    pub latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelTimeBucket {
    pub label: String,
    pub model: String,
    pub requests: i64,
    pub errors: i64,
    pub tokens: i64,
    pub latency_ms: f64,
}

#[derive(Debug, Clone, Default)]
pub struct AnalyticsFilters {
    pub model: Option<String>,
    pub channel_id: Option<String>,
    pub api_key_id: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsFlow {
    pub range_start: String,
    pub range_end: String,
    pub nodes: Vec<AnalyticsFlowNode>,
    pub links: Vec<AnalyticsFlowLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsFlowNode {
    pub id: String,
    pub label: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsFlowLink {
    pub source: String,
    pub target: String,
    pub request_count: i64,
    pub tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPerf {
    pub channel_id: String,
    pub channel_name: String,
    pub requests: i64,
    pub errors: i64,
    pub latency_ms: f64,
    pub last_error: Option<String>,
    pub last_test_at: Option<i64>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardBreakdown {
    pub name: String,
    pub requests: i64,
    pub errors: i64,
    pub average_latency_ms: f64,
    pub tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyUsage {
    pub api_key_id: String,
    pub api_key_name: String,
    pub enabled: bool,
    pub total_requests: i64,
    pub successful_requests: i64,
    pub total_tokens: i64,
    pub average_latency_ms: f64,
}

/// Persistent model metadata registry (NewAPI `models` table parity).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMetadata {
    #[serde(default)]
    pub id: String,
    pub model_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub endpoints: Vec<String>,
    /// 0 exact, 1 prefix, 2 contains, 3 suffix
    #[serde(default)]
    pub name_rule: i32,
    #[serde(default = "default_one")]
    pub status: i32,
    #[serde(default = "default_one")]
    pub sync_official: i32,
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
}

fn default_one() -> i32 {
    1
}

/// A model vendor / supplier (NewAPI `vendors` table parity).
///
/// The reference keeps 44 of these and shows `model_count` on the list, computed
/// from the model registry rather than stored (`model/vendor_meta.go:15-25`), so a
/// rename or delete cannot leave the count stale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vendor {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// lucide icon name, e.g. `OpenAI` / `Claude.Color`.
    #[serde(default)]
    pub icon: String,
    #[serde(default = "default_one")]
    pub status: i32,
    /// Derived, not stored: how many registry models name this vendor.
    #[serde(default)]
    pub model_count: i64,
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
}

/// Multi-key management view for a single key slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelKeyStatus {
    pub index: usize,
    pub preview: String,
    pub status: i32,
    pub disabled_reason: Option<String>,
    pub disabled_time: Option<i64>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    /// The instance owner. The reference separates root from admin because some
    /// operations (option writes, task plugins, system tasks) are root-only.
    Root,
    Admin,
    User,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Admin => "admin",
            Self::User => "user",
        }
    }
    pub fn from_db(value: &str) -> Self {
        match value {
            "root" => Self::Root,
            "admin" => Self::Admin,
            _ => Self::User,
        }
    }

    /// Whether this role may perform administrative operations.
    ///
    /// Root is a superset of admin, so a root check passes for both.
    pub fn is_admin(&self) -> bool {
        matches!(self, Self::Root | Self::Admin)
    }

    /// Whether this role is the instance owner.
    pub fn is_root(&self) -> bool {
        matches!(self, Self::Root)
    }

    /// Privilege rank, ordered so a higher number may manage a lower one.
    ///
    /// The values mirror the reference's constants (`root = 100`, `admin = 10`,
    /// `user = 1`), because its checks are ordinal comparisons rather than set
    /// membership: `canManageTargetRole` is `myRole == root || myRole > targetRole`.
    pub fn rank(&self) -> u8 {
        match self {
            Self::Root => 100,
            Self::Admin => 10,
            Self::User => 1,
        }
    }

    /// Whether `actor` may administer an account holding `target`.
    ///
    /// Ports the reference's `canManageTargetRole` (`controller/user.go:382`):
    /// root may manage anyone, and otherwise the actor must outrank the target.
    /// Without this an ordinary admin can act on a peer or on the owner — which is
    /// how an admin could promote itself to root by editing its own row.
    pub fn can_manage(actor: &Self, target: &Self) -> bool {
        actor.is_root() || actor.rank() > target.rank()
    }

    /// Whether `actor` may assign `assigned` when creating an account.
    ///
    /// Ports the reference's `CreateUser` guard (`controller/user.go:987`), which
    /// refuses `assigned >= actor`. Strictly-below is deliberate: an admin must
    /// not mint another admin, or the hierarchy is not a hierarchy.
    pub fn can_assign(actor: &Self, assigned: &Self) -> bool {
        assigned.rank() < actor.rank()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub email: String,
    pub role: UserRole,
    pub status: String,
    pub balance_micros: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    pub id: String,
    pub user_id: String,
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub id: String,
    pub user_id: String,
    pub amount_micros: i64,
    pub balance_after_micros: i64,
    pub kind: String,
    pub description: String,
    pub reference_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionPlan {
    pub id: String,
    pub name: String,
    pub description: String,
    pub price_micros: i64,
    pub quota_micros: i64,
    pub duration_days: i64,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub user_id: String,
    pub plan_id: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    /// The quota pool this subscription grants, snapshotted from the plan when it
    /// was created. `0` means unlimited, matching the reference's `TotalAmount`
    /// semantics.
    #[serde(default)]
    pub amount_total: i64,
    /// How much of the pool has been consumed.
    #[serde(default)]
    pub amount_used: i64,
}

impl Subscription {
    /// Quota still available. `None` means unlimited.
    pub fn remaining(&self) -> Option<i64> {
        if self.amount_total <= 0 {
            None
        } else {
            Some((self.amount_total - self.amount_used).max(0))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedemptionCode {
    pub id: String,
    pub code: String,
    pub amount_micros: i64,
    pub plan_id: Option<String>,
    pub enabled: bool,
    pub max_uses: i64,
    pub used_count: i64,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentOrder {
    pub id: String,
    pub user_id: String,
    pub amount_micros: i64,
    pub provider: String,
    pub status: String,
    pub external_reference: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticatedUser {
    pub user: User,
    pub expires_at: DateTime<Utc>,
}
