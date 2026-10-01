//! The submit-and-poll orchestration a task plugin drives.
//!
//! The host owns the transport, the retry policy and the store; the plugin owns
//! the dialect. This module is the seam: it turns a plugin's descriptor into one
//! outbound request, interprets the answer through the plugin's own parsers, and
//! reports whether a failure is worth retrying.
//!
//! The hook arities are the reference's, not a simplification:
//! `buildQueryRequest(ctx)` takes one argument, `parseSubmitResponse(ctx,
//! response)` two, and `parseTaskResult(ctx, body, response)` three
//! (`relay/channel/task/jsplugin/adaptor.go:607,492,777`). A host that guessed
//! would call a plugin's parser with the wrong shape and get a plausible `{}`
//! back instead of an error.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::task::{
    build_request_body, validate_request_url, OutboundBody, RequestDescriptor, ResolvedFile,
    SubmitOutcome, TaskResult,
};
use crate::{
    PluginError, PluginHost, HOOK_BUILD_QUERY_REQUEST, HOOK_PARSE_SUBMIT_RESPONSE,
    HOOK_PARSE_TASK_RESULT,
};

/// One outbound request, fully formed: the host only has to send it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundRequest {
    /// Already uppercased, because the reference uppercases the descriptor's
    /// method before creating the request (`adaptor.go:447,634`).
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<OutboundBody>,
    /// The value for `Authorization`, when the credential should ride along. A
    /// descriptor can ask for a credentialless request, which is how a plugin
    /// calls an endpoint that must not receive the operator's key.
    pub authorization: Option<String>,
}

/// What a transport answers with: the three things a plugin's parser may read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpOutcome {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpOutcome {
    /// The body as the reference hands it to a hook: parsed when it is JSON, and
    /// the raw text otherwise, so a plugin's parser can read either
    /// (`adaptor.go:480,766`).
    pub fn body_for_hook(&self) -> serde_json::Value {
        serde_json::from_slice::<serde_json::Value>(&self.body).unwrap_or_else(|_| {
            serde_json::Value::String(String::from_utf8_lossy(&self.body).into_owned())
        })
    }

    /// The response descriptor a parser reads as its last argument
    /// (`adaptor.go:1109`).
    pub fn response_for_hook(&self) -> serde_json::Value {
        let mut headers = serde_json::Map::new();
        for (name, value) in &self.headers {
            headers.insert(name.clone(), serde_json::Value::String(value.clone()));
        }
        serde_json::json!({ "status": self.status, "headers": headers })
    }

    /// Whether the upstream accepted the request.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Whether the response is an event stream.
    pub fn is_event_stream(&self) -> bool {
        self.headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| {
                value
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            })
            .unwrap_or_default()
            == "text/event-stream"
    }
}

/// A transport the flow can drive.
///
/// A trait so the orchestration is exercised without a network, and so the host
/// keeps ownership of the client -- its proxy, its TLS and its timeouts.
pub type TransportFuture<'a> =
    Pin<Box<dyn Future<Output = Result<HttpOutcome, String>> + Send + 'a>>;

pub trait TaskTransport: Send + Sync {
    fn execute(&self, request: OutboundRequest) -> TransportFuture<'_>;
}

/// Why a step of the flow failed, and whether retrying could help.
///
/// The distinction is not cosmetic: a submission that failed *after* the
/// upstream accepted an event stream may already have done billable work, so
/// retrying it would spend twice, and the reference marks every such failure
/// non-retryable (`adaptor.go:460,487`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowError {
    pub message: String,
    pub retryable: bool,
}

impl FlowError {
    pub fn retryable(message: impl Into<String>) -> Self {
        FlowError {
            message: message.into(),
            retryable: true,
        }
    }

    pub fn fatal(message: impl Into<String>) -> Self {
        FlowError {
            message: message.into(),
            retryable: false,
        }
    }
}

impl From<PluginError> for FlowError {
    fn from(error: PluginError) -> Self {
        FlowError::fatal(error.to_string())
    }
}

/// What a submission produced.
#[derive(Debug, Clone, PartialEq)]
pub enum SubmitAnswer {
    /// There is upstream work to poll.
    Pending(SubmitOutcome),
    /// The upstream answered already, so there is nothing to poll.
    Immediate(TaskResult),
}

