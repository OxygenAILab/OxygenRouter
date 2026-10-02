//! JavaScript plugin host: load a plugin, call its hooks out of process-safe
//! single-threaded state, and bound how long a hook may run.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover
//!
//! # Why this shape
//!
//! The reference implements its plugin system with `grafana/sobek`, a pure-Go
//! JavaScript engine (`pkg/jsplugin/engine.go:13`), so an operator can drop a
//! `.js` file in and the gateway picks it up with no toolchain. The Rust
//! equivalent is `boa_engine`, also pure Rust, so the same single-binary
//! guarantee holds and no C compiler is needed to build the project.
//!
//! `boa_engine::Context` is neither `Send` nor `Sync`. Instead of hiding that
//! behind a mutex and hoping, the engine lives on one dedicated thread and every
//! call is a message. That gives three things for free and on purpose:
//!
//! * a plugin that blocks its thread blocks only plugins, never the gateway;
//! * calls are serialised, so two hooks cannot interleave inside one engine;
//! * a timeout is enforced by abandoning the work rather than by trusting the
//!   script to yield.
//!
//! A plugin's script is untrusted code. It is not sandboxed against its own
//! engine — a plugin can loop forever, which is why the timeout exists — but it
//! has no access to the filesystem, the network or the process: the only values
//! it sees are the ones a hook call passes in, and the only thing it can return
//! is JSON.

use std::sync::mpsc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

mod responses;
mod routing;
mod task;
mod task_flow;

pub use responses::{
    decode_event_result, sse_frame, EventLimits, EventResult, ResponsesMachine, SemanticEvent,
    StreamEvent, RESPONSE_COMPLETED, RESPONSE_FAILED, RESPONSE_IN_PROGRESS, RESPONSE_INCOMPLETE,
};

pub use task_flow::{
    apply_image_inline, build_outbound_request, build_query_request, interpret_submit,
    interpret_task_result, plan_image_encoding, poll_once, send_submit, validate_descriptor,
    FlowError, HttpOutcome, ImageInline, OutboundRequest, PollRound, PollSettlement, PollTask,
    SubmitAnswer, TaskFlowContext, TaskTransport, TransportFuture,
};

pub use task::{
    build_request_body, classify_poll_http, image_base64, decide_poll, is_timed_out, known_status,
    parse_absolute_url, poll_failure_reason, replace_private_task_id, settle_plan,
    status_is_in_flight, status_is_terminal, terminal_refund_decision, valid_artifact_key,
    validate_content_request, validate_request_url, validate_task_artifacts, ArtifactContext,
    ClientRequest, ContentRequest, TaskArtifact, ARTIFACT_TYPES, MAX_TASK_ARTIFACTS,
    OutboundBody, PollClass, PollDecision, ResolvedFile, SettlePlan, DEFAULT_MAX_INLINE_FILE_BYTES,
    PROGRESS_SUBMITTED,
    QueryContext, RequestDescriptor, RequestPart, SimpleUrl, SubmitOutcome, TaskResult, TaskView,
    UpstreamKind, STATUS_FAILURE, STATUS_IN_PROGRESS, STATUS_NOT_START, STATUS_QUEUED,
    STATUS_SUBMITTED, STATUS_SUCCESS, STATUS_UNKNOWN,
};

pub use routing::{
    endpoint_index_key, file_reference, normalize_route_method, normalize_route_path,
    parse_file_reference,
    protocol_has_modes, required_modes, unsupported_form_message, BodyFile, BodyKind,
    EndpointClaim, EndpointIndex, ProtocolBinding, ProtocolContext, RequestContext,
};

/// Whether a version string is semver, by the reference's pattern
/// (`pkg/jsplugin/registry.go:40`): a release triple with optional pre-release
/// and build metadata.
fn is_semver(version: &str) -> bool {
    let (core, rest) = match version.split_once('+') {
        Some((core, build)) => {
            if build.is_empty() || !build.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            {
                return false;
            }
            (core, true)
        }
        None => (version, false),
    };
    let _ = rest;
    let core = match core.split_once('-') {
        Some((core, pre)) => {
            if pre.is_empty()
                || !pre
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            {
                return false;
            }
            core
        }
        None => core,
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    parts.iter().all(|part| {
        !part.is_empty()
            && part.chars().all(|c| c.is_ascii_digit())
            && (part == &"0" || !part.starts_with('0'))
    })
}

/// Whether a string is an absolute HTTP(S) URL with a host.
fn is_absolute_http_url(value: &str) -> bool {
    let Some((scheme, rest)) = value.split_once("://") else {
        return false;
    };
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
        return false;
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !host.contains('@')
}

/// Whether a string is an absolute HTTPS URL with a host and no credentials,
/// which is what a plugin's `website` must be
/// (`pkg/jsplugin/registry.go:1207`).
fn is_https_url(value: &str) -> bool {
    match value.split_once("://") {
        Some((scheme, rest)) => {
            scheme.eq_ignore_ascii_case("https")
                && !rest.is_empty()
                && !rest.split(['/', '?', '#']).next().unwrap_or("").contains('@')
                && !value.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\')
        }
        None => false,
    }
}

/// Validate locale-keyed display copy, the reference's `validateLocalizedText`
/// (`pkg/jsplugin/registry.go:2046`).
fn validate_localized_text(
    text: &LocalizedText,
    name: &str,
    max_chars: usize,
) -> Result<(), PluginError> {
    if text.0.is_empty() {
        return Ok(());
    }
    if text.0.len() > MAX_LOCALIZED_LOCALES {
        return Err(PluginError::Load(format!(
            "plugin meta {name} must not exceed {MAX_LOCALIZED_LOCALES} locales"
        )));
    }
    let mut canonical: Vec<String> = Vec::new();
    for (locale, value) in &text.0 {
        if !is_locale_tag(locale) {
            return Err(PluginError::Load(format!(
                "plugin meta {name} has invalid locale {locale:?}"
            )));
        }
        let canonical_locale = locale.to_ascii_lowercase();
        if canonical.contains(&canonical_locale) {
            return Err(PluginError::Load(format!(
                "plugin meta {name} has duplicate locale {canonical_locale:?}"
            )));
        }
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(PluginError::Load(format!(
                "plugin meta {name} value for {locale:?} must be a non-empty string"
            )));
        }
        if trimmed.chars().any(|c| c.is_control()) {
            return Err(PluginError::Load(format!(
                "plugin meta {name} value for {locale:?} must not contain control characters"
            )));
        }
        if trimmed.chars().count() > max_chars {
            return Err(PluginError::Load(format!(
                "plugin meta {name} must not exceed {max_chars} characters"
            )));
        }
        canonical.push(canonical_locale);
    }
    Ok(())
}

/// Whether a string is a locale tag by the reference's pattern
/// (`pkg/jsplugin/registry.go:41`): two or three letters, then
/// dash-separated alphanumeric subtags.
fn is_locale_tag(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let Some(first) = parts.next() else {
        return false;
    };
    if !(first.len() == 2 || first.len() == 3) || !first.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    parts.all(|part| {
        (2..=8).contains(&part.len()) && part.chars().all(|c| c.is_ascii_alphanumeric())
    })
}

/// The manifest API version this host understands.
///
/// Mirrors the reference's `APIVersion1` (`pkg/jsplugin/registry.go:29`), so a
/// manifest written for it is accepted here.
pub const API_VERSION_1: u32 = 1;

/// Default ceiling on one hook call, matching the reference's
/// `DefaultCallTimeout` (`pkg/jsplugin/engine.go:18`).
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Why a plugin could not be loaded or a hook could not run.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PluginError {
    #[error("plugin manifest is not valid JSON: {0}")]
    BadManifest(String),
    #[error("plugin declares apiVersion {found}, but this host speaks {expected}")]
    UnsupportedApiVersion { found: u32, expected: u32 },
    #[error("plugin key {0:?} is not a valid identifier")]
    BadKey(String),
    #[error("plugin meta has unknown field {0:?}")]
    UnknownMetaField(String),
    #[error("plugin source did not run: {0}")]
    Load(String),
    #[error("plugin {key:?} has no hook {hook:?}")]
    NoSuchHook { key: String, hook: String },
    #[error("plugin call did not finish within {0:?}")]
    Timeout(Duration),
    #[error("plugin {key:?} hook {hook:?} threw: {message}")]
    Hook {
        key: String,
        hook: String,
        message: String,
    },
    #[error("plugin host stopped")]
    HostStopped,
}

/// A plugin's declared metadata.
///
/// The field set is the reference's `Meta` (`pkg/jsplugin/registry.go:86`) minus
/// the usage/pricing half, which belongs to the billing work:
/// `requiredCapabilities`, `submitResponseTypes`, `sortPriority`, `website`,
/// `apiVersion`, `key`, `name`, `icon`, `description`, `version`, `author`,
/// `baseUrl`, `channelTypes`, `models`, `fetchMode`, `allowedHosts`,
/// `upstreams`, `routes`, `protocols`.
///
/// A routing mode a manifest declares. The reference keeps this on the *route*
/// (`Route.Type` / `Decode` / `Render`), not on the meta, so a mode is an entry
/// in [`PluginRoute`] rather than a separate list -- an extra list would be a
/// field the reference refuses as unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginRoute {
    pub method: String,
    pub path: String,
    /// `submit`, `query` or `dynamic` (`pkg/jsplugin/routing.go:19`).
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub action: String,
    /// The `native` export that decodes the request.
    #[serde(default)]
    pub decode: String,
    /// The `native` export that renders the response.
    #[serde(default)]
    pub render: String,
    /// The path parameter naming the task id, for a query route.
    #[serde(rename = "taskIdParam", default)]
    pub task_id_param: String,
}

/// A plugin's author, as the reference models it (`pkg/jsplugin/registry.go:177`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginAuthor {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
}

/// The `fetchMode` values a manifest may declare.
pub const FETCH_MODE_PER_TASK: &str = "per_task";
pub const FETCH_MODE_BATCH: &str = "batch";

/// The submission response encodings a plugin may declare. The default is `json`
/// and `sse` is the only other one (`pkg/jsplugin/registry.go:1084`).
pub const SUBMIT_TYPES: &[&str] = &["json", "sse"];

/// A capability the host either has or does not. Only these two exist, and a
/// manifest asking for anything else is refused (`pkg/jsplugin/json_state.go:16`).
pub const CAPABILITY_JSON_CLONE: &str = "json-clone@1";
pub const CAPABILITY_SUBMIT_SSE_DELTA: &str = "submit-sse-delta@1";
pub const CAPABILITIES: &[&str] = &[CAPABILITY_JSON_CLONE, CAPABILITY_SUBMIT_SSE_DELTA];

/// Upstream kinds a driver may address besides its own vendor
/// (`pkg/jsplugin/registry.go:117`).
pub const UPSTREAM_VENDOR: &str = "vendor";
pub const UPSTREAM_NEW_API: &str = "new_api";

/// The longest `description` value, in characters
/// (`pkg/jsplugin/registry.go:33`).
pub const MAX_DESCRIPTION_CHARS: usize = 512;
/// The longest `icon` value, in characters (`pkg/jsplugin/registry.go:1245`).
pub const MAX_ICON_CHARS: usize = 128;
/// The most locales one localized string may carry
/// (`pkg/jsplugin/registry.go:32`).
pub const MAX_LOCALIZED_LOCALES: usize = 16;

/// Display copy, locale-keyed.
///
/// Plugin source may write a bare string, which normalizes to `{"en": "..."}`,
/// or a map, which must include `en` (`pkg/jsplugin/registry.go:48,60`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct LocalizedText(pub std::collections::BTreeMap<String, String>);

impl LocalizedText {
    /// The English text, which every localized string is required to carry.
    pub fn english(&self) -> &str {
        self.0.get("en").map(String::as_str).unwrap_or("")
    }
}

impl<'de> Deserialize<'de> for LocalizedText {
    /// Either spelling the reference accepts: a bare string, normalized to
    /// `{"en": <text>}`, or an object of locales
    /// (`pkg/jsplugin/registry.go:60`).
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = LocalizedText;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string or an object of locales")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                let mut map = std::collections::BTreeMap::new();
                map.insert("en".to_string(), value.to_string());
                Ok(LocalizedText(map))
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Self::Value, A::Error> {
                let object: std::collections::BTreeMap<String, String> =
                    serde::Deserialize::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(LocalizedText(object))
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(LocalizedText(std::collections::BTreeMap::new()))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Every field name a manifest may carry. Anything else is refused, because a
/// silently dropped field is exactly how a manifest's `models` went missing
/// before: the plugin loaded, looked healthy, and bound nothing
/// (`pkg/jsplugin/registry.go:1006`).
pub const META_FIELDS: &[&str] = &[
    "requiredCapabilities",
    "submitResponseTypes",
    "sortPriority",
    "website",
    "apiVersion",
    "key",
    "name",
    "icon",
    "description",
    "version",
    "author",
    "baseUrl",
    "channelTypes",
    "channelType",
    "compatibleChannelTypes",
    "models",
    "fetchMode",
    "allowedHosts",
    "upstreams",
    "routes",
    "protocols",
    "usageSchema",
    "usageExamples",
    "usageProfiles",
    "auth",
    "endpoints",
    "submitPaths",
    "actions",
];

/// A plugin's declared metadata.
///
/// `Deserialize` is derived for the field-by-field read; entry is through
/// [`PluginManifest::from_meta`], which refuses unknown fields first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    #[serde(rename = "apiVersion")]
    pub api_version: u32,
    /// Canonical identifier; the reference restricts it to
    /// `^[a-z0-9][a-z0-9_-]*$` and at most 30 characters.
    pub key: String,
    pub name: String,
    pub version: String,
    /// Display copy, locale-keyed.
    #[serde(default)]
    pub description: LocalizedText,
    /// The models this plugin serves. Required, and at least one, because a
    /// binding names the model it serves: a plugin that declares none is bound
    /// to no endpoint at all (`pkg/jsplugin/registry.go:1287`).
    #[serde(default)]
    pub models: Vec<String>,
    /// How the host collects this plugin's upstream work.
    #[serde(rename = "fetchMode")]
    pub fetch_mode: String,
    /// The submission response encodings this plugin can parse.
    #[serde(rename = "submitResponseTypes", default)]
    pub submit_response_types: Vec<String>,
    /// Capabilities the host must have for this plugin to serve
    /// (`pkg/jsplugin/registry.go:1183`).
    #[serde(rename = "requiredCapabilities")]
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    /// Who wrote it. The name is required
    /// (`pkg/jsplugin/registry.go:1257`).
    #[serde(default)]
    pub author: PluginAuthor,
    /// A LobeHub icon name or short text; a URL or data URI is refused
    /// (`pkg/jsplugin/registry.go:1239`).
    #[serde(default)]
    pub icon: String,
    /// The plugin's own site, when it has one.
    #[serde(default)]
    pub website: String,
    /// The upstream base URL the plugin's hooks build against.
    #[serde(rename = "baseUrl")]
    #[serde(default)]
    pub base_url: String,
    /// Upstream hosts the plugin's descriptors may address
    /// (`pkg/jsplugin/registry.go:1235`).
    #[serde(rename = "allowedHosts")]
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    /// Upstream kinds this driver speaks besides its own vendor
    /// (`pkg/jsplugin/registry.go:1194`).
    #[serde(default)]
    pub upstreams: Vec<String>,
    /// Native routes the plugin serves. Retained as-is: binding them is the
    /// native-route work, and a route the host cannot yet serve is still the
    /// plugin's declaration to make, which the reference also validates
    /// (`pkg/jsplugin/registry.go:1331`).
    #[serde(default)]
    pub routes: Vec<PluginRoute>,
    /// Protocols this plugin claims to serve.
    #[serde(default)]
    pub protocols: Vec<ProtocolClaim>,
}

impl PluginManifest {
    /// Parse a manifest from its `meta` object, refusing unknown fields first.
    ///
    /// The reference checks the field set before it reads anything
    /// (`pkg/jsplugin/registry.go:1004`), because a manifest with a field this
    /// host does not understand is a manifest it cannot promise to honour.
    pub fn from_meta(value: &serde_json::Value) -> Result<Self, PluginError> {
        let object = value.as_object().ok_or_else(|| {
            PluginError::BadManifest("plugin meta must be an object".to_string())
        })?;
        for field in object.keys() {
            if !META_FIELDS.contains(&field.as_str()) {
                return Err(PluginError::UnknownMetaField(field.clone()));
            }
        }
        let mut manifest: PluginManifest = serde_json::from_value(value.clone())
            .map_err(|error| PluginError::BadManifest(error.to_string()))?;
        if manifest.submit_response_types.is_empty() {
            manifest.submit_response_types = vec![SUBMIT_TYPES[0].to_string()];
        }
        Ok(manifest)
    }

