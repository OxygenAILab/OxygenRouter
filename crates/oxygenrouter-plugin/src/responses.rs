//! The semantic event contract, and the Responses state machine it drives.
//!
//! A plugin never writes the Responses wire format. It emits *semantic* events --
//! progress, output, error -- and the host owns everything a client can be misled
//! by: the response id, the model, the sequence numbers, the status, the usage
//! figures and the event type names (`relay/plugin_protocol.go:441,968`). That is
//! why the contract is this small: a plugin cannot smuggle a protocol-owned field
//! into a response, because it never sees one.
//!
//! Every bound here is the reference's (`DefaultPluginProtocolLimits`,
//! `relay/plugin_protocol.go`), and every refusal is one of its own: an unknown
//! field on the result, an unknown field on an event, an event type it does not
//! define, a progress outside 0..100, an output with no data.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use serde_json::{json, Value};

/// The bounds one event result is held to
/// (`relay/plugin_protocol.go`'s `DefaultPluginProtocolLimits`).
#[derive(Debug, Clone, Copy)]
pub struct EventLimits {
    pub max_events_per_tick: usize,
    pub max_events_bytes: usize,
    pub max_event_bytes: usize,
    pub max_event_depth: usize,
    pub max_total_output_bytes: usize,
    pub max_outputs: usize,
    pub max_message_bytes: usize,
    pub max_metadata_value_bytes: usize,
    pub max_code_bytes: usize,
}

impl Default for EventLimits {
    fn default() -> Self {
        EventLimits {
            max_events_per_tick: 16,
            max_events_bytes: 64 << 10,
            max_event_bytes: 32 << 10,
            max_event_depth: 16,
            max_total_output_bytes: 1 << 20,
            max_outputs: 64,
            max_message_bytes: 4 << 10,
            max_metadata_value_bytes: 512,
            max_code_bytes: 128,
        }
    }
}

/// One semantic event a plugin emitted.
#[derive(Debug, Clone, PartialEq)]
pub enum SemanticEvent {
    /// How far along the task is, and optionally what it is doing.
    Progress {
        progress: Option<f64>,
        message: Option<String>,
    },
    /// A piece of the answer.
    Output { text: String },
    /// The task failed, with a message the *host* will replace.
    Error { code: Option<String>, message: String },
}

/// What one tick of `renderEvents` returned.
#[derive(Debug, Clone, PartialEq)]
pub struct EventResult {
    pub events: Vec<SemanticEvent>,
    /// The plugin says it has nothing more to say, whether or not the task has
    /// finished.
    pub done: bool,
}

/// The nesting depth of a JSON value, for the depth bound.
fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(items) => 1 + items.iter().map(json_depth).max().unwrap_or(0),
        Value::Object(map) => 1 + map.values().map(json_depth).max().unwrap_or(0),
        _ => 1,
    }
}

/// Decode what `renderEvents` returned, refusing everything the reference refuses.
pub fn decode_event_result(value: &Value, limits: &EventLimits) -> Result<EventResult, String> {
    let encoded = serde_json::to_string(value)
        .map_err(|error| format!("protocol event result is not JSON-compatible: {error}"))?;
    if encoded.len() > limits.max_events_bytes + (16 << 10) + 4096 {
        return Err("protocol event result is too large".to_string());
    }
    let Some(fields) = value.as_object() else {
        return Err("protocol event result must be an object".to_string());
    };
    for name in fields.keys() {
        match name.as_str() {
            "events" | "state" | "done" => {}
            other => {
                return Err(format!(
                    "protocol event result contains unknown field {other:?}"
                ))
            }
        }
    }

    let Some(raw_events) = fields.get("events").filter(|value| !value.is_null()) else {
        return Err("protocol event result events must be an array".to_string());
    };
    let Some(events) = raw_events.as_array() else {
        return Err("protocol event result events must be an array".to_string());
    };
    if events.len() > limits.max_events_per_tick {
        return Err(format!(
            "protocol events exceed limit of {}",
            limits.max_events_per_tick
        ));
    }

    let Some(done) = fields.get("done").and_then(|value| value.as_bool()) else {
        return Err("protocol event result done must be a boolean".to_string());
    };

    let mut decoded = Vec::with_capacity(events.len());
    for raw in events {
        decoded.push(decode_semantic_event(raw, limits)?);
    }
    Ok(EventResult {
        events: decoded,
        done,
    })
}

