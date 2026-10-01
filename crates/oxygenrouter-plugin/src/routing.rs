//! Endpoint routing for plugin-served protocols.
//!
//! Two jobs, both ported from `pkg/jsplugin/routing.go`:
//!
//! * binding a plugin to a client endpoint. The reference indexes a binding by
//!   `method + path + model` (`routing.go:994,705`), not by path alone, because
//!   one endpoint serves many models and two plugins may share it -- each
//!   declaring a disjoint model set. A path-only lookup cannot express that and
//!   would hand a model to a plugin that never claimed it.
//! * presenting the request to the plugin. The reference's context is
//!   `path`, `method`, `params`, `query`, `body`, `protocol`, `operation`,
//!   `model`, `upstreamModel` and `stream` (`routing.go:283,342`), and its body
//!   is a tagged union -- `{kind: "json", value: ...}` -- because a hook has to
//!   tell a JSON body from a form body from no body at all
//!   (`middleware/task_plugin.go:793`).
//!
//! Nothing here interprets a vendor dialect; that stays the plugin's job.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::BTreeMap;

use crate::{BODY_FORM, BODY_JSON, BODY_MULTIPART, BODY_NONE, HostOperation, HostProtocol};

/// The request-body encodings a client request can carry.
///
/// The names are the reference's (`routing.go:64`) and travel to the plugin as
/// `body.kind`, so a hook that branches on them branches on the same strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    None,
    Json,
    Form,
    Multipart,
}

impl BodyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BodyKind::None => BODY_NONE,
            BodyKind::Json => BODY_JSON,
            BodyKind::Form => BODY_FORM,
            BodyKind::Multipart => BODY_MULTIPART,
        }
    }

    /// The kind a declared name denotes, for matching a request against the
    /// `body_kinds` an operation accepts.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            BODY_NONE => Some(BodyKind::None),
            BODY_JSON => Some(BodyKind::Json),
            BODY_FORM => Some(BodyKind::Form),
            BODY_MULTIPART => Some(BodyKind::Multipart),
            _ => None,
        }
    }
}

/// One uploaded file, as a plugin addresses it.
///
/// The bytes stay host-owned; a plugin gets an opaque reference and asks for the
/// content by name, which is why the reference never puts a file body in the
/// context (`routing.go:248,289`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyFile {
    pub reference: String,
    pub field: String,
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
}

/// The reference spelling of a file reference.
///
/// The first file of a field keeps the plain `request_file:<field>` form and
/// later files append `#<index>`, so a field that repeats (`image[]`) can be
/// addressed per file (`routing.go:252`).
pub fn file_reference(field: &str, index: usize) -> String {
    if index == 0 {
        format!("request_file:{field}")
    } else {
        format!("request_file:{field}#{index}")
    }
}

/// The client request, in the shape a plugin's hooks read.
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    pub path: String,
    pub method: String,
    pub params: BTreeMap<String, String>,
    pub query: BTreeMap<String, Vec<String>>,
    body: Body,
    /// The decoded body a hook most often wants: the JSON object, or the form's
    /// text fields. Not serialized into the context -- it is the *unwrapped*
    /// half of `body`, and a hook that wants it takes it from `body.value`.
    pub request_body: serde_json::Value,
    pub files: Vec<BodyFile>,
}

#[derive(Debug, Clone, Default, PartialEq)]
enum Body {
    #[default]
    None,
    Json(serde_json::Value),
    Form(BTreeMap<String, Vec<String>>),
    Multipart {
        fields: BTreeMap<String, Vec<String>>,
        files: Vec<BodyFile>,
    },
}

impl RequestContext {
    pub fn new(path: impl Into<String>, method: impl Into<String>) -> Self {
        RequestContext {
            path: path.into(),
            method: method.into(),
            params: BTreeMap::new(),
            query: BTreeMap::new(),
            body: Body::None,
            request_body: serde_json::Value::Null,
            files: Vec::new(),
        }
    }

