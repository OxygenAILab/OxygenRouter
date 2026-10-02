//! The submit-stream reader: SSE framing and the plugin's per-event parser.
//!
//! A plugin that declares `submitResponseTypes: ["sse"]` answers a submission
//! with an event stream rather than a JSON body. The host owns the framing and
//! the bounded state; the plugin's `parseSubmitEvent` -- or, with the
//! `submit-sse-delta@1` capability, `parseSubmitEventDelta` -- interprets one
//! event at a time and says when the stream is done
//! (`relay/channel/task/jsplugin/submit_stream.go:21`).
//!
//! No response byte reaches the client from here: the reader's output is the
//! accumulated value, which is exactly what `parseSubmitResponse` then sees as
//! its `body`, the same shape a JSON submission would have handed it.
//!
//! One deliberate difference from the reference: this host's task transport
//! buffers the response before the reader runs, so there is no per-line idle
//! timer to reset (`submit_stream.go:28-34`); a stalled read is bounded by the
//! transport's overall request timeout instead. Framing, limits and the
//! per-event hook contract are unchanged.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

use crate::{PluginHost, HOOK_PARSE_SUBMIT_EVENT, HOOK_PARSE_SUBMIT_EVENT_DELTA};

/// The ceiling on one submit SSE frame and on the accumulated submit result,
/// the reference's shared `maxTaskPluginPersistedJSONBytes`
/// (`relay/channel/task/jsplugin/adaptor.go:81`).
pub const MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES: usize = 1 << 20;
/// The delta capability's control state is a snapshot passed back to JS on the
/// next event, so it has a smaller ceiling of its own.
const MAX_CONTROL_STATE_BYTES: usize = 64 << 10;
const MAX_JSON_DEPTH: usize = 32;
const MAX_JSON_NODES: usize = 32_768;
const MAX_JSON_CHANGES: usize = 256;