fn decode_semantic_event(value: &Value, limits: &EventLimits) -> Result<SemanticEvent, String> {
    let size = serde_json::to_string(value)
        .map(|text| text.len())
        .unwrap_or(0);
    if size > limits.max_event_bytes {
        return Err(format!(
            "protocol event exceeds {} bytes",
            limits.max_event_bytes
        ));
    }
    if json_depth(value) > limits.max_event_depth {
        return Err(format!(
            "protocol event exceeds depth limit of {}",
            limits.max_event_depth
        ));
    }
    let Some(fields) = value.as_object() else {
        return Err("protocol event must be a JSON object".to_string());
    };
    let Some(event_type) = fields.get("type").and_then(|value| value.as_str()) else {
        return Err("protocol event type is required".to_string());
    };

    let allowed = |names: &[&str]| -> Result<(), String> {
        for name in fields.keys() {
            if !names.contains(&name.as_str()) {
                return Err(format!("protocol event contains unknown field {name:?}"));
            }
        }
        Ok(())
    };
    let bounded = |value: &Value,
                   field: &str,
                   max: usize,
                   required: bool|
     -> Result<Option<String>, String> {
        match value {
            Value::Null => {
                if required {
                    Err(format!("{field} must be a string"))
                } else {
                    Ok(None)
                }
            }
            Value::String(text) => {
                if text.len() > max {
                    Err(format!("{field} exceeds {max} bytes"))
                } else {
                    Ok(Some(text.clone()))
                }
            }
            _ => Err(format!("{field} must be a string")),
        }
    };

    match event_type {
        "progress" => {
            allowed(&["type", "progress", "message"])?;
            let progress = match fields.get("progress") {
                None => None,
                Some(value) => {
                    let number = value
                        .as_f64()
                        .filter(|number| number.is_finite() && (0.0..=100.0).contains(number))
                        .ok_or_else(|| {
                            "progress event progress must be between 0 and 100".to_string()
                        })?;
                    Some(number)
                }
            };
            let message = match fields.get("message") {
                None => None,
                Some(value) => bounded(
                    value,
                    "progress event message",
                    limits.max_metadata_value_bytes,
                    false,
                )?,
            };
            Ok(SemanticEvent::Progress { progress, message })
        }
        "output" => {
            allowed(&["type", "data"])?;
            let Some(data) = fields.get("data") else {
                return Err("output event data is required".to_string());
            };
            Ok(SemanticEvent::Output {
                text: output_text(data)?,
            })
        }
        "error" => {
            allowed(&["type", "code", "message"])?;
            let Some(value) = fields.get("message") else {
                return Err("error event message is required".to_string());
            };
            let message = bounded(value, "error event message", limits.max_message_bytes, true)?
                .unwrap_or_default();
            let code = match fields.get("code") {
                None => None,
                Some(value) => bounded(value, "error event code", limits.max_code_bytes, false)?,
            };
            Ok(SemanticEvent::Error { code, message })
        }
        other => Err(format!("unsupported protocol event type {other:?}")),
    }
}

/// The text of an `output` event.
///
/// The reference accepts the shapes a plugin naturally produces: a bare string, or
/// an object carrying the text under one of the replies' own field names
/// (`relay/plugin_protocol.go`'s `pluginOutputText`).
fn output_text(data: &Value) -> Result<String, String> {
    match data {
        Value::String(text) => Ok(text.clone()),
        Value::Object(map) => {
            for key in ["text", "output_text", "content", "delta", "message"] {
                if let Some(Value::String(text)) = map.get(key) {
                    return Ok(text.clone());
                }
            }
            Err("output event data must carry text".to_string())
        }
        _ => Err("output event data must be a string or an object with text".to_string()),
    }
}

/// The Responses statuses the host writes
/// (`relay/plugin_protocol.go`'s `pluginResponseStatus*`).
pub const RESPONSE_IN_PROGRESS: &str = "in_progress";
pub const RESPONSE_COMPLETED: &str = "completed";
pub const RESPONSE_INCOMPLETE: &str = "incomplete";
pub const RESPONSE_FAILED: &str = "failed";

/// One event on a Responses stream, as the host will write it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct StreamEvent {
    pub event_type: String,
    pub payload: Value,
}