    /// Validate the fields the host relies on.
    fn validate(&self) -> Result<(), PluginError> {
        // The reference asks this first, before any other field
        // (`pkg/jsplugin/registry.go:1233`), so a manifest written for another
        // host is told what this host speaks rather than what it left out.
        if self.api_version != API_VERSION_1 {
            return Err(PluginError::UnsupportedApiVersion {
                found: self.api_version,
                expected: API_VERSION_1,
            });
        }
        if !valid_key(&self.key) {
            return Err(PluginError::BadKey(self.key.clone()));
        }
        if self.models.iter().all(|model| model.trim().is_empty()) {
            return Err(PluginError::Load(
                "plugin meta models must contain at least one model".to_string(),
            ));
        }
        if self.fetch_mode != FETCH_MODE_PER_TASK && self.fetch_mode != FETCH_MODE_BATCH {
            return Err(PluginError::Load(
                "plugin meta fetchMode must be per_task or batch".to_string(),
            ));
        }
        if self.author.name.trim().is_empty() {
            return Err(PluginError::Load(
                "plugin meta author name is required".to_string(),
            ));
        }
        if self.author.url.trim() != "" {
            let url = self.author.url.trim();
            if !(url.starts_with("http://") || url.starts_with("https://"))
                || url.split_once("://").map(|(_, rest)| rest).unwrap_or("").is_empty()
            {
                return Err(PluginError::Load(
                    "plugin meta author url must be an absolute HTTP(S) URL".to_string(),
                ));
            }
        }
        // The version is semver, and the reference says `must be semver` rather
        // than naming the pattern (`pkg/jsplugin/registry.go:1281`).
        if !is_semver(&self.version) {
            return Err(PluginError::Load(
                "plugin meta version must be semver".to_string(),
            ));
        }
        // An icon is a LobeHub name or short text. Shipping an image means
        // shipping an `icon.svg`, not inlining bytes in the manifest
        // (`pkg/jsplugin/registry.go:1239`).
        let icon = self.icon.trim();
        if icon.starts_with("data:") || icon.contains("://") {
            return Err(PluginError::Load(
                "plugin meta icon must be a LobeHub icon name or text; ship an image logo as an \
                 icon.svg or icon.png file next to plugin.js instead"
                    .to_string(),
            ));
        }
        if icon.chars().count() > MAX_ICON_CHARS {
            return Err(PluginError::Load(format!(
                "plugin meta icon must not exceed {MAX_ICON_CHARS} characters"
            )));
        }
        if icon.chars().any(|c| c.is_control()) {
            return Err(PluginError::Load(
                "plugin meta icon must not contain control characters".to_string(),
            ));
        }
        validate_localized_text(&self.description, "description", MAX_DESCRIPTION_CHARS)?;
        if !self.website.trim().is_empty() && !is_https_url(self.website.trim()) {
            return Err(PluginError::Load(
                "plugin meta website must be an absolute HTTPS URL without credentials".to_string(),
            ));
        }
        if self.base_url.trim() != "" && !is_absolute_http_url(self.base_url.trim()) {
            return Err(PluginError::Load(
                "plugin meta baseUrl must be an absolute HTTP(S) URL".to_string(),
            ));
        }
        // Capabilities have to exist, be unique, and be satisfiable together
        // (`pkg/jsplugin/registry.go:1183`).
        let mut seen_capabilities: Vec<&str> = Vec::new();
        for capability in &self.required_capabilities {
            if !CAPABILITIES.contains(&capability.as_str())
                || seen_capabilities.contains(&capability.as_str())
            {
                return Err(PluginError::Load(format!(
                    "unsupported or duplicate required capability {capability:?}"
                )));
            }
            if capability == CAPABILITY_SUBMIT_SSE_DELTA
                && !self.submit_response_types.iter().any(|kind| kind == "sse")
            {
                return Err(PluginError::Load(format!(
                    "{CAPABILITY_SUBMIT_SSE_DELTA} requires submitResponseTypes to include sse"
                )));
            }
            seen_capabilities.push(capability.as_str());
        }
        // Upstream kinds are only the two the reference knows, and only once each
        // (`pkg/jsplugin/registry.go:1194`).
        let mut seen_upstreams: Vec<&str> = Vec::new();
        for kind in &self.upstreams {
            if !(kind == UPSTREAM_VENDOR || kind == UPSTREAM_NEW_API)
                || seen_upstreams.contains(&kind.as_str())
            {
                return Err(PluginError::Load(format!(
                    "unsupported or duplicate upstream kind {kind:?}"
                )));
            }
            seen_upstreams.push(kind.as_str());
        }
        // Submission encodings: unique, and only the two that exist
        // (`pkg/jsplugin/registry.go:1084`).
        let mut seen_submit: Vec<&str> = Vec::new();
        for kind in &self.submit_response_types {
            if !SUBMIT_TYPES.contains(&kind.as_str()) || seen_submit.contains(&kind.as_str()) {
                return Err(PluginError::Load(format!(
                    "invalid or duplicate submitResponseTypes value {kind:?}"
                )));
            }
            seen_submit.push(kind.as_str());
        }
        // Native routes the plugin declares are validated even though this host
        // does not bind them yet: a manifest the reference accepts must load here
        // (`pkg/jsplugin/registry.go:1331`).
        let mut route_keys: Vec<String> = Vec::new();
        for route in &self.routes {
            if normalize_route_method(&route.method).is_none() {
                return Err(PluginError::Load(format!(
                    "plugin route method {:?} is not one of GET, POST, PUT, PATCH, DELETE",
                    route.method
                )));
            }
            if let Err(reason) = normalize_route_path(&route.path) {
                return Err(PluginError::Load(reason));
            }
            let key = format!("{} {}", route.method, route.path);
            if route_keys.contains(&key) {
                return Err(PluginError::Load(format!(
                    "plugin meta routes contain duplicate route {} {}",
                    route.method, route.path
                )));
            }
            route_keys.push(key);
        }
        Ok(())
    }

}

/// The reference's `pluginKeyPattern`, transcribed.
pub fn valid_key(key: &str) -> bool {
    if key.is_empty() || key.len() > 30 {
        return false;
    }
    let mut chars = key.chars();
    let first = chars.next().expect("checked non-empty");
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A command sent to the engine thread.
enum Call {
    Load {
        source: String,
        reply: mpsc::Sender<Result<PluginManifest, PluginError>>,
    },
    Hook {
        key: String,
        hook: String,
        /// JSON-encoded argument list.
        input: String,
        reply: mpsc::Sender<Result<String, PluginError>>,
    },
    /// A top-level export, which is where the driver hooks live
    /// (`pkg/jsplugin/registry.go:337`). A separate variant rather than a flag on
    /// `Hook`, because the two resolve through different objects and a caller that
    /// confused them would get a misleading "no such member".
    Export {
        key: String,
        hook: String,
        /// JSON-encoded argument list.
        input: String,
        reply: mpsc::Sender<Result<String, PluginError>>,
    },
}

/// Handle to the engine thread.
///
/// Cheap to clone; every clone shares the one engine.
#[derive(Clone)]
pub struct PluginHost {
    tx: mpsc::Sender<Call>,
}

impl PluginHost {
    /// Start a host with its own engine thread.
    pub fn start() -> Self {
        let (tx, rx) = mpsc::channel::<Call>();
        std::thread::Builder::new()
            .name("oxygenrouter-plugin".to_string())
            .spawn(move || engine_loop(rx))
            .expect("plugin thread");
        Self { tx }
    }

    /// Load a plugin from its source and return the manifest it declared.
    ///
    /// The source is expected to end by calling a global `register(manifest)`,
    /// which is how the reference's launcher contract works; if it never does,
    /// the plugin is rejected rather than silently contributing nothing.
    pub async fn load(&self, source: String, timeout: Duration) -> Result<PluginManifest, PluginError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Call::Load {
                source,
                reply: reply_tx,
            })
            .map_err(|_| PluginError::HostStopped)?;
        await_reply(reply_rx, timeout).await
    }

    /// Call one hook, passing and receiving JSON.
    pub async fn call_hook(
        &self,
        key: &str,
        hook: &str,
        input: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, PluginError> {
        self.call_with_args(key, hook, &[input], timeout).await
    }

    /// `protocols.<protocol>.decodeRequest(ctx)` — the client request, turned
    /// into the request the upstream should receive.
    ///
    /// The argument is the *protocol* name (`openai_responses`), which is the key
    /// the plugin exports under, not the plugin's own key. A caller holding a
    /// plugin key resolves it through that plugin's protocol claims.
    pub async fn decode_request(
        &self,
        protocol: &str,
        ctx: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, PluginError> {
        self.call_with_args(protocol, "decodeRequest", &[ctx], timeout).await
    }

    /// `protocols.<protocol>.renderFinal(ctx, task)` — the upstream's result,
    /// turned into what the client asked for.
    ///
    /// Two arguments because the reference passes both: the original request
    /// context, so a renderer can echo what the client sent, and the task the
    /// upstream produced.
    pub async fn render_final(
        &self,
        protocol: &str,
        ctx: serde_json::Value,
        task: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, PluginError> {
        self.call_with_args(protocol, "renderFinal", &[ctx, task], timeout).await
    }

    /// Call one hook with the exact argument list its contract states.
    ///
    /// A plugin's hooks are not uniform, and neither is where they live:
    ///
    /// * a *driver* hook (`buildSubmitRequest`, `parseSubmitResponse`,
    ///   `parseTaskResult`, `buildQueryRequest`, ...) is an ordinary top-level
    ///   export, and the reference calls it that way (`pkg/jsplugin/registry.go:337`);
    /// * a *protocol* hook (`decodeRequest`, `render`, `renderFinal`,
    ///   `renderEvents`) is a member of the plugin's `protocols` object for that
    ///   protocol (`registry.go:468`).
    ///
    /// The arity is the caller's to state, because the reference's differ:
    /// `buildQueryRequest(ctx)` takes one argument, `parseSubmitResponse(ctx,
    /// response)` two, and `parseTaskResult(ctx, body, response)` three
    /// (`relay/channel/task/jsplugin/adaptor.go:492,607,777`).
    pub async fn call_hook_args(
        &self,
        key: &str,
        hook: &str,
        args: &[serde_json::Value],
        timeout: Duration,
    ) -> Result<serde_json::Value, PluginError> {
        if DRIVER_HOOKS.contains(&hook) {
            return self.call_export_args(key, hook, args, timeout).await;
        }
        self.call_with_args(key, hook, args, timeout).await
    }

    /// Call a top-level export, with the timeout wrapper a public entry point
    /// needs.
    async fn call_export_args(
        &self,
        key: &str,
        hook: &str,
        args: &[serde_json::Value],
        timeout: Duration,
    ) -> Result<serde_json::Value, PluginError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Call::Export {
                key: key.to_string(),
                hook: hook.to_string(),
                input: serde_json::to_string(args).unwrap_or_else(|_| "[]".to_string()),
                reply: reply_tx,
            })
            .map_err(|_| PluginError::HostStopped)?;
        let raw = await_reply(reply_rx, timeout).await?;
        serde_json::from_str(&raw).map_err(|error| PluginError::Hook {
            key: key.to_string(),
            hook: hook.to_string(),
            message: format!("hook returned a value that is not JSON: {error}"),
        })
    }

    /// Send one call and decode the JSON it returned.
    async fn call_with_args(
        &self,
        key: &str,
        hook: &str,
        args: &[serde_json::Value],
        timeout: Duration,
    ) -> Result<serde_json::Value, PluginError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Call::Hook {
                key: key.to_string(),
                hook: hook.to_string(),
                input: serde_json::to_string(args).unwrap_or_else(|_| "[]".to_string()),
                reply: reply_tx,
            })
            .map_err(|_| PluginError::HostStopped)?;
        let raw = await_reply(reply_rx, timeout).await?;
        serde_json::from_str(&raw).map_err(|error| PluginError::Hook {
            key: key.to_string(),
            hook: hook.to_string(),
            message: format!("hook returned a value that is not JSON: {error}"),
        })
    }
}

/// Wait for the engine thread's answer, giving up after `timeout`.
///
/// The engine thread is not interrupted; a hook that never returns keeps its
/// thread busy and later calls queue behind it. That is a deliberate trade: the
/// alternative is letting a plugin block the caller indefinitely, and a plugin
/// busy-looping is a bug its author must fix either way.
async fn await_reply<T: Send + 'static>(
    reply: mpsc::Receiver<Result<T, PluginError>>,
    timeout: Duration,
) -> Result<T, PluginError> {
    // `recv_timeout` would block the async runtime; a blocking thread is used
    // instead so the caller stays cooperative.
    let handle = tokio::task::spawn_blocking(move || {
        reply.recv_timeout(timeout).map_err(|_| PluginError::Timeout(timeout))
    });
    match handle.await {
        Ok(Ok(inner)) => inner,
        Ok(Err(error)) => Err(error),
        Err(_) => Err(PluginError::HostStopped),
    }
}

/// The launcher prelude for the script contract.
///
/// Defines `register` so a plugin written as a plain script can state its
/// manifest. It lives in the script rather than as a host callback because a
/// callback would have to capture shared state across Boa's garbage collector,
/// and it resets the slot so a plugin that forgets to register cannot inherit
/// the previous plugin's manifest.
const PRELUDE: &str = "\
    var __ORM_MANIFEST = null;\n\
    function register(manifest) { __ORM_MANIFEST = manifest; }\n";