/// Read a submission's SSE body through the plugin's per-event parser, and
/// answer with the accumulated result the way a JSON body would have answered.
///
/// `delta` selects the hook the plugin declared it would provide: with the
/// `submit-sse-delta@1` capability the plugin sends changes against a state the
/// host accumulates; without it, every event returns the whole next state.
pub async fn read_submit_events(
    host: &PluginHost,
    plugin_key: &str,
    delta: bool,
    request_context: &Value,
    body: &[u8],
    timeout: Duration,
) -> Result<Value, String> {
    let hook = if delta {
        HOOK_PARSE_SUBMIT_EVENT_DELTA
    } else {
        HOOK_PARSE_SUBMIT_EVENT
    };
    let mut accumulated = delta.then(|| JsonState::new(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES));
    let mut state = Value::Null;
    let mut last_id = String::new();
    let mut data: Vec<String> = Vec::new();
    let mut event_name = String::new();
    let mut frame_bytes = 0usize;
    let mut first_line = true;

    // The line scanner is the reference's, including its edge cases: a trailing
    // newline does not manufacture an empty line, so an event that never got
    // its blank line stays unterminated and the stream fails at EOF rather than
    // flushing a half frame (`bufio.Scanner` semantics).
    let mut rest = body;
    while !rest.is_empty() {
        let (raw_line, next) = match rest.iter().position(|byte| *byte == b'\n') {
            Some(position) => (&rest[..position], Some(&rest[position + 1..])),
            None => (rest, None),
        };
        let mut line_bytes = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        if first_line {
            if let Some(stripped) = line_bytes.strip_prefix(b"\xEF\xBB\xBF") {
                line_bytes = stripped;
            }
            first_line = false;
        }
        frame_bytes += line_bytes.len() + 1;
        if frame_bytes > MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES {
            return Err("task submit SSE event exceeds size limit".to_string());
        }
        let line = String::from_utf8_lossy(line_bytes);
        if !line.is_empty() {
            let (field, value) = match line.find(':') {
                Some(position) => (&line[..position], &line[position + 1..]),
                None => (line.as_ref(), ""),
            };
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "data" => data.push(value.to_string()),
                "event" => event_name = value.to_string(),
                "id" => {
                    // A NUL cannot be part of a header value; the reference
                    // keeps the previous id instead of storing it.
                    if !value.contains('\0') {
                        last_id = value.to_string();
                    }
                }
                _ => {}
            }
        } else {
            frame_bytes = 0;
            if data.is_empty() {
                event_name.clear();
            } else {
                if event_name.is_empty() {
                    event_name = "message".to_string();
                }
                let event = serde_json::json!({
                    "event": event_name,
                    "id": last_id,
                    "data": data.join("\n"),
                });
                let result = host
                    .call_hook_args(
                        plugin_key,
                        hook,
                        &[request_context.clone(), event, state.clone()],
                        timeout,
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                let Value::Object(object) = result else {
                    return Err(format!("{hook} must return an object"));
                };
                let Some(next_state) = object.get("state") else {
                    return Err(format!("{hook} must return state and a boolean done"));
                };
                let Some(done) = object.get("done").and_then(Value::as_bool) else {
                    return Err(format!("{hook} must return state and a boolean done"));
                };
                let encoded =
                    serde_json::to_vec(next_state).map_err(|error| format!("invalid submit stream state: {error}"))?;
                if encoded.len() > MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES {
                    return Err("task submit stream state exceeds size limit".to_string());
                }
                if let Some(accumulated) = &mut accumulated {
                    if object.len() != 3 {
                        return Err(
                            "parseSubmitEventDelta must return only changes, state and done"
                                .to_string(),
                        );
                    }
                    // Control state is separate from the accumulated result and
                    // is the only snapshot passed back to JS on the next event.
                    if encoded.len() > MAX_CONTROL_STATE_BYTES {
                        return Err("task submit stream control state exceeds size limit".to_string());
                    }
                    let changes = object.get("changes").ok_or_else(|| {
                        format!("changes must be an array of at most {MAX_JSON_CHANGES} operations")
                    })?;
                    accumulated
                        .apply(changes)
                        .map_err(|error| format!("invalid submit stream changes: {error}"))?;
                }
                state = next_state.clone();
                if done {
                    return match &accumulated {
                        Some(accumulated) => accumulated.value(),
                        None => Ok(state),
                    };
                }
                data.clear();
                event_name.clear();
            }
        }
        match next {
            Some(next) if !next.is_empty() => rest = next,
            _ => break,
        }
    }
    Err("task submit stream ended before the plugin reported completion".to_string())
}

/// A bounded accumulator for the delta capability's `changes`.
///
/// Ported from `pkg/jsplugin/json_state.go`. Its reason to exist is not
/// convenience: a delta plugin never re-sends the whole result, so the host
/// owns the tree, the budget and the rule that a failed apply poisons the
/// stream -- callers must discard the state rather than continue after an
/// error. `appendText` writes into the string it belongs to, so a document
/// built one chunk at a time stays O(total) instead of O(n^2).
///
/// The byte accounting counts what `serde_json` encodes, which is the same
/// compact encoding this host persists with. (Go's `json.Marshal` additionally
/// escapes `<`, `>` and `&`; that expansion is an artifact of its encoder, not
/// part of the contract, so a payload of those characters is bounded by its
/// real encoded size here.)
#[derive(Debug, Clone)]
pub struct JsonState {
    root: JsonNode,
    limit: usize,
    failed: Option<String>,
}

#[derive(Debug, Clone)]
struct JsonNode {
    kind: NodeKind,
    /// The node's own compact-JSON byte length, tracked so a change can price
    /// itself without re-encoding the tree.
    bytes: usize,
    nodes: usize,
}

#[derive(Debug, Clone)]
enum NodeKind {
    Scalar(Value),
    Object(BTreeMap<String, JsonNode>),
    Array(Vec<JsonNode>),
}

/// The per-construction budget, as the reference spends it
/// (`pkg/jsplugin/json_state.go:49`).
struct Budget {
    bytes: usize,
    nodes: usize,
}