/// The coordinates a plugin's hooks are called with.
#[derive(Debug, Clone)]
pub struct TaskFlowContext {
    pub plugin_key: String,
    /// The client-facing model, which is what a quota was quoted for.
    pub model: String,
    pub base_url: String,
    /// The credential's `Authorization` value, or `None` when this task is
    /// credentialless.
    pub authorization: Option<String>,
    /// Hosts a descriptor may address besides the channel's own
    /// (`pkg/jsplugin/request.go:25`).
    pub allowed_hosts: Vec<String>,
    /// The submission encodings the plugin declared it can parse.
    pub submit_response_types: Vec<String>,
    /// Files the request carried, resolved to their bytes.
    pub files: Vec<ResolvedFile>,
    pub max_inline_bytes: u64,
    pub timeout: Duration,
}

/// Validate a descriptor before anything is sent.
///
/// The reference checks exactly these, in this order (`adaptor.go:1221-1258`):
/// the response type is one the plugin declared, the URL is present and passes
/// the host guard, and -- when the request is pinned to an endpoint -- the
/// descriptor's model is the pinned one.
pub fn validate_descriptor(
    descriptor: &RequestDescriptor,
    resolved_model: &str,
    context: &TaskFlowContext,
) -> Result<(), FlowError> {
    let response_type = descriptor.response_type_or_default();
    let allowed = if context.submit_response_types.is_empty() {
        vec!["json".to_string()]
    } else {
        context.submit_response_types.clone()
    };
    if !allowed.iter().any(|kind| kind == response_type) {
        return Err(FlowError::fatal(format!(
            "plugin does not support submit response type {response_type:?}"
        )));
    }
    if descriptor.url.trim().is_empty() {
        return Err(FlowError::fatal("plugin returned an empty submit URL"));
    }
    if let Err(reason) = validate_request_url(
        descriptor.url.trim(),
        &context.base_url,
        &context.allowed_hosts,
    ) {
        return Err(FlowError::fatal(reason));
    }
    // A descriptor that names a model must name the one this endpoint serves, or
    // the request would be routed and billed for a model the caller never asked
    // for.
    if !descriptor.model.trim().is_empty() && descriptor.model.trim() != resolved_model {
        return Err(FlowError::fatal(
            "plugin submit model does not match the pinned endpoint model",
        ));
    }
    Ok(())
}

/// Turn a descriptor into the request the host should make.
pub fn build_outbound_request(
    descriptor: &RequestDescriptor,
    context: &TaskFlowContext,
) -> Result<OutboundRequest, FlowError> {
    let body = build_request_body(descriptor, &context.files, context.max_inline_bytes)
        .map_err(FlowError::fatal)?;
    let mut headers: Vec<(String, String)> = descriptor
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    // The body builder owns the content type, because only it knows whether the
    // body is multipart and what boundary it chose.
    if let Some(body) = &body {
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case("content-type"));
        headers.push(("content-type".to_string(), body.content_type.clone()));
    }
    Ok(OutboundRequest {
        method: descriptor.method_or_default().to_ascii_uppercase(),
        url: descriptor.url.trim().to_string(),
        headers,
        body,
        authorization: if descriptor.credentialless {
            None
        } else {
            context.authorization.clone()
        },
    })
}

/// Send one submission.
///
/// One attempt, because retrying is the caller's decision: it is the caller that
/// knows whether the upstream may already have spent quota.
pub async fn send_submit(
    transport: &dyn TaskTransport,
    request: OutboundRequest,
) -> Result<HttpOutcome, FlowError> {
    transport
        .execute(request)
        .await
        .map_err(FlowError::retryable)
}