/// Builds a Responses stream from semantic events.
///
/// Ported from `PluginResponsesMachine` (`relay/plugin_protocol.go:362`). It owns
/// every field a client can be misled by -- the ids, the model, the sequence
/// numbers, the status, the usage -- and it refuses to act twice: a stream that has
/// already ended cannot be appended to, and `created` cannot be re-emitted.
#[derive(Debug, Clone)]
pub struct ResponsesMachine {
    task_id: String,
    response_id: String,
    model: String,
    created_at: i64,
    limits: EventLimits,
    next_sequence: u64,
    started: bool,
    terminal: bool,
    status: String,
    metadata: std::collections::BTreeMap<String, String>,
    outputs: Vec<Value>,
    total_output_bytes: usize,
}

impl ResponsesMachine {
    pub fn new(task_id: &str, model: &str, created_at: i64, limits: EventLimits) -> Self {
        let trimmed = task_id.trim();
        let response_id = format!("resp_{}", trimmed.strip_prefix("task_").unwrap_or(trimmed));
        let mut metadata = std::collections::BTreeMap::new();
        metadata.insert("task_id".to_string(), trimmed.to_string());
        metadata.insert("task_status".to_string(), "queued".to_string());
        metadata.insert(
            "retrieval_path".to_string(),
            format!("/v1/responses/{response_id}"),
        );
        ResponsesMachine {
            task_id: trimmed.to_string(),
            response_id,
            model: model.to_string(),
            created_at,
            limits,
            next_sequence: 0,
            started: false,
            terminal: false,
            status: RESPONSE_IN_PROGRESS.to_string(),
            metadata,
            outputs: Vec::new(),
            total_output_bytes: 0,
        }
    }

    /// The id this stream names, which is the id the read surface is addressed by.
    pub fn response_id(&self) -> &str {
        &self.response_id
    }

    pub fn is_terminal(&self) -> bool {
        self.terminal
    }

    pub fn metadata(&self) -> &std::collections::BTreeMap<String, String> {
        &self.metadata
    }