impl Budget {
    fn spend(&mut self, bytes: usize, nodes: usize) -> Result<(), String> {
        if bytes > self.bytes || nodes > self.nodes {
            return Err("JSON state exceeds size or node limit".to_string());
        }
        self.bytes -= bytes;
        self.nodes -= nodes;
        Ok(())
    }
}

impl JsonState {
    /// A state whose root is `null`, matching the reference's four-byte,
    /// one-node root.
    pub fn new(limit: usize) -> Self {
        JsonState {
            root: JsonNode {
                kind: NodeKind::Scalar(Value::Null),
                bytes: 4,
                nodes: 1,
            },
            limit: limit.min(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES),
            failed: None,
        }
    }

    /// The final ordinary JSON tree, verified against the tracked size.
    pub fn value(&self) -> Result<Value, String> {
        if let Some(failed) = &self.failed {
            return Err(failed.clone());
        }
        let value = self.root.to_value();
        let encoded = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
        if encoded.len() != self.root.bytes || encoded.len() > self.limit {
            return Err(
                "JSON state encoded size does not match its bounded representation".to_string(),
            );
        }
        Ok(value)
    }

    /// Apply a bounded batch of `set`, `append` and `appendText` operations.
    ///
    /// A failure invalidates the stream: every later call answers with the
    /// same error, and no partially updated result may be published.
    pub fn apply(&mut self, changes: &Value) -> Result<(), String> {
        if let Some(failed) = &self.failed {
            return Err(failed.clone());
        }
        let result = self.apply_inner(changes);
        if let Err(message) = &result {
            self.failed = Some(message.clone());
        }
        result
    }

    fn apply_inner(&mut self, changes: &Value) -> Result<(), String> {
        let Value::Array(items) = changes else {
            return Err(format!(
                "changes must be an array of at most {MAX_JSON_CHANGES} operations"
            ));
        };
        if items.len() > MAX_JSON_CHANGES {
            return Err(format!(
                "changes must be an array of at most {MAX_JSON_CHANGES} operations"
            ));
        }
        for item in items {
            let Value::Object(change) = item else {
                return Err("each change must contain only op, path and value".to_string());
            };
            if change.len() != 3 {
                return Err("each change must contain only op, path and value".to_string());
            }
            let op = change.get("op").and_then(Value::as_str).unwrap_or("");
            if !matches!(op, "set" | "append" | "appendText") {
                return Err(format!("unsupported JSON change operation {op:?}"));
            }
            let Some(Value::Array(path)) = change.get("path") else {
                return Err(format!(
                    "change requires a JSON value and a path of at most {MAX_JSON_DEPTH} segments"
                ));
            };
            if path.len() > MAX_JSON_DEPTH {
                return Err(format!(
                    "change requires a JSON value and a path of at most {MAX_JSON_DEPTH} segments"
                ));
            }
            let Some(value) = change.get("value") else {
                return Err(format!(
                    "change requires a JSON value and a path of at most {MAX_JSON_DEPTH} segments"
                ));
            };
            let path: Vec<&Value> = path.iter().collect();
            let root_bytes = self.root.bytes;
            let root_nodes = self.root.nodes;
            match op {
                "set" => set_at(
                    &mut self.root,
                    &path,
                    value,
                    0,
                    self.limit,
                    root_bytes,
                    root_nodes,
                )?,
                "append" => append_at(
                    &mut self.root,
                    &path,
                    value,
                    0,
                    self.limit,
                    root_bytes,
                    root_nodes,
                )?,
                "appendText" => append_text_at(
                    &mut self.root,
                    &path,
                    value,
                    self.limit,
                    root_bytes,
                )?,
                _ => unreachable!(),
            }
        }
        Ok(())
    }
}