fn engine_loop(rx: mpsc::Receiver<Call>) {
    let mut engine = Engine::new();
    while let Ok(call) = rx.recv() {
        match call {
            Call::Load { source, reply } => {
                let _ = reply.send(engine.load(&source));
            }
            Call::Hook {
                key,
                hook,
                input,
                reply,
            } => {
                let args: Vec<serde_json::Value> =
                    serde_json::from_str(&input).unwrap_or_else(|_| vec![serde_json::Value::Null]);
                let _ = reply.send(engine.call_member(&key, &hook, &args));
            }
            Call::Export {
                key,
                hook,
                input,
                reply,
            } => {
                let args: Vec<serde_json::Value> =
                    serde_json::from_str(&input).unwrap_or_else(|_| vec![serde_json::Value::Null]);
                let _ = reply.send(engine.call_export(&key, &hook, &args));
            }
        }
    }
}

/// One loaded plugin: the module, and the manifest it exported.
///
/// The module is kept rather than its namespace because a namespace is tied to
/// the context that evaluated it and the host must be able to call members of a
/// plugin loaded long ago.
struct Loaded {
    /// `Some` for the module form, `None` for the script form.
    ///
    /// The distinction is not cosmetic: a module's top-level `var` is
    /// module-scoped and never becomes a global property, so a script-form
    /// plugin's `protocols` can only be found if the script was evaluated *as a
    /// script*. Both forms are therefore kept apart rather than normalised,
    /// because normalising them would silently break one of them.
    module: Option<boa_engine::Module>,
    manifest: PluginManifest,
}

/// The engine thread's private state. Never leaves this thread.
struct Engine {
    context: boa_engine::Context,
    /// Plugins that loaded successfully, by plugin key and by each protocol name
    /// they claim. The second spelling is how the reference addresses protocol
    /// members (`protocols.openai_responses.*`), so both are addressable and a
    /// call site does not have to know which one it holds.
    loaded: std::collections::HashMap<String, Loaded>,
    /// Module path counter, so each loaded plugin has its own identity inside the
    /// loader and cannot shadow another's exports.
    next_module: u32,
}

impl Engine {
    fn new() -> Self {
        Self {
            context: boa_engine::Context::default(),
            loaded: std::collections::HashMap::new(),
            next_module: 0,
        }
    }

    /// Load a plugin and return the manifest it declared.
    ///
    /// The reference compiles plugins as ES modules (`sobek.ParseModule`,
    /// `pkg/jsplugin/engine.go:145`) and reads `meta` from the module's exports,
    /// so a plugin written for it is a module exporting `meta` and `protocols`.
    /// The script form — `register(meta)` plus a `protocols` assignment — is
    /// accepted too, and is evaluated *as a script* rather than as a module,
    /// because a module's top-level `var` is module-scoped and never becomes a
    /// global property; normalising the two forms would silently break one.
    fn load(&mut self, source: &str) -> Result<PluginManifest, PluginError> {
        // Two refusals the reference makes and this host keeps, both because the
        // plugin is untrusted. `import` would pull in code the operator never
        // reviewed; a `sourceMappingURL` comment would let the parser read
        // arbitrary server files while reporting an error.
        if let Some(specifier) = forbidden_import(source) {
            return Err(PluginError::Load(format!(
                "plugin imports are disabled: {specifier}"
            )));
        }
        if has_source_map_directive(source) {
            return Err(PluginError::Load(
                "plugin source maps are disabled: a sourceMappingURL directive can make the parser read server files"
                    .to_string(),
            ));
        }
        // The script contract's `register` must exist before the body runs, since
        // a script-form plugin calls it during evaluation.
        self.context
            .eval(boa_engine::Source::from_bytes(PRELUDE))
            .map_err(|error| PluginError::Load(describe(&error)))?;

        self.next_module += 1;
        let path = format!("plugin{}.js", self.next_module);
        let is_module = looks_like_module(source);

        let (module, mut manifest, exports) = if is_module {
            let module = boa_engine::Module::parse(
                boa_engine::Source::from_bytes(source.as_bytes())
                    .with_path(std::path::Path::new(&path)),
                None,
                &mut self.context,
            )
            .map_err(|error| PluginError::Load(describe(&error)))?;
            // Linking is where the engine checks that every import resolves. The
            // default loader resolves none, so a plugin reaching for a module
            // fails here with a message naming it.
            module
                .link(&mut self.context)
                .map_err(|error| PluginError::Load(describe(&error)))?;
            let promise = module
                .evaluate(&mut self.context)
                .map_err(|error| PluginError::Load(describe(&error)))?;
            // A module body may await, so evaluation yields a promise. No clock is
            // installed, so only the body's own synchronous microtasks can run --
            // but a throw inside the body surfaces here as a *rejected promise*
            // rather than an evaluate() error, and ignoring it would make a
            // broken plugin look like one that declared nothing.
            let _ = self
                .context
                .run_jobs()
                .map_err(|error| PluginError::Load(describe(&error)))?;
            let as_value: boa_engine::JsValue = promise.into();
            let Some(handle) = as_value.as_promise() else {
                return Err(PluginError::Load(
                    "the engine did not return a promise for a module body".to_string(),
                ));
            };
            match handle.state() {
                boa_engine::builtins::promise::PromiseState::Rejected(reason) => {
                    let text = reason
                        .to_string(&mut self.context)
                        .map(|s| s.to_std_string_escaped())
                        .unwrap_or_else(|_| "plugin threw while loading".to_string());
                    return Err(PluginError::Load(describe_text(&text)));
                }
                boa_engine::builtins::promise::PromiseState::Pending => {
                    return Err(PluginError::Load(
                        "plugin top-level await never settled; plugins must be synchronous"
                            .to_string(),
                    ));
                }
                boa_engine::builtins::promise::PromiseState::Fulfilled(_) => {}
            }
            let manifest = self.manifest_from_module(&module)?;
            let exports = self.plugin_exports(Some(&module));
            (Some(module), manifest, exports)
        } else {
            self.context
                .eval(boa_engine::Source::from_bytes(source.as_bytes()))
                .map_err(|error| PluginError::Load(describe(&error)))?;
            let manifest = self.manifest_from_register()?;
            let exports = self.plugin_exports(None);
            (None, manifest, exports)
        };

        manifest.validate()?;
        let mut problems =
            validate_required_hooks(&manifest, &exports);
        problems.extend(validate_protocol_claims(
            &manifest.protocols,
            &manifest.key,
            &exports,
        ));
        if !problems.is_empty() {
            return Err(PluginError::Load(problems.join("; ")));
        }
        normalize_protocol_supports(&mut manifest.protocols);

        // Addressable by plugin key and by every protocol the plugin claims,
        // because those are the two names a caller may hold.
        let mut names = vec![manifest.key.clone()];
        names.extend(manifest.protocols.iter().map(|c| c.name.clone()));
        for name in names {
            self.loaded.insert(
                name,
                Loaded {
                    module: module.clone(),
                    manifest: manifest.clone(),
                },
            );
        }
        Ok(manifest)
    }

    /// The manifest a module declared through its `meta` export.
    fn manifest_from_module(
        &mut self,
        module: &boa_engine::Module,
    ) -> Result<PluginManifest, PluginError> {
        let namespace = module.namespace(&mut self.context);
        let value = namespace.get(boa_engine::js_string!("meta"), &mut self.context);
        match value {
            Ok(value) if !value.is_undefined() => {
                let json = value
                    .to_json(&mut self.context)
                    .map_err(|error| PluginError::BadManifest(describe(&error)))?;
                let value = serde_json::to_value(json).unwrap_or(serde_json::Value::Null);
                PluginManifest::from_meta(&value)
            }
            _ => Err(PluginError::Load(
                "module must export `meta`".to_string(),
            )),
        }
    }

    /// The manifest a script declared by calling `register`.
    fn manifest_from_register(&mut self) -> Result<PluginManifest, PluginError> {
        use boa_engine::Source;

        let raw = self
            .context
            .eval(Source::from_bytes(
                "(typeof __ORM_MANIFEST === 'undefined' || __ORM_MANIFEST === null) ? 'null' : JSON.stringify(__ORM_MANIFEST)",
            ))
            .map_err(|error| PluginError::Load(describe(&error)))?
            .to_string(&mut self.context)
            .map_err(|error| PluginError::Load(describe(&error)))?
            .to_std_string_escaped();
        if raw == "null" || raw == "undefined" {
            return Err(PluginError::Load(
                "plugin neither exported `meta` nor called register(manifest)".to_string(),
            ));
        }
        let value: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|error| PluginError::BadManifest(error.to_string()))?;
        PluginManifest::from_meta(&value)
    }

    /// The plugin's `protocols` object, from a module export or the global a
    /// script assigns.
    fn protocols_object(
        &mut self,
        module: Option<&boa_engine::Module>,
    ) -> Option<boa_engine::JsObject> {
        if let Some(module) = module {
            let namespace = module.namespace(&mut self.context);
            if let Ok(value) = namespace.get(boa_engine::js_string!("protocols"), &mut self.context)
            {
                if let Ok(obj) = value.to_object(&mut self.context) {
                    return Some(obj);
                }
            }
        }
        let global = self.context.global_object();
        let value = global
            .get(boa_engine::js_string!("protocols"), &mut self.context)
            .ok()?;
        value.to_object(&mut self.context).ok()
    }

    /// The string form of an own property key, when it has one.
    ///
    /// A plugin's exports are addressed by name; symbol keys are not
    /// addressable, so they are dropped rather than guessed at.
    fn property_key_name(key: boa_engine::property::PropertyKey) -> Option<String> {
        match key {
            boa_engine::property::PropertyKey::String(s) => Some(s.to_std_string_escaped()),
            boa_engine::property::PropertyKey::Index(i) => Some(i.get().to_string()),
            _ => None,
        }
    }

    /// Everything the plugin's live exports provide, as the host sees them.
    ///
    /// The reference asks its engine the same questions one at a time
    /// (`HasCallablePath`, `Export`; `pkg/jsplugin/registry.go:388-495`). Doing
    /// it in one pass keeps the answers consistent with each other: the set of
    /// protocols present, the callable members under each, and the callable
    /// top-level exports that driver hooks live in.
    fn plugin_exports(&mut self, module: Option<&boa_engine::Module>) -> PluginExports {
        let mut out = PluginExports::default();

        // Top-level: driver hooks (`buildSubmitRequest`, `listArtifacts`, ...)
        // and the `native` members routes reference are ordinary exports.
        let top_level_keys: Vec<String> = match module {
            Some(module) => {
                let namespace = module.namespace(&mut self.context);
                match namespace.own_property_keys(&mut self.context) {
                    Ok(keys) => keys
                        .into_iter()
                        .filter_map(Self::property_key_name)
                        .collect(),
                    Err(_) => Vec::new(),
                }
            }
            None => {
                let global = self.context.global_object();
                match global.own_property_keys(&mut self.context) {
                    Ok(keys) => keys
                        .into_iter()
                        .filter_map(Self::property_key_name)
                        .collect(),
                    Err(_) => Vec::new(),
                }
            }
        };
        for name in top_level_keys {
            let value = match module {
                Some(module) => module
                    .namespace(&mut self.context)
                    .get(boa_engine::js_string!(name.clone()), &mut self.context),
                None => self
                    .context
                    .global_object()
                    .get(boa_engine::js_string!(name.clone()), &mut self.context),
            };
            if let Ok(value) = value {
                if value.as_callable().is_some() {
                    out.top_level.push(name);
                }
            }
        }

        // The `protocols` object: every protocol it names, and every callable
        // member under each. Read the object's own keys rather than a table of
        // expected names, so an unclaimed protocol is visible to the validator.
        let Some(obj) = self.protocols_object(module) else {
            return out;
        };
        let Ok(keys) = obj.own_property_keys(&mut self.context) else {
            return out;
        };
        for key in keys {
            let name = match Self::property_key_name(key) {
                Some(name) => name,
                None => continue,
            };
            out.protocol_names.push(name.clone());
            let Ok(entry) = obj.get(boa_engine::js_string!(name.clone()), &mut self.context) else {
                continue;
            };
            let Ok(entry) = entry.to_object(&mut self.context) else {
                continue;
            };
            let mut members = Vec::new();
            if let Ok(member_keys) = entry.own_property_keys(&mut self.context) {
                for member_key in member_keys {
                    let member = match Self::property_key_name(member_key) {
                        Some(name) => name,
                        None => continue,
                    };
                    let Ok(value) = entry.get(boa_engine::js_string!(member.clone()), &mut self.context)
                    else {
                        continue;
                    };
                    if value.as_callable().is_some() {
                        members.push(member);
                    }
                }
            }
            members.sort();
            out.protocol_members.insert(name, members);
        }
        out
    }
    /// Call `protocols.<name>.<member>` with JSON arguments, returning JSON.
    ///
    /// The member is reached through the module's own `protocols` export, so the
    /// call site matches the shape a plugin is written in and a plugin authored
    /// for the reference works unchanged.
    /// Call a top-level export: the driver hooks (`buildSubmitRequest`,
    /// `buildQueryRequest`, `parseTaskResult`, ...).
    ///
    /// The driver hooks live on the module itself, not under `protocols`, because
    /// the adaptor calls them directly (`pkg/jsplugin/registry.go:337`). A
    /// instance-form plugin keeps them as globals, which is why the fallback is
    /// the global object rather than an error.
    fn call_export(
        &mut self,
        key: &str,
        hook: &str,
        args: &[serde_json::Value],
    ) -> Result<String, PluginError> {
        // `key` may be the plugin's own key or a protocol name; either resolves to
        // the same plugin, because a driver hook lives on the module.

        let Some((module, plugin_key)) = self
            .loaded
            .get(key)
            .map(|loaded| (loaded.module.clone(), loaded.manifest.key.clone()))
        else {
            return Err(PluginError::NoSuchHook {
                key: key.to_string(),
                hook: hook.to_string(),
            });
        };

        let function = match module.as_ref() {
            Some(module) => module
                .namespace(&mut self.context)
                .get(boa_engine::js_string!(hook), &mut self.context),
            None => {
                let global = self.context.global_object();
                global.get(boa_engine::js_string!(hook), &mut self.context)
            }
        }
        .map_err(|error| PluginError::Hook {
            key: key.to_string(),
            hook: hook.to_string(),
            message: describe(&error),
        })?;
        let Some(callable) = function.as_callable() else {
            return Err(PluginError::Hook {
                key: key.to_string(),
                hook: hook.to_string(),
                message: format!(
                    "plugin {plugin_key} has no export {hook:?}; implement it as a top-level function"
                ),
            });
        };
        self.invoke(callable, key, hook, args)
    }

    /// The plugin's `protocols` entry that implements `hook`.
    ///
    /// The reference resolves a protocol hook by looking for a member of that
    /// name in the plugin's *own* protocol objects, in one fixed order
    /// (`pkg/jsplugin/registry.go:468`). Searching by the caller's key instead
    /// would let an object that happens to be keyed like a protocol shadow the
    /// real implementation.
    fn protocol_object_for(
        &mut self,
        module: Option<&boa_engine::Module>,
        protocol: &str,
        plugin_key: &str,
    ) -> Result<boa_engine::JsObject, PluginError> {
        let Some(object) = self.protocols_object(module) else {
            return Err(PluginError::Load(
                "plugin exports no protocols object".to_string(),
            ));
        };
        // Owned names, because a candidate borrowed from the manifest would not
        // outlive the borrow of `self` the lookup needs.
        let mut candidates: Vec<String> = Vec::new();
        for name in [protocol, plugin_key] {
            if !name.is_empty() && !candidates.iter().any(|c| c == name) {
                candidates.push(name.to_string());
            }
        }
        if let Ok(manifest) = self.manifest_for(plugin_key) {
            for claim in &manifest.protocols {
                if !candidates.iter().any(|c| c == &claim.name) {
                    candidates.push(claim.name.clone());
                }
            }
        }
        for name in candidates {
            let entry = object.get(boa_engine::js_string!(name.as_str()), &mut self.context);
            if let Ok(entry) = entry {
                if !entry.is_undefined() && !entry.is_null() {
                    if let Ok(entry) = entry.to_object(&mut self.context) {
                        return Ok(entry);
                    }
                }
            }
        }
        Err(PluginError::Hook {
            key: plugin_key.to_string(),
            hook: protocol.to_string(),
            message: format!("plugin {plugin_key} implements no protocol {protocol:?}"),
        })
    }

    /// A loaded plugin's manifest, by key.
    fn manifest_for(&self, key: &str) -> Result<PluginManifest, PluginError> {
        self.loaded
            .values()
            .find(|loaded| loaded.manifest.key == key)
            .map(|loaded| loaded.manifest.clone())
            .ok_or_else(|| PluginError::NoSuchHook {
                key: key.to_string(),
                hook: String::new(),
            })
    }

    /// Convert arguments to JavaScript values, call, and convert the result back.
    fn invoke(
        &mut self,
        callable: boa_engine::JsObject,
        key: &str,
        hook: &str,
        args: &[serde_json::Value],
    ) -> Result<String, PluginError> {
        let mut argv = Vec::new();
        for arg in args {
            let value = boa_engine::JsValue::from_json(
                &serde_json::to_value(arg).unwrap_or(serde_json::Value::Null),
                &mut self.context,
            )
            .map_err(|error| PluginError::Hook {
                key: key.to_string(),
                hook: hook.to_string(),
                message: describe(&error),
            })?;
            argv.push(value);
        }
        let result = callable
            .call(&boa_engine::JsValue::undefined(), &argv, &mut self.context)
            .map_err(|error| PluginError::Hook {
                key: key.to_string(),
                hook: hook.to_string(),
                message: describe(&error),
            })?;
        let json = result.to_json(&mut self.context).map_err(|error| PluginError::Hook {
            key: key.to_string(),
            hook: hook.to_string(),
            message: describe(&error),
        })?;
        serde_json::to_string(&serde_json::to_value(json).unwrap_or(serde_json::Value::Null))
            .map_err(|error| PluginError::Hook {
                key: key.to_string(),
                hook: hook.to_string(),
                message: error.to_string(),
            })
    }

    /// Call `protocols.<name>.<member>` with JSON arguments, returning JSON.
    fn call_member(
        &mut self,
        key: &str,
        member: &str,
        args: &[serde_json::Value],
    ) -> Result<String, PluginError> {
        // The module is borrowed once and cloned out, so the rest of the call can
        // take `&mut self` for the engine.
        let Some((module, plugin_key)) = self
            .loaded
            .get(key)
            .map(|loaded| (loaded.module.clone(), loaded.manifest.key.clone()))
        else {
            return Err(PluginError::NoSuchHook {
                key: key.to_string(),
                hook: member.to_string(),
            });
        };
        let entry = self.protocol_object_for(module.as_ref(), key, &plugin_key)?;
        let function = entry
            .get(boa_engine::js_string!(member), &mut self.context)
            .map_err(|error| PluginError::Hook {
                key: key.to_string(),
                hook: member.to_string(),
                message: describe(&error),
            })?;
        let Some(callable) = function.as_callable() else {
            return Err(PluginError::Hook {
                key: key.to_string(),
                hook: member.to_string(),
                message: format!(
                    "plugin {plugin_key} has no protocol member {member:?} for {key:?}; implement it under protocols.{key}"
                ),
            });
        };
        self.invoke(callable, key, member, args)
    }
}

