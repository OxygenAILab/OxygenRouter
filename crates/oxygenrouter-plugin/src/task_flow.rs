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
use crate::submit_stream::read_submit_events;
use crate::{
    PluginError, PluginHost, PluginUsage, CAPABILITY_SUBMIT_SSE_DELTA, HOOK_BUILD_QUERY_REQUEST,
    HOOK_EXTRACT_USAGE_ON_COMPLETE, HOOK_PARSE_SUBMIT_RESPONSE, HOOK_PARSE_TASK_RESULT,
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
    /// The upstream answered already, so there is nothing to poll. `BodyKind` is
    /// the response a renderer replaced, which the host may still rewrite for a
    /// client that asked for a different encoding
    /// (`controller/plugin_protocol_image.go:147`).
    Immediate {
        result: TaskResult,
        body: serde_json::Value,
        /// What the plugin stored for itself; the submit-time usage hook reads
        /// it (`adaptor.go:193`).
        task_data: serde_json::Value,
    },
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
    /// The capabilities the plugin required at load time. The delta encoding
    /// is the one the submit-stream reader has to know about, because it
    /// selects which per-event hook the plugin exports.
    pub required_capabilities: Vec<String>,
    /// The usage schema the plugin declared, so a completion hook's facts can
    /// be validated against the model they were reported for.
    pub usage: PluginUsage,
    /// Files the request carried, resolved to their bytes.
    pub files: Vec<ResolvedFile>,
    pub max_inline_bytes: u64,
    pub timeout: Duration,
    /// The request the client sent, after the plugin's decoder normalized it. A
    /// host-owned decision such as `response_format` is read from here, because it
    /// is the caller's instruction rather than the vendor's dialect.
    pub request_body: serde_json::Value,
    /// The host's clock, for the `created` field a renderer may leave out.
    pub created_at: i64,
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
    outcome: &mut HttpOutcome,
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

    // An SSE submission is read through the plugin's per-event parser; what it
    // accumulates is the `body` the response parser then sees, exactly as if
    // the upstream had answered one JSON document (`submit_stream.go:21`).
    let body = if streaming {
        if !outcome.is_event_stream() {
            return Err(FlowError::fatal(
                "expected a text/event-stream submit response",
            ));
        }
        let delta = context
            .required_capabilities
            .iter()
            .any(|capability| capability == CAPABILITY_SUBMIT_SSE_DELTA);
        match read_submit_events(
            host,
            &context.plugin_key,
            delta,
            &request_context,
            &outcome.body,
            context.timeout,
        )
        .await
        {
            Ok(body) => body,
            Err(message) => return Err(FlowError::fatal(message)),
        }
    } else {
        outcome.body_for_hook()
    };

    let parsed = host
        .call_hook_args(
            &context.plugin_key,
            HOOK_PARSE_SUBMIT_RESPONSE,
            &[request_context, submission_input(outcome, body.clone())],
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
        Some(immediate) => {
            // What the upstream answered, before the render hook shapes it. The
            // host's own encoding decision is applied to the hook's *output*
            // (`controller/plugin_protocol_image.go:147`), so the caller reports
            // the raw body and the caller-side rewrite happens in the host.
            Ok(SubmitAnswer::Immediate {
                result: immediate,
                body,
                task_data: submission.task_data,
            })
        }
        None => Ok(SubmitAnswer::Pending(submission)),
    }
}

/// The second argument to `parseSubmitResponse`: the status, the headers and the
/// body, because a plugin decides from all three (`adaptor.go:492`).
fn submission_input(outcome: &HttpOutcome, body: serde_json::Value) -> serde_json::Value {
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
        "body": body,
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
                query_context.clone(),
                outcome.body_for_hook(),
                outcome.response_for_hook(),
            ],
            context.timeout,
        )
        .await?;
    let mut result: TaskResult = serde_json::from_value(value).map_err(|error| {
        FlowError::fatal(format!("parseTaskResult returned an unusable shape: {error}"))
    })?;
    // The completion usage hook runs at this boundary because the raw poll body
    // only exists here (`adaptor.go:805`). A hook that is absent, throws, or
    // returns an invalid shape leaves the result untouched: the reference logs
    // and carries on, and the reservation then stands.
    let body = outcome.body_for_hook();
    match host
        .call_hook_args(
            &context.plugin_key,
            HOOK_EXTRACT_USAGE_ON_COMPLETE,
            &[
                query_context.clone(),
                serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
                body,
            ],
            context.timeout,
        )
        .await
    {
        Ok(facts) => {
            let models: Vec<String> = ["upstreamModel", "model"]
                .iter()
                .filter_map(|key| query_context.get(key).and_then(serde_json::Value::as_str))
                .map(String::from)
                .collect();
            let model_refs: Vec<&str> = models.iter().map(String::as_str).collect();
            match crate::validate_completion_facts(&facts, context.usage.for_models(&model_refs)) {
                Ok(validated) => crate::apply_completion_usage(&mut result, &validated),
                Err(reason) => eprintln!(
                    "[OxygenRouter] plugin {} rejected invalid extractUsageOnComplete facts: {reason}",
                    context.plugin_key
                ),
            }
        }
        Err(PluginError::NoSuchHook { .. }) => {}
        Err(error) => eprintln!(
            "[OxygenRouter] plugin {} extractUsageOnComplete failed: {error}",
            context.plugin_key
        ),
    }
    Ok(result)
}