/// Interpret a submission's answer through the plugin's own parser.
///
/// Ported from `ParseResponse` (`adaptor.go:453`), including the two refusals it
/// makes because the plugin's answer is the only thing the host can act on: an
/// SSE body for a JSON submission means the two sides disagree about the
/// encoding, and a `clientResponse` would let a plugin answer the *client* on the
/// submission's behalf, bypassing the host's own accounting.
pub async fn interpret_submit(
    host: &PluginHost,
    descriptor: &RequestDescriptor,
    context: &TaskFlowContext,
    request_context: serde_json::Value,
    outcome: &HttpOutcome,
) -> Result<SubmitAnswer, FlowError> {
    let streaming = descriptor.response_type_or_default() == "sse";
    if !streaming && outcome.is_event_stream() {
        return Err(FlowError::fatal(
            "unexpected SSE response for a JSON submission",
        ));
    }
    if !outcome.is_success() {
        return Err(FlowError {
            message: format!("upstream answered {}", outcome.status),
            retryable: !streaming,
        });
    }

    let parsed = host
        .call_hook_args(
            &context.plugin_key,
            HOOK_PARSE_SUBMIT_RESPONSE,
            &[request_context, submission_input(outcome)],
            context.timeout,
        )
        .await;
    let parsed = match parsed {
        Ok(value) => value,
        Err(error) => {
            return Err(FlowError {
                message: error.to_string(),
                retryable: !streaming,
            })
        }
    };

    if parsed.get("clientResponse").is_some() {
        return Err(FlowError::fatal(
            "parseSubmitResponse must not return clientResponse",
        ));
    }
    let submission: SubmitOutcome = serde_json::from_value(parsed).map_err(|error| {
        FlowError::fatal(format!(
            "parseSubmitResponse returned an unusable shape: {error}"
        ))
    })?;
    if submission.task_id.trim().is_empty() {
        return Err(FlowError::fatal("plugin returned an empty taskId"));
    }
    match submission.immediate.clone() {
        Some(immediate) => Ok(SubmitAnswer::Immediate(immediate)),
        None => Ok(SubmitAnswer::Pending(submission)),
    }
}

/// The second argument to `parseSubmitResponse`: the status, the headers and the
/// body, because a plugin decides from all three (`adaptor.go:492`).
fn submission_input(outcome: &HttpOutcome) -> serde_json::Value {
    let mut headers = serde_json::Map::new();
    for (name, value) in &outcome.headers {
        let entry = headers
            .entry(name.clone())
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        if let serde_json::Value::Array(values) = entry {
            values.push(serde_json::Value::String(value.clone()));
        }
    }
    serde_json::json!({
        "statusCode": outcome.status,
        "headers": headers,
        "body": outcome.body_for_hook(),
    })
}

/// Ask the plugin how to poll, and hand the host a request it can make.
///
/// `buildQueryRequest(ctx)` takes one argument, and its method defaults to GET
/// rather than POST -- a poll is a read (`adaptor.go:634,607`).
pub async fn build_query_request(
    host: &PluginHost,
    context: &TaskFlowContext,
    query_context: serde_json::Value,
) -> Result<OutboundRequest, FlowError> {
    let value = host
        .call_hook_args(
            &context.plugin_key,
            HOOK_BUILD_QUERY_REQUEST,
            &[query_context],
            context.timeout,
        )
        .await?;
    let descriptor: RequestDescriptor = serde_json::from_value(value).map_err(|error| {
        FlowError::fatal(format!(
            "buildQueryRequest returned an unusable shape: {error}"
        ))
    })?;
    if descriptor.url.trim().is_empty() {
        return Err(FlowError::fatal("plugin returned an empty query URL"));
    }
    validate_request_url(
        descriptor.url.trim(),
        &context.base_url,
        &context.allowed_hosts,
    )
    .map_err(FlowError::fatal)?;

    let mut request = build_outbound_request(&descriptor, context)?;
    // A poll is a read unless the plugin says otherwise, so silence means GET.
    if descriptor.method.trim().is_empty() {
        request.method = "GET".to_string();
    }
    Ok(request)
}

