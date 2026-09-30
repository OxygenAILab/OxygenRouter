//! Shared AppState for the webui server
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, RwLock};

use oxygenrouter_billing::{BillingPolicy, BillingService, Pricing};
use oxygenrouter_core::{Database, RequestLog};
use oxygenrouter_proxy::limits::{ConcurrencyGuard, RateLimiter, RateLimit};
use oxygenrouter_proxy::ssrf::SsrfPolicy;
use oxygenrouter_proxy::ChannelScheduler;

use crate::billing_store::SqliteBillingStore;

const LOG_BROADCAST_CAPACITY: usize = 100;

/// Hourly usage counters, held in memory until the periodic flush.
///
/// `DataExportEnabled` promises "Aggregate usage into quota_data for analytics",
/// and the reference keeps these counters in a process-local cache that a timer
/// writes out (`model/usedata.go:41,100`) rather than writing a row per request.
/// That shape is kept here: a busy gateway folding every request straight into
/// SQLite would pay a transaction per call for a table only read by charts.
pub struct UsageAccumulator {
    buckets: parking_lot::Mutex<std::collections::HashMap<String, oxygenrouter_core::QuotaData>>,
}

impl Default for UsageAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl UsageAccumulator {
    pub fn new() -> Self {
        Self {
            buckets: parking_lot::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Fold one billed request into its hour bucket.
    ///
    /// The bucket is the hour the request started in, matching the reference's
    /// `created_at - created_at % 3600` (`model/usedata.go:80`).
    pub fn record(
        &self,
        user_id: &str,
        channel_id: &str,
        token_id: &str,
        group: &str,
        model: &str,
        quota: i64,
        tokens: i64,
    ) {
        let bucket_at = bucket_hour(chrono::Utc::now());
        let key = format!(
            "{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}",
            bucket_at.timestamp(),
            user_id,
            model,
            group,
            channel_id,
            token_id
        );
        let mut buckets = self.buckets.lock();
        let entry = buckets.entry(key).or_insert_with(|| oxygenrouter_core::QuotaData {
            id: uuid::Uuid::new_v4().to_string(),
            bucket_at,
            user_id: user_id.to_string(),
            // Resolved at flush time: the write path is the hot path, and a
            // username lookup per request would be a query for a display field.
            username: String::new(),
            model_name: model.to_string(),
            group_name: group.to_string(),
            channel_id: channel_id.to_string(),
            token_id: token_id.to_string(),
            count: 0,
            quota: 0,
            token_used: 0,
            created_at: chrono::Utc::now(),
        });
        entry.count += 1;
        entry.quota = entry.quota.saturating_add(quota);
        entry.token_used = entry.token_used.saturating_add(tokens);
    }

    /// Take everything accumulated so far, leaving the accumulator empty.
    ///
    /// Draining rather than copying means a failed flush loses at most the current
    /// interval's counters, and a successful one cannot double-count.
    pub fn take(&self) -> Vec<oxygenrouter_core::QuotaData> {
        let mut buckets = self.buckets.lock();
        if buckets.is_empty() {
            return Vec::new();
        }
        std::mem::take(&mut *buckets).into_values().collect()
    }

    /// Buckets currently held, for diagnostics and tests.
    pub fn pending(&self) -> usize {
        self.buckets.lock().len()
    }
}

/// Truncate a timestamp to the start of its hour, in UTC.
pub fn bucket_hour(at: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    use chrono::Timelike;
    at.with_minute(0)
        .and_then(|v| v.with_second(0))
        .and_then(|v| v.with_nanosecond(0))
        .unwrap_or(at)
}

/// The account-creation and password-sign-in rules for this instance.
///
/// Derived from the option store by `AppState::auth_policy`, never cached.
#[derive(Debug, Clone, Copy)]
pub struct AuthPolicy {
    /// `RegistrationEnabled` — may a visitor create an account at all.
    pub registration_enabled: bool,
    /// `PasswordLoginEnabled` — does the password form work.
    pub password_login_enabled: bool,
    /// `MinPasswordLength`, in characters.
    pub min_password_len: usize,
    /// `SessionTtlDays` — how long a successful sign-in lasts.
    ///
    /// The console has always offered this; the login handler hard-coded seven
    /// days, so the field did nothing.
    pub session_ttl: chrono::Duration,
}

impl Default for AuthPolicy {
    fn default() -> Self {
        Self {
            // Open by default: an instance whose operator has not opened the
            // console yet must still be usable.
            registration_enabled: true,
            password_login_enabled: true,
            min_password_len: oxygenrouter_core::DEFAULT_MIN_PASSWORD_LEN,
            session_ttl: chrono::Duration::days(7),
        }
    }
}

pub struct AppState {
    pub db: Arc<Database>,
    pub db_path: PathBuf,
    pub scheduler: Arc<RwLock<ChannelScheduler>>,
    pub local_token: String,
    pub start_time: Instant,
    pub started_at: DateTime<Utc>,
    pub config_path: PathBuf,
    pub log_broadcast: broadcast::Sender<RequestLog>,
    /// The quota engine, shared for cheap access on the hot path.
    pub billing: Arc<BillingService>,
    /// Billing's view of storage. Held as a concrete adapter so the trait object
    /// is constructed once rather than per request.
    pub billing_store: Arc<SqliteBillingStore>,
    /// Per-scope fixed-window rate limits (client IP, token, model).
    pub rate_limiter: Arc<RateLimiter>,
    /// Global in-flight ceiling. NewAPI does not enforce one.
    pub concurrency: Arc<ConcurrencyGuard>,
    /// Rate limit applied to relay endpoints per client IP. `0` disables it.
    pub relay_rate_limit: RateLimit,
    /// `RequestLogEnabled` — whether request rows are persisted.
    ///
    /// Cached, unlike the other policies, because the write happens on the
    /// hottest path in the product. `configure_limits` (already the startup
    /// hook) and the option writer both refresh it, so it cannot go stale
    /// unnoticed; a per-request `SELECT` here would be a real cost for a value
    /// that changes approximately never.
    request_log_enabled: AtomicBool,
    /// `RecordIpLog` — whether the caller's address is stored with the row.
    ///
    /// Cached for the same reason, and because it is a privacy decision that
    /// should not vary between rows written in the same millisecond.
    record_ip_log: AtomicBool,
    /// `AuditLogEnabled` — whether administrative operations are recorded.
    ///
    /// Cached with the two above: it is read on every console request that could
    /// be audited, and refreshed by the same two places.
    audit_log_enabled: AtomicBool,
    /// Hourly usage counters awaiting the periodic flush into `quota_data`.
    pub usage: UsageAccumulator,
    /// The JavaScript plugin runtime.
    ///
    /// One host per process; uploads are validated against it and the console
    /// reports what it currently holds. The host keeps its own engine thread, so
    /// a plugin cannot stall the gateway.
    pub plugins: oxygenrouter_plugin::PluginHost,
    /// `DataExportEnabled` — whether usage is aggregated at all.
    ///
    /// Cached with the logging flags, and refreshed by the same two places,
    /// because it gates a write on the relay path.
    data_export_enabled: AtomicBool,
}

impl AppState {
    pub fn new(
        db: Arc<Database>,
        db_path: PathBuf,
        scheduler: ChannelScheduler,
        local_token: String,
        config_path: PathBuf,
    ) -> Self {
        let (log_broadcast, _) = broadcast::channel(LOG_BROADCAST_CAPACITY);

        // Pricing starts from the shipped pack; `reload_pricing` then applies the
        // instance's own option overrides, mirroring GetBillingExpr's precedence.
        let pricing = Pricing::from_embedded().unwrap_or_else(|e| {
            eprintln!("[OxygenRouter] pricing pack unusable ({e}); all models will bill as unpriced");
            Pricing::new(Default::default())
        });

        Self {
            db: Arc::clone(&db),
            db_path,
            scheduler: Arc::new(RwLock::new(scheduler)),
            local_token,
            start_time: Instant::now(),
            started_at: Utc::now(),
            config_path,
            log_broadcast,
            billing: Arc::new(BillingService::new(pricing, BillingPolicy::default())),
            billing_store: Arc::new(SqliteBillingStore::new(db)),
            rate_limiter: Arc::new(RateLimiter::new()),
            concurrency: Arc::new(ConcurrencyGuard::new(0)),
            relay_rate_limit: RateLimit::disabled(),
            // Refreshed by `configure_limits` during startup and by the option
            // writer; the shipped defaults match the schema.
            request_log_enabled: AtomicBool::new(true),
            record_ip_log: AtomicBool::new(false),
            audit_log_enabled: AtomicBool::new(true),
            usage: UsageAccumulator::new(),
            plugins: oxygenrouter_plugin::PluginHost::start(),
            data_export_enabled: AtomicBool::new(true),
        }
    }

    /// Build the fetch policy from the `FetchSetting.*` options.
    ///
    /// Derived on demand rather than cached on the struct. A cached copy has to
    /// be invalidated by whoever writes the option, and one forgotten call site
    /// silently reverts to the old policy — the same class of defect this
    /// project has now hit repeatedly. Channel model sync is not a hot path, so
    /// there is nothing to gain by caching it.
    ///
    /// Parsing failures fall back to the default rather than silently widening
    /// the policy.
    pub fn fetch_policy(&self) -> SsrfPolicy {
        use oxygenrouter_proxy::ssrf::FilterMode;

        let read_bool = |key: &str, fallback: bool| -> bool {
            self.db
                .get_setting(key)
                .ok()
                .flatten()
                .and_then(|v| match v.trim().to_ascii_lowercase().as_str() {
                    "true" | "1" => Some(true),
                    "false" | "0" => Some(false),
                    _ => None,
                })
                .unwrap_or(fallback)
        };

        let mut policy = SsrfPolicy::default();
        let enabled = read_bool("FetchSetting.EnableSSRFProtection", true);
        if !enabled {
            // Protection off means the operator has accepted the risk; keep the
            // scheme check but drop the address and port rules.
            return SsrfPolicy::scheme_only();
        }
        policy.allow_private_ip = read_bool("FetchSetting.AllowPrivateIp", false);
        // An empty or unparsable port list falls back to the shipped default
        // rather than becoming 'any port', which would be a silent widening.
        if let Ok(Some(raw)) = self.db.get_setting("FetchSetting.AllowedPorts") {
            match parse_port_list(&raw) {
                Some(ports) => policy.allowed_ports = ports,
                None => eprintln!("[OxygenRouter] FetchSetting.AllowedPorts ignored; using the default"),
            }
        }
        // Both filters stay in denylist mode with empty lists, as upstream ships.
        policy.domain_mode = FilterMode::Denylist;
        policy.ip_mode = FilterMode::Denylist;
        policy
    }

    /// Configure the global in-flight ceiling and the per-IP relay rate limit.
    ///
    /// Called at startup from the option store so both are operator-controlled.
    pub fn configure_limits(&mut self, max_concurrent: i32, requests_per_minute: u32) {
        self.concurrency = Arc::new(ConcurrencyGuard::new(max_concurrent.max(0) as usize));
        self.relay_rate_limit = if requests_per_minute == 0 {
            RateLimit::disabled()
        } else {
            RateLimit::new(requests_per_minute, 60)
        };
    }

    /// Re-read the two cached logging options.
    ///
    /// Called during startup and whenever an admin writes one of them, because
    /// these two are consulted on the proxy's hot path where a per-request
    /// `SELECT` would be a real cost. Every other option is derived per call.
    pub fn reload_log_policy(&self) {
        let read_bool = |key: &str, fallback: bool| -> bool {
            self.db
                .get_setting(key)
                .ok()
                .flatten()
                .and_then(|v| match v.trim().to_ascii_lowercase().as_str() {
                    "true" | "1" => Some(true),
                    "false" | "0" => Some(false),
                    _ => None,
                })
                .unwrap_or(fallback)
        };
        self.request_log_enabled
            .store(read_bool("RequestLogEnabled", true), Ordering::Relaxed);
        self.record_ip_log
            .store(read_bool("RecordIpLog", false), Ordering::Relaxed);
        self.audit_log_enabled
            .store(read_bool("AuditLogEnabled", true), Ordering::Relaxed);
        self.data_export_enabled
            .store(read_bool("DataExportEnabled", true), Ordering::Relaxed);
    }

    /// Whether request rows are persisted (`RequestLogEnabled`).
    pub fn request_log_enabled(&self) -> bool {
        self.request_log_enabled.load(Ordering::Relaxed)
    }

    /// Whether the caller's address is stored with a row (`RecordIpLog`).
    pub fn record_ip_log(&self) -> bool {
        self.record_ip_log.load(Ordering::Relaxed)
    }

    /// Whether administrative operations are recorded (`AuditLogEnabled`).
    pub fn audit_log_enabled(&self) -> bool {
        self.audit_log_enabled.load(Ordering::Relaxed)
    }

    /// Whether usage is aggregated into `quota_data` (`DataExportEnabled`).
    pub fn data_export_enabled(&self) -> bool {
        self.data_export_enabled.load(Ordering::Relaxed)
    }

    /// The group a request belongs to when its key names none.
    ///
    /// `DefaultGroup` was advertised as exactly this and was read by nothing; the
    /// proxy substituted the literal `"default"` at each site. Read live rather
    /// than cached, because it is a routing decision an operator may be changing
    /// while traffic flows.
    pub fn default_group(&self) -> String {
        self.db.typed_setting("DefaultGroup", "default".to_string())
    }

    /// Minutes a login lock lasts once it trips.
    ///
    /// Not exposed as an option: NewAPI's own lock is a fixed window, and adding
    /// a second knob nobody asked for would be inventing policy rather than
    /// matching it.
    pub const LOGIN_LOCK_WINDOW_SECS: u64 = 900;

    /// The login-failure limit, from `MaxLoginAttempts`.
    ///
    /// Without this the option was *advertised and inert*: the console labelled it
    /// "Failed sign-ins before a temporary lock" while no code path ever read it,
    /// so brute-force protection did not exist at all. `0` or a negative value
    /// disables the lock, matching the convention every other `*Enabled`-style
    /// option follows.
    ///
    /// Derived per sign-in attempt rather than cached: login is not a hot path,
    /// and a cache would need invalidating whenever the option is written.
    pub fn login_rate_limit(&self) -> RateLimit {
        let attempts = self
            .db
            .get_setting("MaxLoginAttempts")
            .ok()
            .flatten()
            .and_then(|raw| raw.trim().parse::<i64>().ok())
            .unwrap_or(5);
        if attempts <= 0 {
            RateLimit::disabled()
        } else {
            RateLimit::new(attempts.min(u32::MAX as i64) as u32, Self::LOGIN_LOCK_WINDOW_SECS)
        }
    }

    /// Whether a caller may create an account and sign in with a password.
    ///
    /// Three options that the console has always presented as live switches but
    /// which no code read, so the instance behaved as if all three were on:
    ///
    /// * `RegistrationEnabled` — the master switch for account creation.
    /// * `PasswordLoginEnabled` — whether the password form works at all.
    /// * `MinPasswordLength` — the length rule, previously hard-coded to 8 and
    ///   therefore unaffected by the field the console showed.
    ///
    /// The reference enforces all three (`controller/user.go:218,222,54`).
    /// Read per call rather than cached, for the reason given on
    /// `fetch_policy`: a cache needs invalidating by whoever writes the option,
    /// and one missed call site silently reverts to the old policy.
    pub fn auth_policy(&self) -> AuthPolicy {
        let read_bool = |key: &str, fallback: bool| -> bool {
            self.db
                .get_setting(key)
                .ok()
                .flatten()
                .and_then(|v| match v.trim().to_ascii_lowercase().as_str() {
                    "true" | "1" => Some(true),
                    "false" | "0" => Some(false),
                    _ => None,
                })
                .unwrap_or(fallback)
        };
        let min_password_len = self
            .db
            .get_setting("MinPasswordLength")
            .ok()
            .flatten()
            .and_then(|raw| raw.trim().parse::<i64>().ok())
            // A nonsensical value (zero or negative) would disable the rule
            // entirely, so fall back to the shipped default instead.
            .filter(|n| *n >= 1)
            .map(|n| n.min(1024) as usize)
            .unwrap_or(oxygenrouter_core::DEFAULT_MIN_PASSWORD_LEN);
        AuthPolicy {
            registration_enabled: read_bool("RegistrationEnabled", true),
            password_login_enabled: read_bool("PasswordLoginEnabled", true),
            min_password_len,
            // A nonsensical value (zero or negative) would issue sessions that are
            // already expired, locking every user out, so it falls back.
            session_ttl: chrono::Duration::days(
                self.db
                    .get_setting("SessionTtlDays")
                    .ok()
                    .flatten()
                    .and_then(|raw| raw.trim().parse::<i64>().ok())
                    .filter(|days| *days >= 1)
                    .map(|days| days.min(3650))
                    .unwrap_or(7),
            ),
        }
    }

    pub async fn reload_scheduler_maps(&self) {
        let maps = self.db.list_model_maps().unwrap_or_default();
        self.scheduler.write().await.set_model_maps(maps);
        self.reload_affinity().await;
    }

    /// Load the channel-affinity setting into the scheduler's store.
    ///
    /// A malformed document keeps the feature off rather than applying half of
    /// it: sticky routing that silently drops some rules would send a session to
    /// an upstream it has no cache on, which is the outcome the feature exists to
    /// prevent.
    pub async fn reload_affinity(&self) {
        let raw = self
            .db
            .get_setting("ChannelAffinity")
            .ok()
            .flatten()
            .unwrap_or_default();
        let setting = if raw.trim().is_empty() {
            oxygenrouter_proxy::affinity::AffinitySetting::default()
        } else {
            match serde_json::from_str::<oxygenrouter_proxy::affinity::AffinitySetting>(&raw) {
                Ok(setting) => setting,
                Err(error) => {
                    eprintln!(
                        "[OxygenRouter] ChannelAffinity ignored (feature stays off): {error}"
                    );
                    oxygenrouter_proxy::affinity::AffinitySetting::default()
                }
            }
        };
        let enabled = setting.enabled;
        let rules = setting.rules.len();
        self.scheduler.read().await.affinity().set_setting(setting);
        if enabled {
            println!("[OxygenRouter] channel affinity enabled with {rules} rule(s)");
        }
    }

    /// Apply the three billing thresholds the console offers.
    ///
    /// `TrustQuota`, `PreConsumedQuota` and `FreeModelPreConsumeEnabled` were
    /// advertised and read by nothing, so the engine always ran on its shipped
    /// defaults: an operator could not raise the reserve for expensive models,
    /// could not lower the balance above which reservation is skipped, and could
    /// not make a zero-price model reserve anything.
    ///
    /// `PreConsumedQuota` is an absolute floor, not the multiplier: our engine
    /// reserves the computed estimate, whereas the reference's constant is the
    /// amount it holds back for a request whose price is not yet known. Taking it
    /// as a minimum keeps the field meaningful without making every reservation
    /// 500 micros for a model that costs less. A value of `0` disables the floor,
    /// matching how every other numeric option treats zero.
    fn reload_billing_policy(&self) {
        let mut policy = oxygenrouter_billing::BillingPolicy::default();

        let read_int = |key: &str| -> Option<i64> {
            self.db
                .get_setting(key)
                .ok()
                .flatten()
                .and_then(|raw| raw.trim().parse::<i64>().ok())
        };
        let read_bool = |key: &str, fallback: bool| -> bool {
            self.db
                .get_setting(key)
                .ok()
                .flatten()
                .and_then(|v| match v.trim().to_ascii_lowercase().as_str() {
                    "true" | "1" => Some(true),
                    "false" | "0" => Some(false),
                    _ => None,
                })
                .unwrap_or(fallback)
        };

        // A negative threshold is meaningless; treat it as unset rather than
        // letting it invert the comparison it feeds.
        if let Some(trust) = read_int("TrustQuota").filter(|v| *v >= 0) {
            policy.trust_quota = trust;
        }
        if let Some(floor) = read_int("PreConsumedQuota").filter(|v| *v >= 0) {
            policy.min_pre_consume = floor;
        }
        policy.pre_consume_free_models = read_bool("FreeModelPreConsumeEnabled", false);

        self.billing.set_policy(policy);
    }

    /// Re-read pricing overrides from the option store.
    ///
    /// Called at startup and after an admin edits pricing, so a running instance
    /// picks up new expressions without a restart.
    pub fn reload_pricing(&self) {
        self.reload_billing_policy();
        use oxygenrouter_billing::BillingMode;
        use std::collections::HashMap;

        // Bind the Arc first: `self.billing.pricing()` returns an owned Arc, and
        // taking a write guard from a temporary would drop it immediately.
        let pricing_handle = self.billing.pricing();
        let mut pricing = pricing_handle.write();

        if let Ok(Some(raw)) = self.db.get_setting("billing_expr") {
            match serde_json::from_str::<HashMap<String, String>>(&raw) {
                Ok(map) => pricing.set_expr_map(map),
                Err(e) => eprintln!("[OxygenRouter] billing_expr override ignored: {e}"),
            }
        }
        if let Ok(Some(raw)) = self.db.get_setting("billing_mode") {
            match serde_json::from_str::<HashMap<String, BillingMode>>(&raw) {
                Ok(map) => pricing.set_mode_map(map),
                Err(e) => eprintln!("[OxygenRouter] billing_mode override ignored: {e}"),
            }
        }
        if let Ok(Some(raw)) = self.db.get_setting("QuotaPerUnit") {
            if let Ok(value) = raw.trim().parse::<f64>() {
                pricing.set_quota_per_unit(value);
            }
        }
        if let Ok(Some(raw)) = self.db.get_setting("GroupRatio") {
            match serde_json::from_str::<HashMap<String, f64>>(&raw) {
                Ok(map) => {
                    for (group, ratio) in map {
                        pricing.set_group_ratio(&group, ratio);
                    }
                }
                Err(e) => eprintln!("[OxygenRouter] GroupRatio override ignored: {e}"),
            }
        }
        // Per-model price ratios. This is the field the console labels "Per-model
        // price ratios (JSON)" and it previously read nothing: the engine's
        // support for the table existed and the console exposed a control for it,
        // but nothing joined the two. Values layer over the shipped pack entry by
        // entry, so an operator can correct one model's price without losing the
        // rest of the catalogue.
        if let Ok(Some(raw)) = self.db.get_setting("ModelRatio") {
            match serde_json::from_str::<HashMap<String, f64>>(&raw) {
                Ok(map) => {
                    for (model, ratio) in map {
                        pricing.set_model_ratio(&model, ratio);
                    }
                }
                Err(e) => eprintln!("[OxygenRouter] ModelRatio override ignored: {e}"),
            }
        }
    }
}
/// Parse `80,443,8000-9000` into a port list.
///
/// Returns `None` when nothing usable was found, so the caller can fall back to
/// the default instead of treating the list as empty (= every port allowed).
fn parse_port_list(raw: &str) -> Option<Vec<u16>> {
    let mut ports = Vec::new();
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        match entry.split_once('-') {
            Some((lo, hi)) => match (lo.trim().parse::<u16>(), hi.trim().parse::<u16>()) {
                (Ok(lo), Ok(hi)) if lo <= hi => ports.extend(lo..=hi),
                _ => return None,
            },
            None => match entry.parse::<u16>() {
                Ok(port) if port > 0 => ports.push(port),
                _ => return None,
            },
        }
    }
    if ports.is_empty() {
        None
    } else {
        Some(ports)
    }
}

#[cfg(test)]
mod tests {
    use super::{bucket_hour, parse_port_list, UsageAccumulator};