/// `set`: replace the node at `path`, creating a missing object key when the
/// path names one directly.
fn set_at(
    node: &mut JsonNode,
    path: &[&Value],
    value: &Value,
    depth: usize,
    limit: usize,
    root_bytes: usize,
    root_nodes: usize,
) -> Result<(), String> {
    if path.is_empty() {
        let next = build_json_node(value, depth, &mut Budget { bytes: limit, nodes: MAX_JSON_NODES })?;
        check_totals(
            root_bytes,
            root_nodes,
            next.bytes as i64 - node.bytes as i64,
            next.nodes as i64 - node.nodes as i64,
            limit,
        )?;
        *node = next;
        return Ok(());
    }
    let (head, rest) = path.split_first().expect("non-empty");
    let is_last = rest.is_empty();
    match &mut node.kind {
        NodeKind::Object(fields) => {
            let Some(key) = head.as_str() else {
                return Err("JSON object paths require valid string keys".to_string());
            };
            if key.len() > limit {
                return Err("JSON object paths require valid string keys".to_string());
            }
            if is_last {
                let old = fields.get(key).map(|child| (child.bytes, child.nodes));
                let next =
                    build_json_node(value, depth + 1, &mut Budget { bytes: limit, nodes: MAX_JSON_NODES })?;
                let key_cost = if old.is_none() {
                    serde_json::to_vec(key).map_err(|error| error.to_string())?.len()
                        + 1
                        + usize::from(!fields.is_empty())
                } else {
                    0
                };
                let (old_bytes, old_nodes) = old.unwrap_or((0, 0));
                let delta_bytes = key_cost as i64 + next.bytes as i64 - old_bytes as i64;
                let delta_nodes = next.nodes as i64 - old_nodes as i64;
                check_totals(root_bytes, root_nodes, delta_bytes, delta_nodes, limit)?;
                fields.insert(key.to_string(), next);
                node.bytes = (node.bytes as i64 + delta_bytes) as usize;
                node.nodes = (node.nodes as i64 + delta_nodes) as usize;
                return Ok(());
            }
            let Some(child) = fields.get_mut(key) else {
                return Err("JSON change target does not exist".to_string());
            };
            set_at(child, rest, value, depth + 1, limit, root_bytes, root_nodes)?;
            Ok(())
        }
        NodeKind::Array(items) => {
            let index = array_index(head, items.len())?;
            let before = (items[index].bytes, items[index].nodes);
            if is_last {
                let next =
                    build_json_node(value, depth + 1, &mut Budget { bytes: limit, nodes: MAX_JSON_NODES })?;
                let delta_bytes = next.bytes as i64 - before.0 as i64;
                let delta_nodes = next.nodes as i64 - before.1 as i64;
                check_totals(root_bytes, root_nodes, delta_bytes, delta_nodes, limit)?;
                items[index] = next;
                node.bytes = (node.bytes as i64 + delta_bytes) as usize;
                node.nodes = (node.nodes as i64 + delta_nodes) as usize;
                return Ok(());
            }
            set_at(&mut items[index], rest, value, depth + 1, limit, root_bytes, root_nodes)?;
            let delta_bytes = items[index].bytes as i64 - before.0 as i64;
            let delta_nodes = items[index].nodes as i64 - before.1 as i64;
            node.bytes = (node.bytes as i64 + delta_bytes) as usize;
            node.nodes = (node.nodes as i64 + delta_nodes) as usize;
            Ok(())
        }
        NodeKind::Scalar(_) => Err("JSON change path traverses a scalar".to_string()),
    }
}

