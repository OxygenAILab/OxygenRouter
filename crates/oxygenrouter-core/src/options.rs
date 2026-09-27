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
    schema!("RetryTimes", Routing, Int, "3", "How many times a failed channel is retried"),
    schema!("RetryIntervalMs", Routing, Int, "500", "Base delay between retries"),
    schema!("RetryBackoff", Routing, String, "exponential", "fixed | linear | exponential"),
    schema!("ChannelDisableThreshold", Routing, Int, "3", "Consecutive failures before auto-disabling a channel"),
    schema!("ChannelAffinityEnabled", Routing, Bool, "false", "Prefer the last healthy channel for a token"),
    schema!("DefaultGroup", Routing, String, "default", "Group used when no token group is set"),
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
    schema!("ListenHost", Operations, String, "127.0.0.1", "Bind address for the HTTP server"),
    schema!("ListenPort", Operations, Int, "3001", "HTTP port for WebUI and proxy"),
    schema!("UpstreamTimeoutMs", Operations, Int, "120000", "Maximum duration of one upstream call"),
    schema!("MaxConcurrentRequests", Operations, Int, "64", "In-flight upstream request limit"),
    schema!("LogRetentionDays", Operations, Int, "30", "0 keeps logs forever"),
    schema!("LogLevel", Operations, String, "info", "trace | debug | info | warn | error"),
    schema!("RequestLogEnabled", Operations, Bool, "true", "Persist request logs"),
    schema!("RecordIpLog", Operations, Bool, "false", "Store client IP addresses on requests"),
    schema!("DataExportEnabled", Operations, Bool, "true", "Aggregate usage into quota_data for analytics"),
    schema!("UserAgent", Operations, String, "OxygenRouter/0.1.0", "User-Agent sent upstream"),
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
    schema!("ModelRatioOverride", Models, String, "{}", "Per-model price ratios (JSON)"),
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
