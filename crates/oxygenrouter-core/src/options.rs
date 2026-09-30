//! Typed option registry — NewAPI-style key-value system settings.
//!
//! Options live in the `settings` table as strings. This module describes the
//! schema (section, type, default, label) so the API can expose a structured
//! catalogue and validate writes, mirroring NewAPI's `GetOptions`/`UpdateOption`.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionKind {
    String,
    Int,
    Bool,
    Float,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionSection {
    Site,
    Auth,
    Routing,
    Billing,
    Operations,
    Security,
    Models,
    /// Values read once at startup, which therefore need a restart to take
    /// effect. Kept in their own group so the console can say so plainly instead
    /// of presenting them beside settings that apply immediately.
    Bootstrap,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionSchema {
    pub key: &'static str,
    pub section: OptionSection,
    pub kind: OptionKind,
    pub default: &'static str,
    pub description: &'static str,
    /// true when the value must never be echoed back to non-root callers
    pub secret: bool,
}

macro_rules! schema {
    ($key:literal, $section:ident, $kind:ident, $default:literal, $desc:literal) => {
        OptionSchema {
            key: $key,
            section: OptionSection::$section,
            kind: OptionKind::$kind,
            default: $default,
            description: $desc,
            secret: false,
        }
    };
    ($key:literal, $section:ident, $kind:ident, $default:literal, $desc:literal, secret) => {
        OptionSchema {
            key: $key,
            section: OptionSection::$section,
            kind: OptionKind::$kind,
            default: $default,
            description: $desc,
            secret: true,
        }
    };
}

/// The complete option catalogue. Order defines the UI presentation order.
pub const OPTION_SCHEMA: &[OptionSchema] = &[
    // Site
    schema!("SiteName", Site, String, "OxygenRouter", "Display name of this instance"),
    schema!("ServerAddress", Site, String, "http://127.0.0.1:3001", "Public base address used in examples"),
    schema!("Notice", Site, String, "", "Announcement shown on the overview page"),
    schema!("Footer", Site, String, "GitHub@OxygenAILab | OxygenAILab@StarsailsClover", "Footer watermark text"),
    // Auth
    schema!("RegistrationEnabled", Auth, Bool, "true", "Allow new users to register"),
    schema!("PasswordLoginEnabled", Auth, Bool, "true", "Allow password based sign-in"),
    schema!("SessionTtlDays", Auth, Int, "7", "Session lifetime in days"),
    schema!("MinPasswordLength", Auth, Int, "8", "Minimum accepted password length"),
    // Routing
    // Read once when the scheduler is built, so they belong with the other
    // startup values rather than beside the routing rules that apply live.
    schema!("RetryTimes", Bootstrap, Int, "3", "How many times a failed channel is retried"),
    schema!("RetryIntervalMs", Bootstrap, Int, "500", "Base delay between retries"),
    schema!("RetryBackoff", Bootstrap, String, "exponential", "fixed | linear | exponential"),
    schema!("ChannelDisableThreshold", Bootstrap, Int, "3", "Consecutive failures before auto-disabling a channel"),
    // Consulted per request, so it lives with the routing rules.
    schema!("DefaultGroup", Routing, String, "default", "Group used when no token group is set"),
    // A structured setting, not the boolean this key used to be. The reference's
    // feature is rule-driven session stickiness (`channel_affinity_setting.go`):
    // rules match a request by model, path and user agent, derive an identity from
    // its headers or body, and keep that identity on one channel for a TTL. A
    // switch cannot express which requests are sticky or on what identity, which
    // is how the old key came to be inert in the first place.
    schema!(
        "ChannelAffinity",
        Routing,
        String,
        "{\"enabled\":false,\"max_entries\":4096,\"default_ttl_seconds\":900,\"rules\":[]}",
        "Session stickiness: {enabled, session_mode, max_entries, default_ttl_seconds, rules[{name, model_patterns, path_patterns, user_agent_includes, key_sources[{type, name|path}], ttl_seconds, session_mode, include_model_name, include_rule_name, include_using_group}]}"
    ),
    // Billing
    schema!("QuotaPerUnit", Billing, Float, "500000", "Micros charged per quota unit"),
    schema!("PreConsumedQuota", Billing, Int, "500", "Reserved quota before a request settles"),
    schema!("TrustQuota", Billing, Int, "10000000", "Balance above which pre-consume is skipped"),
    schema!("FreeModelPreConsumeEnabled", Billing, Bool, "false", "Reserve quota even for zero-price models"),
    schema!("Currency", Billing, String, "USD", "Display currency for wallet amounts"),
    // Operations
    // --- SSRF protection (NewAPI `fetch_setting`) -------------------------
    // Applies to server-side fetches of a stored URL (channel model sync).
    // Provider base URLs used for relay are deliberately exempt, matching
    // NewAPI: they are operator-managed deployment targets that may legitimately
    // point at private networks.
    schema!("FetchSetting.EnableSSRFProtection", Security, Bool, "true", "Validate URLs before fetching them server-side"),
    schema!("FetchSetting.AllowPrivateIp", Security, Bool, "false", "Permit fetching private/loopback/link-local addresses (needed for a LAN upstream)"),
    schema!("FetchSetting.AllowedPorts", Security, String, "80,443,8080,8443", "Comma-separated ports, or ranges like 8000-9000; empty allows any"),
    schema!("ListenHost", Bootstrap, String, "127.0.0.1", "Bind address for the HTTP server"),
    schema!("ListenPort", Bootstrap, Int, "3001", "HTTP port for WebUI and proxy"),
    schema!("UpstreamTimeoutMs", Bootstrap, Int, "120000", "Maximum duration of one upstream call"),
    schema!("MaxConcurrentRequests", Bootstrap, Int, "64", "In-flight upstream request limit"),
    schema!("LogRetentionDays", Operations, Int, "30", "0 keeps logs forever"),
    schema!("LogLevel", Bootstrap, String, "info", "trace | debug | info | warn | error"),
    // Display preferences. They used to live only in `config.json`; the settings
    // table is now the authority, so they are declared here like every other
    // setting an operator can change from the console.
    schema!("Theme", Bootstrap, String, "dark", "Console theme"),
    schema!("Language", Bootstrap, String, "auto", "Console language"),
    schema!("RequestLogEnabled", Operations, Bool, "true", "Persist request logs"),
    schema!("RecordIpLog", Operations, Bool, "false", "Store client IP addresses on requests"),
    schema!("DataExportEnabled", Operations, Bool, "true", "Aggregate usage into quota_data for analytics"),
    // The reference pairs the switch with an interval (`model/usedata.go:47`),
    // because the counters are held in memory and only written out periodically.
    schema!("DataExportInterval", Operations, Int, "5", "Minutes between usage-summary flushes"),
    schema!("UserAgent", Bootstrap, String, "OxygenRouter/0.1.0", "User-Agent sent upstream"),
    // Security
    schema!("LocalApiToken", Security, String, "", "Bearer token required from local clients", secret),
    // No `IpAllowlistEnabled`. The reference has no such switch: a token's own
    // `allow_ips` is enforced whenever it is set (`middleware/auth.go:424`), and
    // an empty list means unrestricted. Offering a global switch here would let
    // an operator believe they had disabled a protection that is still active in
    // the only direction that matters, and its shipped default of `false` would
    // have contradicted the enforcement we already had.
    schema!("MaxLoginAttempts", Security, Int, "5", "Failed sign-ins before a temporary lock"),
    schema!("AuditLogEnabled", Security, Bool, "true", "Record administrative operations"),
    // Models
    schema!("GlobalModelMapping", Models, String, "{}", "Requested -> upstream model mapping (JSON)"),
    // The pricing tables the engine actually consumes. The previous pair here
    // was named for what the *engine* calls its override layer
    // (`ModelRatioOverride`) rather than for the option the reference stores,
    // and the reference spells these `ModelRatio` and `GroupRatio`
    // (`model/option.go:152,156`). More to the point, the write path read
    // `billing_expr` and `GroupRatio` while the console showed
    // `ModelRatioOverride`, so an operator editing the field they could see
    // changed nothing, and the setting that did work was invisible.
    schema!("ModelRatio", Models, String, "{}", "Per-model price ratio overrides (JSON)"),
    schema!("GroupRatio", Models, String, "{}", "Per-group price multipliers (JSON)"),
    // The key keeps its lowercase spelling because it *is* the storage key the
    // pricing loader has always read; renaming it to match this file's convention
    // would strand every value an existing instance has already stored here.
    schema!("billing_expr", Models, String, "{}", "Per-model billing expressions (JSON)"),
    // The billing-mode table has the same problem and is exposed for the same
    // reason: it drives pricing and was previously not in the console at all.
    schema!("billing_mode", Models, String, "{}", "Per-model billing mode override (JSON)"),
    schema!("UpstreamModelSyncEnabled", Models, Bool, "false", "Automatically refresh channel model lists"),
    schema!("ModelSyncIntervalMinutes", Models, Int, "60", "Minutes between automatic model syncs"),
];

pub fn find_schema(key: &str) -> Option<&'static OptionSchema> {
    OPTION_SCHEMA.iter().find(|entry| entry.key == key)
}

/// Validate a raw string against the declared option kind.
pub fn validate(kind: OptionKind, value: &str) -> Result<(), String> {
    match kind {
        OptionKind::String => Ok(()),
        OptionKind::Int => value
            .trim()
            .parse::<i64>()
            .map(|_| ())
            .map_err(|_| format!("expected an integer, got {value:?}")),
        OptionKind::Float => value
            .trim()
            .parse::<f64>()
            .map(|_| ())
            .map_err(|_| format!("expected a number, got {value:?}")),
        OptionKind::Bool => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "false" | "1" | "0" => Ok(()),
            other => Err(format!("expected a boolean, got {other:?}")),
        },
    }
}