/// `append`: push one value onto the array `path` names.
fn append_at(
    node: &mut JsonNode,
    path: &[&Value],
    value: &Value,
    depth: usize,
    limit: usize,
    root_bytes: usize,
    root_nodes: usize,
) -> Result<(), String> {
    if path.is_empty() {
        let NodeKind::Array(items) = &mut node.kind else {
            return Err("append requires an array target".to_string());
        };
        let next =
            build_json_node(value, depth + 1, &mut Budget { bytes: limit, nodes: MAX_JSON_NODES })?;
        let comma = usize::from(!items.is_empty());
        let delta_bytes = (comma + next.bytes) as i64;
        let delta_nodes = next.nodes as i64;
        check_totals(root_bytes, root_nodes, delta_bytes, delta_nodes, limit)?;
        node.bytes = (node.bytes as i64 + delta_bytes) as usize;
        node.nodes = (node.nodes as i64 + delta_nodes) as usize;
        items.push(next);
        return Ok(());
    }
    let (head, rest) = path.split_first().expect("non-empty");
    match &mut node.kind {
        NodeKind::Object(fields) => {
            let Some(key) = head.as_str() else {
                return Err("JSON object paths require valid string keys".to_string());
            };
            let Some(child) = fields.get_mut(key) else {
                return Err("JSON change target does not exist".to_string());
            };
            let before = (child.bytes, child.nodes);
            append_at(child, rest, value, depth + 1, limit, root_bytes, root_nodes)?;
            let delta_bytes = child.bytes as i64 - before.0 as i64;
            let delta_nodes = child.nodes as i64 - before.1 as i64;
            node.bytes = (node.bytes as i64 + delta_bytes) as usize;
            node.nodes = (node.nodes as i64 + delta_nodes) as usize;
            Ok(())
        }
        NodeKind::Array(items) => {
            let index = array_index(head, items.len())?;
            let child = &mut items[index];
            let before = (child.bytes, child.nodes);
            append_at(
                child,
                rest,
                value,
                depth + 1,
                limit,
                root_bytes,
                root_nodes,
            )?;
            let delta_bytes = items[index].bytes as i64 - before.0 as i64;
            let delta_nodes = items[index].nodes as i64 - before.1 as i64;
            node.bytes = (node.bytes as i64 + delta_bytes) as usize;
            node.nodes = (node.nodes as i64 + delta_nodes) as usize;
            Ok(())
        }
        NodeKind::Scalar(_) => Err("JSON change path traverses a scalar".to_string()),
    }
}

/// `appendText`: append to the string `path` names, in place.
fn append_text_at(
    node: &mut JsonNode,
    path: &[&Value],
    value: &Value,
    limit: usize,
    root_bytes: usize,
) -> Result<(), String> {
    if path.is_empty() {
        let NodeKind::Scalar(Value::String(previous)) = &mut node.kind else {
            return Err("appendText requires strings within the state size limit".to_string());
        };
        let Value::String(text) = value else {
            return Err("appendText requires strings within the state size limit".to_string());
        };
        if text.len() > limit.saturating_sub(root_bytes) {
            return Err("appendText requires strings within the state size limit".to_string());
        }
        let encoded = serde_json::to_vec(text).map_err(|error| error.to_string())?;
        let added = encoded.len() - 2;
        if added > limit.saturating_sub(root_bytes) {
            return Err("JSON state exceeds size limit".to_string());
        }
        if !text.is_empty() {
            previous.push_str(text);
        }
        node.bytes += added;
        return Ok(());
    }
    let (head, rest) = path.split_first().expect("non-empty");
    match &mut node.kind {
        NodeKind::Object(fields) => {
            let Some(key) = head.as_str() else {
                return Err("JSON object paths require valid string keys".to_string());
            };
            let Some(child) = fields.get_mut(key) else {
                return Err("JSON change target does not exist".to_string());
            };
            let before = child.bytes;
            append_text_at(child, rest, value, limit, root_bytes)?;
            node.bytes += child.bytes - before;
            Ok(())
        }
        NodeKind::Array(items) => {
            let index = array_index(head, items.len())?;
            let child = &mut items[index];
            let before = child.bytes;
            append_text_at(child, rest, value, limit, root_bytes)?;
            node.bytes += items[index].bytes - before;
            Ok(())
        }
        NodeKind::Scalar(_) => Err("JSON change path traverses a scalar".to_string()),
    }
}