/// The first `import`/`export ... from` specifier in `source`, if any.
///
/// The keyword is located in a copy with comments and string bodies blanked, so
/// a plugin describing an import in prose is not refused for it, but the
/// specifier itself is read from the **original** text: the blanked copy no
/// longer contains it, which is a mistake this function made in its first form.
/// The reference draws the same distinction before its own scan
/// (`pkg/jsplugin/engine.go:136`).
fn forbidden_import(source: &str) -> Option<String> {
    let stripped = strip_comments_and_strings(source);

    let mut from = 0usize;
    while let Some(rel) = stripped[from..].find("import") {
        let at = from + rel;
        if keyword_boundary_ok(&stripped, at, "import") {
            if let Some(spec) = read_import_specifier(&source[at..]) {
                return Some(spec);
            }
        }
        from = at + "import".len();
    }

    // `export ... from "..."` re-exports another module's code, which is the same
    // hazard by another spelling.
    let mut from = 0usize;
    while let Some(rel) = stripped[from..].find("export") {
        let at = from + rel;
        if keyword_boundary_ok(&stripped, at, "export") {
            if let Some(spec) = read_export_from_specifier(&source[at..]) {
                return Some(spec);
            }
        }
        from = at + "export".len();
    }
    None
}

/// True when `keyword` at `at` is a standalone word rather than part of a longer
/// identifier or a member access.
fn keyword_boundary_ok(text: &str, at: usize, keyword: &str) -> bool {
    let before_ok = text[..at]
        .chars()
        .next_back()
        .map(|c| !c.is_alphanumeric() && c != '_' && c != '.')
        .unwrap_or(true);
    let after_ok = text[at + keyword.len()..]
        .chars()
        .next()
        .map(|c| !c.is_alphanumeric() && c != '_')
        .unwrap_or(true);
    before_ok && after_ok
}

/// The specifier of an `import` statement beginning at `rest`.
fn read_import_specifier(rest: &str) -> Option<String> {
    let after = rest.get("import".len()..)?.trim_start();
    // A dynamic import loads code just as surely as a static one, and is refused
    // without trying to name what it loads.
    if after.starts_with('(') {
        return Some("(dynamic import)".to_string());
    }
    // `import "x"` has the specifier immediately after the keyword.
    if let Some(spec) = read_quoted(after) {
        return Some(spec);
    }
    let from = after.find(" from ")?;
    read_quoted(&after[from + " from ".len()..])
}

fn read_export_from_specifier(rest: &str) -> Option<String> {
    let after = rest.get("export".len()..)?;
    let from = after.find(" from ")?;
    read_quoted(&after[from + " from ".len()..])
}

/// The contents of the quoted string `s` begins with, if it begins with one.
fn read_quoted(s: &str) -> Option<String> {
    let s = s.trim_start();
    let quote = s.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let body = s.get(1..)?;
    let end = body.find(quote)?;
    Some(body[..end].to_string())
}

/// Whether a plugin source should be parsed as a module.
///
/// Module syntax is what decides it: `export` or a static `import` is only legal
/// in a module, so a source carrying either must be one, and a source carrying
/// neither is treated as a script -- which is also the form the `register`
/// contract is written in.
fn looks_like_module(source: &str) -> bool {
    let stripped = strip_comments_and_strings(source);
    let has = |keyword: &str| {
        let mut from = 0usize;
        while let Some(rel) = stripped[from..].find(keyword) {
            let at = from + rel;
            if keyword_boundary_ok(&stripped, at, keyword) {
                return true;
            }
            from = at + keyword.len();
        }
        false
    };
    has("export") || has("import")
}

/// True when the source carries a `sourceMappingURL` directive.
fn has_source_map_directive(source: &str) -> bool {
    source.contains("sourceMappingURL")
}

/// Replace comment and string bodies with spaces, so a scan for keywords cannot
/// be fooled by prose.
///
/// Characters are replaced rather than removed to keep byte offsets meaningful,
/// and because a scanner that deletes text can splice two tokens into a third
/// that was never written.
fn strip_comments_and_strings(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    out[i] = b' ';
                    i += 1;
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                out[i] = b' ';
                out[i + 1] = b' ';
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                        out[i] = b' ';
                        out[i + 1] = b' ';
                        i += 2;
                        break;
                    }
                    if bytes[i] != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
            }
            quote @ (b'"' | b'\'' | b'`') => {
                out[i] = b' ';
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        out[i] = b' ';
                        if i + 1 < bytes.len() {
                            out[i + 1] = b' ';
                        }
                        i += 2;
                        continue;
                    }
                    if bytes[i] == quote {
                        out[i] = b' ';
                        i += 1;
                        break;
                    }
                    if bytes[i] != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Flatten and truncate a message the engine already rendered to text.
fn describe_text(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if out.len() >= 512 {
            break;
        }
        out.push(if ch.is_control() { ' ' } else { ch });
    }
    if out.is_empty() {
        "plugin hook failed".to_string()
    } else {
        out
    }
}

/// Turn an engine error into a short, single-line message.
///
/// A plugin's stack trace is not the caller's business, and a hook error may be
/// surfaced to an API client, so the text is flattened and truncated.
fn describe(error: &boa_engine::JsError) -> String {
    let text = error.to_string();
    let mut out = String::new();
    for ch in text.chars() {
        if out.len() >= 512 {
            break;
        }
        out.push(if ch.is_control() { ' ' } else { ch });
    }
    if out.is_empty() {
        "plugin hook failed".to_string()
    } else {
        out
    }
}

// ── Host protocols ─────────────────────────────────────────────────────────────

/// One client request form an operation accepts, and the hook implementing it.
///
/// Ported from the reference's `ProtocolMode` (`pkg/jsplugin/routing.go:57`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolMode {
    /// The request form's name, e.g. `stream` or `sync`.
    pub name: &'static str,
    /// The hook a plugin must export to serve it.
    pub hook: &'static str,
}

/// One endpoint an operation serves.
///
/// Ported from `HostProtocolOperation` (`pkg/jsplugin/routing.go:71`). The
/// operation is what a plugin is contracted to implement: to serve
/// `POST /v1/responses` a plugin must export the members listed here plus the
/// hook for whichever request forms it claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostOperation {
    pub name: &'static str,
    /// The HTTP methods this operation answers. Two entries where the
    /// reference registers two (`HEAD` for the artifact content endpoint,
    /// `pkg/jsplugin/routing.go:95`).
    pub methods: &'static [&'static str],
    pub path: &'static str,
    /// The request body encodings the operation accepts, as the reference's
    /// `BodyKinds`. A caller sending a form to a JSON-only operation is
    /// refused the way the reference refuses it
    /// (`middleware/task_plugin.go:575`).
    pub body_kinds: &'static [&'static str],
    /// The body field naming the model this endpoint serves, or empty when the
    /// host answers the operation itself.
    ///
    /// An operation with no model field is not bound to any plugin: retrieval
    /// reads a task the host already recorded, so there is no model to choose by
    /// (`pkg/jsplugin/routing.go:989`).
    pub model_field: &'static str,
    /// Members every plugin serving this operation must export, whatever forms
    /// it claims.
    pub required_members: &'static [&'static str],
    /// Request forms, each naming the hook that implements it. Empty for a
    /// protocol that has none -- the image API is synchronous, so there is
    /// nothing to choose between.
    pub modes: &'static [ProtocolMode],
    /// Hooks required regardless of the forms claimed.
    pub required_driver_hooks: &'static [&'static str],
    /// The hook that shapes a completed response, when the operation has one.
    ///
    /// Per operation rather than derived from the modes, because the image
    /// protocol's renderer is simply `render`: it has no modes to name one.
    pub render_hook: Option<&'static str>,
}

/// A client-facing protocol the host knows how to serve from a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostProtocol {
    pub name: &'static str,
    pub operations: &'static [HostOperation],
}

/// The request body encodings a host operation accepts.
///
/// Spelled the way the reference spells them (`pkg/jsplugin/routing.go:64`) so
/// the names also appear in the context a plugin reads as `body.kind`.
pub const BODY_NONE: &str = "none";
pub const BODY_JSON: &str = "json";
pub const BODY_FORM: &str = "form";
pub const BODY_MULTIPART: &str = "multipart";

impl HostProtocol {
    /// Every client request form the protocol accepts, in table order.
    ///
    /// Two shapes matter to a plugin author: whether the protocol has forms at
    /// all (a mode-less protocol must not be given `supports`), and the full
    /// ordered list to name when a claim declared none
    /// (`pkg/jsplugin/protocol_supports_test.go:13`).
    pub fn defined_modes(&self) -> Vec<&'static ProtocolMode> {
        let mut out: Vec<&'static ProtocolMode> = Vec::new();
        for operation in self.operations {
            for mode in operation.modes {
                if !out.iter().any(|m| m.name == mode.name) {
                    out.push(mode);
                }
            }
        }
        out
    }
}

/// The protocols a plugin may claim.
///
/// A curated list rather than free naming: each entry is a promise about the
/// hook contract, and the host can only keep a promise it knows. Ported whole
/// from the reference's `hostProtocols` (`pkg/jsplugin/routing.go:87`), in the
/// same order, because a manifest written for one host must load in the other.
pub const HOST_PROTOCOLS: &[HostProtocol] = &[
    HostProtocol {
        name: PROTOCOL_OPENAI_RESPONSES,
        operations: &[
            HostOperation {
                name: "create",
                methods: &["POST"],
                path: "/v1/responses",
                body_kinds: &[BODY_JSON],
                required_members: &["decodeRequest"],
                model_field: "model",
                modes: &[
                    ProtocolMode {
                        name: "stream",
                        hook: "renderEvents",
                    },
                    ProtocolMode {
                        name: "sync",
                        hook: "renderFinal",
                    },
                    ProtocolMode {
                        name: "background",
                        hook: "renderFinal",
                    },
                ],
                required_driver_hooks: &[],
                render_hook: Some("renderFinal"),
            },
            // Retrieval reads a task the host already recorded. It declares no
            // members because the host answers it itself; a plugin cannot
            // claim "retrieve" as a form, which the reference says explicitly
            // (`pkg/jsplugin/protocol_supports_test.go:55`).
            HostOperation {
                name: "retrieve",
                methods: &["GET"],
                path: "/v1/responses/:response_id",
                body_kinds: &[BODY_NONE],
                required_members: &[],
                model_field: "",
                modes: &[],
                required_driver_hooks: &[],
                render_hook: None,
            },
        ],
    },
    HostProtocol {
        name: PROTOCOL_OPENAI_VIDEO,
        operations: &[
            HostOperation {
                name: "create",
                methods: &["POST"],
                path: "/v1/videos",
                body_kinds: &[BODY_JSON, BODY_MULTIPART],
                required_members: &["decodeRequest"],
                model_field: "model",
                modes: &[],
                required_driver_hooks: &[],
                render_hook: None,
            },
            HostOperation {
                name: "retrieve",
                methods: &["GET"],
                path: "/v1/videos/:task_id",
                body_kinds: &[BODY_NONE],
                required_members: &["render"],
                model_field: "",
                modes: &[],
                required_driver_hooks: &[],
                render_hook: Some("render"),
            },
            HostOperation {
                name: "content",
                methods: &["GET", "HEAD"],
                path: "/v1/videos/:task_id/content",
                body_kinds: &[BODY_NONE],
                required_members: &[],
                modes: &[],
                required_driver_hooks: &["listArtifacts", "buildContentRequest"],
                model_field: "",
                render_hook: None,
            },
        ],
    },
    HostProtocol {
        name: PROTOCOL_OPENAI_IMAGE,
        operations: &[
            HostOperation {
                name: "generate",
                methods: &["POST"],
                path: "/v1/images/generations",
                body_kinds: &[BODY_JSON],
                model_field: "model",
                // Both members are required whatever the plugin claims: without
                // a decoder there is no request to send, and without a renderer
                // there is no answer to return. There are no modes here, so
                // nothing can narrow the requirement away.
                required_members: &["decodeRequest", "render"],
                modes: &[],
                required_driver_hooks: &[],
                render_hook: Some("render"),
            },
            HostOperation {
                name: "edit",
                methods: &["POST"],
                path: "/v1/images/edits",
                body_kinds: &[BODY_JSON, BODY_MULTIPART],
                required_members: &["decodeRequest", "render"],
                model_field: "model",
                modes: &[],
                required_driver_hooks: &[],
                render_hook: Some("render"),
            },
        ],
    },
];

