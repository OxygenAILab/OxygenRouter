//! Adaptor-driven dispatch: the bridge from a `Channel` row to a provider that
//! may not speak OpenAI.
//!
//! This is what makes `Channel.provider` real. For each attempt the scheduler
//! picks a channel; this module then
//!
//! 1. resolves the channel's `ApiType` from its `provider` identity,
//! 2. constructs the matching adaptor,
//! 3. translates the inbound OpenAI-shaped body into that provider's wire
//!    format, and
//! 4. translates the response (including SSE) back into the client's format,
//!    extracting billing usage on the way.
//!
//! Providers whose adaptor is a pure pass-through (OpenAI and its many
//! OpenAI-compatible shims) take the same path with `RelayFormat::Raw`, so there
//! is a single code path with no pass-through special case.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::time::Duration;

use reqwest::header::{HeaderName, HeaderValue};
use reqwest::Client;

use oxygenrouter_core::Channel;
use oxygenrouter_relay::{
    channel_type_to_api_type, get_adaptor, provider_str_to_channel_type,
    value::{RelayFormat, RelayInfo},
    UpstreamResponse,
};

use crate::upstream::{ProxyError, ProxyRequest, ProxyResult};
/// What the adaptor did with a request, beyond the response itself.
///
/// Usage deliberately lives on `ProxyResult` (it is the thing billing needs);
/// this carries only the translation metadata, so there is one source of truth.
#[derive(Debug, Clone)]
pub struct RelayOutcome {
    pub result: ProxyResult,
    /// Adaptor name, e.g. `anthropic`.
    pub adaptor: &'static str,
    /// True when the request was translated into a non-OpenAI wire format.
    pub translated: bool,
}

pub struct RelayClient {
    client: Client,
    user_agent: String,
}

impl RelayClient {
    pub fn new(user_agent: String, timeout_ms: u64) -> Self {
        let timeout = if timeout_ms == 0 { 120_000 } else { timeout_ms };
        let client = Client::builder()
            .timeout(Duration::from_millis(timeout))
            .connect_timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(8)
            .build()
            .unwrap_or_default();
        Self { client, user_agent }
    }