/// Whether the accumulated tree would still fit after a change.
fn check_totals(
    root_bytes: usize,
    root_nodes: usize,
    delta_bytes: i64,
    delta_nodes: i64,
    limit: usize,
) -> Result<(), String> {
    let bytes = root_bytes as i64 + delta_bytes;
    let nodes = root_nodes as i64 + delta_nodes;
    if bytes > limit as i64 || nodes > MAX_JSON_NODES as i64 || bytes < 0 || nodes < 0 {
        return Err("JSON state exceeds size or node limit".to_string());
    }
    Ok(())
}

fn array_index(segment: &Value, length: usize) -> Result<usize, String> {
    let number = segment
        .as_f64()
        .ok_or_else(|| "JSON array paths require integer indices".to_string())?;
    if number < 0.0 || number >= length as f64 || number.trunc() != number {
        return Err("JSON array index is out of range".to_string());
    }
    Ok(number as usize)
}

/// Build one bounded node from an ordinary JSON value.
fn build_json_node(value: &Value, depth: usize, budget: &mut Budget) -> Result<JsonNode, String> {
    if depth > MAX_JSON_DEPTH {
        return Err("JSON state exceeds depth limit".to_string());
    }
    budget.spend(0, 1)?;
    match value {
        Value::Object(fields) => {
            if fields.len() > budget.nodes {
                return Err("JSON object exceeds node limit".to_string());
            }
            budget.spend(2, 0)?;
            let mut children = BTreeMap::new();
            let mut bytes = 2usize;
            let mut nodes = 1usize;
            for (index, (key, child)) in fields.iter().enumerate() {
                if key.len() > budget.bytes {
                    return Err("invalid or oversized JSON object key".to_string());
                }
                let encoded = serde_json::to_vec(key).map_err(|error| error.to_string())?;
                let cost = encoded.len() + 1 + usize::from(index != 0);
                budget.spend(cost, 0)?;
                let child = build_json_node(child, depth + 1, budget)?;
                bytes += cost + child.bytes;
                nodes += child.nodes;
                children.insert(key.clone(), child);
            }
            Ok(JsonNode {
                kind: NodeKind::Object(children),
                bytes,
                nodes,
            })
        }
        Value::Array(items) => {
            if items.len() > budget.nodes {
                return Err("JSON array exceeds node limit".to_string());
            }
            let separators = items.len().saturating_sub(1);
            budget.spend(2 + separators, 0)?;
            let mut children = Vec::with_capacity(items.len());
            let mut bytes = 2 + separators;
            let mut nodes = 1usize;
            for child in items {
                let child = build_json_node(child, depth + 1, budget)?;
                bytes += child.bytes;
                nodes += child.nodes;
                children.push(child);
            }
            Ok(JsonNode {
                kind: NodeKind::Array(children),
                bytes,
                nodes,
            })
        }
        Value::String(text) => {
            if text.len() > budget.bytes {
                return Err("JSON string exceeds size limit".to_string());
            }
            let encoded = serde_json::to_vec(text).map_err(|error| error.to_string())?;
            budget.spend(encoded.len(), 0)?;
            Ok(JsonNode {
                kind: NodeKind::Scalar(Value::String(text.clone())),
                bytes: encoded.len(),
                nodes: 1,
            })
        }
        Value::Null => {
            budget.spend(4, 0)?;
            Ok(JsonNode {
                kind: NodeKind::Scalar(Value::Null),
                bytes: 4,
                nodes: 1,
            })
        }
        Value::Bool(value) => {
            let bytes = if *value { 4 } else { 5 };
            budget.spend(bytes, 0)?;
            Ok(JsonNode {
                kind: NodeKind::Scalar(Value::Bool(*value)),
                bytes,
                nodes: 1,
            })
        }
        Value::Number(number) => {
            let encoded =
                serde_json::to_vec(&Value::Number(number.clone())).map_err(|error| error.to_string())?;
            budget.spend(encoded.len(), 0)?;
            Ok(JsonNode {
                kind: NodeKind::Scalar(Value::Number(number.clone())),
                bytes: encoded.len(),
                nodes: 1,
            })
        }
    }
}

