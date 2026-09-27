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
    }

    /// Re-read pricing overrides from the option store.
    ///
    /// Called at startup and after an admin edits pricing, so a running instance
    /// picks up new expressions without a restart.
    pub fn reload_pricing(&self) {
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
    use super::parse_port_list;

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