/// The host protocol serving the OpenAI Images API from a plugin
/// (`POST /v1/images/generations` and `POST /v1/images/edits`).
///
/// Synchronous, unlike the Responses protocol: both operations create a task and
/// answer once it is terminal, so there are no request forms to choose between
/// and the renderer is plain `render` (`pkg/jsplugin/routing.go:97`).
pub const PROTOCOL_OPENAI_IMAGE: &str = "openai_image";

/// The host protocol serving the OpenAI Videos API from a plugin
/// (`POST /v1/videos`, `GET /v1/videos/:task_id` and their content endpoint).
pub const PROTOCOL_OPENAI_VIDEO: &str = "openai_video";

/// The host protocol serving the OpenAI Responses API from a plugin
/// (`POST /v1/responses`, `GET /v1/responses/:response_id`).
pub const PROTOCOL_OPENAI_RESPONSES: &str = "openai_responses";

/// Look up a protocol by the name a manifest claims.
pub fn host_protocol(name: &str) -> Option<&'static HostProtocol> {
    HOST_PROTOCOLS.iter().find(|p| p.name == name)
}

/// What a manifest claims about one protocol.
///
/// The reference accepts two spellings and so does this: the bare name
/// (`protocols: ["openai_image"]`) for a protocol with nothing to configure, and
/// the object form when models or request forms need narrowing
/// (`pkg/jsplugin/routing_test.go:111,124`). Accepting only one would reject a
/// plugin the reference loads, and the reference also *phrases its refusals*
/// differently per spelling ("replace the bare string with ..." versus "add
/// supports: [...]", `pkg/jsplugin/registry.go:1362`), which is why which
/// spelling was written is remembered here rather than discarded.
#[derive(Debug, Clone, Serialize)]
pub struct ProtocolClaim {
    pub name: String,
    /// Models this claim covers; empty means every model the plugin claims.
    #[serde(default)]
    pub models: Vec<String>,
    /// Which request forms the plugin serves. Only meaningful for a protocol that
    /// has modes; a mode-less protocol must leave this undeclared.
    #[serde(default)]
    pub supports: Vec<String>,
    /// Whether `supports` was written at all. `None` is "not declared", which is
    /// an error for a mode-bearing protocol and is *not* the same as an empty
    /// list (`pkg/jsplugin/registry.go:1385`).
    #[serde(skip)]
    pub supports_declared: bool,
    /// Whether the claim was written as an object rather than a bare name.
    #[serde(skip)]
    pub object_form: bool,
}


impl<'de> Deserialize<'de> for ProtocolClaim {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ClaimVisitor;

        impl<'de> serde::de::Visitor<'de> for ClaimVisitor {
            type Value = ProtocolClaim;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a protocol name or a protocol claim object")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(ProtocolClaim {
                    name: value.to_string(),
                    models: Vec::new(),
                    supports: Vec::new(),
                    supports_declared: false,
                    object_form: false,
                })
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Self::Value, A::Error> {
                #[derive(Deserialize)]
                struct Object {
                    name: String,
                    #[serde(default)]
                    models: Vec<String>,
                    #[serde(default)]
                    supports: Option<Vec<String>>,
                }
                let object = Object::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(ProtocolClaim {
                    name: object.name,
                    models: object.models,
                    supports: object.supports.clone().unwrap_or_default(),
                    supports_declared: object.supports.is_some(),
                    object_form: true,
                })
            }
        }

        deserializer.deserialize_any(ClaimVisitor)
    }
}

/// What a plugin's live exports actually provide.
///
/// The host asks the engine for this rather than trusting the manifest, because
/// a manifest is a claim and an export is a fact
/// (`pkg/jsplugin/registry.go:432`).
#[derive(Debug, Clone, Default)]
pub struct PluginExports {
    /// Callable members under `protocols.<protocol>.<member>`.
    pub protocol_members: std::collections::BTreeMap<String, Vec<String>>,
    /// Callable top-level exports: the driver hooks and `native` members.
    pub top_level: Vec<String>,
    /// Every protocol name present in the plugin's `protocols` object.
    pub protocol_names: Vec<String>,
}

impl PluginExports {
    fn has_member(&self, protocol: &str, member: &str) -> bool {
        self.protocol_members
            .get(protocol)
            .map(|members| members.iter().any(|m| m == member))
            .unwrap_or(false)
    }
}

/// Render a list the way the reference does in its guidance
/// (`pkg/jsplugin/registry.go:440`): `"a", "b"`.
fn quoted_join(items: &[&str]) -> String {
    quoted_join_with(items, ", ")
}

/// The same, with the reference's other separator -- `"a" or "b"` -- which it
/// uses when listing the forms that would make an otherwise-dead hook live
/// (`pkg/jsplugin/registry.go:464`).
fn quoted_join_with(items: &[&str], separator: &str) -> String {
    items
        .iter()
        .map(|item| format!("{item:?}"))
        .collect::<Vec<_>>()
        .join(separator)
}

/// The exports the reference refuses outright, because a host that honoured
/// them would be a different (older) contract
/// (`pkg/jsplugin/registry.go:499`).
const REMOVED_EXPORTS: &[&str] = &["resolveRequest", "renderError", "renderers"];

/// Check a set of claims against the host's protocol table and what the plugin
/// actually exported.
///
/// Every refusal is worded the way the reference words it, because the message
/// is the only documentation a plugin author has and the reference's tests pin
/// the phrasing (`pkg/jsplugin/protocol_supports_test.go:35`). Each problem
/// names the protocol and the hook: "supports sync but does not export
/// protocols.openai_responses.renderFinal" tells an author exactly what to
/// write, where "invalid plugin" does not.
pub fn validate_protocol_claims(
    claims: &[ProtocolClaim],
    key: &str,
    exports: &PluginExports,
) -> Vec<String> {
    let mut problems = Vec::new();

    for claim in claims {
        let Some(protocol) = host_protocol(&claim.name) else {
            // An unknown protocol cannot be judged on modes, but a `supports`
            // list is still wrong, and the reference says so before it says the
            // protocol is unknown (`pkg/jsplugin/registry.go:1385,1389`).
            if claim.supports_declared {
                problems.push(format!(
                    "plugin {key} protocol {:?} does not define modes; supports is not allowed",
                    claim.name
                ));
            } else {
                problems.push(format!("plugin {key} protocol {:?} is unknown", claim.name));
            }
            continue;
        };

        let modes = protocol.defined_modes();
        let mode_names: Vec<&str> = modes.iter().map(|mode| mode.name).collect();

        if !modes.is_empty() {
            let choosing_from = quoted_join(&mode_names);
            if !claim.supports_declared {
                if claim.object_form {
                    problems.push(format!(
                        "plugin {key} protocol {:?} must declare supports; add supports: [...] choosing from {choosing_from}",
                        claim.name
                    ));
                } else {
                    problems.push(format!(
                        "plugin {key} protocol {:?} must declare supports; replace the bare string with {{name: {:?}, supports: [...]}} choosing from {choosing_from}",
                        claim.name, claim.name
                    ));
                }
                continue;
            }
            if claim.supports.is_empty() {
                problems.push(format!(
                    "plugin {key} protocol {:?} supports must contain at least one of {choosing_from}",
                    claim.name
                ));
                continue;
            }
            let mut seen: Vec<&str> = Vec::new();
            let mut bad_mode = false;
            for support in &claim.supports {
                if seen.contains(&support.as_str()) {
                    problems.push(format!(
                        "plugin {key} protocol {:?} supports must be unique",
                        claim.name
                    ));
                    bad_mode = true;
                    break;
                }
                seen.push(support.as_str());
                if !mode_names.contains(&support.as_str()) {
                    if support == "retrieve" {
                        problems.push(format!(
                            "plugin {key} protocol {:?} has no mode {:?}; retrieval of a created response is always available and is never declared",
                            claim.name, support
                        ));
                    } else {
                        problems.push(format!(
                            "plugin {key} protocol {:?} has no mode {:?}",
                            claim.name, support
                        ));
                    }
                    bad_mode = true;
                }
            }
            if bad_mode {
                continue;
            }
        } else if claim.supports_declared {
            problems.push(format!(
                "plugin {key} protocol {:?} does not define modes; supports is not allowed",
                claim.name
            ));
            continue;
        }

        // Which hooks a callable protocol member must be, and which modes use
        // each hook, so a missing hook can suggest the claim to declare instead.
        let mut required: Vec<&str> = Vec::new();
        let mut mode_hook_users: Vec<(&str, Vec<&str>)> = Vec::new();
        for operation in protocol.operations {
            for member in operation.required_members {
                if !required.contains(member) {
                    required.push(member);
                }
            }
            for mode in operation.modes {
                if !mode_hook_users.iter().any(|(hook, _)| *hook == mode.hook) {
                    mode_hook_users.push((mode.hook, Vec::new()));
                }
                if let Some((_, users)) = mode_hook_users.iter_mut().find(|(hook, _)| *hook == mode.hook)
                {
                    if !users.contains(&mode.name) {
                        users.push(mode.name);
                    }
                }
                if claim.supports.iter().any(|support| support == mode.name)
                    && !required.contains(&mode.hook)
                {
                    required.push(mode.hook);
                }
            }
        }

        for member in &required {
            if exports.has_member(&claim.name, member) {
                continue;
            }
            let users: Vec<&str> = mode_hook_users
                .iter()
                .find(|(hook, _)| hook == member)
                .map(|(_, users)| users.clone())
                .unwrap_or_default();
            if !users.is_empty() {
                // Which claimed form pulls this hook in, and which forms would
                // have worked instead.
                let mentioned = claim
                    .supports
                    .iter()
                    .find(|support| users.contains(&support.as_str()))
                    .cloned()
                    .unwrap_or_default();
                let mut suggested: Vec<&str> = Vec::new();
                for mode in &modes {
                    if exports.has_member(&claim.name, mode.hook) && !suggested.contains(&mode.name) {
                        suggested.push(mode.name);
                    }
                }
                let mut message = format!(
                    "plugin {key} protocol {:?} supports {mentioned:?} but does not export protocols.{}.{member}; implement it",
                    claim.name, claim.name
                );
                if !suggested.is_empty() {
                    message.push_str(&format!(
                        " or declare supports: [{}]",
                        quoted_join(&suggested)
                    ));
                }
                problems.push(message);
            } else {
                problems.push(format!(
                    "plugin {key} protocol {:?} is missing hook {member:?}; implement it",
                    claim.name
                ));
            }
        }

        // A mode hook that is exported but that no claimed form uses is dead
        // weight the reference refuses, naming the forms that would use it
        // (`pkg/jsplugin/protocol_supports_test.go:79`).
        for (hook, users) in &mode_hook_users {
            if required.contains(hook) {
                continue;
            }
            if !exports.has_member(&claim.name, hook) {
                continue;
            }
            let users: Vec<&str> = users
                .iter()
                .filter(|name| mode_names.contains(*name))
                .copied()
                .collect();
            if users.is_empty() {
                continue;
            }
            problems.push(format!(
                "plugin {key} protocol {:?} exports protocols.{}.{hook} but no supported mode uses it; add {} to supports or remove the hook",
                claim.name,
                claim.name,
                quoted_join_with(&users, " or ")
            ));
        }

        // Driver hooks live at the top level, not under the protocol, because
        // the adaptor calls them directly (`pkg/jsplugin/registry.go:401`).
        for operation in protocol.operations {
            for hook in operation.required_driver_hooks {
                if !exports.top_level.iter().any(|name| name == hook) {
                    problems.push(format!(
                        "plugin {key} protocol {:?} is missing driver hook {hook:?}",
                        claim.name
                    ));
                }
            }
        }
    }

    // A `protocols` object the manifest never claimed is an implementation the
    // host would never call; the reference refuses it
    // (`pkg/jsplugin/registry.go:492`).
    for name in &exports.protocol_names {
        if !claims.iter().any(|claim| &claim.name == name) {
            problems.push(format!("plugin {key} implements unclaimed protocol {name:?}"));
        }
    }

    for removed in REMOVED_EXPORTS {
        if exports.top_level.iter().any(|name| name == removed) {
            problems.push(format!("plugin {key} export {removed:?} is no longer supported"));
        }
    }

    problems
}
/// Put each claim's `supports` in the protocol table's order.
///
/// The reference does this in `normalizeV1Meta` (`pkg/jsplugin/registry.go:1384`),
/// so two manifests claiming the same forms are indistinguishable afterwards --
/// the console shows one order and a snapshot comparison would otherwise see the
/// same plugin as two.
pub fn normalize_protocol_supports(claims: &mut [ProtocolClaim]) {
    for claim in claims.iter_mut() {
        if !claim.supports_declared {
            continue;
        }
        let Some(protocol) = host_protocol(&claim.name) else {
            continue;
        };
        let order: Vec<&str> = protocol.defined_modes().iter().map(|mode| mode.name).collect();
        claim.supports.sort_by_key(|support| {
            order
                .iter()
                .position(|name| name == support)
                .unwrap_or(usize::MAX)
        });
    }
}