    /// Attach a JSON body, the form an OpenAI-compatible client sends.
    pub fn with_json(mut self, value: serde_json::Value) -> Self {
        self.request_body = value.clone();
        self.body = Body::Json(value);
        self
    }

    /// Attach URL-encoded form fields.
    pub fn with_form(mut self, fields: BTreeMap<String, Vec<String>>) -> Self {
        self.request_body = serde_json::to_value(&fields).unwrap_or(serde_json::Value::Null);
        self.body = Body::Form(fields);
        self
    }

    /// Attach multipart text fields and the files that came with them.
    pub fn with_multipart(
        mut self,
        fields: BTreeMap<String, Vec<String>>,
        files: Vec<BodyFile>,
    ) -> Self {
        self.request_body = serde_json::to_value(&fields).unwrap_or(serde_json::Value::Null);
        self.files = files.clone();
        self.body = Body::Multipart { fields, files };
        self
    }

    pub fn body_kind(&self) -> BodyKind {
        match self.body {
            Body::None => BodyKind::None,
            Body::Json(_) => BodyKind::Json,
            Body::Form(_) => BodyKind::Form,
            Body::Multipart { .. } => BodyKind::Multipart,
        }
    }

    /// The `body` a hook reads: a tagged union carrying why-it-is-empty.
    pub fn body_value(&self) -> serde_json::Value {
        match &self.body {
            Body::None => serde_json::json!({ "kind": BODY_NONE }),
            Body::Json(value) => serde_json::json!({ "kind": BODY_JSON, "value": value }),
            Body::Form(fields) => serde_json::json!({ "kind": BODY_FORM, "fields": fields }),
            Body::Multipart { fields, files } => serde_json::json!({
                "kind": BODY_MULTIPART,
                "fields": fields,
                "files": files.iter().map(file_to_value).collect::<Vec<_>>(),
            }),
        }
    }

    /// The context as the plugin sees it.
    pub fn js_value(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path,
            "method": self.method,
            "params": self.params,
            "query": self.query,
            "body": self.body_value(),
        })
    }
}

fn file_to_value(file: &BodyFile) -> serde_json::Value {
    serde_json::json!({
        "ref": file.reference,
        "field": file.field,
        "filename": file.filename,
        "mimeType": file.mime_type,
        "size": file.size,
    })
}

/// A request plus the protocol coordinates a plugin's hooks are keyed by.
#[derive(Debug, Clone)]
pub struct ProtocolContext {
    pub request: RequestContext,
    pub protocol: &'static str,
    pub operation: &'static str,
    pub model: String,
    /// The declared machine identity when `model` is a channel alias; empty
    /// otherwise, and omitted from the context when empty, because a hook that
    /// keys a rate table by model must not see "" and think it is a model
    /// (`routing.go:347`).
    pub upstream_model: String,
    pub stream: bool,
}

impl ProtocolContext {
    pub fn js_value(&self) -> serde_json::Value {
        let mut value = self.request.js_value();
        let object = value.as_object_mut().expect("request context is an object");
        object.insert("protocol".to_string(), serde_json::json!(self.protocol));
        object.insert("operation".to_string(), serde_json::json!(self.operation));
        object.insert("model".to_string(), serde_json::json!(self.model));
        if !self.upstream_model.is_empty() {
            object.insert(
                "upstreamModel".to_string(),
                serde_json::json!(self.upstream_model),
            );
        }
        object.insert("stream".to_string(), serde_json::json!(self.stream));
        value
    }
}

/// The HTTP method a lookup uses, canonicalized.
///
/// The reference refuses anything outside its five-method pattern rather than
/// uppercasing blindly, so a lookup for `BREW` misses instead of matching a
/// `POST` (`routing.go:624`).
pub fn normalize_route_method(method: &str) -> Option<&'static str> {
    match method {
        "GET" | "get" => Some("GET"),
        "POST" | "post" => Some("POST"),
        "PUT" | "put" => Some("PUT"),
        "PATCH" | "patch" => Some("PATCH"),
        "DELETE" | "delete" => Some("DELETE"),
        _ => None,
    }
}