/// A task as the poller needs to see it.
///
/// Deliberately its own shape rather than a borrow of the stored record: the
/// plugin crate cannot depend on the store, and the poller only needs these
/// fields to build a query and to know what state it is comparing and setting.
#[derive(Debug, Clone, PartialEq)]
pub struct PollTask {
    /// The public id, which is what a settlement is recorded against.
    pub task_id: String,
    /// The status the caller read. Completion is a compare-and-set against it, so
    /// a second finisher loses the race instead of settling twice.
    pub status: String,
    /// The id the upstream knows, which is what a query addresses.
    pub upstream_task_id: String,
    pub action: String,
    pub model: String,
    pub upstream_model: String,
    /// When the task was created, which is the clock the timeout measures.
    pub created_at: i64,
    /// The plugin's own persisted data, handed back on every poll.
    pub data: serde_json::Value,
    /// The opaque state the plugin asked the host to keep.
    pub state: serde_json::Value,
}

/// What one poll round produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollRound {
    /// Still running; the progress was updated.
    Continued,
    /// A terminal status; settle the quota.
    Settled,
    /// The upstream does not know the task, or it outlived its budget. It is a
    /// failure, so it refunds.
    Failed,
    /// The poll itself failed; the task is untouched and will be polled again.
    Retried,
    /// The answer is not something the host can act on; the task is untouched and
    /// an operator should look.
    Refused,
}

/// What a poll round asks the caller to store.
#[derive(Debug, Clone, PartialEq)]
pub struct PollSettlement {
    pub task_id: String,
    /// The status the caller read, so it can compare-and-set against it.
    pub expected_status: String,
    /// The new status, or `None` when the poll changed nothing.
    pub status: Option<String>,
    pub progress: Option<String>,
    pub reason: Option<String>,
    /// The plugin's updated opaque state, when it asked the host to keep one.
    pub plugin_state: Option<serde_json::Value>,
    /// What the quota should do. `None` means "leave it alone for now".
    pub settle: Option<crate::SettlePlan>,
    /// The billable tokens a usage-reporting answer carried, so the caller can
    /// settle against real usage rather than the reservation.
    pub usage_tokens: i64,
    pub round: PollRound,
}

impl PollSettlement {
    fn untouched(task: &PollTask, round: PollRound, reason: String) -> Self {
        PollSettlement {
            task_id: task.task_id.clone(),
            expected_status: task.status.clone(),
            status: None,
            progress: None,
            reason: Some(reason),
            plugin_state: None,
            settle: None,
            usage_tokens: 0,
            round,
        }
    }
}

/// Run one poll round for one task and report what should be stored.
///
/// The caller owns the CAS and the money; this decides *what* to do, which is the
/// part with the invariants in it. It never retries on its own: a backlog is
/// drained by calling this once per task per tick, so a slow upstream cannot make
/// one request hold a worker.
pub async fn poll_once(
    host: &PluginHost,
    transport: &dyn TaskTransport,
    context: &TaskFlowContext,
    task: &PollTask,
    now: i64,
    timeout_secs: i64,
    per_call_pricing: bool,
) -> PollSettlement {
    // A task that outlived its budget is failed, because an upstream answering
    // "still running" forever would otherwise hold the caller's reservation
    // indefinitely (`service/task_polling.go:70`).
    if crate::is_timed_out(task.created_at, now, timeout_secs) {
        return PollSettlement {
            task_id: task.task_id.clone(),
            expected_status: task.status.clone(),
            status: Some(crate::STATUS_FAILURE.to_string()),
            progress: None,
            reason: Some(format!(
                "task did not finish within {timeout_secs}s; it was failed and refunded"
            )),
            plugin_state: None,
            settle: Some(crate::SettlePlan::Refund),
            usage_tokens: 0,
            round: PollRound::Failed,
        };
    }

    let query_context = crate::QueryContext {
        task_id: task.upstream_task_id.clone(),
        public_task_id: task.task_id.clone(),
        action: task.action.clone(),
        model: task.model.clone(),
        upstream_model: task.upstream_model.clone(),
        base_url: context.base_url.clone(),
        data: task.data.clone(),
        state: task.state.clone(),
        upstream: None,
    };
    let query_value = match serde_json::to_value(&query_context) {
        Ok(value) => value,
        Err(error) => {
            return PollSettlement::untouched(
                task,
                PollRound::Refused,
                format!("task state could not be presented to the plugin: {error}"),
            )
        }
    };

    let request = match build_query_request(host, context, query_value.clone()).await {
        Ok(request) => request,
        Err(error) => {
            // A plugin that cannot describe its own poll is a plugin bug, not an
            // upstream fault, so the task is left alone rather than failed.
            return PollSettlement::untouched(task, PollRound::Refused, error.message);
        }
    };
    let outcome = match transport.execute(request).await {
        Ok(outcome) => outcome,
        Err(reason) => return PollSettlement::untouched(task, PollRound::Retried, reason),
    };

    // A parser failure must not hide the HTTP answer: a 503 needs no parser, and a
    // 200 with an unusable body is a plugin gap the decision below reports.
    let parsed = interpret_task_result(host, context, query_value, &outcome)
        .await
        .ok();

    match crate::decide_poll(&outcome, parsed.as_ref()) {
        crate::PollDecision::Continue { progress } => PollSettlement {
            task_id: task.task_id.clone(),
            expected_status: task.status.clone(),
            status: Some(
                parsed
                    .as_ref()
                    .map(|result| result.status.clone())
                    .unwrap_or_else(|| task.status.clone()),
            ),
            progress: Some(progress),
            reason: None,
            plugin_state: plugin_state_of(parsed.as_ref()),
            settle: None,
            usage_tokens: 0,
            round: PollRound::Continued,
        },
        crate::PollDecision::Terminal {
            status,
            progress,
            reason,
            result,
        } => {
            let usage_tokens = crate::billable_tokens(&result);
            PollSettlement {
                task_id: task.task_id.clone(),
                expected_status: task.status.clone(),
                status: Some(status.clone()),
                progress: Some(progress),
                reason: if reason.is_empty() { None } else { Some(reason) },
                plugin_state: plugin_state_of(parsed.as_ref()),
                settle: Some(crate::settle_plan(&status, usage_tokens > 0, per_call_pricing)),
                usage_tokens,
                round: PollRound::Settled,
            }
        }
        crate::PollDecision::Fail { reason } => PollSettlement {
            task_id: task.task_id.clone(),
            expected_status: task.status.clone(),
            status: Some(crate::STATUS_FAILURE.to_string()),
            progress: None,
            reason: Some(reason),
            plugin_state: None,
            // A task that failed costs nothing, so it refunds in full. A failure
            // that already settled is not refunded again by the caller.
            settle: Some(crate::SettlePlan::Refund),
            usage_tokens: 0,
            round: PollRound::Failed,
        },
        crate::PollDecision::Retry { reason } => {
            PollSettlement::untouched(task, PollRound::Retried, reason)
        }
        crate::PollDecision::Refuse { reason } => {
            PollSettlement::untouched(task, PollRound::Refused, reason)
        }
    }
}