/// The hooks that live as top-level exports rather than under `protocols`.
///
/// One list, because two places depend on it: the load-time check, and the call
/// routing above. A hook in this list is called as an export; anything else is a
/// protocol member.
pub const DRIVER_HOOKS: &[&str] = &[
    HOOK_BUILD_SUBMIT_REQUEST,
    HOOK_PARSE_SUBMIT_RESPONSE,
    HOOK_PARSE_SUBMIT_EVENT,
    HOOK_PARSE_SUBMIT_EVENT_DELTA,
    HOOK_BUILD_QUERY_REQUEST,
    HOOK_PARSE_TASK_RESULT,
    HOOK_BUILD_BATCH_QUERY_REQUEST,
    HOOK_PARSE_BATCH_RESULT,
    HOOK_LIST_ARTIFACTS,
    HOOK_BUILD_CONTENT_REQUEST,
    HOOK_EXTRACT_USAGE,
    HOOK_EXTRACT_USAGE_ON_SUBMIT,
    HOOK_EXTRACT_USAGE_ON_COMPLETE,
];

/// The driver hook names, as the reference spells them.
///
/// One place, because two callers depend on the same spelling: the load-time
/// check that a plugin exports them, and the submit/poll path that calls them. A
/// typo in one would otherwise make a valid plugin look incomplete.
pub const HOOK_BUILD_SUBMIT_REQUEST: &str = "buildSubmitRequest";
pub const HOOK_PARSE_SUBMIT_RESPONSE: &str = "parseSubmitResponse";
pub const HOOK_PARSE_SUBMIT_EVENT: &str = "parseSubmitEvent";
pub const HOOK_PARSE_SUBMIT_EVENT_DELTA: &str = "parseSubmitEventDelta";
pub const HOOK_BUILD_QUERY_REQUEST: &str = "buildQueryRequest";
pub const HOOK_PARSE_TASK_RESULT: &str = "parseTaskResult";
pub const HOOK_BUILD_BATCH_QUERY_REQUEST: &str = "buildBatchQueryRequest";
pub const HOOK_PARSE_BATCH_RESULT: &str = "parseBatchResult";
pub const HOOK_LIST_ARTIFACTS: &str = "listArtifacts";
pub const HOOK_BUILD_CONTENT_REQUEST: &str = "buildContentRequest";
/// Optional: the usage hooks a plugin may add. Their absence is not an error,
/// which is why they are not in the required set.
pub const HOOK_EXTRACT_USAGE: &str = "extractUsage";
pub const HOOK_EXTRACT_USAGE_ON_SUBMIT: &str = "extractUsageOnSubmit";
pub const HOOK_EXTRACT_USAGE_ON_COMPLETE: &str = "extractUsageOnComplete";

/// The top-level exports every plugin must have, and the two extra ones a
/// `batch` plugin needs instead of `buildQueryRequest`
/// (`pkg/jsplugin/registry.go:324,332`).
///
/// This is the check that would have caught the missing `models` field the same
/// way the unknown-field refusal now does: a plugin that loads but cannot be
/// called is worse than one that fails to load.
pub fn validate_required_hooks(manifest: &PluginManifest, exports: &PluginExports) -> Vec<String> {
    let mut required: Vec<&str> = vec![
        HOOK_BUILD_SUBMIT_REQUEST,
        HOOK_PARSE_SUBMIT_RESPONSE,
        HOOK_PARSE_TASK_RESULT,
    ];
    // A plugin that accepts an SSE submission must parse one event at a time,
    // and which hook depends on whether it asked for the delta capability.
    if manifest.submit_response_types.iter().any(|kind| kind == "sse") {
        if manifest
            .required_capabilities
            .iter()
            .any(|capability| capability == CAPABILITY_SUBMIT_SSE_DELTA)
        {
            required.push(HOOK_PARSE_SUBMIT_EVENT_DELTA);
        } else {
            required.push(HOOK_PARSE_SUBMIT_EVENT);
        }
    }
    if manifest.fetch_mode == FETCH_MODE_BATCH {
        required.push(HOOK_BUILD_BATCH_QUERY_REQUEST);
        required.push(HOOK_PARSE_BATCH_RESULT);
    } else {
        required.push(HOOK_BUILD_QUERY_REQUEST);
    }

    let mut problems = Vec::new();
    for hook in &required {
        if !exports.top_level.iter().any(|name| name == hook) {
            problems.push(format!(
                "plugin {} is missing required export {hook:?}",
                manifest.key
            ));
        }
    }
    // The two artifact hooks are optional, but only together: the content
    // endpoint needs both to answer anything (`pkg/jsplugin/registry.go:364`).
    let lists = exports.top_level.iter().any(|name| name == HOOK_LIST_ARTIFACTS);
    let builds = exports
        .top_level
        .iter()
        .any(|name| name == HOOK_BUILD_CONTENT_REQUEST);
    if lists != builds {
        problems.push(format!(
            "plugin {} must export listArtifacts and buildContentRequest together",
            manifest.key
        ));
    }
    problems
}

/// The two claim spellings, for callers that hold a claim they did not parse.
///
/// The parser keeps which spelling was written because the refusals differ
/// per spelling; a test or a host-side caller that only has a name and a
/// `supports` list states the spelling it means.
impl ProtocolClaim {
    /// A bare-name claim: `protocols: ["openai_image"]`.
    pub fn bare(name: &str) -> Self {
        ProtocolClaim {
            name: name.to_string(),
            models: Vec::new(),
            supports: Vec::new(),
            supports_declared: false,
            object_form: false,
        }
    }