    /// The snapshot a client can also fetch by id.
    pub fn snapshot(&self, error: Option<(&str, &str)>) -> Value {
        let mut value = json!({
            "id": self.response_id,
            "object": "response",
            "created_at": self.created_at,
            "model": self.model,
            "status": self.status,
            "output": self.outputs,
            "metadata": self.metadata,
            "usage": {
                "input_tokens": 0,
                "output_tokens": 0,
                "total_tokens": 0,
            },
        });
        if let Some((code, message)) = error {
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "error".to_string(),
                    json!({ "code": code, "message": message }),
                );
            }
        }
        value
    }

    /// `response.created`, which must be the first event on a stream.
    pub fn created(&mut self) -> Result<StreamEvent, String> {
        if self.started {
            return Err("response.created was already emitted".to_string());
        }
        if self.terminal {
            return Err("response is already terminal".to_string());
        }
        self.started = true;
        Ok(self.response_event("response.created", None))
    }

    /// Apply one tick's semantic events, and map them onto host-owned events.
    pub fn apply_tick(
        &mut self,
        result: &EventResult,
        task_status: &str,
    ) -> Result<Vec<StreamEvent>, String> {
        if !self.started {
            return Err("response.created must be emitted before applying events".to_string());
        }
        if self.terminal {
            return Err("response is already terminal".to_string());
        }
        // A failed task ends the stream before anything else is considered, so a
        // plugin cannot report progress for work the host knows has failed.
        if task_status.eq_ignore_ascii_case("FAILURE") {
            self.metadata
                .insert("task_status".to_string(), "failed".to_string());
            return Ok(vec![self.fail("server_error", "The task failed.")]);
        }

        // The bounds are checked before any event is emitted, so a tick that
        // exceeds one leaves the stream exactly as it was.
        let mut additional_bytes = 0usize;
        let mut additional_outputs = 0usize;
        for event in &result.events {
            match event {
                SemanticEvent::Output { text } => {
                    if text.len() > self.limits.max_event_bytes {
                        return Err(format!(
                            "output event data exceeds {} bytes",
                            self.limits.max_event_bytes
                        ));
                    }
                    additional_bytes += text.len();
                    additional_outputs += 1;
                }
                SemanticEvent::Error { message, .. } => {
                    if message.trim().is_empty() {
                        return Err("error event message is required".to_string());
                    }
                }
                SemanticEvent::Progress { message, .. } => {
                    if let Some(message) = message {
                        if message.len() > self.limits.max_metadata_value_bytes {
                            return Err(format!(
                                "progress event message exceeds {} bytes",
                                self.limits.max_metadata_value_bytes
                            ));
                        }
                    }
                }
            }
        }
        if self.outputs.len() + additional_outputs > self.limits.max_outputs {
            return Err(format!(
                "response outputs exceed limit of {}",
                self.limits.max_outputs
            ));
        }
        if self.total_output_bytes + additional_bytes > self.limits.max_total_output_bytes {
            return Err(format!(
                "response output exceeds cumulative limit of {} bytes",
                self.limits.max_total_output_bytes
            ));
        }

        self.metadata
            .insert("task_status".to_string(), task_status_token(task_status));
        let mut events = Vec::new();
        for event in &result.events {
            match event {
                SemanticEvent::Progress { progress, message } => {
                    if let Some(progress) = progress {
                        self.metadata
                            .insert("task_progress".to_string(), format!("{progress}"));
                    }
                    if let Some(message) = message {
                        self.metadata
                            .insert("task_message".to_string(), message.clone());
                    }
                    // Metadata only, and only until there is real content: the
                    // reference skips this once an output exists, because a
                    // progress snapshot after the answer started is noise.
                    if self.outputs.is_empty() {
                        events.push(self.progress_event());
                    }
                }
                SemanticEvent::Output { text } => {
                    events.extend(self.append_output(text)?);
                }
                SemanticEvent::Error { .. } => {
                    // The plugin's own message never reaches a client: the host
                    // decides what a failure says, so an upstream's internals
                    // cannot leak through a plugin's error event.
                    events.push(self.fail("server_error", "The task failed."));
                    return Ok(events);
                }
            }
        }

        match task_status.trim().to_ascii_uppercase().as_str() {
            "SUCCESS" => events.push(self.complete()),
            "FAILURE" => events.push(self.fail("server_error", "The task failed.")),
            _ => {
                if result.done {
                    events.push(self.incomplete());
                }
            }
        }
        Ok(events)
    }

    /// End an already-started stream because the host could not continue.
    pub fn failure(&mut self, task_status: Option<&str>) -> Result<StreamEvent, String> {
        if !self.started {
            return Err("response.created must be emitted before response.failed".to_string());
        }
        if self.terminal {
            return Err("response is already terminal".to_string());
        }
        if let Some(status) = task_status {
            self.metadata
                .insert("task_status".to_string(), task_status_token(status));
        }
        Ok(self.fail("server_error", "The task could not be observed."))
    }

    /// End a stream whose task outlived its budget.
    pub fn timeout(&mut self, task_status: Option<&str>) -> Result<StreamEvent, String> {
        if !self.started {
            return Err("response.created must be emitted before response.incomplete".to_string());
        }
        if self.terminal {
            return Err("response is already terminal".to_string());
        }
        if let Some(status) = task_status {
            self.metadata
                .insert("task_status".to_string(), task_status_token(status));
        }
        Ok(self.incomplete())
    }

    /// The final non-streaming answer, from a plugin-authored payload.
    ///
    /// The plugin's own identity and lifecycle fields are overwritten, because the
    /// host owns them: a plugin stating a different response id or status would
    /// name something that does not exist.
    pub fn final_response(&mut self, payload: &Value, task_status: &str) -> Result<Value, String> {
        if self.started || self.terminal {
            return Err("response state machine has already started".to_string());
        }
        let mut value = self.snapshot(None);
        let Some(object) = value.as_object_mut() else {
            return Err("response snapshot is not an object".to_string());
        };
        if let Some(authored) = payload.as_object() {
            for (key, item) in authored {
                if matches!(
                    key.as_str(),
                    "id" | "object"
                        | "created_at"
                        | "model"
                        | "status"
                        | "usage"
                        | "metadata"
                        | "error"
                ) {
                    continue;
                }
                object.insert(key.clone(), item.clone());
            }
            if let Some(output) = authored.get("output").and_then(|value| value.as_array()) {
                self.outputs = output.clone();
                object.insert("output".to_string(), Value::Array(self.outputs.clone()));
            }
        }
        self.status = match task_status.trim().to_ascii_uppercase().as_str() {
            "SUCCESS" => RESPONSE_COMPLETED.to_string(),
            "FAILURE" => RESPONSE_FAILED.to_string(),
            _ => RESPONSE_INCOMPLETE.to_string(),
        };
        self.terminal = true;
        object.insert("status".to_string(), json!(self.status));
        Ok(value)
    }

    fn append_output(&mut self, text: &str) -> Result<Vec<StreamEvent>, String> {
        let output_index = self.outputs.len();
        let item_id = format!("msg_{}_{}", self.task_id, output_index);
        let content_id = format!("content_{}_{}", self.task_id, output_index);
        self.outputs.push(json!({
            "id": item_id,
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{
                "id": content_id,
                "type": "output_text",
                "text": text,
                "annotations": [],
                "logprobs": [],
            }],
        }));
        self.total_output_bytes += text.len();

        Ok(vec![
            self.event(json!({
                "type": "response.output_item.added",
                "output_index": output_index,
                "item": {
                    "id": item_id,
                    "type": "message",
                    "status": "in_progress",
                    "role": "assistant",
                    "content": [],
                },
            })),
            self.event(json!({
                "type": "response.content_part.added",
                "output_index": output_index,
                "content_index": 0,
                "item_id": item_id,
                "part": {
                    "id": content_id,
                    "type": "output_text",
                    "text": "",
                    "annotations": [],
                    "logprobs": [],
                },
            })),
            self.event(json!({
                "type": "response.output_text.delta",
                "output_index": output_index,
                "content_index": 0,
                "item_id": item_id,
                "delta": text,
                "logprobs": [],
            })),
            self.event(json!({
                "type": "response.output_text.done",
                "output_index": output_index,
                "content_index": 0,
                "item_id": item_id,
                "text": text,
                "logprobs": [],
            })),
            self.event(json!({
                "type": "response.content_part.done",
                "output_index": output_index,
                "content_index": 0,
                "item_id": item_id,
                "part": {
                    "id": content_id,
                    "type": "output_text",
                    "text": text,
                    "annotations": [],
                    "logprobs": [],
                },
            })),
            self.event(json!({
                "type": "response.output_item.done",
                "output_index": output_index,
                "item": {
                    "id": item_id,
                    "type": "message",
                    "status": "completed",
                    "role": "assistant",
                    "content": [{
                        "id": content_id,
                        "type": "output_text",
                        "text": text,
                        "annotations": [],
                        "logprobs": [],
                    }],
                },
            })),
        ])
    }

    fn complete(&mut self) -> StreamEvent {
        self.status = RESPONSE_COMPLETED.to_string();
        self.metadata
            .insert("task_status".to_string(), "completed".to_string());
        self.terminal = true;
        self.response_event("response.completed", None)
    }

    fn incomplete(&mut self) -> StreamEvent {
        self.status = RESPONSE_INCOMPLETE.to_string();
        self.terminal = true;
        self.response_event("response.incomplete", None)
    }

    fn fail(&mut self, code: &str, message: &str) -> StreamEvent {
        self.status = RESPONSE_FAILED.to_string();
        self.terminal = true;
        let snapshot = self.snapshot(Some((code, message)));
        self.event(json!({ "type": "response.failed", "response": snapshot }))
    }

    fn response_event(&mut self, event_type: &str, error: Option<(&str, &str)>) -> StreamEvent {
        let snapshot = self.snapshot(error);
        self.event(json!({ "type": event_type, "response": snapshot }))
    }

    fn progress_event(&mut self) -> StreamEvent {
        // The output list is deliberately empty: the content is already
        // represented by its own events, and repeating it in every progress
        // snapshot would repeat a megabyte of text per tick.
        let mut snapshot = self.snapshot(None);
        if let Some(object) = snapshot.as_object_mut() {
            object.insert("output".to_string(), Value::Array(Vec::new()));
        }
        self.event(json!({ "type": "response.in_progress", "response": snapshot }))
    }

    fn event(&mut self, mut payload: Value) -> StreamEvent {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        if let Some(object) = payload.as_object_mut() {
            object.insert("sequence_number".to_string(), json!(sequence));
        }
        StreamEvent {
            event_type: payload
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
            payload,
        }
    }
}