/// Whether a character can appear in a static path segment.
fn is_static_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._~-".contains(c))
}

/// Whether a name is a valid parameter name: `^[A-Za-z_][A-Za-z0-9_]*$`.
fn is_path_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Validate a canonical route path, the reference's `NormalizeRoutePath`
/// (`routing.go:637`). A trailing slash is allowed and significant.
pub fn normalize_route_path(path: &str) -> Result<&str, String> {
    if path.is_empty() || !path.starts_with('/') {
        return Err("plugin route path must start with /".to_string());
    }
    if path == "/" {
        return Err("plugin route path / is reserved".to_string());
    }
    if path.contains(['?', '#', '%']) {
        return Err(format!(
            "plugin route path {path:?} must not contain a query, fragment, or percent-encoding"
        ));
    }
    if path.contains("//") {
        return Err(format!(
            "plugin route path {path:?} must not contain empty segments"
        ));
    }
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let mut seen: Vec<&str> = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        if segment.is_empty() && index == segments.len() - 1 {
            continue;
        }
        if *segment == "." || *segment == ".." {
            return Err(format!(
                "plugin route path {path:?} must not contain dot segments"
            ));
        }
        if let Some(name) = segment.strip_prefix(':') {
            if !is_path_name(name) {
                return Err(format!(
                    "plugin route path {path:?} has invalid parameter {segment:?}"
                ));
            }
            if seen.contains(&name) {
                return Err(format!(
                    "plugin route path {path:?} repeats parameter {name:?}"
                ));
            }
            seen.push(name);
            continue;
        }
        if let Some(name) = segment.strip_prefix('*') {
            if index != segments.len() - 1 || !is_path_name(name) {
                return Err(format!(
                    "plugin route path {path:?} has an invalid catch-all segment"
                ));
            }
            if seen.contains(&name) {
                return Err(format!(
                    "plugin route path {path:?} repeats parameter {name:?}"
                ));
            }
            seen.push(name);
            continue;
        }
        if !is_static_segment(segment) {
            return Err(format!(
                "plugin route path {path:?} has invalid segment {segment:?}"
            ));
        }
    }
    Ok(path)
}

/// The `method + path + model` key a binding is indexed under.
pub fn endpoint_index_key(method: &str, path: &str, model: &str) -> String {
    format!("{method}\u{0}{path}\u{0}{model}")
}

/// One plugin's claim, as the index builder needs it.
///
/// Borrowed rather than owned so the caller can index straight off its own
/// storage without building a parallel structure that could drift.
#[derive(Debug, Clone, Copy)]
pub struct EndpointClaim<'a> {
    pub plugin_key: &'a str,
    pub protocol: &'a str,
    /// Models the claim narrows to; empty means every model the plugin declares.
    pub claim_models: &'a [String],
    /// Every model the plugin declares.
    pub models: &'a [String],
    /// The request forms the claim declares. Empty for a mode-less protocol.
    pub supports: &'a [String],
}

/// Where a plugin serves a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolBinding {
    pub plugin_key: String,
    pub protocol: &'static str,
    pub operation: &'static HostOperation,
    pub model: String,
    /// The request forms this binding's plugin claimed for the protocol.
    pub supports: Vec<String>,
}

impl ProtocolBinding {
    /// Whether the plugin serving this endpoint accepts a request form.
    pub fn supports_mode(&self, mode: &str) -> bool {
        self.supports.iter().any(|support| support == mode)
    }
}

/// The request forms a request needs, given what its body asks for.
///
/// `stream` and `background` are read from the body; anything else is the
/// synchronous form (`middleware/task_plugin.go:382`).
pub fn required_modes(stream: bool, background: bool) -> Vec<&'static str> {
    let mut required = Vec::new();
    if stream {
        required.push("stream");
    }
    if background {
        required.push("background");
    }
    if !stream && !background {
        required.push("sync");
    }
    required
}