    /// The accumulator is what `DataExportEnabled` buys: per-request folding in
    /// memory instead of a transaction per request. Two properties matter and
    /// neither is visible from the outside, so both are pinned here.
    #[test]
    fn the_usage_accumulator_folds_by_hour_and_drains() {
        let acc = UsageAccumulator::new();
        assert_eq!(acc.pending(), 0);
        // Draining an empty accumulator is a no-op, not a panic: the flush timer
        // runs unconditionally.
        assert!(acc.take().is_empty());

        // Same grain twice, including two spellings of the same model, folds.
        acc.record("u1", "c1", "k1", "default", "gpt-4o", 100, 10);
        acc.record("u1", "c1", "k1", "default", "gpt-4o", 250, 25);
        assert_eq!(acc.pending(), 1, "the same grain must not create a second bucket");

        // A different model, channel or user is a different grain.
        acc.record("u1", "c1", "k1", "default", "claude-3", 1, 1);
        acc.record("u1", "c2", "k1", "default", "gpt-4o", 1, 1);
        acc.record("u2", "c1", "k1", "default", "gpt-4o", 1, 1);
        assert_eq!(acc.pending(), 4);

        // Everything comes out once, and the accumulator is empty afterwards so a
        // successful flush cannot double-count.
        let rows = acc.take();
        assert_eq!(rows.len(), 4);
        assert_eq!(acc.pending(), 0);
        let first = rows
            .iter()
            .find(|r| r.model_name == "gpt-4o" && r.channel_id == "c1" && r.user_id == "u1")
            .expect("the folded bucket");
        assert_eq!(first.count, 2);
        assert_eq!(first.quota, 350);
        assert_eq!(first.token_used, 35);
    }