/// The plugin state a poll asked the host to keep, when it sent one.
fn plugin_state_of(result: Option<&TaskResult>) -> Option<serde_json::Value> {
    result
        .map(|result| result.state.clone())
        .filter(|state| !state.is_null())
}

/// The host-owned part of an image answer, planned rather than performed.
///
/// Two things a client may ask for that a *plugin* is not in a position to know,
/// because they are host policy rather than vendor dialect:
///
/// * `created` is the host's own clock, filled when the plugin left it out
///   (`controller/plugin_protocol_image.go:138`);
/// * `response_format: "b64_json"` inlines each image, and the reference does that
///   *after* the render hook has run -- so a plugin renders upstream URLs and the
///   host decides what the caller receives (`:147`).
///
/// Fetching belongs to the transport that already exists rather than here, so this
/// only says what to fetch and where it goes. `response_json_format` is the shape
/// the answer uses, which decides whether the images sit in a `data` array (the
/// images API) or at a success/failure pair (the responses API).
pub fn plan_image_encoding(
    body: &mut serde_json::Value,
    response_format: Option<&str>,
    created_at: i64,
) -> Vec<ImageInline> {
    let Some(object) = body.as_object_mut() else {
        return Vec::new();
    };
    // `created` is host-owned and only filled when the plugin did not state one:
    // a plugin that knows the upstream's timestamp keeps it.
    object
        .entry("created".to_string())
        .or_insert_with(|| serde_json::json!(created_at));

    if response_format != Some("b64_json") {
        return Vec::new();
    }
    let Some(data) = object.get("data").and_then(|value| value.as_array()) else {
        return Vec::new();
    };
    let mut plan = Vec::new();
    for (index, entry) in data.iter().enumerate() {
        let Some(item) = entry.as_object() else {
            continue;
        };
        let url = item
            .get("url")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        // An image the plugin already inlined is left alone, and so is an entry
        // with no URL to fetch.
        if url.trim().is_empty() || item.contains_key("b64_json") {
            continue;
        }
        plan.push(ImageInline {
            index,
            url,
            mime_hint: item
                .get("mime_type")
                .and_then(|value| value.as_str())
                .map(String::from),
        });
    }
    plan
}

/// One image the host should inline, and where in the answer it belongs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageInline {
    /// Position in the answer's `data` array.
    pub index: usize,
    pub url: String,
    /// A mime type the plugin stated, when it stated one.
    pub mime_hint: Option<String>,
}