/// Bindings by endpoint, so a lookup is one map hit.
#[derive(Debug, Default, Clone)]
pub struct EndpointIndex {
    bindings: BTreeMap<String, Vec<ProtocolBinding>>,
}

impl EndpointIndex {
    /// Index every claim. Operations with no model field are not indexed
    /// (`routing.go:989`): the host answers them itself, so there is nothing to
    /// choose between.
    pub fn build<'a>(claims: impl IntoIterator<Item = EndpointClaim<'a>>) -> Self {
        let mut bindings: BTreeMap<String, Vec<ProtocolBinding>> = BTreeMap::new();
        for claim in claims {
            let Some(protocol) = crate::host_protocol(claim.protocol) else {
                continue;
            };
            let models: &[String] = if claim.claim_models.is_empty() {
                claim.models
            } else {
                claim.claim_models
            };
            for operation in protocol.operations {
                if operation.model_field.is_empty() {
                    continue;
                }
                for method in operation.methods {
                    for model in models {
                        bindings
                            .entry(endpoint_index_key(method, operation.path, model))
                            .or_default()
                            .push(ProtocolBinding {
                                plugin_key: claim.plugin_key.to_string(),
                                protocol: protocol.name,
                                operation,
                                model: model.clone(),
                                supports: claim.supports.to_vec(),
                            });
                    }
                }
            }
        }
        EndpointIndex { bindings }
    }

    /// Every plugin serving one endpoint, in the order they were indexed.
    pub fn candidates(&self, method: &str, path: &str, model: &str) -> Vec<ProtocolBinding> {
        let Some(method) = normalize_route_method(method) else {
            return Vec::new();
        };
        self.bindings
            .get(&endpoint_index_key(method, path, model))
            .cloned()
            .unwrap_or_default()
    }

    /// The first plugin serving one endpoint.
    pub fn lookup(&self, method: &str, path: &str, model: &str) -> Option<ProtocolBinding> {
        self.candidates(method, path, model).into_iter().next()
    }

    pub fn len(&self) -> usize {
        self.bindings.values().map(|v| v.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

/// Whether a protocol's definitions have any request form at all.
pub fn protocol_has_modes(protocol: &HostProtocol) -> bool {
    !protocol.defined_modes().is_empty()
}

/// The refusal a request gets when every candidate for its endpoint declines the
/// request form it asked for.
///
/// The reference aborts with the *form* it needed rather than a generic 400, so
/// a client that sent `stream: true` to a sync-only plugin is told exactly that
/// (`middleware/task_plugin.go:412`).
pub fn unsupported_form_message(
    candidates: &[ProtocolBinding],
    protocol: &str,
    stream: bool,
    background: bool,
) -> String {
    let required: Vec<&str> = required_modes(stream, background)
        .into_iter()
        .filter(|mode| *mode != "sync")
        .collect();
    let asked = if required.is_empty() {
        "a synchronous (non-streaming) request".to_string()
    } else {
        format!("a {} request", required.join(" and "))
    };
    let offered: Vec<String> = candidates
        .iter()
        .flat_map(|candidate| candidate.supports.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if offered.is_empty() {
        format!("no plugin serving {protocol} accepts {asked}")
    } else {
        format!(
            "no plugin serving {protocol} accepts {asked}; the plugins bound to this model declare {}",
            offered
                .iter()
                .map(|form| format!("{form:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim<'a>(
        plugin_key: &'a str,
        protocol: &'a str,
        claim_models: &'a [String],
        models: &'a [String],
        supports: &'a [String],
    ) -> EndpointClaim<'a> {
        EndpointClaim {
            plugin_key,
            protocol,
            claim_models,
            models,
            supports,
        }
    }

    /// A bare claim binds both image endpoints for every declared model, and
    /// nothing else, which is the reference's
    /// `TestOpenAIImageProtocolClaimBindsBothEndpointsAndRequiresRender`
    /// (`pkg/jsplugin/routing_test.go:105`).
    #[test]
    fn an_image_claim_binds_both_endpoints_for_each_declared_model() {
        let models: Vec<String> = vec!["image-a".into(), "image-b".into()];
        let index = EndpointIndex::build([claim(
            "image-plugin",
            "openai_image",
            &[],
            &models,
            &[],
        )]);

        for path in ["/v1/images/generations", "/v1/images/edits"] {
            for model in ["image-a", "image-b"] {
                let binding = index
                    .lookup("POST", path, model)
                    .unwrap_or_else(|| panic!("{path} {model} must bind"));
                assert_eq!(binding.protocol, "openai_image");
                assert_eq!(binding.plugin_key, "image-plugin");
            }
        }

        // An image claim must not bind the video endpoint, and a model the
        // plugin never declared must not bind at all.
        assert!(index.lookup("POST", "/v1/videos", "image-a").is_none());
        assert!(index
            .lookup("POST", "/v1/images/generations", "image-c")
            .is_none());
    }

    /// A claim's `models` narrows the binding, which is the half of the
    /// reference's image test that a path-only lookup cannot express.
    #[test]
    fn a_claim_narrows_the_bindings_to_the_models_it_names() {
        let models: Vec<String> = vec!["image-a".into(), "image-b".into()];
        let scoped: Vec<String> = vec!["image-a".into()];
        let index = EndpointIndex::build([claim(
            "image-scoped",
            "openai_image",
            &scoped,
            &models,
            &[],
        )]);

        assert!(index.lookup("POST", "/v1/images/generations", "image-a").is_some());
        assert!(
            index.lookup("POST", "/v1/images/generations", "image-b").is_none(),
            "models narrows the image endpoint bindings"
        );
    }

    /// Two plugins that share an endpoint but declare disjoint model sets both
    /// keep their binding -- the case a path-only index collapses
    /// (`pkg/jsplugin/routing_test.go:190`).
    #[test]
    fn plugins_on_one_endpoint_keep_their_own_models() {
        let a: Vec<String> = vec!["model-a".into()];
        let b: Vec<String> = vec!["model-b".into()];
        let index = EndpointIndex::build([
            claim("plugin-a", "openai_responses", &[], &a, &["sync".into()]),
            claim("plugin-b", "openai_responses", &[], &b, &["sync".into()]),
        ]);

        assert_eq!(
            index.lookup("POST", "/v1/responses", "model-a").unwrap().plugin_key,
            "plugin-a"
        );
        assert_eq!(
            index.lookup("POST", "/v1/responses", "model-b").unwrap().plugin_key,
            "plugin-b"
        );
    }

    /// Two plugins claiming the *same* model endpoint are both candidates, in
    /// the order they were indexed, because each still decodes before the host
    /// distributes (`middleware/task_plugin.go:378`).
    #[test]
    fn a_shared_model_endpoint_yields_every_candidate() {
        let shared: Vec<String> = vec!["shared-model".into()];
        let index = EndpointIndex::build([
            claim("plugin-a", "openai_responses", &[], &shared, &["stream".into()]),
            claim("plugin-b", "openai_responses", &[], &shared, &["sync".into()]),
        ]);

        let candidates = index.candidates("POST", "/v1/responses", "shared-model");
        assert_eq!(candidates.len(), 2, "{candidates:?}");
        assert_eq!(candidates[0].plugin_key, "plugin-a");
        assert_eq!(candidates[1].plugin_key, "plugin-b");
        assert!(candidates[0].supports_mode("stream"));
        assert!(!candidates[0].supports_mode("sync"));
        assert!(candidates[1].supports_mode("sync"));
    }

    /// A lookup canonicalizes the method and refuses one outside the pattern, so
    /// `POST` and `post` agree and a misspelled verb misses
    /// (`pkg/jsplugin/routing.go:517`).
    #[test]
    fn the_lookup_canonicalizes_the_method_and_refuses_the_rest() {
        let models: Vec<String> = vec!["m".into()];
        let index = EndpointIndex::build([claim(
            "p",
            "openai_image",
            &[],
            &models,
            &[],
        )]);

        assert!(index.lookup("post", "/v1/images/generations", "m").is_some());
        assert!(index.lookup("POST", "/v1/images/generations", "m").is_some());
        assert!(index.lookup("BREW", "/v1/images/generations", "m").is_none());

        // The path is matched exactly, including the trailing-slash rule.
        assert!(index.lookup("POST", "/v1/images/generations/", "m").is_none());
    }

    /// An operation the host answers itself is bound to no plugin, so a GET of a
    /// recorded response never lands on one (`pkg/jsplugin/routing.go:989`).
    #[test]
    fn a_host_answered_operation_binds_nothing() {
        let models: Vec<String> = vec!["m".into()];
        let index = EndpointIndex::build([claim(
            "p",
            "openai_responses",
            &[],
            &models,
            &["sync".into()],
        )]);

        assert!(index.lookup("GET", "/v1/responses/abc", "m").is_none());
        assert!(index.lookup("POST", "/v1/responses", "m").is_some());
    }

    /// The request a plugin sees: the canonical fields, and a body that says
    /// whether it is JSON, a form, or absent (`pkg/jsplugin/routing.go:293`).
    #[test]
    fn the_plugin_context_is_the_reference_shape() {
        let request = RequestContext::new("/v1/responses", "POST")
            .with_json(serde_json::json!({"model": "acme", "input": "hi", "stream": true}));
        assert_eq!(request.body_kind(), BodyKind::Json);

        let context = ProtocolContext {
            request,
            protocol: "openai_responses",
            operation: "create",
            model: "acme".into(),
            upstream_model: "acme-upstream".into(),
            stream: true,
        };
        let value = context.js_value();

        assert_eq!(value["path"], "/v1/responses");
        assert_eq!(value["method"], "POST");
        assert_eq!(value["protocol"], "openai_responses");
        assert_eq!(value["operation"], "create");
        assert_eq!(value["model"], "acme");
        assert_eq!(value["upstreamModel"], "acme-upstream");
        assert_eq!(value["stream"], true);
        // The body is tagged, and the decoded payload lives under `value`, which
        // is what the reference's own fixtures read
        // (`pkg/jsplugin/routing_test.go:107`).
        assert_eq!(value["body"]["kind"], "json");
        assert_eq!(value["body"]["value"]["input"], "hi");

        // An absent body is still tagged, so a hook can tell "nothing was sent"
        // from "an empty object was sent".
        let empty = ProtocolContext {
            request: RequestContext::new("/v1/responses", "POST"),
            protocol: "openai_responses",
            operation: "create",
            model: "acme".into(),
            upstream_model: String::new(),
            stream: false,
        }
        .js_value();
        assert_eq!(empty["body"]["kind"], "none");
        assert!(
            empty.get("upstreamModel").is_none(),
            "an empty upstream model is omitted, not sent as \"\""
        );
        assert_eq!(empty["stream"], false);
    }

    /// A form body and a multipart body each declare themselves, and a multipart
    /// file is addressable without its bytes being handed over
    /// (`pkg/jsplugin/routing.go:252`, `middleware/task_plugin.go:955`).
    #[test]
    fn form_and_multipart_bodies_carry_their_fields_and_file_references() {
        let mut fields = BTreeMap::new();
        fields.insert("prompt".to_string(), vec!["a cat".to_string()]);
        let form = RequestContext::new("/v1/images/edits", "POST").with_form(fields.clone());
        assert_eq!(form.body_kind(), BodyKind::Form);
        assert_eq!(form.body_value()["kind"], "form");
        assert_eq!(form.body_value()["fields"]["prompt"][0], "a cat");

        let files = vec![BodyFile {
            reference: file_reference("image[]", 0),
            field: "image[]".to_string(),
            filename: "cat.png".to_string(),
            mime_type: "image/png".to_string(),
            size: 2048,
        }];
        let multipart =
            RequestContext::new("/v1/images/edits", "POST").with_multipart(fields, files);
        assert_eq!(multipart.body_kind(), BodyKind::Multipart);
        let value = multipart.body_value();
        assert_eq!(value["kind"], "multipart");
        assert_eq!(value["files"][0]["ref"], "request_file:image[]");
        assert_eq!(value["files"][0]["mimeType"], "image/png");
        assert_eq!(value["files"][0]["size"], 2048);

        // Repeated fields are addressable one file at a time.
        assert_eq!(file_reference("image[]", 2), "request_file:image[]#2");
    }

    /// The request forms a request needs, derived from what its body asked for
    /// (`middleware/task_plugin.go:382`).
    #[test]
    fn a_request_names_the_form_it_needs() {
        assert_eq!(required_modes(false, false), vec!["sync"]);
        assert_eq!(required_modes(true, false), vec!["stream"]);
        assert_eq!(required_modes(false, true), vec!["background"]);
        assert_eq!(required_modes(true, true), vec!["stream", "background"]);

        let shared: Vec<String> = vec!["m".into()];
        let index = EndpointIndex::build([claim(
            "p",
            "openai_responses",
            &[],
            &shared,
            &["stream".into()],
        )]);
        let candidates = index.candidates("POST", "/v1/responses", "m");
        assert!(candidates[0].supports_mode("stream"));
        assert!(!candidates[0].supports_mode("sync"));

        // The refusal names the form that was asked for and the ones on offer,
        // rather than a bare 400 (`middleware/task_plugin.go:412`).
        let message = unsupported_form_message(&candidates, "openai_responses", false, false);
        assert!(message.contains("synchronous"), "{message}");
        assert!(message.contains("\"stream\""), "{message}");
    }

    /// The path validator is the reference's, so a path the reference refuses to
    /// declare is refused here too (`pkg/jsplugin/routing.go:637`).
    #[test]
    fn route_paths_are_validated_the_way_the_reference_validates_them() {
        assert!(normalize_route_path("/v1/images/generations").is_ok());
        assert!(normalize_route_path("/v1/responses/").is_ok(), "trailing slash is significant, not invalid");
        assert!(normalize_route_path("/v1/responses/:id").is_ok());
        assert!(normalize_route_path("/v1/files/*rest").is_ok());

        // A static segment may start with a digit (`staticSegment` allows
        // `[A-Za-z0-9._~-]`); only a *parameter name* must start with a letter or
        // underscore. So `/a/1x` is legal and `/a/:1x` is not.
        assert!(normalize_route_path("/a/1x").is_ok());
        for bad in ["", "v1/x", "/", "/a?b", "/a#b", "/a%20b", "/a//b", "/a/./b", "/a/../b", "/a/:1x", "/a/:id/:id", "/a/*rest/more"] {
            assert!(normalize_route_path(bad).is_err(), "{bad:?} must be refused");
        }
        // The parameter name must be an identifier, and a catch-all must end the
        // path.
        assert!(normalize_route_path("/a/:x_y9").is_ok());
        assert!(normalize_route_path("/a/:x-y").is_err());
        assert!(normalize_route_path("/a/*x-y").is_err());
    }

    /// A mode-less protocol is still indexed: `supports` is simply empty, and a
    /// caller asking whether it has forms is told no.
    #[test]
    fn a_mode_less_protocol_is_indexed_without_forms() {
        let image = crate::host_protocol("openai_image").expect("image");
        assert!(!protocol_has_modes(image));
        let responses = crate::host_protocol("openai_responses").expect("responses");
        assert!(protocol_has_modes(responses));
    }
}
