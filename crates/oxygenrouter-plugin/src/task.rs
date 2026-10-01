//! The task driver a plugin implements, and the task view it renders from.
//!
//! Every host protocol this instance serves is `fetchMode: per_task`: a plugin's
//! `decodeRequest` produces a *task submission*, the host performs it, persists
//! the task, polls it to a terminal state, and only then is there a task view for
//! the plugin's render hook to shape
//! (`relay/channel/task/jsplugin/adaptor.go:453`, `controller/plugin_protocol.go:925`).
//!
//! This module is the host half of that contract: the descriptor a plugin states
//! through `buildSubmitRequest`, the two answers it gives from
//! `parseSubmitResponse` and `parseTaskResult`, the context `buildQueryRequest`
//! reads, and the deliberately narrow view a renderer sees. The SSRF guard is
//! here because a plugin states an absolute URL and a channel credential is
//! attached to it, so the host has to be the one that decides which hosts are
//! allowed (`pkg/jsplugin/request.go:12`).
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use serde::{Deserialize, Serialize};

/// The lifecycle of a submitted task, using the reference's literals so a plugin
/// that branches on a status string branches on the same one
/// (`model/task.go:36`).
pub const STATUS_NOT_START: &str = "NOT_START";
pub const STATUS_SUBMITTED: &str = "SUBMITTED";
pub const STATUS_QUEUED: &str = "QUEUED";
pub const STATUS_IN_PROGRESS: &str = "IN_PROGRESS";
pub const STATUS_FAILURE: &str = "FAILURE";
pub const STATUS_SUCCESS: &str = "SUCCESS";
pub const STATUS_UNKNOWN: &str = "UNKNOWN";

/// Whether a status is terminal, which is when a task's quota settles and when a
/// renderer has something final to shape (`model/task.go:26`).
pub fn status_is_terminal(status: &str) -> bool {
    status == STATUS_SUCCESS || status == STATUS_FAILURE
}

/// The upstream request a plugin states, from `buildSubmitRequest`.
///
/// Ported from `requestDescriptor` (`relay/channel/task/jsplugin/adaptor.go:36`).
/// A part is either a scalar, so it becomes a form field, or a reference to an
/// uploaded file, so the bytes never enter the plugin at all.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RequestDescriptor {
    /// `json` unless declared, because that is what the host sends by default
    /// (`adaptor.go:1221`).
    #[serde(rename = "responseType", default)]
    pub response_type: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub body: serde_json::Value,
    /// Drop the channel credential from this one request, for an upstream call
    /// that must not carry it.
    #[serde(default)]
    pub credentialless: bool,
    /// A sub-operation the plugin wants, which the host records on the task.
    #[serde(default)]
    pub action: String,
    /// The model this request is for, which must be the pinned one
    /// (`adaptor.go:1242`).
    #[serde(default)]
    pub model: String,
    /// The upstream model to bill and record instead of the client's
    /// (`adaptor.go:1256`).
    #[serde(rename = "rewriteModel", default)]
    pub rewrite_model: String,
    /// `json`, `form` or `multipart`, deciding how `parts` and `body` are sent.
    #[serde(rename = "bodyType", default)]
    pub body_type: String,
    #[serde(default)]
    pub parts: Vec<RequestPart>,
}

/// One field of a form or multipart submission.
///
/// Either a value or a `fileRef` into the request's uploaded files, never both
/// (`adaptor.go:50`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RequestPart {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: serde_json::Value,
    /// A reference produced by [`crate::file_reference`].
    #[serde(rename = "fileRef", default)]
    pub file_ref: String,
    #[serde(default)]
    pub filename: String,
}

impl RequestDescriptor {
    /// The body encoding, with the reference's default applied.
    pub fn body_type_or_default(&self) -> &str {
        if self.body_type.trim().is_empty() {
            "json"
        } else {
            self.body_type.trim()
        }
    }

    /// The response encoding, with the reference's default applied.
    pub fn response_type_or_default(&self) -> &str {
        if self.response_type.trim().is_empty() {
            "json"
        } else {
            self.response_type.trim()
        }
    }

    /// The HTTP method, defaulting to POST (`adaptor.go:1260`).
    pub fn method_or_default(&self) -> &str {
        if self.method.trim().is_empty() {
            "POST"
        } else {
            self.method.trim()
        }
    }
}