impl JsonNode {
    fn to_value(&self) -> Value {
        match &self.kind {
            NodeKind::Scalar(value) => value.clone(),
            NodeKind::Object(fields) => Value::Object(
                fields
                    .iter()
                    .map(|(key, child)| (key.clone(), child.to_value()))
                    .collect(),
            ),
            NodeKind::Array(items) => {
                Value::Array(items.iter().map(|child| child.to_value()).collect())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(state: &mut JsonState, changes: Value) -> Result<(), String> {
        state.apply(&changes)
    }

    /// The three operations build the tree the reference's delta plugins
    /// build, and `value()` cross-checks the incremental byte accounting
    /// against a fresh encoding of the result.
    #[test]
    fn set_append_and_append_text_build_the_accumulated_tree() {
        let mut state = JsonState::new(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES);
        apply(
            &mut state,
            serde_json::json!([{ "op": "set", "path": [], "value": { "document": "hello", "units": 2 } }]),
        )
        .expect("set");
        apply(
            &mut state,
            serde_json::json!([
                { "op": "appendText", "path": ["document"], "value": " world" },
                { "op": "set", "path": ["units"], "value": 0 },
                { "op": "set", "path": ["tags"], "value": [] },
                { "op": "append", "path": ["tags"], "value": "a" },
                { "op": "append", "path": ["tags"], "value": "b" }
            ]),
        )
        .expect("changes");
        assert_eq!(
            state.value().expect("value"),
            serde_json::json!({ "document": "hello world", "units": 0, "tags": ["a", "b"] })
        );
    }

    /// Floats, escaped strings and nested arrays all have to keep the tracked
    /// size equal to what this host's encoder produces, or `value()` refuses.
    #[test]
    fn accounting_matches_the_encoder_for_awkward_scalars() {
        let mut state = JsonState::new(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES);
        apply(
            &mut state,
            serde_json::json!([{ "op": "set", "path": [], "value": {
                "float": 1.0,
                "big": 9007199254740993i64,
                "escaped": "quote\" newline\n tab\t",
                "nested": [[1, 2], [], {}]
            }}]),
        )
        .expect("set");
        assert_eq!(
            state.value().expect("value"),
            serde_json::json!({
                "float": 1.0,
                "big": 9007199254740993i64,
                "escaped": "quote\" newline\n tab\t",
                "nested": [[1, 2], [], {}]
            })
        );
    }

    /// A failed apply poisons the stream: the reference never lets a caller
    /// continue from a partially updated tree.
    #[test]
    fn a_failed_change_invalidates_the_state() {
        let mut state = JsonState::new(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES);
        apply(
            &mut state,
            serde_json::json!([{ "op": "set", "path": [], "value": { "document": "x" } }]),
        )
        .expect("set");
        let error = apply(
            &mut state,
            serde_json::json!([{ "op": "appendText", "path": ["missing"], "value": "y" }]),
        )
        .expect_err("missing target");
        assert!(error.contains("does not exist"), "{error}");
        // Even a well-formed change cannot rescue the stream afterwards.
        let error = apply(
            &mut state,
            serde_json::json!([{ "op": "set", "path": [], "value": {} }]),
        )
        .expect_err("poisoned");
        assert!(error.contains("does not exist"), "{error}");
        assert!(state.value().is_err());
    }

    /// The state budget refuses an oversized value at the operation that
    /// introduced it, even when a later operation would have replaced it.
    #[test]
    fn an_oversized_intermediate_value_is_refused() {
        let mut state = JsonState::new(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES);
        let too_big = "x".repeat(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES);
        let error = apply(
            &mut state,
            serde_json::json!([
                { "op": "set", "path": [], "value": { "document": too_big } },
                { "op": "set", "path": [], "value": { "document": "" } }
            ]),
        )
        .expect_err("oversized");
        assert!(error.contains("size"), "{error}");
    }

    /// A string that just fits is accepted, and one byte more is not: the
    /// boundary is the encoded size, not the raw text length.
    #[test]
    fn the_string_budget_is_the_encoded_size() {
        let mut state = JsonState::new(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES);
        let fits = "x".repeat(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES - 2);
        apply(
            &mut state,
            serde_json::json!([{ "op": "set", "path": [], "value": fits }]),
        )
        .expect("exactly at the limit");
        assert_eq!(state.value().expect("value").as_str().unwrap().len(), MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES - 2);

        let mut state = JsonState::new(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES);
        let too_large = "x".repeat(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES - 1);
        let error = apply(
            &mut state,
            serde_json::json!([{ "op": "set", "path": [], "value": too_large }]),
        )
        .expect_err("one byte over");
        assert!(error.contains("size"), "{error}");
    }

    /// Only a string can be appended to as text, and only an array can grow.
    #[test]
    fn an_operation_refuses_the_wrong_target_kind() {
        let mut state = JsonState::new(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES);
        apply(
            &mut state,
            serde_json::json!([{ "op": "set", "path": [], "value": { "n": 1, "s": "a" } }]),
        )
        .expect("set");
        let error = apply(
            &mut state,
            serde_json::json!([{ "op": "appendText", "path": ["n"], "value": "b" }]),
        )
        .expect_err("number is not text");
        assert!(error.contains("appendText"), "{error}");

        let mut state = JsonState::new(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES);
        apply(
            &mut state,
            serde_json::json!([{ "op": "set", "path": [], "value": { "s": "a" } }]),
        )
        .expect("set");
        let error = apply(
            &mut state,
            serde_json::json!([{ "op": "append", "path": ["s"], "value": 1 }]),
        )
        .expect_err("string is not an array");
        assert!(error.contains("array target"), "{error}");
    }

    /// The change batch and the object it contains are both bounded.
    #[test]
    fn malformed_changes_are_refused_by_shape() {
        let mut state = JsonState::new(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES);
        let error = apply(&mut state, serde_json::json!({})).expect_err("not an array");
        assert!(error.contains("array of at most 256"), "{error}");

        let mut state = JsonState::new(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES);
        let error = apply(
            &mut state,
            serde_json::json!([{ "op": "set", "path": [], "value": 1, "extra": true }]),
        )
        .expect_err("extra field");
        assert!(error.contains("only op, path and value"), "{error}");

        let mut state = JsonState::new(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES);
        let error = apply(
            &mut state,
            serde_json::json!([{ "op": "move", "path": [], "value": 1 }]),
        )
        .expect_err("unknown op");
        assert!(error.contains("unsupported JSON change operation"), "{error}");

        let mut state = JsonState::new(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES);
        let too_many: Vec<Value> = (0..=MAX_JSON_CHANGES)
            .map(|_| serde_json::json!({ "op": "set", "path": [], "value": 1 }))
            .collect();
        let error = apply(&mut state, Value::Array(too_many)).expect_err("too many");
        assert!(error.contains("array of at most 256"), "{error}");
    }

    /// Depth and node counts are bounded while a value is built.
    #[test]
    fn the_shape_budget_is_enforced_while_building() {
        let mut deep = Value::Null;
        for _ in 0..(MAX_JSON_DEPTH + 1) {
            deep = Value::Array(vec![deep]);
        }
        let mut state = JsonState::new(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES);
        let error = apply(
            &mut state,
            serde_json::json!([{ "op": "set", "path": [], "value": deep }]),
        )
        .expect_err("too deep");
        assert!(error.contains("depth limit"), "{error}");

        let wide: Vec<Value> = (0..=MAX_JSON_NODES).map(|_| Value::Null).collect();
        let mut state = JsonState::new(MAX_TASK_PLUGIN_PERSISTED_JSON_BYTES);
        let error = apply(
            &mut state,
            serde_json::json!([{ "op": "set", "path": [], "value": wide }]),
        )
        .expect_err("too many nodes");
        assert!(error.contains("node limit"), "{error}");
    }
}
