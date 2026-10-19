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
    // Display content, mirroring the reference's `console_setting` group: a
    // switch per block so an operator can hide it without deleting the copy.
    schema!("AnnouncementsEnabled", Site, Bool, "true", "Show announcements on the overview"),
    schema!(
        "Announcements",
        Site,
        String,
        "[]",
        "Announcements (JSON array of {content, publishDate, type, extra})"
    ),
    schema!("FaqEnabled", Site, Bool, "true", "Show the FAQ on the overview"),
    schema!("Faq", Site, String, "[]", "FAQ entries (JSON array of {question, answer})"),
    // The instance's own addresses, shown to a visitor beside the endpoint box.
    // Shape mirrors the reference's `console_setting.api_info`
    // (`{id, url, route, description, color}`).
    schema!("ApiInfoEnabled", Site, Bool, "true", "Show the API address list"),
    schema!(
        "ApiInfo",
        Site,
        String,
        "[]",
        "API addresses (JSON array of {url, route, description, color})"
    ),
    // Chat client presets. Each entry is a one-key object mapping a client name
    // to its URL template; `{address}` and `{key}` are resolved only when the
    // signed-in user explicitly asks to open a preset.
    schema!(
        "Chats",
        Site,
        String,
        "[]",
        "Chat client presets (JSON array of {client name: url template})"
    ),
    // Legal and about copy, served publicly like the reference's `/api/about`,
    // `/api/user-agreement` and `/api/privacy-policy`.
    schema!("About", Site, String, "", "About this instance (plain text)"),
    schema!("UserAgreement", Site, String, "", "Terms of use (plain text)"),
    schema!("PrivacyPolicy", Site, String, "", "Privacy policy (plain text)"),
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
    // Daily check-in, matching the reference's `checkin_setting`
    // (`setting/operation_setting/checkin_setting.go:12`): off by default, with
    // a quota range the award is drawn from. The reference's defaults are in
    // *its* quota units, so the range is expressed here in micros -- the unit
    // this instance's wallet uses -- as the USD values its comment states.
    schema!("CheckinEnabled", Operations, Bool, "false", "Let users claim a daily quota award"),
    schema!("CheckinMinQuota", Operations, Int, "2000", "Smallest daily check-in award, in micros (0.002 USD)"),
    schema!("CheckinMaxQuota", Operations, Int, "20000", "Largest daily check-in award, in micros (0.02 USD)"),
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
    // No `GlobalModelMapping`. The reference has no such key: its model rewriting
    // lives on the channel (`channel.ModelMapping`, relay/helper/model_mapped.go),
    // which is what this project's `model_maps` rows already are, complete with a
    // console page. A global layer on top would be a second place to configure the
    // same decision, and the key was never read in any case -- the same shape as
    // the other invented options removed from this list.
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
    // The reference keeps the human labels for a group in their own map
    // (`setting/user_usable_group.go:12`), separate from the ratio table, so a
    // group can be renamed in the console without touching its price.
    schema!("UserUsableGroups", Models, String, "{}", "Per-group display descriptions (JSON)"),
    // The key keeps its lowercase spelling because it *is* the storage key the
    // pricing loader has always read; renaming it to match this file's convention
    // would strand every value an existing instance has already stored here.
    schema!("billing_expr", Models, String, "{}", "Per-model billing expressions (JSON)"),
    // The billing-mode table has the same problem and is exposed for the same
    // reason: it drives pricing and was previously not in the console at all.
    schema!("billing_mode", Models, String, "{}", "Per-model billing mode override (JSON)"),
    schema!("UpstreamModelSyncEnabled", Models, Bool, "false", "Automatically refresh channel model lists"),
    schema!("ModelSyncIntervalMinutes", Models, Int, "60", "Minutes between automatic model syncs"),
    // Uptime-Kuma status panel. Matches the reference's `console_setting`
    // (`setting/console_setting/config.go`): a switch plus a JSON array of groups.
    schema!("UptimeKumaEnabled", Operations, Bool, "false", "Show the Uptime-Kuma status panel in the console"),
    schema!("UptimeKumaGroups", Operations, String, "[]", "Uptime-Kuma monitor groups (JSON array of {url, slug, categoryName})"),
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

/// Validate the raw shape of the JSON-list options the console edits as text.
///
/// Kind validation alone would accept `"not json"` for a string option and then
/// silently render an empty public list, which makes a typo look like a cleared
/// setting. These checks reject the write while the operator still has the
/// editor open.
fn validate_json_array(key: &str, value: &str) -> Result<(), String> {
    serde_json::from_str::<Vec<serde_json::Value>>(value)
        .map(|_| ())
        .map_err(|error| format!("{key} must be a JSON array: {error}"))
}

fn validate_chat_presets(value: &str) -> Result<(), String> {
    let entries: Vec<serde_json::Value> = serde_json::from_str(value)
        .map_err(|error| format!("Chats must be a JSON array: {error}"))?;
    for (index, entry) in entries.iter().enumerate() {
        let Some(object) = entry.as_object() else {
            return Err(format!("Chats entry {} must be an object", index + 1));
        };
        if object.len() != 1 {
            return Err(format!(
                "Chats entry {} must contain exactly one client name",
                index + 1
            ));
        }
        let Some((name, target)) = object.iter().next() else {
            return Err(format!("Chats entry {} is empty", index + 1));
        };
        if name.trim().is_empty() {
            return Err(format!("Chats entry {} has an empty client name", index + 1));
        }
        let Some(target) = target.as_str() else {
            return Err(format!(
                "Chats entry {} ({name}) must map to a URL string",
                index + 1
            ));
        };
        if target.trim().is_empty() {
            return Err(format!("Chats entry {} ({name}) has an empty URL", index + 1));
        }
    }
    Ok(())
}

/// Validate a value against both its kind and the shape implied by its key.
pub fn validate_for_key(key: &str, kind: OptionKind, value: &str) -> Result<(), String> {
    validate(kind, value)?;
    match key {
        "Chats" => validate_chat_presets(value),
        "ApiInfo" | "Announcements" | "Faq" | "UptimeKumaGroups" => {
            validate_json_array(key, value)
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_presets_require_one_named_url_per_entry() {
        assert!(validate_for_key(
            "Chats",
            OptionKind::String,
            r#"[{"Cherry Studio":"cherrystudio://providers"}]"#
        )
        .is_ok());
        assert!(validate_for_key("Chats", OptionKind::String, r#"[{"name":"url","extra":1}]"#)
            .is_err());
        assert!(validate_for_key("Chats", OptionKind::String, r#"[{"name":1}]"#).is_err());
        assert!(validate_for_key("Chats", OptionKind::String, "{}").is_err());
    }

    #[test]
    fn list_options_refuse_malformed_json_but_leave_plain_strings_alone() {
        assert!(validate_for_key("ApiInfo", OptionKind::String, "[]").is_ok());
        assert!(validate_for_key("ApiInfo", OptionKind::String, "not json").is_err());
        assert!(validate_for_key("SiteName", OptionKind::String, "not json").is_ok());
    }
}