/// What a plugin answers from `parseSubmitResponse`.
///
/// Ported from `submitResponse` (`adaptor.go:57`). `task_id` is what the host
/// polls with; `immediate` is an upstream that answered already, which is how the
/// synchronous image protocol works; `state` is opaque data the host stores and
/// hands back on every poll.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubmitOutcome {
    #[serde(rename = "taskId", default)]
    pub task_id: String,
    #[serde(rename = "taskData", default)]
    pub task_data: serde_json::Value,
    #[serde(default)]
    pub immediate: Option<TaskResult>,
    #[serde(default)]
    pub state: serde_json::Value,
}

/// One poll's answer, from `parseTaskResult` or from an immediate submission.
///
/// Ported from `taskResult` (`adaptor.go:63`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskResult {
    #[serde(default)]
    pub code: i64,
    #[serde(rename = "taskId", default)]
    pub task_id: String,
    /// One of the [`STATUS_*`](STATUS_SUCCESS) literals.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub progress: String,
    /// Why it failed, when it did.
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub url: String,
    #[serde(rename = "remoteUrl", default)]
    pub remote_url: String,
    #[serde(rename = "completionTokens", default)]
    pub completion_tokens: f64,
    #[serde(rename = "totalTokens", default)]
    pub total_tokens: f64,
    #[serde(default)]
    pub state: serde_json::Value,
}

impl TaskResult {
    /// A successful, immediately-complete result, which is what a synchronous
    /// upstream answers (`controller/plugin_protocol_image.go:97`).
    pub fn immediate_success() -> Self {
        TaskResult {
            status: STATUS_SUCCESS.to_string(),
            progress: "100%".to_string(),
            ..Default::default()
        }
    }

    pub fn is_terminal(&self) -> bool {
        status_is_terminal(&self.status)
    }

    /// A count the host can bill, clamped the way the reference clamps it: a
    /// negative or fractional token count is not a count
    /// (`adaptor.go:1708`).
    pub fn positive_int(value: f64) -> i64 {
        if value.is_finite() && value > 0.0 {
            value as i64
        } else {
            0
        }
    }
}

/// The task as a plugin's renderer sees it.
///
/// Deliberately narrow (`service/task_plugin_view.go:11`): the task's own id and
/// lifecycle, and the `data` the plugin itself produced -- never the private
/// upstream id, the channel, or the credential. The private id is rewritten to
/// the public one only inside fields that *are* task ids, so a URL that happens
/// to contain the id keeps working (`task_plugin_view.go:37`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskView {
    #[serde(rename = "taskId")]
    pub task_id: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub progress: String,
    #[serde(rename = "failReason", default)]
    pub fail_reason: String,
    #[serde(rename = "createdAt", default)]
    pub created_at: i64,
    #[serde(rename = "updatedAt", default)]
    pub updated_at: i64,
    #[serde(rename = "finishedAt", default)]
    pub finished_at: i64,
    #[serde(default)]
    pub data: serde_json::Value,
}

impl TaskView {
    /// Build the view, rewriting the private upstream id wherever a field is
    /// explicitly a task id.
    pub fn build(
        task_id: &str,
        platform: &str,
        status: &str,
        progress: &str,
        fail_reason: &str,
        created_at: i64,
        updated_at: i64,
        finished_at: i64,
        data: serde_json::Value,
        private_task_id: &str,
    ) -> Self {
        TaskView {
            task_id: task_id.to_string(),
            platform: platform.to_string(),
            status: status.to_string(),
            progress: progress.to_string(),
            fail_reason: fail_reason.to_string(),
            created_at,
            updated_at,
            finished_at,
            data: replace_private_task_id(data, private_task_id, task_id),
        }
    }
}

/// Rewrite the private upstream id to the public one, but only in fields that
/// are task ids by name. A map key, an opaque string or a URL is left alone
/// (`service/task_plugin_view.go:37`).
pub fn replace_private_task_id(
    value: serde_json::Value,
    private_task_id: &str,
    public_task_id: &str,
) -> serde_json::Value {
    if private_task_id.is_empty() || private_task_id == public_task_id {
        return value;
    }
    match value {
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .map(|item| replace_private_task_id(item, private_task_id, public_task_id))
                .collect(),
        ),
        serde_json::Value::Object(map) => {
            let mut replaced = serde_json::Map::with_capacity(map.len());
            for (key, item) in map {
                let is_task_id_field = key == "id" || key == "task_id" || key == "taskId";
                if is_task_id_field && item.as_str() == Some(private_task_id) {
                    replaced.insert(
                        key,
                        serde_json::Value::String(public_task_id.to_string()),
                    );
                    continue;
                }
                replaced.insert(
                    key,
                    replace_private_task_id(item, private_task_id, public_task_id),
                );
            }
            serde_json::Value::Object(replaced)
        }
        other => other,
    }
}