/// Place a fetched image into the answer, keeping the URL beside it.
///
/// The URL is kept on purpose: a client that asked for base64 can still tell which
/// upstream artifact it received, and a fetch failure leaves the answer usable
/// rather than failing the request (`controller/plugin_protocol_image.go:154`).
pub fn apply_image_inline(
    body: &mut serde_json::Value,
    inline: &ImageInline,
    mime_type: &str,
    encoded: &str,
) -> bool {
    let Some(item) = body
        .get_mut("data")
        .and_then(|value| value.as_array_mut())
        .and_then(|data| data.get_mut(inline.index))
        .and_then(|entry| entry.as_object_mut())
    else {
        return false;
    };
    // The entry may have moved on since the plan was made, so the URL is checked
    // again: filling the wrong image would be worse than not filling it.
    if item.get("url").and_then(|value| value.as_str()) != Some(inline.url.as_str()) {
        return false;
    }
    item.insert(
        "b64_json".to_string(),
        serde_json::Value::String(encoded.to_string()),
    );
    item.entry("mime_type".to_string())
        .or_insert_with(|| {
            serde_json::Value::String(
                inline
                    .mime_hint
                    .clone()
                    .unwrap_or_else(|| mime_type.to_string()),
            )
        });
    true
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
            required_capabilities: Vec::new(),
            usage: PluginUsage::default(),
            files: Vec::new(),
            max_inline_bytes: 0,
            timeout: DEFAULT_CALL_TIMEOUT,
            request_body: serde_json::Value::Null,
            created_at: 1_700_000_000,
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
            &mut outcome.clone(),
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
        let answer = interpret_submit(&host, &descriptor, &ctx, serde_json::json!({}), &mut outcome.clone())
            .await
            .expect("interpret");
        match answer {
            SubmitAnswer::Immediate { result, body, .. } => {
                assert_eq!(result.status, STATUS_SUCCESS);
                assert_eq!(result.url, "https://cdn/1.png");
                // The raw upstream body travels with the result, because the
                // host's own encoding decision is applied to the renderer's
                // *output* rather than to this -- and it is what the renderer is
                // shown as the task's data.
                assert_eq!(body["status"], "queued");
                assert_eq!(body["id"], "vendor-1");
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
            &mut json_outcome(200, "{}"),
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
            &mut json_outcome(200, "{}"),
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
        let error = interpret_submit(&host, &descriptor, &ctx, serde_json::json!({}), &mut sse.clone())
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
            &mut json_outcome(500, "{}"),
        )
        .await
        .expect_err("must refuse");
        assert!(!error.retryable, "an accepted stream is not retried");
        });
    }

    /// A plugin that answers a submission with an event stream is read through
    /// its own per-event parser, and what it accumulates is the body its
    /// response parser sees -- the ported contract from
    /// `relay/channel/task/jsplugin/adaptor_test.go:1681`.
    #[test]
    fn an_sse_submission_is_read_through_the_plugins_event_parser() {
        runtime().block_on(async {
            const SOURCE: &str = r#"
                export const meta = {apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                    author:{name:"Test"}, models:["acme-video"], fetchMode:"per_task",
                    submitResponseTypes:["json","sse"], protocols:["openai_video"]};
                export function buildSubmitRequest(ctx) {
                    return {url: ctx.baseUrl + "/compile", responseType: ctx.requestBody.responseType || "sse"};
                }
                export function parseSubmitEvent(ctx, event, previous) {
                    const chunk = JSON.parse(event.data);
                    if (chunk.error) throw new Error("provider stream failure");
                    if (chunk.badState) return {state:null};
                    if (chunk.largeState) return {state:{document:"x".repeat(1048577)},done:chunk.complete === true};
                    if (chunk.escapedState) return {state:{document:String.fromCharCode(0).repeat(200000)},done:true};
                    if (chunk.resetState) return {state:{document:"",units:0},done:true};
                    const state = Object.assign({}, previous || {document:"",units:0});
                    state.document += chunk.part || "";
                    if (chunk.units !== undefined) state.units = chunk.units;
                    state.event = event.event; state.id = event.id;
                    return {state:state,done:chunk.complete === true};
                }
                export function parseSubmitResponse(ctx, response) {
                    return {taskId:"vendor-document", taskData: response.body, immediate:{status:"SUCCESS"}};
                }
                export function buildQueryRequest() { return {url:"https://api.vendor.example/query"}; }
                export function parseTaskResult() { return {status:"SUCCESS"}; }
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

            let mut ctx = context();
            ctx.submit_response_types = vec!["json".to_string(), "sse".to_string()];
            let mut descriptor = new_descriptor("https://api.vendor.example/compile");
            descriptor.response_type = "sse".to_string();

            let sse = |content_type: &str, body: Vec<u8>| HttpOutcome {
                status: 200,
                headers: vec![("content-type".to_string(), content_type.to_string())],
                body,
            };
            let cases: Vec<(&str, &str, Vec<u8>, bool)> = vec![
                (
                    "multiline and CRLF",
                    "text/event-stream; charset=utf-8",
                    b": heartbeat\r\nid: doc-1\r\nevent: update\r\ndata: {\"part\":\r\ndata: \"hello\",\"units\":2}\r\n\r\ndata: {\"part\":\"world\",\"units\":3,\"complete\":true}\n\n".to_vec(),
                    true,
                ),
                (
                    "zero actual units",
                    "text/event-stream",
                    b"data: {\"part\":\"free\",\"units\":0,\"complete\":true}\n\n".to_vec(),
                    true,
                ),
                (
                    "premature EOF",
                    "text/event-stream",
                    b"data: {\"part\":\"partial\"}\n\n".to_vec(),
                    false,
                ),
                (
                    "unterminated event",
                    "text/event-stream",
                    b"data: {\"complete\":true}".to_vec(),
                    false,
                ),
                (
                    "provider error",
                    "text/event-stream",
                    b"data: {\"error\":true}\n\n".to_vec(),
                    false,
                ),
                (
                    "invalid hook result",
                    "text/event-stream",
                    b"data: {\"badState\":true}\n\n".to_vec(),
                    false,
                ),
                (
                    "state limit",
                    "text/event-stream",
                    b"data: {\"largeState\":true}\n\n".to_vec(),
                    false,
                ),
                (
                    "intermediate state limit",
                    "text/event-stream",
                    b"data: {\"largeState\":true}\n\ndata: {\"resetState\":true}\n\n".to_vec(),
                    false,
                ),
                (
                    "escaped state limit",
                    "text/event-stream",
                    b"data: {\"escapedState\":true}\n\n".to_vec(),
                    false,
                ),
                (
                    "event limit",
                    "text/event-stream",
                    format!("data: {}\n\n", "x".repeat(1 << 20)).into_bytes(),
                    false,
                ),
                ("wrong response type", "application/json", b"{}".to_vec(), false),
            ];
            for (name, content_type, body, valid) in cases {
                let mut outcome = sse(content_type, body);
                let answer = interpret_submit(
                    &host,
                    &descriptor,
                    &ctx,
                    serde_json::json!({}),
                    &mut outcome,
                )
                .await;
                if !valid {
                    let error = answer.expect_err(name);
                    assert!(!error.retryable, "{name}: an accepted stream is not retried");
                    continue;
                }
                let SubmitAnswer::Immediate { body, .. } = answer.expect(name) else {
                    panic!("{name}: expected an immediate answer");
                };
                if name == "zero actual units" {
                    assert_eq!(body["units"], 0, "{name}");
                } else {
                    assert_eq!(
                        body,
                        serde_json::json!({"document":"helloworld","units":3,"event":"message","id":"doc-1"}),
                        "{name}"
                    );
                }
            }

            // A descriptor that never declared SSE refuses an event stream
            // outright: the two sides disagree about the encoding.
            let json_descriptor = new_descriptor("https://api.vendor.example/compile");
            let error = interpret_submit(
                &host,
                &json_descriptor,
                &ctx,
                serde_json::json!({}),
                &mut sse("text/event-stream", b"data: {}\n\n".to_vec()),
            )
            .await
            .expect_err("undeclared stream");
            assert!(
                error.message.contains("unexpected SSE response"),
                "{}",
                error.message
            );
            assert!(!error.retryable);

            // A streaming descriptor whose upstream answered JSON is refused
            // after acceptance, so a retry could pay twice.
            let error = interpret_submit(
                &host,
                &descriptor,
                &ctx,
                serde_json::json!({}),
                &mut sse("application/json", b"{}".to_vec()),
            )
            .await
            .expect_err("not a stream");
            assert!(
                error.message.contains("expected a text/event-stream"),
                "{}",
                error.message
            );
            assert!(!error.retryable);
        });
    }

    /// The delta capability's events carry only changes and control state; the
    /// host owns the accumulated result and its budget
    /// (`relay/channel/task/jsplugin/adaptor_test.go:1814`).
    #[test]
    fn an_sse_delta_submission_accumulates_changes() {
        runtime().block_on(async {
            const SOURCE: &str = r#"
                export const meta = {apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                    author:{name:"Test"}, models:["acme-video"], fetchMode:"per_task",
                    submitResponseTypes:["json","sse"], requiredCapabilities:["submit-sse-delta@1"],
                    protocols:["openai_video"]};
                export function buildSubmitRequest(ctx) {
                    return {url: ctx.baseUrl + "/compile", responseType: "sse"};
                }
                export function parseSubmitEventDelta(ctx, event, previous) {
                    if (previous && previous.document !== undefined) throw new Error("full result leaked into control state");
                    const chunk = JSON.parse(event.data);
                    let changes = chunk.changes;
                    if (chunk.largeResult) changes = [{op:"set",path:[],value:{document:"x".repeat(1048577)}}];
                    if (chunk.resetAfterLarge) changes.push({op:"set",path:[],value:{document:"",units:0}});
                    const result = {changes:changes, state:{events:(previous ? previous.events : 0)+1}, done:chunk.complete === true};
                    if (chunk.largeControl) result.state = {text:"x".repeat(65537)};
                    if (chunk.extra) result.extra = true;
                    return result;
                }
                export function parseSubmitResponse(ctx, response) {
                    return {taskId:"vendor-document", taskData: response.body, immediate:{status:"SUCCESS"}};
                }
                export function buildQueryRequest() { return {url:"https://api.vendor.example/query"}; }
                export function parseTaskResult() { return {status:"SUCCESS"}; }
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

            let mut ctx = context();
            ctx.submit_response_types = vec!["json".to_string(), "sse".to_string()];
            ctx.required_capabilities = vec![CAPABILITY_SUBMIT_SSE_DELTA.to_string()];
            let mut descriptor = new_descriptor("https://api.vendor.example/compile");
            descriptor.response_type = "sse".to_string();

            const FIRST: &str = r#"{"changes":[{"op":"set","path":[],"value":{"document":"hello","units":2}}]}"#;
            const LAST: &str = r#"{"changes":[{"op":"appendText","path":["document"],"value":"world"},{"op":"set","path":["units"],"value":0}],"complete":true}"#;
            let stream = |frames: &[&str]| {
                let mut body = String::new();
                for frame in frames {
                    body.push_str("data: ");
                    body.push_str(frame);
                    body.push_str("\n\n");
                }
                HttpOutcome {
                    status: 200,
                    headers: vec![("content-type".to_string(), "text/event-stream".to_string())],
                    body: body.into_bytes(),
                }
            };
            let cases: Vec<(&str, Vec<&str>, &str)> = vec![
                ("control state and zero usage", vec![FIRST, LAST], ""),
                (
                    "oversized intermediate result",
                    vec![r#"{"largeResult":true}"#, LAST],
                    "size",
                ),
                (
                    "oversized operation before reset",
                    vec![r#"{"largeResult":true,"resetAfterLarge":true,"complete":true}"#],
                    "size",
                ),
                (
                    "control state limit",
                    vec![FIRST, r#"{"changes":[],"largeControl":true,"complete":true}"#],
                    "control state",
                ),
                (
                    "missing changes",
                    vec![r#"{"complete":true}"#],
                    "changes",
                ),
                (
                    "extra result fields",
                    vec![FIRST, r#"{"changes":[],"extra":true,"complete":true}"#],
                    "only changes, state and done",
                ),
                (
                    "unfinished delta stream",
                    vec![FIRST],
                    "ended before the plugin reported completion",
                ),
            ];
            for (name, frames, expected_error) in cases {
                let mut outcome = stream(&frames);
                let answer = interpret_submit(
                    &host,
                    &descriptor,
                    &ctx,
                    serde_json::json!({}),
                    &mut outcome,
                )
                .await;
                if expected_error.is_empty() {
                    let SubmitAnswer::Immediate { body, .. } = answer.expect(name) else {
                        panic!("{name}: expected an immediate answer");
                    };
                    assert_eq!(
                        body,
                        serde_json::json!({"document":"helloworld","units":0}),
                        "{name}"
                    );
                    continue;
                }
                let error = answer.expect_err(name);
                assert!(
                    error.message.contains(expected_error),
                    "{name}: {}",
                    error.message
                );
                assert!(!error.retryable, "{name}: an accepted stream is not retried");
            }
        });
    }

    /// A completed poll runs the plugin's own usage hook, validates what it
    /// reported against the shape the manifest declared, and turns the
    /// host-owned counters into the token count settlement reads
    /// (`adaptor.go:805`).
    #[test]
    fn a_completion_hook_turns_usage_facts_into_tokens() {
        runtime().block_on(async {
            const SOURCE: &str = r#"
                export const meta = {apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                    author:{name:"Test"}, models:["acme-video"], fetchMode:"per_task",
                    usageSchema:{seconds:{type:"number",unit:"second"}, mode:{enum:["std","pro"]}},
                    protocols:["openai_video"]};
                export function buildSubmitRequest(ctx) { return {url: ctx.baseUrl + "/jobs"}; }
                export function parseSubmitResponse(ctx, response) { return {taskId:"vendor-1"}; }
                export function buildQueryRequest(ctx) { return {url: ctx.baseUrl + "/jobs/1"}; }
                export function parseTaskResult(ctx, body, response) { return {status:"SUCCESS"}; }
                export function extractUsageOnComplete(ctx, result, body) {
                    if (body.seconds !== 5) throw new Error("no body argument");
                    if (!result || result.status !== "SUCCESS") throw new Error("no result argument");
                    if (ctx.upstreamModel !== "vendor-model") throw new Error("no upstream model in context");
                    return {seconds:5, mode:"pro", upstreamUnits: 3};
                }
                export const protocols = {openai_video: {
                    decodeRequest: function(ctx) { return {kind:"submit", model: ctx.model}; },
                    render: function(ctx, task) { return task; }
                }};
                export function listArtifacts() { return []; }
                export function buildContentRequest() { return {}; }
            "#;
            let host = PluginHost::start();
            let manifest = host
                .load(SOURCE.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("a manifest with a usage schema loads");
            assert!(manifest.usage_schema.contains_key("seconds"));

            let mut ctx = context();
            ctx.usage = PluginUsage {
                schema: manifest.usage_schema.clone(),
                profiles: manifest.usage_profiles.clone(),
            };
            let outcome = HttpOutcome {
                status: 200,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                body: br#"{"seconds":5}"#.to_vec(),
            };
            let result = interpret_task_result(
                &host,
                &ctx,
                serde_json::json!({"model":"acme-video","upstreamModel":"vendor-model"}),
                &outcome,
            )
            .await
            .expect("parsed");
            assert_eq!(result.total_tokens, 3.0, "upstreamUnits wins");
            assert_eq!(result.usage_facts["mode"], "pro");
            assert_eq!(crate::billable_tokens(&result), 3);

            // A hook that reports a fact the schema refuses leaves the result
            // untouched: the reservation then stands rather than billing a
            // value nobody validated.
            let host = PluginHost::start();
            host.load(
                SOURCE.replace(
                    r#"return {seconds:5, mode:"pro", upstreamUnits: 3};"#,
                    r#"return {seconds:"five", mode:"pro", upstreamUnits: 3};"#,
                ),
                DEFAULT_CALL_TIMEOUT,
            )
            .await
            .expect("load");
            let result = interpret_task_result(
                &host,
                &ctx,
                serde_json::json!({"model":"acme-video","upstreamModel":"vendor-model"}),
                &outcome,
            )
            .await
            .expect("parsed");
            assert_eq!(crate::billable_tokens(&result), 0);
            assert!(result.usage_facts.is_null());
        });
    }

    /// The schema itself is validated at load time, so a plugin cannot ship a
    /// unit the host would not know how to bound
    /// (`pkg/jsplugin/registry.go:1550`).
    #[test]
    fn an_invalid_usage_schema_is_refused_at_load() {
        runtime().block_on(async {
            const SOURCE: &str = r#"
                export const meta = {apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
                    author:{name:"Test"}, models:["acme-video"], fetchMode:"per_task",
                    usageSchema:{seconds:{type:"number",unit:"bogus"}},
                    protocols:["openai_video"]};
                export function buildSubmitRequest(ctx) { return {url: ctx.baseUrl + "/jobs"}; }
                export function parseSubmitResponse(ctx, response) { return {taskId:"vendor-1"}; }
                export function buildQueryRequest(ctx) { return {url: ctx.baseUrl + "/jobs/1"}; }
                export function parseTaskResult() { return {status:"SUCCESS"}; }
                export const protocols = {openai_video: {
                    decodeRequest: function(ctx) { return {kind:"submit", model: ctx.model}; },
                    render: function(ctx, task) { return task; }
                }};
                export function listArtifacts() { return []; }
                export function buildContentRequest() { return {}; }
            "#;
            let error = PluginHost::start()
                .load(SOURCE.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("invalid unit");
            assert!(
                error.to_string().contains("unit must be second, count, token, or credit"),
                "{error}"
            );
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

    /// A plugin serving several protocols resolves a hook against the object that
    /// *defines* it, not the first one that exists.
    ///
    /// Found live: a plugin claiming `openai_image`, `openai_video` and
    /// `openai_responses` was reported as missing `renderEvents`, because the
    /// resolver fell through to whichever protocol object came first and looked
    /// there. The protocols are keyed by protocol name, so the caller's key is not
    /// the right address for a member that lives under another one.
    #[test]
    fn a_hook_resolves_against_the_protocol_that_defines_it() {
        runtime().block_on(async {
            const SOURCE: &str = r#"
                export const meta = {apiVersion:1, key:"multi", name:"Multi", version:"1.0.0",
                    author:{name:"Test"}, models:["m"], fetchMode:"per_task",
                    protocols:["openai_image", {name:"openai_responses", supports:["sync","stream"]}]};
                export function buildSubmitRequest() { return {}; }
                export function parseSubmitResponse() { return {taskId: "1"}; }
                export function buildQueryRequest() { return {}; }
                export function parseTaskResult() { return {status: "IN_PROGRESS"}; }
                export const protocols = {
                    openai_image: {
                        decodeRequest: function(ctx) { return {kind:"submit", model: ctx.model}; },
                        render: function(ctx, task) { return {from: "image"}; }
                    },
                    openai_responses: {
                        decodeRequest: function(ctx) { return {kind:"submit", model: ctx.model}; },
                        // This member exists only on the *second* protocol object,
                        // which is what the resolver has to find.
                        renderEvents: function(ctx, task) {
                            return {events: [{type: "output", data: "ok"}], done: false};
                        },
                        renderFinal: function(ctx, task) { return {from: "responses"}; }
                    }
                };
            "#;
            let host = PluginHost::start();
            host.load(SOURCE.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");

            let value = host
                .call_hook_args(
                    "multi",
                    "renderEvents",
                    &[serde_json::json!({}), serde_json::json!({})],
                    DEFAULT_CALL_TIMEOUT,
                )
                .await
                .expect("renderEvents must resolve");
            assert_eq!(value["events"][0]["data"], "ok");

            // A hook that no protocol object implements is still named as missing,
            // rather than being reported as an absent protocol.
            let error = host
                .call_hook_args("multi", "renderNothing", &[], DEFAULT_CALL_TIMEOUT)
                .await
                .expect_err("must fail");
            let text = error.to_string();
            assert!(text.contains("no protocol member"), "{text}");
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

    /// A plugin whose poll the host can read, for the poller's tests.
    const POLLING_PLUGIN: &str = r#"
        export const meta = {apiVersion:1, key:"acme", name:"Acme", version:"1.0.0",
            author:{name:"Test"}, models:["acme-video"], fetchMode:"per_task",
            protocols:["openai_video"]};
        export function buildSubmitRequest() { return {}; }
        export function parseSubmitResponse() { return {taskId: "vendor-1"}; }
        export function buildQueryRequest(ctx) { return {url: ctx.baseUrl + "/jobs/" + ctx.taskId}; }
        export function parseTaskResult(ctx, body, response) { return body; }
        export function listArtifacts() { return []; }
        export function buildContentRequest() { return {}; }
        export const protocols = {openai_video: {
            decodeRequest: function(ctx) { return {kind:"submit", model: ctx.model}; },
            render: function(ctx, task) { return task; }
        }};
    "#;

    fn poll_task(status: &str) -> PollTask {
        PollTask {
            task_id: "task_public".to_string(),
            status: status.to_string(),
            upstream_task_id: "vendor-1".to_string(),
            action: "text_to_video".to_string(),
            model: "acme-video".to_string(),
            upstream_model: "acme-video".to_string(),
            created_at: 1_000_000,
            data: serde_json::json!({ "kind": "video" }),
            state: serde_json::json!({ "cursor": 1 }),
        }
    }

    /// A whole poll round: the descriptor is addressed by the task's *upstream*
    /// id, the answer is read by the plugin, and the settlement says what the
    /// caller should store.
    #[test]
    fn a_poll_round_reports_what_to_store_and_what_it_costs() {
        runtime().block_on(async {
            let host = PluginHost::start();
            host.load(POLLING_PLUGIN.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");
            let ctx = context();
            let task = poll_task(crate::STATUS_QUEUED);

            // Still running: progress moves, nothing settles.
            let transport = StubTransport::answering(vec![json_outcome(
                200,
                r#"{"status":"IN_PROGRESS","progress":"40%"}"#,
            )]);
            let settlement = poll_once(&host, &transport, &ctx, &task, 1_000_100, 0, false).await;
            assert_eq!(settlement.round, PollRound::Continued);
            assert_eq!(settlement.status.as_deref(), Some(crate::STATUS_IN_PROGRESS));
            assert_eq!(settlement.progress.as_deref(), Some("40%"));
            assert_eq!(settlement.settle, None, "a running task settles nothing");
            assert_eq!(settlement.expected_status, crate::STATUS_QUEUED);
            {
                let sent = transport.sent.lock().expect("lock");
                assert_eq!(sent[0].method, "GET");
                assert_eq!(sent[0].url, "https://api.vendor.example/jobs/vendor-1");
            }

            // Terminal success with usage: settle against what actually happened.
            let transport = StubTransport::answering(vec![json_outcome(
                200,
                r#"{"status":"SUCCESS","progress":"100%","totalTokens":8}"#,
            )]);
            let settlement = poll_once(&host, &transport, &ctx, &task, 1_000_100, 0, false).await;
            assert_eq!(settlement.round, PollRound::Settled);
            assert_eq!(settlement.status.as_deref(), Some(crate::STATUS_SUCCESS));
            assert_eq!(
                settlement.settle,
                Some(crate::SettlePlan::SettleWithUsage)
            );

            // The same success under a per-call price keeps its reservation.
            let transport = StubTransport::answering(vec![json_outcome(
                200,
                r#"{"status":"SUCCESS","progress":"100%","totalTokens":8}"#,
            )]);
            let settlement = poll_once(&host, &transport, &ctx, &task, 1_000_100, 0, true).await;
            assert_eq!(
                settlement.settle,
                Some(crate::SettlePlan::KeepReservation),
                "a per-call price never gets a second look"
            );

            // Terminal failure refunds, and carries the plugin's reason.
            let transport = StubTransport::answering(vec![json_outcome(
                200,
                r#"{"status":"FAILURE","reason":"upstream refused the prompt"}"#,
            )]);
            let settlement = poll_once(&host, &transport, &ctx, &task, 1_000_100, 0, false).await;
            assert_eq!(settlement.round, PollRound::Settled);
            assert_eq!(settlement.settle, Some(crate::SettlePlan::Refund));
            assert_eq!(
                settlement.reason.as_deref(),
                Some("upstream refused the prompt")
            );
        });
    }

    /// A poll the host should not act on leaves the task exactly as it found it,
    /// and a poll that says the task is gone fails it and refunds.
    #[test]
    fn a_poll_that_cannot_be_acted_on_does_not_touch_the_task() {
        runtime().block_on(async {
            let host = PluginHost::start();
            host.load(POLLING_PLUGIN.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");
            let ctx = context();
            let task = poll_task(crate::STATUS_IN_PROGRESS);

            // An auth failure is the operator's problem: the task is untouched and
            // nothing settles, so a broken credential cannot burn a reservation.
            let transport = StubTransport::answering(vec![json_outcome(401, "{}")]);
            let settlement = poll_once(&host, &transport, &ctx, &task, 1_000_100, 0, false).await;
            assert_eq!(settlement.round, PollRound::Retried);
            assert_eq!(settlement.status, None);
            assert_eq!(settlement.settle, None);
            assert!(settlement
                .reason
                .as_deref()
                .expect("reason")
                .contains("auth"));

            // A transport failure is the same: retry, do not judge.
            let settlement = poll_once(
                &host,
                &StubTransport::default(),
                &ctx,
                &task,
                1_000_100,
                0,
                false,
            )
            .await;
            assert_eq!(settlement.round, PollRound::Retried);
            assert_eq!(settlement.status, None);

            // The upstream forgot the task, so it fails and refunds rather than
            // being polled forever.
            let transport = StubTransport::answering(vec![json_outcome(404, "{}")]);
            let settlement = poll_once(&host, &transport, &ctx, &task, 1_000_100, 0, false).await;
            assert_eq!(settlement.round, PollRound::Failed);
            assert_eq!(
                settlement.status.as_deref(),
                Some(crate::STATUS_FAILURE)
            );
            assert_eq!(settlement.settle, Some(crate::SettlePlan::Refund));
            assert!(settlement
                .reason
                .as_deref()
                .expect("reason")
                .contains("not found"));

            // A status the plugin may not report is refused, not settled.
            let transport = StubTransport::answering(vec![json_outcome(
                200,
                r#"{"status":"VIBING","progress":"?"}"#,
            )]);
            let settlement = poll_once(&host, &transport, &ctx, &task, 1_000_100, 0, false).await;
            assert_eq!(settlement.round, PollRound::Refused);
            assert_eq!(settlement.status, None);
            assert_eq!(settlement.settle, None);
        });
    }

    /// A task that outlived its budget is failed and refunded without the
    /// upstream being asked at all, because an upstream that answers "still
    /// running" forever would hold the caller's reservation indefinitely.
    #[test]
    fn a_task_that_outlived_its_budget_is_failed_without_asking_the_upstream() {
        runtime().block_on(async {
            let host = PluginHost::start();
            host.load(POLLING_PLUGIN.to_string(), DEFAULT_CALL_TIMEOUT)
                .await
                .expect("load");
            let transport = StubTransport::answering(vec![json_outcome(
                200,
                r#"{"status":"IN_PROGRESS"}"#,
            )]);
            let settlement = poll_once(
                &host,
                &transport,
                &context(),
                &poll_task(crate::STATUS_IN_PROGRESS),
                1_003_601,
                3_600,
                false,
            )
            .await;

            assert_eq!(settlement.round, PollRound::Failed);
            assert_eq!(settlement.status.as_deref(), Some(crate::STATUS_FAILURE));
            assert_eq!(settlement.settle, Some(crate::SettlePlan::Refund));
            assert!(settlement
                .reason
                .as_deref()
                .expect("reason")
                .contains("3600s"));
            assert!(
                transport.sent.lock().expect("lock").is_empty(),
                "the upstream must not be asked about a task that is already over budget"
            );
        });
    }

    /// A plugin's updated poll state is carried back for storage, because a
    /// plugin that asked the host to remember something must get it next round.
    #[test]
    fn a_poll_carries_the_plugin_state_it_wants_kept() {
        runtime().block_on(async {
            let host = PluginHost::start();
            host.load(
                POLLING_PLUGIN.replace(
                    "export function parseTaskResult(ctx, body, response) { return body; }",
                    "export function parseTaskResult(ctx, body, response) { return {status: \"IN_PROGRESS\", progress: \"20%\", state: {cursor: 9}}; }",
                ),
                DEFAULT_CALL_TIMEOUT,
            )
            .await
            .expect("load");
            let transport = StubTransport::answering(vec![json_outcome(200, "{}")]);
            let settlement = poll_once(
                &host,
                &transport,
                &context(),
                &poll_task(crate::STATUS_QUEUED),
                1_000_100,
                0,
                false,
            )
            .await;
            assert_eq!(settlement.round, PollRound::Continued);
            assert_eq!(
                settlement.plugin_state,
                Some(serde_json::json!({ "cursor": 9 }))
            );
        });
    }
}