/// Interpret a poll's answer through the plugin's own parser.
///
/// `parseTaskResult(ctx, body, response)` takes three arguments
/// (`adaptor.go:777`), and the third is where an upstream's own status lives,
/// which a plugin needs to tell "the task failed" from "the poll failed".
pub async fn interpret_task_result(
    host: &PluginHost,
    context: &TaskFlowContext,
    query_context: serde_json::Value,
    outcome: &HttpOutcome,
) -> Result<TaskResult, FlowError> {
    let value = host
        .call_hook_args(
            &context.plugin_key,
            HOOK_PARSE_TASK_RESULT,
            &[
                query_context,
                outcome.body_for_hook(),
                outcome.response_for_hook(),
            ],
            context.timeout,
        )
        .await?;
    serde_json::from_value(value).map_err(|error| {
        FlowError::fatal(format!("parseTaskResult returned an unusable shape: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{RequestPart, STATUS_SUCCESS};
    use crate::DEFAULT_CALL_TIMEOUT;
    use crate::tests::runtime;
    use std::sync::Mutex;

    /// A transport that answers from a script and records what it was asked to
    /// send, so a test can assert on the request as well as the outcome.
    #[derive(Default)]
    struct StubTransport {
        answers: Mutex<Vec<Result<HttpOutcome, String>>>,
        sent: Mutex<Vec<OutboundRequest>>,
    }

    impl StubTransport {
        fn answering(answers: Vec<HttpOutcome>) -> Self {
            StubTransport {
                answers: Mutex::new(answers.into_iter().map(Ok).collect()),
                sent: Mutex::new(Vec::new()),
            }
        }
    }

    impl TaskTransport for StubTransport {
        fn execute(&self, request: OutboundRequest) -> TransportFuture<'_> {
            self.sent.lock().expect("lock").push(request);
            let next = self
                .answers
                .lock()
                .expect("lock")
                .pop()
                .unwrap_or_else(|| Err("no scripted answer".to_string()));
            Box::pin(async move { next })
        }
    }

    fn json_outcome(status: u16, body: &str) -> HttpOutcome {
        HttpOutcome {
            status,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: body.as_bytes().to_vec(),
        }
    }

    fn context() -> TaskFlowContext {
        TaskFlowContext {
            plugin_key: "acme".to_string(),
            model: "acme-video".to_string(),
            base_url: "https://api.vendor.example".to_string(),
            authorization: Some("Bearer channel-key".to_string()),
            allowed_hosts: vec!["cdn.vendor.example".to_string()],
            submit_response_types: vec!["json".to_string()],
            files: Vec::new(),
            max_inline_bytes: 0,
            timeout: DEFAULT_CALL_TIMEOUT,
        }
    }

    fn new_descriptor(url: &str) -> RequestDescriptor {
        serde_json::from_value(serde_json::json!({ "url": url })).expect("descriptor")
    }

    /// The guard is the reference's, and its order is the reference's: the
    /// response type first, then the URL, then the model pin
    /// (`adaptor.go:1221,1231,1242`).
    #[test]
    fn a_descriptor_is_validated_before_anything_is_sent() {
        let mut ctx = context();
        assert!(validate_descriptor(
            &new_descriptor("https://api.vendor.example/x"),
            "acme-video",
            &ctx
        )
        .is_ok());

        // A response type the plugin never declared.
        let mut sse = new_descriptor("https://api.vendor.example/x");
        sse.response_type = "sse".to_string();
        let error = validate_descriptor(&sse, "acme-video", &ctx).expect_err("must refuse");
        assert!(
            error.message.contains("does not support submit response type"),
            "{}",
            error.message
        );
        assert!(!error.retryable);

        // An empty URL, an off-host URL, and a model that is not the pinned one.
        assert!(validate_descriptor(&new_descriptor("   "), "acme-video", &ctx)
            .expect_err("must refuse")
            .message
            .contains("empty submit URL"));
        assert!(
            validate_descriptor(&new_descriptor("https://evil.example/x"), "acme-video", &ctx)
                .expect_err("must refuse")
                .message
                .contains("not allowed")
        );
        let mut wrong_model = new_descriptor("https://api.vendor.example/x");
        wrong_model.model = "other-model".to_string();
        assert!(
            validate_descriptor(&wrong_model, "acme-video", &ctx)
                .expect_err("must refuse")
                .message
                .contains("pinned endpoint model")
        );

        // A declared SSE plugin accepts an SSE descriptor, and an allow-listed
        // host is acceptable for a submission too.
        ctx.submit_response_types = vec!["json".to_string(), "sse".to_string()];
        assert!(validate_descriptor(&sse, "acme-video", &ctx).is_ok());
        assert!(validate_descriptor(
            &new_descriptor("https://cdn.vendor.example/y"),
            "acme-video",
            &ctx
        )
        .is_ok());
    }

    /// The outbound request carries the descriptor's method, headers, body and
    /// the channel credential -- unless the descriptor asked to go without it.
    #[test]
    fn an_outbound_request_carries_the_descriptor_and_the_credential() {
        let ctx = context();
        let mut descriptor = new_descriptor("https://api.vendor.example/v1/jobs");
        descriptor.method = "put".to_string();
        descriptor
            .headers
            .insert("X-Trace".to_string(), "abc".to_string());
        descriptor.body = serde_json::json!({ "prompt": "a cat" });

        let request = build_outbound_request(&descriptor, &ctx).expect("request");
        assert_eq!(request.method, "PUT", "the reference uppercases the method");
        assert_eq!(request.url, "https://api.vendor.example/v1/jobs");
        assert!(request
            .headers
            .contains(&("X-Trace".to_string(), "abc".to_string())));
        // The body builder decides the content type, because only it knows the
        // boundary for a multipart body.
        assert!(request
            .headers
            .iter()
            .any(|(name, value)| name == "content-type" && value == "application/json"));
        assert_eq!(request.authorization.as_deref(), Some("Bearer channel-key"));
        let value: serde_json::Value =
            serde_json::from_slice(&request.body.expect("body").bytes).expect("json");
        assert_eq!(value["prompt"], "a cat");

        // A credentialless descriptor drops the credential even when there is one.
        let mut bare = new_descriptor("https://api.vendor.example/x");
        bare.credentialless = true;
        bare.body = serde_json::json!({ "x": 1 });
        assert!(build_outbound_request(&bare, &ctx)
            .expect("request")
            .authorization
            .is_none());
    }

    /// The whole submission path against a real plugin: the descriptor is sent,
    /// and the answer is read by the plugin's own parser at the arity the
    /// reference uses.
    #[test]
    fn a_submission_is_sent_and_interpreted_by_the_plugin() {
        runtime().block_on(async {
        // The plugin asserts its own view of the arity:
        // `parseSubmitResponse(ctx, response)` reads its second argument, so a
        // one-argument call would throw.
        const SOURCE: &str = r#"
            export const meta = {apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                author:{name:"Test"}, models:["acme-video"], fetchMode:"per_task",
                protocols:["openai_video"]};
            export function buildSubmitRequest(ctx) { return {url: ctx.baseUrl + "/jobs", body: {prompt: "a cat"}}; }
            export function parseSubmitResponse(ctx, response) {
                if (!response || typeof response.statusCode !== "number") { throw new Error("no response argument"); }
                return {taskId: "vendor-1", taskData: {kind: "video"}, state: {cursor: 1}};
            }
            export function buildQueryRequest() { return {url: "https://api.vendor.example/jobs/vendor-1"}; }
            export function parseTaskResult() { return {status: "IN_PROGRESS"}; }
            export const protocols = {openai_video: {
                decodeRequest: function(ctx) {
                    return {kind:"submit", model: ctx.model, requestBody: ctx.body.value};
                },
                render: function(ctx, task) { return task; }
            }};
            export function listArtifacts() { return []; }
            export function buildContentRequest() { return {}; }

        "#;
        let host = PluginHost::start();
        host.load(SOURCE.to_string(), DEFAULT_CALL_TIMEOUT)
            .await
            .expect("load");

        let transport = StubTransport::answering(vec![json_outcome(
            201,
            r#"{"id":"vendor-1","status":"queued"}"#,
        )]);
        let ctx = context();
        let descriptor = new_descriptor("https://api.vendor.example/jobs");
        let outcome = send_submit(
            &transport,
            build_outbound_request(&descriptor, &ctx).expect("request"),
        )
        .await
        .expect("send");
        assert_eq!(outcome.status, 201);
        {
            let sent = transport.sent.lock().expect("lock");
            assert_eq!(sent[0].method, "POST");
            assert_eq!(sent[0].url, "https://api.vendor.example/jobs");
        }

        let answer = interpret_submit(
            &host,
            &descriptor,
            &ctx,
            serde_json::json!({ "model": "acme-video" }),
            &outcome,
        )
        .await
        .expect("interpret");
        match answer {
            SubmitAnswer::Pending(submission) => {
                assert_eq!(submission.task_id, "vendor-1");
                assert_eq!(submission.task_data["kind"], "video");
                assert_eq!(submission.state["cursor"], 1);
            }
            other => panic!("expected a pending submission, got {other:?}"),
        }

        // A parser that answers with an immediate result is a synchronous
        // upstream: there is nothing to poll.
        let host = PluginHost::start();
        host.load(
            SOURCE.replace(
                r#"return {taskId: "vendor-1", taskData: {kind: "video"}, state: {cursor: 1}};"#,
                r#"return {taskId: "vendor-1", immediate: {status: "SUCCESS", progress: "100%", url: "https://cdn/1.png"}};"#,
            ),
            DEFAULT_CALL_TIMEOUT,
        )
        .await
        .expect("load");
        let answer = interpret_submit(&host, &descriptor, &ctx, serde_json::json!({}), &outcome)
            .await
            .expect("interpret");
        match answer {
            SubmitAnswer::Immediate(result) => {
                assert_eq!(result.status, STATUS_SUCCESS);
                assert_eq!(result.url, "https://cdn/1.png");
            }
            other => panic!("expected an immediate result, got {other:?}"),
        }
        });
    }

    /// The refusals the reference makes on a submission's answer
    /// (`adaptor.go:466,505,509`), each a plugin bug the host must not paper over.
    #[test]
    fn a_bad_submission_answer_is_refused_by_name() {
        runtime().block_on(async {
        const TEMPLATE: &str = r#"
            export const meta = {apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                author:{name:"Test"}, models:["acme-video"], fetchMode:"per_task",
                protocols:["openai_video"]};
            export function buildSubmitRequest() { return {}; }
            export function parseSubmitResponse() { return ANSWER_HERE; }
            export function buildQueryRequest() { return {}; }
            export function parseTaskResult() { return {}; }
            export const protocols = {openai_video: {
                decodeRequest: function(ctx) { return {kind:"submit", model: ctx.model}; },
                render: function(ctx, task) { return task; }
            }};
            export function listArtifacts() { return []; }
            export function buildContentRequest() { return {}; }

        "#;
        let ctx = context();
        let descriptor = new_descriptor("https://api.vendor.example/jobs");
        let load = |answer: &str| TEMPLATE.replace("ANSWER_HERE", answer);

        // An answer the host cannot act on: no task id at all.
        let host = PluginHost::start();
        host.load(load(r#"{taskId: "   "}"#), DEFAULT_CALL_TIMEOUT)
            .await
            .expect("load");
        let error = interpret_submit(
            &host,
            &descriptor,
            &ctx,
            serde_json::json!({}),
            &json_outcome(200, "{}"),
        )
        .await
        .expect_err("must refuse");
        assert!(error.message.contains("empty taskId"), "{}", error.message);
        assert!(!error.retryable, "a plugin bug is not worth retrying");

        // A plugin answering the client itself would bypass the host's
        // accounting, so it is refused outright.
        let host = PluginHost::start();
        host.load(
            load(r#"{taskId: "vendor-1", clientResponse: {status: 200}}"#),
            DEFAULT_CALL_TIMEOUT,
        )
        .await
        .expect("load");
        let error = interpret_submit(
            &host,
            &descriptor,
            &ctx,
            serde_json::json!({}),
            &json_outcome(200, "{}"),
        )
        .await
        .expect_err("must refuse");
        assert!(error.message.contains("clientResponse"), "{}", error.message);

        // An SSE body for a JSON submission means the two sides disagree about
        // the encoding, which a retry would not fix.
        let host = PluginHost::start();
        host.load(load(r#"{taskId: "vendor-1"}"#), DEFAULT_CALL_TIMEOUT)
            .await
            .expect("load");
        let sse = HttpOutcome {
            status: 200,
            headers: vec![("content-type".to_string(), "text/event-stream".to_string())],
            body: b"data: {}\n\n".to_vec(),
        };
        let error = interpret_submit(&host, &descriptor, &ctx, serde_json::json!({}), &sse)
            .await
            .expect_err("must refuse");
        assert!(
            error.message.contains("unexpected SSE response"),
            "{}",
            error.message
        );
        assert!(!error.retryable);

        // A transport failure is retryable; a rejection on an *accepted* stream
        // is not, because the upstream may already have done billable work.
        let error = send_submit(
            &StubTransport::default(),
            build_outbound_request(&descriptor, &ctx).expect("request"),
        )
        .await
        .expect_err("no scripted answer");
        assert!(error.retryable, "{}", error.message);

        let mut streaming = context();
        streaming.submit_response_types = vec!["sse".to_string()];
        let mut sse_descriptor = descriptor.clone();
        sse_descriptor.response_type = "sse".to_string();
        let error = interpret_submit(
            &PluginHost::start(),
            &sse_descriptor,
            &streaming,
            serde_json::json!({}),
            &json_outcome(500, "{}"),
        )
        .await
        .expect_err("must refuse");
        assert!(!error.retryable, "an accepted stream is not retried");
        });
    }

    /// The host resolves both kinds of hook through one entry point: a protocol
    /// member when the plugin defines one, and a top-level export otherwise.
    ///
    /// The reference keeps the two apart -- a protocol hook is a member of the
    /// plugin's protocol object, a driver hook is a module export
    /// (`pkg/jsplugin/registry.go:337,468`). A host that only looked in one place
    /// would report a perfectly good plugin as having no such hook, which is
    /// exactly what this test caught.
    #[test]
    fn a_hook_resolves_to_a_protocol_member_or_a_top_level_export() {
        runtime().block_on(async {
            const SOURCE: &str = r#"
                export const meta = {apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                    author:{name:"Test"}, models:["m"], fetchMode:"per_task",
                    protocols:[{name:"openai_responses", supports:["sync"]}]};
                export function buildSubmitRequest() { return {url: "https://api.vendor.example/j"}; }
                export function parseSubmitResponse() { return {taskId: "1"}; }
                export function buildQueryRequest() { return {url: "https://api.vendor.example/q"}; }
                export function parseTaskResult() { return {status: "IN_PROGRESS"}; }
                export const protocols = {openai_responses: {
                    decodeRequest: function(ctx) { return {model: ctx.model, from: "protocol member"}; },
                    renderFinal: function(ctx, task) { return {from: "protocol member"}; }
                }};
            "#;
            let host = PluginHost::start();
            host.load(SOURCE.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");

            // A protocol member is found where the plugin defined it.
            let decoded = host
                .decode_request("openai_responses", serde_json::json!({ "model": "m" }), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("decodeRequest");
            assert_eq!(decoded["from"], "protocol member");

            // A driver hook is found on the module, addressed by either the
            // plugin's key or its protocol name.
            for key in ["acme", "openai_responses"] {
                let value = host
                    .call_hook_args(key, "buildQueryRequest", &[serde_json::json!({})], DEFAULT_CALL_TIMEOUT)
                    .await
                    .unwrap_or_else(|error| panic!("{key}: {error}"));
                assert_eq!(value["url"], "https://api.vendor.example/q", "{key}");
            }

            // A hook nobody implements names the plugin and the hook, rather than
            // reporting a missing protocol object.
            let error = host
                .call_hook_args("acme", "parseSubmitEvent", &[], DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("must fail");
            let text = error.to_string();
            assert!(text.contains("parseSubmitEvent"), "{text}");
        });
    }

    /// The poll path: the query descriptor defaults to GET, and the plugin reads
    /// its three arguments.
    #[test]
    fn a_poll_builds_a_query_request_and_parses_three_arguments() {
        runtime().block_on(async {
        const SOURCE: &str = r#"
            export const meta = {apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                author:{name:"Test"}, models:["acme-video"], fetchMode:"per_task",
                protocols:["openai_video"]};
            export function buildSubmitRequest() { return {}; }
            export function parseSubmitResponse() { return {taskId: "vendor-1"}; }
            export function buildQueryRequest(ctx) { return {url: ctx.baseUrl + "/jobs/" + ctx.taskId}; }
            export function parseTaskResult(ctx, body, response) {
                if (typeof response.status !== "number") { throw new Error("no response argument"); }
                return {status: body.status, progress: body.progress, totalTokens: 8,
                        completionTokens: 3.9, url: body.url || ""};
            }
            export const protocols = {openai_video: {
                decodeRequest: function(ctx) { return {kind:"submit", model: ctx.model}; },
                render: function(ctx, task) { return task; }
            }};
            export function listArtifacts() { return []; }
            export function buildContentRequest() { return {}; }

        "#;
        let host = PluginHost::start();
        host.load(SOURCE.to_string(), DEFAULT_CALL_TIMEOUT)
            .await
            .expect("load");
        let ctx = context();
        let query_context =
            serde_json::json!({ "taskId": "vendor-1", "baseUrl": "https://api.vendor.example" });

        let request = build_query_request(&host, &ctx, query_context.clone())
            .await
            .expect("query request");
        // A poll is a read, so a descriptor that states no method gets GET.
        assert_eq!(request.method, "GET");
        assert_eq!(request.url, "https://api.vendor.example/jobs/vendor-1");

        let running = interpret_task_result(
            &host,
            &ctx,
            query_context.clone(),
            &json_outcome(200, r#"{"status":"IN_PROGRESS","progress":"40%"}"#),
        )
        .await
        .expect("parse");
        assert_eq!(running.progress, "40%");
        assert!(!running.is_terminal(), "a running task is not terminal");

        let done = interpret_task_result(
            &host,
            &ctx,
            query_context,
            &json_outcome(
                200,
                r#"{"status":"SUCCESS","progress":"100%","url":"https://cdn/x.mp4"}"#,
            ),
        )
        .await
        .expect("parse");
        assert!(done.is_terminal());
        assert_eq!(done.url, "https://cdn/x.mp4");
        // The token counts are clamped the way the reference clamps them.
        assert_eq!(done.total_tokens, 8.0);
        assert_eq!(TaskResult::positive_int(done.completion_tokens), 3);

        // A query URL off the channel's host is refused, so a plugin cannot
        // redirect the credential on the poll path either.
        let host = PluginHost::start();
        host.load(
            SOURCE.replace(
                r#"return {url: ctx.baseUrl + "/jobs/" + ctx.taskId};"#,
                r#"return {url: "https://evil.example/steal"};"#,
            ),
            DEFAULT_CALL_TIMEOUT,
        )
        .await
        .expect("load");
        let error = build_query_request(&host, &ctx, serde_json::json!({}))
            .await
            .expect_err("must refuse");
        assert!(error.message.contains("not allowed"), "{}", error.message);
        });
    }

    /// A query descriptor that does state a method keeps it, and a multipart
    /// submission reaches the transport as a multipart body rather than JSON.
    #[test]
    fn a_stated_query_method_is_kept_and_multipart_is_sent_as_multipart() {
        runtime().block_on(async {
        const SOURCE: &str = r#"
            export const meta = {apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                author:{name:"Test"}, models:["acme-video"], fetchMode:"per_task",
                protocols:["openai_video"]};
            export function buildSubmitRequest() { return {}; }
            export function parseSubmitResponse() { return {taskId: "vendor-1"}; }
            export function buildQueryRequest() {
                return {url: "https://api.vendor.example/poll", method: "post", body: {ids: ["a"]}};
            }
            export function parseTaskResult() { return {status: "IN_PROGRESS"}; }
            export const protocols = {openai_video: {
                decodeRequest: function(ctx) { return {kind:"submit", model: ctx.model}; },
                render: function(ctx, task) { return task; }
            }};
            export function listArtifacts() { return []; }
            export function buildContentRequest() { return {}; }

        "#;
        let host = PluginHost::start();
        host.load(SOURCE.to_string(), DEFAULT_CALL_TIMEOUT)
            .await
            .expect("load");
        let ctx = context();
        let request = build_query_request(&host, &ctx, serde_json::json!({}))
            .await
            .expect("query request");
        assert_eq!(request.method, "POST");
        let value: serde_json::Value =
            serde_json::from_slice(&request.body.expect("body").bytes).expect("json");
        assert_eq!(value["ids"][0], "a");

        let mut multipart = new_descriptor("https://api.vendor.example/jobs");
        multipart.body_type = "multipart".to_string();
        multipart.parts = vec![
            RequestPart {
                name: "model".to_string(),
                value: serde_json::json!("acme-video"),
                ..Default::default()
            },
            RequestPart {
                name: "image".to_string(),
                file_ref: "request_file:image".to_string(),
                ..Default::default()
            },
        ];
        let mut with_file = context();
        with_file.files = vec![ResolvedFile {
            reference: "request_file:image".to_string(),
            field: "image".to_string(),
            filename: "cat.png".to_string(),
            mime_type: "image/png".to_string(),
            bytes: b"PNG".to_vec(),
        }];
        let request = build_outbound_request(&multipart, &with_file).expect("request");
        let body = request.body.expect("body");
        assert!(
            body.content_type.starts_with("multipart/form-data"),
            "{}",
            body.content_type
        );
        let text = String::from_utf8_lossy(&body.bytes);
        assert!(text.contains("name=\"model\"\r\n\r\nacme-video"), "{text}");
        assert!(text.contains("filename=\"cat.png\""), "{text}");
        assert!(text.contains("PNG"), "{text}");
        assert!(
            request
                .headers
                .iter()
                .any(|(name, value)| name == "content-type"
                    && value.starts_with("multipart/form-data")),
            "{:?}",
            request.headers
        );
        });
    }
}