/// The context a plugin's `buildQueryRequest` and `parseTaskResult` read.
///
/// Ported from `queryContext` (`adaptor.go:1013`): the upstream task id, the
/// public one, the action, the models, the plugin's own persisted `data` and
/// `state`, and the channel's base URL. The credential is applied separately, by
/// the caller that owns it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueryContext {
    #[serde(rename = "taskId", default)]
    pub task_id: String,
    #[serde(rename = "publicTaskId", default)]
    pub public_task_id: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub model: String,
    #[serde(rename = "upstreamModel", default)]
    pub upstream_model: String,
    #[serde(rename = "baseUrl", default)]
    pub base_url: String,
    #[serde(default)]
    pub data: serde_json::Value,
    #[serde(default)]
    pub state: serde_json::Value,
    /// Whether the upstream is another instance of this gateway rather than the
    /// vendor itself, which changes what the plugin's hooks may assume
    /// (`adaptor.go:1096`).
    #[serde(default)]
    pub upstream: Option<UpstreamKind>,
}

/// Which upstream a plugin is addressing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamKind {
    pub kind: String,
}

/// The absolute-host guard for a plugin-stated URL.
///
/// A descriptor names an absolute URL and the channel's credential is attached
/// to it, so the host decides which hosts are acceptable: the channel's own base
/// URL, or one an administrator listed in the manifest's `allowedHosts`
/// (`pkg/jsplugin/request.go:12`).
pub fn validate_request_url(
    request_url: &str,
    base_url: &str,
    allowed_hosts: &[String],
) -> Result<(), String> {
    let request = parse_absolute_url(request_url)
        .ok_or_else(|| "plugin request URL must be absolute".to_string())?;
    let base = parse_absolute_url(base_url)
        .ok_or_else(|| "channel base URL is invalid".to_string())?;
    let request_host = canonical_host(&request);
    if request_host == canonical_host(&base) {
        return Ok(());
    }
    for allowed in allowed_hosts {
        // Parsed with the request's scheme so an explicit default port in an
        // allow-list entry matches the same way it does in the request.
        let candidate = format!("{}://{}", request.scheme, allowed.trim());
        if let Some(parsed) = parse_absolute_url(&candidate) {
            if request_host == canonical_host(&parsed) {
                return Ok(());
            }
        }
    }
    Err(format!(
        "plugin request host {:?} is not allowed",
        request.host
    ))
}

/// A parsed absolute URL, kept minimal because only the host and scheme matter
/// here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleUrl {
    pub scheme: String,
    pub host: String,
    pub port: String,
    pub path: String,
}

impl std::fmt::Display for SimpleUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}://{}", self.scheme, self.host)?;
        if !self.port.is_empty() {
            write!(f, ":{}", self.port)?;
        }
        write!(f, "{}", self.path)
    }
}

/// Parse an absolute URL, or return `None` when it is relative or malformed
/// enough that its host cannot be trusted.
pub fn parse_absolute_url(value: &str) -> Option<SimpleUrl> {
    let (scheme, rest) = value.split_once("://")?;
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
        return None;
    }
    let (authority, path) = match rest.find(['/', '?', '#']) {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => {
            (host.to_string(), port.to_string())
        }
        _ => (authority.to_string(), String::new()),
    };
    if host.is_empty() {
        return None;
    }
    Some(SimpleUrl {
        scheme: scheme.to_ascii_lowercase(),
        host: host.to_ascii_lowercase(),
        port,
        path: if path.is_empty() {
            "/".to_string()
        } else {
            path.to_string()
        },
    })
}