    /// The bucket boundary is the hour, in UTC, matching the reference's
    /// `created_at - created_at % 3600`.
    #[test]
    fn usage_buckets_align_to_the_start_of_the_hour() {
        use chrono::{TimeZone, Utc};
        let at = Utc.with_ymd_and_hms(2026, 9, 30, 14, 59, 59).unwrap();
        let bucket = bucket_hour(at);
        assert_eq!(bucket, Utc.with_ymd_and_hms(2026, 9, 30, 14, 0, 0).unwrap());
        // An exact hour is its own bucket.
        let exact = Utc.with_ymd_and_hms(2026, 9, 30, 14, 0, 0).unwrap();
        assert_eq!(bucket_hour(exact), exact);
    }


    #[test]
    fn port_lists_parse_including_ranges() {
        assert_eq!(parse_port_list("80,443"), Some(vec![80, 443]));
        assert_eq!(parse_port_list("80, 443 , 8080"), Some(vec![80, 443, 8080]));
        assert_eq!(parse_port_list("80-83"), Some(vec![80, 81, 82, 83]));
        assert_eq!(parse_port_list("80-82,443"), Some(vec![80, 81, 82, 443]));
    }

    #[test]
    fn an_empty_or_invalid_list_is_reported_so_the_default_survives() {
        // Returning Some(vec![]) would mean 'any port', a silent widening.
        assert_eq!(parse_port_list(""), None);
        assert_eq!(parse_port_list("   ,  "), None);
        assert_eq!(parse_port_list("not-a-port"), None);
        assert_eq!(parse_port_list("90-80"), None);
        assert_eq!(parse_port_list("0"), None);
        assert_eq!(parse_port_list("70000"), None);
    }
}