    /// Send one attempt through the adaptor for `channel`.
    ///
    /// `req.path` is the client-facing path (e.g. `/v1/chat/completions`); the
    /// adaptor decides how it maps onto the provider's URL.
    pub async fn send(
        &self,
        channel: &Channel,
        model: &str,
        req: &ProxyRequest,
        relay_format: RelayFormat,
    ) -> Result<RelayOutcome, ProxyError> {
        let channel_type = provider_str_to_channel_type(&channel.provider);
        let api_type = channel_type_to_api_type(channel_type);
        let adaptor = get_adaptor(api_type).ok_or_else(|| {
            ProxyError::Internal(format!(
                "channel `{}` uses provider `{}`, which has no adaptor",
                channel.name, channel.provider
            ))
        })?;

        // One line of a multi-key channel; the whole line is also the raw
        // credential, because compound credentials (Bedrock `ak|sk|region`,
        // Vertex `project|location|token`) arrive as a single key line.
        let (key, _index) = match channel.next_enabled_key() {
            Some(pair) => pair,
            None => {
                return Err(ProxyError::Internal(format!(
                    "channel `{}` has no usable API key",
                    channel.name
                )))
            }
        };

        let info = RelayInfo {
            channel_id: channel.id.clone(),
            channel_type,
            api_type,
            base_url: if channel.base_url.trim().is_empty() {
                channel_type.default_base_url().to_string()
            } else {
                channel.base_url.trim().to_string()
            },
            api_key: key.clone(),
            credential_raw: key,
            request_path: req.path.clone(),
            origin_model: model.to_string(),
            upstream_model: model.to_string(),
            relay_format,
            is_stream: req.stream,
            group: channel.group_name.clone(),
            user_agent: self.user_agent.clone(),
        };

        let inbound: serde_json::Value = match &req.body {
            Some(bytes) if !bytes.is_empty() => serde_json::from_slice(bytes)
                .map_err(|e| ProxyError::Internal(format!("invalid request body: {}", e)))?,
            _ => serde_json::json!({}),
        };

        let adapted = adaptor
            .convert_request(&info, &inbound)
            .map_err(|e| map_relay_error(e, model))?;

        // Channel-level parameter overrides win over the client's values, except
        // for `model`, which the adaptor already resolved.
        let mut body_value = adapted.into_raw();
        if let Some(overrides) = channel.override_parameters.as_object() {
            if let Some(obj) = body_value.as_object_mut() {
                for (k, v) in overrides {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
        let body = serde_json::to_vec(&body_value)
            .map_err(|e| ProxyError::Internal(format!("serialize request: {}", e)))?;

        let url = adaptor
            .request_url(&info)
            .map_err(|e| map_relay_error(e, model))?;

        let mut headers = reqwest::header::HeaderMap::new();
        if !self.user_agent.is_empty() {
            if let Ok(value) = HeaderValue::from_str(&self.user_agent) {
                headers.insert(reqwest::header::USER_AGENT, value);
            }
        }
        if req.stream {
            headers.insert(
                reqwest::header::ACCEPT,
                HeaderValue::from_static("text/event-stream"),
            );
        }
        // Forward client headers that are safe to relay, preserving the previous
        // behaviour of the pass-through client.
        for (name, value) in &req.headers {
            let lower = name.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "authorization" | "host" | "content-length" | "cookie" | "accept-encoding"
            ) {
                continue;
            }
            if let (Ok(n), Ok(v)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                headers.insert(n, v);
            }
        }
        // Adaptor headers come last so provider auth cannot be shadowed by a
        // forwarded client header.
        adaptor
            .setup_headers(&mut headers, &info)
            .map_err(|e| map_relay_error(e, model))?;
        adaptor
            .sign_request(&info, &url, &mut headers, &body)
            .map_err(|e| map_relay_error(e, model))?;

        let response = self
            .client
            .request(
                reqwest::Method::from_bytes(adaptor.method().as_bytes())
                    .map_err(|e| ProxyError::Internal(format!("bad method: {}", e)))?,
                &url,
            )
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|e| ProxyError::Network(format!("{}: {}", adaptor.name(), e)))?;

        let status = response.status().as_u16();

        // Streaming bodies are buffered: the adaptors translate whole SSE
        // payloads. True incremental relay needs an adaptor-level streaming API
        // and is tracked separately.
        let raw = response
            .bytes()
            .await
            .map_err(|e| ProxyError::Network(format!("read body: {}", e)))?;

        let upstream = UpstreamResponse {
            status,
            headers: reqwest::header::HeaderMap::new(),
            body: raw,
            is_stream: req.stream,
        };

        if status >= 400 {
            let text = String::from_utf8_lossy(&upstream.body).to_string();
            if is_context_exceeded_error(&text) {
                return Err(ProxyError::ContextExceeded(text));
            }
            return Err(ProxyError::Upstream {
                status: map_status(channel, status),
                body: text,
            });
        }

        let adapted = adaptor
            .convert_response(&info, &upstream)
            .map_err(|e| map_relay_error(e, model))?;

        let mut out_headers: Vec<(String, String)> = Vec::new();
        if let Some(overrides) = channel.response_headers.as_object() {
            for (k, v) in overrides {
                if let Some(v) = v.as_str() {
                    out_headers.push((k.clone(), v.to_string()));
                }
            }
        }

        Ok(RelayOutcome {
            result: ProxyResult {
                status: map_status(channel, status),
                headers: out_headers,
                body: adapted.body,
                model_used: info.upstream_model.clone(),
                channel_id: channel.id.clone(),
                usage: adapted.usage,
                adaptor: adaptor.name().to_string(),
            },
            adaptor: adaptor.name(),
            translated: !matches!(relay_format, RelayFormat::Raw | RelayFormat::OpenAiChat),
        })
    }
}

/// Map a client-facing path to the wire format the client sent.
///
/// This is what tells the adaptor how to shape the response: a client posting to
/// `/v1/messages` gets Anthropic back, not OpenAI.
pub fn relay_format_for_path(path: &str) -> RelayFormat {
    let p = path.split('?').next().unwrap_or(path);
    if p.starts_with("/v1/messages") {
        return RelayFormat::Claude;
    }
    if p.starts_with("/v1beta") {
        return RelayFormat::Gemini;
    }
    if p.starts_with("/v1/responses") {
        return RelayFormat::OpenAiResponses;
    }
    if p.starts_with("/v1/completions") {
        return RelayFormat::OpenAiText;
    }
    if p.starts_with("/v1/embeddings") {
        return RelayFormat::OpenAiEmbedding;
    }
    if p.starts_with("/v1/chat/completions") {
        return RelayFormat::OpenAiChat;
    }
    RelayFormat::Raw
}

/// Apply the channel's status-code mapping override.
fn map_status(channel: &Channel, status: u16) -> u16 {
    channel
        .status_code_mapping
        .get(status.to_string())
        .and_then(|v| v.as_u64())
        .and_then(|v| u16::try_from(v).ok())
        .unwrap_or(status)
}

/// Convert a relay-layer error into the scheduler's error vocabulary so retry
/// and failover behave identically for translated and pass-through providers.
fn map_relay_error(error: oxygenrouter_relay::RelayError, model: &str) -> ProxyError {
    use oxygenrouter_relay::RelayError as R;
    match error {
        R::Upstream { status, body } => {
            if is_context_exceeded_error(&body) {
                ProxyError::ContextExceeded(body)
            } else {
                ProxyError::Upstream { status, body }
            }
        }
        // An adaptor refusing the request is our bug, not the upstream's.
        R::Unsupported(what) => ProxyError::Internal(format!("unsupported: {}", what)),
        R::InvalidRequest(msg) => ProxyError::Internal(format!("invalid request: {}", msg)),
        R::Conversion(msg) => ProxyError::Internal(format!(
            "conversion failed for model `{}`: {}",
            model, msg
        )),
        // Auth failures are retryable so the scheduler switches channel — the
        // same behaviour as NewAPI's automatic 401 disable.
        R::Auth(msg) => ProxyError::Upstream {
            status: 401,
            body: msg,
        },
        R::Network(msg) => ProxyError::Network(msg),
        R::Internal(msg) => ProxyError::Internal(msg),
    }
}

fn is_context_exceeded_error(body: &str) -> bool {
    let lower = body.to_lowercase();
    lower.contains("context_length_exceeded")
        || lower.contains("context length exceeded")
        || lower.contains("maximum context length")
        || lower.contains("string too long")
        || lower.contains("reduce the length of the messages")
        || lower.contains("too many tokens")
        || lower.contains("input is too long")
        || lower.contains("prompt is too long")
}