/// The host and port that identity comparisons use: a default port is not part
/// of a host's identity (`pkg/jsplugin/request.go:36`).
pub fn canonical_host(url: &SimpleUrl) -> String {
    let default = (url.scheme == "http" && url.port == "80")
        || (url.scheme == "https" && url.port == "443")
        || url.port.is_empty();
    if default {
        url.host.clone()
    } else {
        format!("{}:{}", url.host, url.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two response spellings a plugin may state, each defaulted the way the
    /// reference defaults it (`adaptor.go:1221,1260`).
    #[test]
    fn a_descriptor_defaults_its_encodings_and_method() {
        let descriptor: RequestDescriptor = serde_json::from_value(serde_json::json!({
            "url": "https://vendor.example/images"
        }))
        .expect("descriptor");
        assert_eq!(descriptor.body_type_or_default(), "json");
        assert_eq!(descriptor.response_type_or_default(), "json");
        assert_eq!(descriptor.method_or_default(), "POST");
        assert!(!descriptor.credentialless);

        let stated: RequestDescriptor = serde_json::from_value(serde_json::json!({
            "url": "https://vendor.example/images",
            "method": "put",
            "bodyType": "multipart",
            "responseType": "sse",
            "rewriteModel": "vendor-model",
            "parts": [{ "name": "prompt", "value": "a cat" },
                      { "name": "image", "fileRef": "request_file:image" }],
            "credentialless": true
        }))
        .expect("descriptor");
        assert_eq!(stated.method_or_default(), "put");
        assert_eq!(stated.body_type_or_default(), "multipart");
        assert_eq!(stated.response_type_or_default(), "sse");
        assert_eq!(stated.rewrite_model, "vendor-model");
        assert_eq!(stated.parts.len(), 2);
        assert_eq!(stated.parts[1].file_ref, "request_file:image");
        assert!(stated.credentialless);
    }

    /// A submission names the upstream task to poll, or answers already.
    #[test]
    fn a_submission_reports_a_task_id_or_an_immediate_result() {
        let pending: SubmitOutcome = serde_json::from_value(serde_json::json!({
            "taskId": "vendor-1",
            "taskData": { "kind": "video" },
            "state": { "cursor": 3 }
        }))
        .expect("outcome");
        assert_eq!(pending.task_id, "vendor-1");
        assert_eq!(pending.task_data["kind"], "video");
        assert_eq!(pending.state["cursor"], 3);
        assert!(pending.immediate.is_none());

        let immediate: SubmitOutcome = serde_json::from_value(serde_json::json!({
            "taskId": "vendor-2",
            "immediate": { "status": "SUCCESS", "progress": "100%", "url": "https://cdn/1.png" }
        }))
        .expect("outcome");
        let result = immediate.immediate.expect("immediate");
        assert!(result.is_terminal());
        assert_eq!(result.url, "https://cdn/1.png");
    }

    /// Only the two success and failure literals are terminal, so a poll that
    /// answers `IN_PROGRESS` keeps polling and settles nothing
    /// (`model/task.go:26`).
    #[test]
    fn only_success_and_failure_are_terminal() {
        for status in [
            STATUS_NOT_START,
            STATUS_SUBMITTED,
            STATUS_QUEUED,
            STATUS_IN_PROGRESS,
            STATUS_UNKNOWN,
            "",
        ] {
            assert!(!status_is_terminal(status), "{status} must not be terminal");
        }
        assert!(status_is_terminal(STATUS_SUCCESS));
        assert!(status_is_terminal(STATUS_FAILURE));
    }

    /// A token count is a count, so a fractional, negative or non-finite value
    /// is not one (`adaptor.go:1708`).
    #[test]
    fn a_token_count_is_clamped_like_the_reference_clamps_it() {
        assert_eq!(TaskResult::positive_int(8.0), 8);
        assert_eq!(TaskResult::positive_int(0.9), 0);
        assert_eq!(TaskResult::positive_int(-3.0), 0);
        assert_eq!(TaskResult::positive_int(0.0), 0);
        assert_eq!(TaskResult::positive_int(f64::NAN), 0);
        assert_eq!(TaskResult::positive_int(f64::INFINITY), 0);
    }

    /// The view carries the task's lifecycle and the plugin's own data, and the
    /// private upstream id is rewritten only inside fields named as task ids
    /// (`service/task_plugin_view.go:37`).
    #[test]
    fn the_task_view_rewrites_the_private_id_only_in_task_id_fields() {
        let view = TaskView::build(
            "task_public",
            "alibaba",
            STATUS_SUCCESS,
            "100%",
            "",
            10,
            20,
            30,
            serde_json::json!({
                "id": "vendor-9",
                "task_id": "vendor-9",
                "taskId": "vendor-9",
                "url": "https://cdn.example/vendor-9.png",
                "nested": [{ "id": "vendor-9" }, { "id": "other" }],
                "third_party": "vendor-9"
            }),
            "vendor-9",
        );
        assert_eq!(view.task_id, "task_public");
        assert_eq!(view.data["id"], "task_public");
        assert_eq!(view.data["task_id"], "task_public");
        assert_eq!(view.data["taskId"], "task_public");
        assert_eq!(view.data["nested"][0]["id"], "task_public");
        assert_eq!(view.data["nested"][1]["id"], "other");
        // A URL containing the id is preserved, and so is an opaque field.
        assert_eq!(view.data["url"], "https://cdn.example/vendor-9.png");
        assert_eq!(view.data["third_party"], "vendor-9");

        // When the ids agree, or there is no private one, nothing is touched.
        let untouched = TaskView::build(
            "same",
            "p",
            STATUS_SUCCESS,
            "",
            "",
            0,
            0,
            0,
            serde_json::json!({ "id": "same" }),
            "same",
        );
        assert_eq!(untouched.data["id"], "same");
    }

    /// A plugin's URL is only acceptable on the channel's own host or on one the
    /// manifest allowed, because the channel credential rides with the request
    /// (`pkg/jsplugin/request.go:12`).
    #[test]
    fn a_plugin_url_is_guarded_to_the_channel_host_and_its_allow_list() {
        let allowed = vec!["cdn.vendor.example".to_string(), "alt.example:8443".to_string()];

        // The channel's own host, with or without the default port.
        assert!(validate_request_url("https://api.vendor.example/v1/x", "https://api.vendor.example", &[]).is_ok());
        assert!(validate_request_url("https://api.vendor.example/x", "https://api.vendor.example:443", &[]).is_ok());
        // A declared allow-list entry, including one with a non-default port.
        assert!(validate_request_url("https://cdn.vendor.example/x", "https://api.vendor.example", &allowed).is_ok());
        assert!(validate_request_url("https://alt.example:8443/x", "https://api.vendor.example", &allowed).is_ok());
        // The same host on a different port is a different host.
        assert!(validate_request_url("https://alt.example:9443/x", "https://api.vendor.example", &allowed).is_err());

        // Anything else is refused, and the refusal names the host.
        let error = validate_request_url("https://evil.example/x", "https://api.vendor.example", &allowed)
            .expect_err("must be refused");
        assert!(error.contains("evil.example"), "{error}");
        assert!(error.contains("not allowed"), "{error}");

        // A relative URL, a credential-bearing one, and an unusable base URL are
        // all refused before any comparison happens.
        assert!(validate_request_url("/v1/x", "https://api.vendor.example", &[]).is_err());
        assert!(validate_request_url("https://u:p@api.vendor.example/x", "https://api.vendor.example", &[]).is_err());
        assert!(validate_request_url("https://api.vendor.example/x", "not a url", &[]).is_err());
    }

    /// A query context carries what a poll needs and nothing that belongs to the
    /// channel: the credential is applied by the caller that owns it
    /// (`adaptor.go:1013`).
    #[test]
    fn a_query_context_carries_the_task_and_its_persisted_state() {
        let context = QueryContext {
            task_id: "vendor-1".into(),
            public_task_id: "task_public".into(),
            action: "text_to_video".into(),
            model: "wan-video".into(),
            upstream_model: "wan-video-v2".into(),
            base_url: "https://api.vendor.example".into(),
            data: serde_json::json!({ "kind": "video" }),
            state: serde_json::json!({ "cursor": 3 }),
            upstream: Some(UpstreamKind {
                kind: "vendor".into(),
            }),
        };
        let value = serde_json::to_value(&context).expect("json");
        assert_eq!(value["taskId"], "vendor-1");
        assert_eq!(value["publicTaskId"], "task_public");
        assert_eq!(value["upstreamModel"], "wan-video-v2");
        assert_eq!(value["state"]["cursor"], 3);
        assert_eq!(value["upstream"]["kind"], "vendor");
        // The credential is not part of this shape at all.
        assert!(value.get("apiKey").is_none());
        assert!(value.get("authHeader").is_none());
    }
}