/// The metadata token the host records for a durable task status
/// (`relay/plugin_protocol.go`'s `pluginTaskStatus`).
fn task_status_token(status: &str) -> String {
    match status.trim().to_ascii_uppercase().as_str() {
        "SUCCESS" => "completed".to_string(),
        "FAILURE" => "failed".to_string(),
        "NOT_START" | "SUBMITTED" | "QUEUED" => "queued".to_string(),
        _ => "in_progress".to_string(),
    }
}

/// Write one Responses stream event in the SSE shape a client expects.
pub fn sse_frame(event: &StreamEvent) -> String {
    format!(
        "event: {}\ndata: {}\n\n",
        event.event_type,
        serde_json::to_string(&event.payload).unwrap_or_else(|_| "{}".to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> EventLimits {
        EventLimits::default()
    }

    /// The three event types, and the refusals the reference makes about them
    /// (`relay/plugin_protocol.go`'s `decodePluginSemanticEvent`).
    #[test]
    fn a_semantic_event_is_decoded_and_refused_the_reference_way() {
        let value = json!({
            "events": [
                { "type": "progress", "progress": 40, "message": "rendering" },
                { "type": "output", "data": "hello" },
                { "type": "error", "message": "vendor said no", "code": "vendor_error" }
            ],
            "state": null,
            "done": false
        });
        let result = decode_event_result(&value, &limits()).expect("valid");
        assert_eq!(result.events.len(), 3);
        assert_eq!(
            result.events[0],
            SemanticEvent::Progress {
                progress: Some(40.0),
                message: Some("rendering".to_string())
            }
        );
        assert_eq!(
            result.events[1],
            SemanticEvent::Output {
                text: "hello".to_string()
            }
        );
        assert!(!result.done);

        // An output event may carry its text under the field names a plugin
        // naturally reaches for.
        for data in [
            json!({ "text": "a" }),
            json!({ "output_text": "a" }),
            json!({ "content": "a" }),
            json!({ "delta": "a" }),
        ] {
            let value = json!({ "events": [{ "type": "output", "data": data }], "done": true });
            let result = decode_event_result(&value, &limits()).expect("valid");
            assert_eq!(
                result.events[0],
                SemanticEvent::Output {
                    text: "a".to_string()
                },
                "{value}"
            );
        }

        for bad in [
            // Not an object, or with a field the contract does not define.
            json!([{ "type": "output", "data": "a" }]),
            json!({ "events": [], "done": false, "extra": 1 }),
            // No events array, or not one.
            json!({ "done": false }),
            json!({ "events": "nope", "done": false }),
            // No done flag, or not a boolean.
            json!({ "events": [] }),
            json!({ "events": [], "done": "yes" }),
            // An event type the host does not define.
            json!({ "events": [{ "type": "teleport" }], "done": false }),
            // An event with an unknown field: the host owns the wire format, so a
            // plugin cannot smuggle one in.
            json!({ "events": [{ "type": "progress", "progress": 1, "extra": 1 }], "done": false }),
            json!({ "events": [{ "type": "output", "data": "a", "id": "x" }], "done": false }),
            // Progress outside the range, or not a number.
            json!({ "events": [{ "type": "progress", "progress": 101 }], "done": false }),
            json!({ "events": [{ "type": "progress", "progress": -1 }], "done": false }),
            json!({ "events": [{ "type": "progress", "progress": "40" }], "done": false }),
            // An output with nothing to say, and an error with no message.
            json!({ "events": [{ "type": "output" }], "done": false }),
            json!({ "events": [{ "type": "output", "data": {} }], "done": false }),
            json!({ "events": [{ "type": "error" }], "done": false }),
        ] {
            assert!(
                decode_event_result(&bad, &limits()).is_err(),
                "{bad} must be refused"
            );
        }

        // The per-tick event count is bounded.
        let many: Vec<Value> = (0..limits().max_events_per_tick + 1)
            .map(|_| json!({ "type": "progress" }))
            .collect();
        assert!(
            decode_event_result(&json!({ "events": many, "done": false }), &limits()).is_err()
        );
    }

    /// A stream starts with `created`, maps semantic events onto host-owned ones,
    /// and ends exactly once.
    #[test]
    fn a_stream_is_created_ticked_and_ended_once() {
        let mut machine = ResponsesMachine::new("task_abc", "acme-video", 1_700_000_000, limits());
        // The id a stream names is the id the read surface is addressed by.
        assert_eq!(machine.response_id(), "resp_abc");

        // Nothing may be applied before `created`, and `created` may not repeat.
        let tick = EventResult {
            events: vec![],
            done: false,
        };
        assert!(machine.apply_tick(&tick, "IN_PROGRESS").is_err());
        let created = machine.created().expect("created");
        assert_eq!(created.event_type, "response.created");
        assert_eq!(created.payload["sequence_number"], 0);
        assert_eq!(created.payload["response"]["status"], "in_progress");
        assert_eq!(created.payload["response"]["model"], "acme-video");
        assert!(machine.created().is_err(), "created is emitted once");

        // A progress tick before any output reports progress in the metadata.
        let progress = EventResult {
            events: vec![SemanticEvent::Progress {
                progress: Some(40.0),
                message: Some("rendering".to_string()),
            }],
            done: false,
        };
        let events = machine.apply_tick(&progress, "IN_PROGRESS").expect("tick");
        // Progress alone is one event: the tick is not terminal and the plugin did
        // not say it was done, so there is nothing to end the stream with.
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].event_type, "response.in_progress");
        assert_eq!(
            events[0].payload["response"]["metadata"]["task_progress"],
            "40"
        );
        assert_eq!(
            events[0].payload["response"]["metadata"]["task_message"],
            "rendering"
        );
        // The progress snapshot carries no output, so it cannot repeat the answer.
        assert_eq!(events[0].payload["response"]["output"], json!([]));
        // Sequence numbers are the host's, and they never repeat: the next tick
        // continues from where this one left off.
        assert_eq!(events[0].payload["sequence_number"], 1);

        // An output tick emits the six events that describe one message, with
        // host-chosen ids.
        let output = EventResult {
            events: vec![SemanticEvent::Output {
                text: "hello".to_string(),
            }],
            done: false,
        };
        let events = machine.apply_tick(&output, "IN_PROGRESS").expect("tick");
        let types: Vec<&str> = events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect();
        assert_eq!(
            types,
            vec![
                "response.output_item.added",
                "response.content_part.added",
                "response.output_text.delta",
                "response.output_text.done",
                "response.content_part.done",
                "response.output_item.done",
            ]
        );
        assert_eq!(events[0].payload["sequence_number"], 2, "sequence continues");
        assert_eq!(events[2].payload["delta"], "hello");
        assert_eq!(events[0].payload["item"]["id"], "msg_task_abc_0");
        assert_eq!(events[1].payload["part"]["id"], "content_task_abc_0");

        // A success ends the stream, and a second end is refused.
        let events = machine.apply_tick(&tick, "SUCCESS").expect("tick");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "response.completed");
        assert_eq!(events[0].payload["response"]["status"], "completed");
        assert!(machine.is_terminal());
        assert!(machine.apply_tick(&tick, "SUCCESS").is_err());

        // The snapshot is what a client would fetch afterwards, and it carries the
        // output the stream described.
        let snapshot = machine.snapshot(None);
        assert_eq!(snapshot["id"], "resp_abc");
        assert_eq!(snapshot["output"][0]["content"][0]["text"], "hello");
        assert!(snapshot.get("error").is_none());
    }

    /// The endings the host can choose, and the rule that a plugin's own failure
    /// text never reaches a client.
    #[test]
    fn a_stream_ends_by_the_host_rules() {
        // A failed task ends the stream before anything else is considered.
        let mut machine = ResponsesMachine::new("task_1", "m", 0, limits());
        machine.created().expect("created");
        let events = machine
            .apply_tick(
                &EventResult {
                    events: vec![SemanticEvent::Output {
                        text: "should not appear".to_string(),
                    }],
                    done: false,
                },
                "FAILURE",
            )
            .expect("tick");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "response.failed");
        assert_eq!(events[0].payload["response"]["status"], "failed");

        // A plugin error ends the stream with the *host's* wording.
        let mut machine = ResponsesMachine::new("task_2", "m", 0, limits());
        machine.created().expect("created");
        let events = machine
            .apply_tick(
                &EventResult {
                    events: vec![SemanticEvent::Error {
                        code: Some("vendor_secret".to_string()),
                        message: "internal vendor stack trace".to_string(),
                    }],
                    done: false,
                },
                "IN_PROGRESS",
            )
            .expect("tick");
        let rendered = serde_json::to_string(&events).expect("json");
        assert!(!rendered.contains("vendor_secret"), "{rendered}");
        assert!(!rendered.contains("stack trace"), "{rendered}");
        assert_eq!(events.last().expect("event").event_type, "response.failed");

        // `done` without a terminal task ends as incomplete rather than complete.
        let mut machine = ResponsesMachine::new("task_3", "m", 0, limits());
        machine.created().expect("created");
        let events = machine
            .apply_tick(
                &EventResult {
                    events: vec![],
                    done: true,
                },
                "IN_PROGRESS",
            )
            .expect("tick");
        assert_eq!(
            events.last().expect("event").event_type,
            "response.incomplete"
        );

        // The host can end a stream it can no longer observe, and a timeout is its
        // own ending rather than a failure.
        let mut machine = ResponsesMachine::new("task_4", "m", 0, limits());
        machine.created().expect("created");
        assert_eq!(
            machine.failure(Some("FAILURE")).expect("failure").event_type,
            "response.failed"
        );
        let mut machine = ResponsesMachine::new("task_5", "m", 0, limits());
        machine.created().expect("created");
        assert_eq!(
            machine.timeout(Some("IN_PROGRESS")).expect("timeout").event_type,
            "response.incomplete"
        );
        // And neither before `created`.
        let mut fresh = ResponsesMachine::new("task_6", "m", 0, limits());
        assert!(fresh.failure(None).is_err());
        assert!(fresh.timeout(None).is_err());
    }

    /// The bounds are checked before anything is emitted, so a tick that exceeds
    /// one leaves the stream exactly as it was -- a half-applied tick would put a
    /// client's view out of step with the snapshot it can fetch.
    #[test]
    fn a_tick_that_exceeds_a_bound_changes_nothing() {
        let mut machine = ResponsesMachine::new("task_1", "m", 0, limits());
        machine.created().expect("created");
        let before = machine.snapshot(None);

        // One output larger than the per-event bound.
        let oversized = "x".repeat(limits().max_event_bytes + 1);
        let error = machine
            .apply_tick(
                &EventResult {
                    events: vec![SemanticEvent::Output { text: oversized }],
                    done: false,
                },
                "IN_PROGRESS",
            )
            .expect_err("must refuse");
        assert!(error.contains("exceeds"), "{error}");
        assert_eq!(machine.snapshot(None), before, "the stream must be untouched");

        // More outputs than the cumulative bound allows.
        let mut machine = ResponsesMachine::new("task_2", "m", 0, limits());
        machine.created().expect("created");
        for index in 0..limits().max_outputs {
            let events = machine
                .apply_tick(
                    &EventResult {
                        events: vec![SemanticEvent::Output {
                            text: format!("{index}"),
                        }],
                        done: false,
                    },
                    "IN_PROGRESS",
                )
                .expect("output");
            assert_eq!(events.len(), 6);
        }
        let error = machine
            .apply_tick(
                &EventResult {
                    events: vec![SemanticEvent::Output {
                        text: "one too many".to_string(),
                    }],
                    done: false,
                },
                "IN_PROGRESS",
            )
            .expect_err("must refuse");
        assert!(error.contains("outputs exceed"), "{error}");
    }

    /// The final non-streaming answer, with the host's identity fields restored.
    #[test]
    fn a_final_response_is_host_owned_except_for_the_plugins_content() {
        let mut machine = ResponsesMachine::new("task_1", "acme", 123, limits());
        let payload = json!({
            "id": "resp_something_else",
            "model": "lied-about",
            "status": "completed",
            "output": [{ "type": "message", "content": [{ "type": "output_text", "text": "hi" }] }],
            "custom_field": { "kept": true }
        });
        let value = machine.final_response(&payload, "SUCCESS").expect("final");
        // The identity and lifecycle belong to the host.
        assert_eq!(value["id"], "resp_1");
        assert_eq!(value["model"], "acme");
        assert_eq!(value["status"], "completed");
        assert_eq!(value["created_at"], 123);
        // The plugin's content is kept, including fields the next contract adds.
        assert_eq!(value["output"][0]["content"][0]["text"], "hi");
        assert_eq!(value["custom_field"]["kept"], true);

        // A failure settles the status, and the machine will not settle twice.
        let mut machine = ResponsesMachine::new("task_2", "acme", 0, limits());
        let value = machine.final_response(&json!({}), "FAILURE").expect("final");
        assert_eq!(value["status"], "failed");
        assert!(machine.final_response(&json!({}), "SUCCESS").is_err());
    }

    /// The SSE frame a client reads.
    #[test]
    fn an_event_is_framed_as_sse() {
        let mut machine = ResponsesMachine::new("task_1", "m", 0, limits());
        let created = machine.created().expect("created");
        let frame = sse_frame(&created);
        assert!(frame.starts_with("event: response.created\ndata: "), "{frame}");
        assert!(frame.ends_with("\n\n"), "{frame}");
        // The payload is one line, which is what makes it one SSE data field.
        let data = frame
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .expect("data");
        assert!(!data.contains('\n'), "{data}");
        let parsed: Value = serde_json::from_str(data).expect("json");
        assert_eq!(parsed["type"], "response.created");
        assert_eq!(parsed["sequence_number"], 0);
    }
}