    /// An object claim: `protocols: [{name: "openai_responses", supports: [...]}]`.
    pub fn object(name: &str, supports: &[&str]) -> Self {
        ProtocolClaim {
            name: name.to_string(),
            models: Vec::new(),
            supports: supports.iter().map(|s| s.to_string()).collect(),
            supports_declared: true,
            object_form: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plugin source in the shape the reference's fixtures use
    /// (`pkg/jsplugin/protocol_supports_test.go:128`), so the same plugin the
    /// reference accepts or refuses is the one tested here.
    fn protocol_plugin_source(key: &str, models: &str, protocols: &str, exports: &str) -> String {
        format!(
            r#"
export const meta = {{
    apiVersion: 1, key: {key:?}, name: {key:?}, version: "1.0.0",
    author: {{name: "Test"}},
    models: {models}, fetchMode: "per_task",
    protocols: {protocols},
}};
export function buildSubmitRequest() {{ return {{}}; }}
export function parseSubmitResponse() {{ return {{}}; }}
export function buildQueryRequest() {{ return {{}}; }}
export function parseTaskResult() {{ return {{}}; }}
{exports}
"#
        )
    }

    fn compile(key: &str, models: &str, protocols: &str, exports: &str) -> Result<PluginManifest, PluginError> {
        let host = PluginHost::start();
        runtime().block_on(host.load(
            protocol_plugin_source(key, models, protocols, exports),
            DEFAULT_CALL_TIMEOUT,
        ))
    }

    const RESPONSES_DECODE_ONLY: &str = r#"export const protocols = {openai_responses: {
        decodeRequest: function(ctx) { return ctx; }
    }};"#;
    const RESPONSES_DECODE_EVENTS: &str = r#"export const protocols = {openai_responses: {
        decodeRequest: function(ctx) { return ctx; },
        renderEvents: function() { return {events: [], state: null, done: false}; }
    }};"#;
    const RESPONSES_DECODE_FINAL: &str = r#"export const protocols = {openai_responses: {
        decodeRequest: function(ctx) { return ctx; },
        renderFinal: function(ctx, task) { return task; }
    }};"#;
    const RESPONSES_DECODE_BOTH: &str = r#"export const protocols = {openai_responses: {
        decodeRequest: function(ctx) { return ctx; },
        renderEvents: function() { return {events: [], state: null, done: false}; },
        renderFinal: function(ctx, task) { return task; }
    }};"#;
    const VIDEO_PROTOCOL_EXPORT: &str = r#"export const protocols = {openai_video: {
        decodeRequest: function(ctx) { return ctx; },
        render: function(ctx, task) { return task; }
    }};
    export function listArtifacts() { return []; }
    export function buildContentRequest() { return {}; }"#;

    /// The host table is the contract, so its shape is worth pinning. Ported
    /// from the reference so the two tables cannot drift
    /// (`pkg/jsplugin/routing.go:87`).
    #[test]
    fn the_host_table_matches_the_reference() {
        let names: Vec<&str> = HOST_PROTOCOLS.iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["openai_responses", "openai_video", "openai_image"]);
        assert!(host_protocol("nope").is_none());

        let responses = host_protocol(PROTOCOL_OPENAI_RESPONSES).expect("responses");
        let create = responses
            .operations
            .iter()
            .find(|op| op.name == "create")
            .expect("create");
        assert_eq!(create.methods, &["POST"]);
        assert_eq!(create.path, "/v1/responses");
        assert_eq!(create.body_kinds, &[BODY_JSON]);
        assert_eq!(create.required_members, &["decodeRequest"]);
        assert_eq!(
            responses
                .defined_modes()
                .iter()
                .map(|m| m.name)
                .collect::<Vec<_>>(),
            vec!["stream", "sync", "background"]
        );
        assert_eq!(
            create.modes.iter().find(|m| m.name == "stream").unwrap().hook,
            "renderEvents"
        );
        assert_eq!(
            create.modes.iter().find(|m| m.name == "sync").unwrap().hook,
            "renderFinal"
        );
        // Retrieval is host-answered and declares no members, which is what
        // makes "retrieve" not a claimable form.
        let retrieve = responses
            .operations
            .iter()
            .find(|op| op.name == "retrieve")
            .expect("retrieve");
        assert_eq!(retrieve.methods, &["GET"]);
        assert!(retrieve.required_members.is_empty());

        let video = host_protocol(PROTOCOL_OPENAI_VIDEO).expect("video");
        assert!(video.defined_modes().is_empty(), "video is mode-less");
        let content = video
            .operations
            .iter()
            .find(|op| op.name == "content")
            .expect("content");
        assert_eq!(content.methods, &["GET", "HEAD"]);
        assert_eq!(
            content.required_driver_hooks,
            &["listArtifacts", "buildContentRequest"]
        );

        let image = host_protocol(PROTOCOL_OPENAI_IMAGE).expect("image");
        assert!(image.defined_modes().is_empty(), "image is mode-less");
        for operation in image.operations {
            assert_eq!(operation.required_members, &["decodeRequest", "render"]);
            assert_eq!(operation.render_hook, Some("render"));
        }
        let generate = image
            .operations
            .iter()
            .find(|op| op.name == "generate")
            .expect("generate");
        assert_eq!(generate.methods, &["POST"]);
        assert_eq!(generate.path, "/v1/images/generations");
        assert_eq!(generate.body_kinds, &[BODY_JSON]);
        let edit = image
            .operations
            .iter()
            .find(|op| op.name == "edit")
            .expect("edit");
        assert_eq!(edit.path, "/v1/images/edits");
        assert_eq!(edit.body_kinds, &[BODY_JSON, BODY_MULTIPART]);
    }

    /// Every refusal the reference's `TestProtocolSupportsLoadErrors` pins,
    /// reproduced against the same fixtures
    /// (`pkg/jsplugin/protocol_supports_test.go:35`). The wording is the
    /// contract: a plugin author reads it and knows what to write.
    #[test]
    fn protocol_supports_load_errors_match_the_reference() {
        let cases: Vec<(&str, &str, &str, &str)> = vec![
            (
                "bare string",
                r#"["openai_responses"]"#,
                RESPONSES_DECODE_BOTH,
                r#"protocol "openai_responses" must declare supports; replace the bare string with {name: "openai_responses", supports: [...]} choosing from "stream", "sync", "background""#,
            ),
            (
                "object without supports",
                r#"[{name: "openai_responses"}]"#,
                RESPONSES_DECODE_BOTH,
                r#"protocol "openai_responses" must declare supports; add supports: [...] choosing from "stream", "sync", "background""#,
            ),
            (
                "supports sync but only renderEvents",
                r#"[{name: "openai_responses", supports: ["sync"]}]"#,
                RESPONSES_DECODE_EVENTS,
                r#"protocol "openai_responses" supports "sync" but does not export protocols.openai_responses.renderFinal; implement it or declare supports: ["stream"]"#,
            ),
            (
                "supports sync with only decodeRequest",
                r#"[{name: "openai_responses", supports: ["sync"]}]"#,
                RESPONSES_DECODE_ONLY,
                r#"protocol "openai_responses" supports "sync" but does not export protocols.openai_responses.renderFinal; implement it"#,
            ),
            (
                "supports stream but also exports renderFinal",
                r#"[{name: "openai_responses", supports: ["stream"]}]"#,
                RESPONSES_DECODE_BOTH,
                r#"protocol "openai_responses" exports protocols.openai_responses.renderFinal but no supported mode uses it; add "sync" or "background" to supports or remove the hook"#,
            ),
            (
                "supports sync and background but also exports renderEvents",
                r#"[{name: "openai_responses", supports: ["sync", "background"]}]"#,
                RESPONSES_DECODE_BOTH,
                r#"protocol "openai_responses" exports protocols.openai_responses.renderEvents but no supported mode uses it; add "stream" to supports or remove the hook"#,
            ),
            (
                "empty supports",
                r#"[{name: "openai_responses", supports: []}]"#,
                RESPONSES_DECODE_BOTH,
                r#"protocol "openai_responses" supports must contain at least one of "stream", "sync", "background""#,
            ),
            (
                "duplicate supports",
                r#"[{name: "openai_responses", supports: ["stream", "stream"]}]"#,
                RESPONSES_DECODE_BOTH,
                r#"protocol "openai_responses" supports must be unique"#,
            ),
            (
                "retrieve is not a mode",
                r#"[{name: "openai_responses", supports: ["retrieve"]}]"#,
                RESPONSES_DECODE_BOTH,
                r#"protocol "openai_responses" has no mode "retrieve"; retrieval of a created response is always available and is never declared"#,
            ),
            (
                "openai_video forbids supports",
                r#"[{name: "openai_video", supports: ["stream"]}]"#,
                VIDEO_PROTOCOL_EXPORT,
                r#"protocol "openai_video" does not define modes; supports is not allowed"#,
            ),
            (
                "unknown protocol forbids supports",
                r#"[{name: "openai_custom", supports: ["stream"]}]"#,
                "",
                r#"protocol "openai_custom" does not define modes; supports is not allowed"#,
            ),
        ];
        for (name, protocols, exports, want) in cases {
            let error = compile("acme", r#"["model"]"#, protocols, exports)
                .expect_err(&format!("{name} must be refused"));
            let text = error.to_string();
            assert!(text.contains(want), "{name}: {text:?} does not contain {want:?}");
        }
    }

    /// The claims the reference loads, including the mode-less protocols
    /// (`pkg/jsplugin/protocol_supports_test.go:113`).
    #[test]
    fn protocol_supports_happy_paths_match_the_reference() {
        let cases: Vec<(&str, &str, &str, &str, Vec<&str>)> = vec![
            (
                "stream only with renderEvents",
                r#"["model"]"#,
                r#"[{name: "openai_responses", supports: ["stream"]}]"#,
                RESPONSES_DECODE_EVENTS,
                vec!["stream"],
            ),
            (
                "sync and background with renderFinal",
                r#"["model"]"#,
                r#"[{name: "openai_responses", supports: ["sync", "background"]}]"#,
                RESPONSES_DECODE_FINAL,
                vec!["sync", "background"],
            ),
            (
                "all modes normalize to table order",
                r#"["model"]"#,
                r#"[{name: "openai_responses", supports: ["background", "stream", "sync"]}]"#,
                RESPONSES_DECODE_BOTH,
                vec!["stream", "sync", "background"],
            ),
            (
                "openai_video object without supports",
                r#"["gpt-5.5", "gpt-5.6"]"#,
                r#"[{name: "openai_video", models: ["gpt-5.5"]}]"#,
                VIDEO_PROTOCOL_EXPORT,
                vec![],
            ),
            (
                "bare openai_video",
                r#"["model"]"#,
                r#"["openai_video"]"#,
                VIDEO_PROTOCOL_EXPORT,
                vec![],
            ),
        ];
        for (name, models, protocols, exports, want) in cases {
            let manifest = compile("acme", models, protocols, exports)
                .unwrap_or_else(|error| panic!("{name} must load: {error}"));
            assert_eq!(manifest.protocols.len(), 1, "{name}");
            // Order is normalized to the table's, which is what the reference's
            // `orderProtocolSupports` does, so the same claim renders identically.
            let got: Vec<String> = manifest.protocols[0].supports.clone();
            let got: Vec<&str> = got.iter().map(|s| s.as_str()).collect();
            assert_eq!(got, want, "{name}");
        }
    }

    /// The image protocol's refusals, against the reference's fixtures
    /// (`pkg/jsplugin/routing_test.go:129`).
    #[test]
    fn the_image_protocol_demands_a_renderer_and_refuses_modes() {
        let error = compile(
            "image-no-render",
            r#"["image-a"]"#,
            r#"["openai_image"]"#,
            r#"export const protocols = {openai_image: {decodeRequest: function(ctx) { return {kind: "submit", model: ctx.model}; }}};"#,
        )
        .expect_err("missing render must be refused");
        assert!(
            error.to_string().contains(r#"missing hook "render""#),
            "{error}"
        );

        let error = compile(
            "image-modes",
            r#"["image-a"]"#,
            r#"[{name: "openai_image", supports: ["sync"]}]"#,
            r#"export const protocols = {openai_image: {
                decodeRequest: function(ctx) { return {kind: "submit", model: ctx.model}; },
                render: function(ctx, task) { return {data: []}; }
            }};"#,
        )
        .expect_err("supports on a mode-less protocol must be refused");
        assert!(error.to_string().contains("does not define modes"), "{error}");

        // The claim the reference binds at both image endpoints.
        let manifest = compile(
            "image-ok",
            r#"["image-a", "image-b"]"#,
            r#"["openai_image"]"#,
            r#"export const protocols = {openai_image: {
                decodeRequest: function(ctx) { return {kind: "submit", model: ctx.model, requestBody: ctx.body.value}; },
                render: function(ctx, task) { return {data: []}; }
            }};"#,
        )
        .expect("a complete image plugin loads");
        assert_eq!(manifest.protocols[0].name, "openai_image");
        assert!(!manifest.protocols[0].supports_declared);
    }

    /// Implementations the manifest never claimed, and exports the contract has
    /// retired, are both refused (`pkg/jsplugin/registry.go:492,499`).
    #[test]
    fn unclaimed_protocols_and_removed_exports_are_refused() {
        let error = compile(
            "acme",
            r#"["model"]"#,
            r#"[{name: "openai_responses", supports: ["sync"]}]"#,
            r#"export const protocols = {
                openai_responses: { decodeRequest: function(){return {};}, renderFinal: function(){return {};} },
                made_up: { decodeRequest: function(){return {};} }
            };"#,
        )
        .expect_err("an unclaimed protocol must be refused");
        assert!(
            error.to_string().contains("implements unclaimed protocol"),
            "{error}"
        );

        for removed in ["resolveRequest", "renderError", "renderers"] {
            let error = compile(
                "acme",
                r#"["model"]"#,
                r#"[{name: "openai_responses", supports: ["sync"]}]"#,
                &format!(
                    r#"export const protocols = {{openai_responses: {{
                        decodeRequest: function(){{return {{}};}},
                        renderFinal: function(){{return {{}};}}
                    }}}};
                    export function {removed}() {{ return {{}}; }}"#
                ),
            )
            .expect_err("a retired export must be refused");
            assert!(
                error.to_string().contains("is no longer supported"),
                "{removed}: {error}"
            );
        }
    }

    /// A driver hook the protocol's content operation needs is a top-level
    /// export, not a protocol member, and its absence has its own message
    /// (`pkg/jsplugin/registry.go:401`).
    #[test]
    fn a_missing_driver_hook_names_itself() {
        let error = compile(
            "acme-video",
            r#"["model"]"#,
            r#"["openai_video"]"#,
            r#"export const protocols = {openai_video: {
                decodeRequest: function(ctx) { return ctx; },
                render: function(ctx, task) { return task; }
            }};"#,
        )
        .expect_err("missing artifact hooks must be refused");
        let text = error.to_string();
        assert!(text.contains(r#"missing driver hook "listArtifacts""#), "{text}");
        assert!(
            text.contains(r#"missing driver hook "buildContentRequest""#),
            "{text}"
        );
    }

    /// A manifest field this host does not understand is refused, because a
    /// silently dropped field is how a manifest's `models` went missing before:
    /// the plugin loaded, looked healthy, and bound nothing
    /// (`pkg/jsplugin/registry.go:1004`).
    #[test]
    fn an_unknown_manifest_field_is_refused_by_name() {
        let error = compile(
            "acme",
            r#"["model"]"#,
            r#"[{name: "openai_responses", supports: ["sync"]}]"#,
            RESPONSES_DECODE_FINAL,
        )
        .expect("the reference's own meta must load");
        assert_eq!(error.key, "acme");

        let host = PluginHost::start();
        let source = protocol_plugin_source(
            "acme",
            r#"["model"]"#,
            r#"[{name: "openai_responses", supports: ["sync"]}]"#,
            RESPONSES_DECODE_FINAL,
        )
        // A field the reference has never heard of either.
        .replace("protocols:", "madeUpField: 1, protocols:");
        let error = runtime()
            .block_on(host.load(source, DEFAULT_CALL_TIMEOUT))
            .expect_err("an unknown field must be refused");
        let text = error.to_string();
        assert!(text.contains("unknown field"), "{text}");
        assert!(text.contains("madeUpField"), "{text}");
    }

    /// The top-level exports every plugin must have. This is the check the
    /// reference makes per manifest shape (`pkg/jsplugin/registry.go:324,332`),
    /// and the one that would have caught a plugin that loads but cannot be
    /// called.
    #[test]
    fn the_required_driver_hooks_are_demanded_by_name() {
        let host = PluginHost::start();
        let source = protocol_plugin_source(
            "acme",
            r#"["model"]"#,
            r#"[{name: "openai_responses", supports: ["sync"]}]"#,
            RESPONSES_DECODE_FINAL,
        )
        .replace("export function parseTaskResult() { return {}; }", "");
        let error = runtime()
            .block_on(host.load(source, DEFAULT_CALL_TIMEOUT))
            .expect_err("a missing driver hook must be refused");
        let text = error.to_string();
        assert!(
            text.contains(r#"missing required export "parseTaskResult""#),
            "{text}"
        );

        // A `batch` fetch mode needs a different pair of hooks than `per_task`.
        let batch = protocol_plugin_source(
            "acme",
            r#"["model"]"#,
            r#"[{name: "openai_responses", supports: ["sync"]}]"#,
            RESPONSES_DECODE_FINAL,
        )
        .replace(r#"fetchMode: "per_task""#, r#"fetchMode: "batch""#);
        let error = runtime()
            .block_on(host.load(batch, DEFAULT_CALL_TIMEOUT))
            .expect_err("a batch plugin needs its own hooks");
        let text = error.to_string();
        assert!(text.contains("buildBatchQueryRequest"), "{text}");
        assert!(text.contains("parseBatchResult"), "{text}");
        // The per-task hook it did export is not demanded instead.
        assert!(!text.contains("buildQueryRequest"), "{text}");
    }

    /// An SSE submission needs an event parser, and which one depends on whether
    /// the plugin asked for the delta capability
    /// (`pkg/jsplugin/registry.go:325`).
    #[test]
    fn an_sse_submission_demands_the_event_parser_it_asked_for() {
        let host = PluginHost::start();
        let sse = protocol_plugin_source(
            "acme",
            r#"["model"]"#,
            r#"[{name: "openai_responses", supports: ["sync"]}]"#,
            RESPONSES_DECODE_FINAL,
        )
        .replace(
            r#"fetchMode: "per_task""#,
            r#"fetchMode: "per_task", submitResponseTypes: ["sse"]"#,
        );
        let error = runtime()
            .block_on(host.load(sse.clone(), DEFAULT_CALL_TIMEOUT))
            .expect_err("an SSE plugin needs an event parser");
        assert!(
            error.to_string().contains("parseSubmitEvent"),
            "{error}"
        );

        // Asking for the delta capability switches which hook is required, and a
        // plugin that asks for it must also accept SSE.
        let delta = sse.replace(
            r#"submitResponseTypes: ["sse"]"#,
            r#"submitResponseTypes: ["sse"], requiredCapabilities: ["submit-sse-delta@1"]"#,
        );
        let error = runtime()
            .block_on(host.load(delta, DEFAULT_CALL_TIMEOUT))
            .expect_err("the delta capability needs its own hook");
        assert!(
            error.to_string().contains("parseSubmitEventDelta"),
            "{error}"
        );

        // A capability the host does not have is refused, and so is asking for
        // one twice.
        for (capabilities, want) in [
            (r#"["nope@1"]"#, "unsupported or duplicate required capability"),
            (
                r#"["json-clone@1", "json-clone@1"]"#,
                "unsupported or duplicate required capability",
            ),
            (r#"["submit-sse-delta@1"]"#, "requires submitResponseTypes to include sse"),
        ] {
            let source = protocol_plugin_source(
                "acme",
                r#"["model"]"#,
                r#"[{name: "openai_responses", supports: ["sync"]}]"#,
                RESPONSES_DECODE_FINAL,
            )
            .replace(
                r#"fetchMode: "per_task""#,
                &format!(r#"fetchMode: "per_task", requiredCapabilities: {capabilities}"#),
            );
            let error = runtime()
                .block_on(PluginHost::start().load(source, DEFAULT_CALL_TIMEOUT))
                .expect_err("a bad capability list must be refused");
            assert!(error.to_string().contains(want), "{error}");
        }
    }

    /// The manifest checks the reference makes on its own fields: a semver
    /// version, a usable icon, an HTTPS website, a valid upstream kind, and
    /// locale-keyed copy that is well formed.
    #[test]
    fn the_manifest_field_rules_match_the_reference() {
        fn refusal(replace_from: &str, replace_to: &str) -> String {
            let source = protocol_plugin_source(
                "acme",
                r#"["model"]"#,
                r#"[{name: "openai_responses", supports: ["sync"]}]"#,
                RESPONSES_DECODE_FINAL,
            )
            .replace(replace_from, replace_to);
            runtime()
                .block_on(PluginHost::start().load(source, DEFAULT_CALL_TIMEOUT))
                .expect_err("must be refused")
                .to_string()
        }

        // A version is semver, and the reference says so in those words.
        assert!(refusal(r#"version: "1.0.0""#, r#"version: "1.0""#)
            .contains("version must be semver"));
        assert!(refusal(r#"version: "1.0.0""#, r#"version: "01.0.0""#)
            .contains("version must be semver"));

        // An icon is a name or short text, never an inlined image.
        assert!(
            refusal(r#"author: {name: "Test"}"#, r#"author: {name: "Test"}, icon: "https://x/y.png""#)
                .contains("icon must be a LobeHub icon name")
        );
        assert!(refusal(
            r#"author: {name: "Test"}"#,
            &format!(r#"author: {{name: "Test"}}, icon: "{}""#, "x".repeat(200))
        )
        .contains("icon must not exceed"));

        // A website must be HTTPS and credential-free.
        assert!(refusal(r#"author: {name: "Test"}"#, r#"author: {name: "Test"}, website: "http://x/""#)
            .contains("website must be an absolute HTTPS URL"));
        assert!(
            refusal(r#"author: {name: "Test"}"#, r#"author: {name: "Test"}, website: "https://u:p@x/""#)
                .contains("website must be an absolute HTTPS URL")
        );

        // An upstream kind is one of two, and only once each.
        assert!(refusal(r#"author: {name: "Test"}"#, r#"author: {name: "Test"}, upstreams: ["nope"]"#)
            .contains("unsupported or duplicate upstream kind"));
        assert!(refusal(
            r#"author: {name: "Test"}"#,
            r#"author: {name: "Test"}, upstreams: ["new_api", "new_api"]"#
        )
        .contains("unsupported or duplicate upstream kind"));

        // Display copy is locale-keyed, must include a usable `en`, and has a
        // length ceiling (`pkg/jsplugin/registry.go:2046`).
        // The locale pattern is `^[a-zA-Z]{2,3}(-[a-zA-Z0-9]{2,8})*$`, so a
        // one-letter tag and an underscored one are both invalid while an unusual
        // but well-formed tag such as `zz` is accepted
        // (`pkg/jsplugin/registry.go:41`).
        assert!(refusal(r#"name: "acme""#, r#"name: "acme", description: {e: "hi"}"#)
            .contains("description has invalid locale"));
        assert!(refusal(r#"name: "acme""#, r#"name: "acme", description: {en_US: "hi"}"#)
            .contains("description has invalid locale"));
        let odd = protocol_plugin_source(
            "acme",
            r#"["model"]"#,
            r#"[{name: "openai_responses", supports: ["sync"]}]"#,
            RESPONSES_DECODE_FINAL,
        )
        .replace(
            r#"name: "acme""#,
            r#"name: "acme", description: {zz: "hi", en: "hi"}"#,
        );
        runtime()
            .block_on(PluginHost::start().load(odd, DEFAULT_CALL_TIMEOUT))
            .expect("a well-formed locale tag is accepted whatever the language");
        assert!(refusal(r#"name: "acme""#, r#"name: "acme", description: {"en": "  "}"#)
            .contains("must be a non-empty string"));

        // The author is required, and its url, when present, must be absolute.
        assert!(refusal(r#"author: {name: "Test"}"#, r#"author: {name: "  "}"#)
            .contains("author name is required"));
        assert!(
            refusal(r#"author: {name: "Test"}"#, r#"author: {name: "Test", url: "nope"}"#)
                .contains("author url must be an absolute HTTP(S) URL")
        );

        // A bare-string description is accepted and normalized to `en`, which is
        // what the reference's own `UnmarshalJSON` does
        // (`pkg/jsplugin/registry.go:60`).
        let manifest = compile(
            "acme",
            r#"["model"]"#,
            r#"[{name: "openai_responses", supports: ["sync"]}]"#,
            RESPONSES_DECODE_FINAL,
        )
        .expect("a bare description loads");
        assert!(manifest.description.english().is_empty(), "this fixture declares none");

        let described = protocol_plugin_source(
            "acme",
            r#"["model"]"#,
            r#"[{name: "openai_responses", supports: ["sync"]}]"#,
            RESPONSES_DECODE_FINAL,
        )
        .replace(
            r#"name: "acme""#,
            r#"name: "acme", description: "a demo plugin""#,
        );
        let manifest = runtime()
            .block_on(PluginHost::start().load(described, DEFAULT_CALL_TIMEOUT))
            .expect("a bare string description is the reference's own spelling");
        assert_eq!(manifest.description.english(), "a demo plugin");
    }

    /// A route the plugin declares is validated even though this host does not
    /// bind native routes yet, because a manifest the reference accepts must load
    /// here (`pkg/jsplugin/registry.go:1331`).
    #[test]
    fn declared_routes_are_validated() {
        fn refusal(routes: &str) -> String {
            let source = protocol_plugin_source(
                "acme",
                r#"["model"]"#,
                r#"[{name: "openai_responses", supports: ["sync"]}]"#,
                RESPONSES_DECODE_FINAL,
            )
            .replace(r#"fetchMode: "per_task""#, &format!(r#"fetchMode: "per_task", routes: {routes}"#));
            runtime()
                .block_on(PluginHost::start().load(source, DEFAULT_CALL_TIMEOUT))
                .expect_err("must be refused")
                .to_string()
        }

        assert!(refusal(r#"[{method: "BREW", path: "/x"}]"#).contains("route method"));
        assert!(refusal(r#"[{method: "POST", path: "x"}]"#).contains("must start with /"));
        assert!(refusal(r#"[{method: "POST", path: "/a/:1x"}]"#).contains("invalid parameter"));
        assert!(
            refusal(r#"[{method: "POST", path: "/a"}, {method: "POST", path: "/a"}]"#)
                .contains("duplicate route")
        );

        // A well-formed route list loads.
        let good = protocol_plugin_source(
            "acme",
            r#"["model"]"#,
            r#"[{name: "openai_responses", supports: ["sync"]}]"#,
            RESPONSES_DECODE_FINAL,
        )
        .replace(
            r#"fetchMode: "per_task""#,
            r#"fetchMode: "per_task", routes: [{method: "POST", path: "/apiary/jobs", type: "submit", decode: "decode", render: "render"}]"#,
        );
        let manifest = runtime()
            .block_on(PluginHost::start().load(good, DEFAULT_CALL_TIMEOUT))
            .expect("a valid route loads");
        assert_eq!(manifest.routes.len(), 1);
        assert_eq!(manifest.routes[0].kind, "submit");
    }

    /// The claims the validator sees directly, including the shapes a parsed
    /// manifest cannot produce.
    #[test]
    fn validate_reports_every_problem_it_finds() {
        // A declaration and a factual export disagreeing is one problem per
        // (claimed form, missing member), so an author who fixes one claim still
        // sees the other.
        let exports = PluginExports {
            protocol_members: [(
                "openai_responses".to_string(),
                Vec::<String>::new(),
            )]
            .into_iter()
            .collect(),
            top_level: Vec::new(),
            protocol_names: vec!["openai_responses".to_string()],
        };
        let problems = validate_protocol_claims(
            &[ProtocolClaim::object(
                "openai_responses",
                &["stream", "sync"],
            )],
            "acme",
            &exports,
        );
        assert_eq!(problems.len(), 3, "{problems:?}");
        for member in ["decodeRequest", "renderEvents", "renderFinal"] {
            assert!(
                problems.iter().any(|p| p.contains(member)),
                "{member} missing from {problems:?}"
            );
        }
        assert!(validate_protocol_claims(
            &[ProtocolClaim::object("openai_responses", &["stream", "sync"])],
            "acme",
            &PluginExports {
                protocol_members: [(
                    "openai_responses".to_string(),
                    vec![
                        "decodeRequest".to_string(),
                        "renderEvents".to_string(),
                        "renderFinal".to_string()
                    ],
                )]
                .into_iter()
                .collect(),
                top_level: Vec::new(),
                protocol_names: vec!["openai_responses".to_string()],
            },
        )
        .is_empty());
    }
    /// A plugin in the shape the reference actually writes them: an ES module
    /// exporting `meta` and `protocols`
    /// (`pkg/jsplugin/protocol_supports_test.go:13`).
    const DEMO: &str = r#"
        export const meta = {
            apiVersion: 1,
            key: "demo",
            name: "Demo Plugin",
            version: "1.0.0",
            description: "renders a fixed reply",
            author: { name: "Test" },
            models: ["acme-large"],
            fetchMode: "per_task",
            protocols: [{ name: "openai_responses", supports: ["sync"] }]
        };
        export const protocols = {
            openai_responses: {
                decodeRequest: function (ctx) {
                    return { model: ctx.body.model, input: ctx.body.input };
                },
                renderFinal: function (ctx, task) {
                    return { id: task.id, model: ctx.body.model, output: task.output };
                }
            }
        };
        export function buildSubmitRequest() { return {}; }
        export function parseSubmitResponse() { return {}; }
        export function buildQueryRequest() { return {}; }
        export function parseTaskResult() { return {}; }
    "#;

    /// A plain script is accepted too: `register(meta)` plus a `protocols` value.
    /// It costs nothing to support and is a reasonable thing to write.
    const SCRIPT_FORM: &str = r#"
        register({apiVersion:1, key:"scripted", name:"Scripted", version:"1.0.0",
            author:{name:"Test"}, models:["acme-large"], fetchMode:"per_task",
            protocols:[{name:"openai_responses", supports:["sync"]}]});
        var protocols = { openai_responses: {
            decodeRequest: function (ctx) { return ctx; },
            renderFinal: function (ctx, task) { return task; }
        } };
        function buildSubmitRequest() { return {}; }
        function parseSubmitResponse() { return {}; }
        function buildQueryRequest() { return {}; }
        function parseTaskResult() { return {}; }
    "#;

    pub(crate) fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    }

    #[test]
    fn a_key_must_match_the_reference_pattern() {
        assert!(valid_key("demo"));
        assert!(valid_key("my-plugin_2"));
        assert!(!valid_key(""));
        assert!(!valid_key("Demo"));          // uppercase
        assert!(!valid_key("-leading"));      // must start alphanumeric
        assert!(!valid_key("has space"));
        assert!(!valid_key(&"a".repeat(31))); // reference caps at 30
        assert!(valid_key(&"a".repeat(30)));
    }

    /// `import` pulls in code the operator never reviewed, in every spelling, so
    /// all of them are refused. Prose and strings about imports are not.
    #[test]
    fn imports_are_refused_but_prose_about_them_is_not() {
        assert_eq!(
            forbidden_import("import x from 'evil.js';").as_deref(),
            Some("evil.js")
        );
        assert_eq!(
            forbidden_import("import 'evil.js';").as_deref(),
            Some("evil.js")
        );
        assert_eq!(
            forbidden_import("import { a } from \"evil.js\";").as_deref(),
            Some("evil.js")
        );
        // Re-exporting another module's code is the same hazard.
        assert_eq!(
            forbidden_import("export { a } from 'evil.js';").as_deref(),
            Some("evil.js")
        );
        // Dynamic import is code loading by another name.
        assert!(forbidden_import("const m = import('evil.js');").is_some());

        // Comments and strings are stripped first, so these are clean.
        assert!(forbidden_import("// we import nothing here").is_none());
        assert!(forbidden_import("/* import x from 'evil.js' */").is_none());
        assert!(forbidden_import("const s = \"import x from 'evil.js'\";").is_none());
        // The word inside a longer identifier is not the keyword.
        assert!(forbidden_import("const a = obj.important;").is_none());
        assert!(forbidden_import("function reimport() { return 1; }").is_none());
    }

    /// A source-mapping directive can make the parser read arbitrary server
    /// files, so it is refused outright.
    #[test]
    fn source_mapping_directives_are_refused() {
        assert!(has_source_map_directive("//# sourceMappingURL=file:///etc/passwd"));
        assert!(!has_source_map_directive("export const meta = {};"));
    }

    /// The module contract end to end: an ES module loads, its `meta` becomes the
    /// manifest, and both protocol members answer with their own arity.
    #[test]
    fn a_module_plugin_loads_and_its_members_answer() {
        let host = PluginHost::start();
        runtime().block_on(async {
            let manifest = host
                .load(DEMO.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");
            assert_eq!(manifest.key, "demo");
            assert_eq!(manifest.name, "Demo Plugin");
            assert_eq!(manifest.protocols.len(), 1);
            assert_eq!(manifest.protocols[0].name, "openai_responses");
            assert_eq!(manifest.protocols[0].supports, vec!["sync".to_string()]);

            let ctx = serde_json::json!({
                "path": "/v1/responses",
                "method": "POST",
                "body": { "model": "acme-large", "input": "hello" }
            });
            // Addressed by protocol name, which is how the export is keyed.
            let upstream = host
                .decode_request("openai_responses", ctx.clone(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("decodeRequest");
            assert_eq!(upstream["model"], "acme-large");

            let task = serde_json::json!({ "id": "resp_1", "output": "hi there" });
            let rendered = host
                .render_final("openai_responses", ctx.clone(), task, DEFAULT_CALL_TIMEOUT)
                .await
                .expect("renderFinal");
            // The second argument arriving is what proves the arity; a
            // one-argument call would throw on `task.id`.
            assert_eq!(rendered["id"], "resp_1");
            assert_eq!(rendered["model"], "acme-large");
            assert_eq!(rendered["output"], "hi there");
        });
    }

    /// The script form is still accepted, because it is a reasonable thing to
    /// write and costs nothing to support.
    #[test]
    fn the_script_form_still_loads() {
        let host = PluginHost::start();
        runtime().block_on(async {
            let manifest = host
                .load(SCRIPT_FORM.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");
            assert_eq!(manifest.key, "scripted");
            let out = host
                .decode_request("openai_responses", serde_json::json!({"a": 1}), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("decodeRequest");
            assert_eq!(out["a"], 1);
        });
    }

    /// A module that throws while loading must report why. This is the case that
    /// a discarded evaluate() promise hides: the throw arrives as a rejected
    /// promise, not as an error from evaluate(), and the failure then looks like
    /// a plugin that simply declared nothing.
    #[test]
    fn a_module_that_throws_reports_the_reason() {
        let host = PluginHost::start();
        runtime().block_on(async {
            let blowing_up = "export const meta = undefinedFunction();";
            let error = host
                .load(blowing_up.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("must not load");
            let text = error.to_string();
            assert!(!text.contains("declared no manifest"), "{text}");
            assert!(text.contains("undefinedFunction"), "{text}");
        });
    }

    /// An unknown protocol, or one whose members were not exported, is refused
    /// with a message naming the missing member. The host asks the engine what
    /// the source exports rather than trusting the manifest.
    #[test]
    fn protocol_claims_are_checked_against_what_the_source_exports() {
        let host = PluginHost::start();
        runtime().block_on(async {
            const MISSING: &str = r#"
                export const meta = { apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                    author:{name:"Test"}, models:["acme-large"], fetchMode:"per_task",
                    protocols:[{name:"openai_responses", supports:["stream"]}] };
                export const protocols = { openai_responses: {
                    decodeRequest: function (ctx) { return ctx; }
                } };
            "#;
            const UNKNOWN: &str = r#"
                export const meta = { apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                    author:{name:"Test"}, models:["acme-large"], fetchMode:"per_task",
                    protocols:[{name:"made_up", supports:["stream"]}] };
                export const protocols = { made_up: { decodeRequest: function(){return {};} } };
            "#;
            const NO_SUPPORTS: &str = r#"
                export const meta = { apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                    author:{name:"Test"}, models:["acme-large"], fetchMode:"per_task",
                    protocols:[{name:"openai_responses", supports:[]}] };
                export const protocols = { openai_responses: { decodeRequest: function(){return {};} } };
            "#;
            for (label, source, want) in [
                ("missing member", MISSING, "renderEvents"),
                // The reference judges an unknown protocol's `supports` before it
                // says the protocol is unknown, so that is the message here too
                // (`pkg/jsplugin/registry.go:1385,1389`).
                ("unknown protocol", UNKNOWN, "does not define modes"),
                // An empty list is not the same as an omitted one: the reference
                // demands at least one form (`pkg/jsplugin/registry.go:1368`).
                ("no supports", NO_SUPPORTS, "supports must contain at least one of"),
            ] {
                let error = host
                    .load(source.to_string(), DEFAULT_CALL_TIMEOUT)
                    .await
                    .expect_err(label);
                let text = error.to_string();
                assert!(text.contains(want), "{label}: expected {want:?} in {text:?}");
            }
        });
    }

    #[test]
    fn an_unsupported_api_version_is_refused() {
        let host = PluginHost::start();
        runtime().block_on(async {
            let old = r#"
                export const meta = { apiVersion: 7, key:"old", name:"Old", version:"1.0.0", author:{name:"Test"}, models:["m"], fetchMode:"per_task" };
                export const protocols = {};
            "#;
            let error = host
                .load(old.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("must not load");
            assert!(
                matches!(error, PluginError::UnsupportedApiVersion { found: 7, .. }),
                "{error:?}"
            );
        });
    }

    /// A plugin that throws at *call* time is reported, and a member that does
    /// not exist is distinguished from one that failed.
    #[test]
    fn call_time_failures_are_reported() {
        let host = PluginHost::start();
        runtime().block_on(async {
            host.load(DEMO.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");
            let error = host
                .decode_request("openai_responses", serde_json::json!({}), DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("no body");
            assert!(matches!(error, PluginError::Hook { .. }), "{error:?}");

            let error = host
                .call_hook("demo", "renderEvents", serde_json::json!({}), DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("not implemented");
            let text = error.to_string();
            assert!(text.contains("renderEvents"), "{text}");

            let error = host
                .call_hook("nobody", "decodeRequest", serde_json::json!({}), DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("unknown plugin");
            assert!(matches!(error, PluginError::NoSuchHook { .. }), "{error:?}");
        });
    }

    /// A hook that never returns is cut off and the caller is released. This is
    /// the only defence a gateway has against a plugin it did not write.
    #[test]
    fn a_hook_that_never_returns_is_cut_off() {
        let host = PluginHost::start();
        runtime().block_on(async {
            const SPINNING: &str = r#"
                export const meta = { apiVersion:1, key:"spin", name:"Spin", version:"1.0.0",
                    author:{name:"Test"}, models:["acme-large"], fetchMode:"per_task",
                    protocols:[{name:"openai_responses", supports:["sync"]}] };
                export const protocols = { openai_responses: {
                    decodeRequest: function () { while (true) {} },
                    renderFinal: function () { while (true) {} }
                } };
                export function buildSubmitRequest() { return {}; }
                export function parseSubmitResponse() { return {}; }
                export function buildQueryRequest() { return {}; }
                export function parseTaskResult() { return {}; }
            "#;
            host.load(SPINNING.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");
            let started = std::time::Instant::now();
            let error = host
                .decode_request(
                    "openai_responses",
                    serde_json::json!({}),
                    Duration::from_millis(300),
                )
                .await
                .expect_err("must time out");
            assert!(matches!(error, PluginError::Timeout(_)), "{error:?}");
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the caller waited far longer than the timeout"
            );
        });
    }

}
