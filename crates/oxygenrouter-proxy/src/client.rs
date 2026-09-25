//! Upstream HTTP client: build URL, sign, send, parse errors
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use bytes::Bytes;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Client;
use std::time::Duration;

use crate::upstream::{ModelContextExceeded, ProxyError, ProxyRequest, ProxyResult};
use oxygenrouter_core::Channel;

pub struct UpstreamClient {
    client: Client,
}

impl UpstreamClient {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(8)
            .build()
            .unwrap_or_default();
        Self { client }
    }

    pub async fn send(
        &self,
        channel: &Channel,
        model: &str,
        req: &ProxyRequest,
    ) -> Result<ProxyResult, ProxyError> {
        let base = channel.base_url.trim_end_matches('/');
        let path = req.path.trim_start_matches('/');
        // If base already contains the first path segment (e.g. base="https://api.openai.com/v1",
        // path="/v1/chat/completions" → "https://api.openai.com/v1" is missing the rest, so we
        // still need to append. Only avoid duplicating the segment when path == first-segment.
        let first_segment = path.split('/').next().unwrap_or("");
        let url = if !first_segment.is_empty()
            && base.ends_with(&format!("/{}", first_segment))
            && path == first_segment
        {
            base.to_string()
        } else {
            format!("{}/{}", base, path)
        };

        // Multi-key: split api_key by newline, pick randomly
        let keys: Vec<&str> = channel.api_key.split('\n').filter(|k| !k.trim().is_empty()).collect();
        let active_key = if keys.is_empty() { channel.api_key.as_str() } else if keys.len() == 1 { keys[0] } else {
            let idx = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros() as usize % keys.len();
            keys[idx]
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {}", active_key))
                .map_err(|e| ProxyError::Internal(format!("auth header: {}", e)))?,
        );
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        if req.stream {
            headers.insert("Accept", HeaderValue::from_static("text/event-stream"));
        }
        for (k, v) in &req.headers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                if !name.as_str().eq_ignore_ascii_case("authorization")
                    && !name.as_str().eq_ignore_ascii_case("host")
                    && !name.as_str().eq_ignore_ascii_case("content-length")
                {
                    headers.insert(name, value);
                }
            }
        }

        let mut body_json = serde_json::Map::new();
        if let Some(b) = &req.body {
            if let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(b) {
                if let Some(obj) = v.as_object_mut() {
                    obj.insert(
                        "model".to_string(),
                        serde_json::Value::String(model.to_string()),
                    );
                    body_json = obj.clone();
                }
            }
        } else {
            body_json.insert(
                "model".to_string(),
                serde_json::Value::String(model.to_string()),
            );
        }
        if let Some(overrides) = channel.override_parameters.as_object() {
            for (key, value) in overrides {
                body_json.insert(key.clone(), value.clone());
            }
        }
        let body_str = serde_json::to_string(&body_json)
            .map_err(|e| ProxyError::Internal(format!("body json: {}", e)))?;

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .body(body_str)
            .send()
            .await
            .map_err(|e| classify_error(e, model))?;

        let status = resp.status().as_u16();
        let mut resp_headers: Vec<(String, String)> = Vec::new();
        for (k, v) in resp.headers() {
            if let Ok(s) = v.to_str() {
                resp_headers.push((k.as_str().to_string(), s.to_string()));
            }
        }
        if let Some(headers) = channel.response_headers.as_object() {
            for (key, value) in headers {
                if let Some(value) = value.as_str() {
                    resp_headers.retain(|(existing, _)| !existing.eq_ignore_ascii_case(key));
                    resp_headers.push((key.clone(), value.to_string()));
                }
            }
        }
        let status = channel
            .status_code_mapping
            .get(status.to_string())
            .and_then(|value| value.as_u64())
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(status);

        let body_bytes = if req.stream {
            let mut full = Vec::new();
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(b) => full.extend_from_slice(&b),
                    Err(e) => {
                        return Err(ProxyError::Network(format!("stream chunk: {}", e)));
                    }
                }
            }
            Bytes::from(full)
        } else {
            resp.bytes()
                .await
                .map_err(|e| ProxyError::Network(e.to_string()))?
        };

        if status >= 400 {
            let body_str = String::from_utf8_lossy(&body_bytes).to_string();
            if is_context_exceeded_error(&body_str) {
                return Err(ProxyError::ContextExceeded(body_str));
            }
            return Err(ProxyError::Upstream {
                status,
                body: body_str,
            });
        }

        // This client is the legacy OpenAI pass-through, kept for reference and
        // for the channel tester. It reads usage the same way the adaptor layer
        // does so billing sees consistent numbers regardless of the path.
        let usage = if req.stream {
            oxygenrouter_relay::sse::openai::extract_stream_usage(&body_bytes).1
        } else {
            oxygenrouter_relay::usage::extract_openai_usage(&body_bytes)
        };

        Ok(ProxyResult {
            status,
            headers: resp_headers,
            body: body_bytes,
            model_used: model.to_string(),
            channel_id: channel.id.clone(),
            usage,
            adaptor: "openai".to_string(),
            failed_channels: Vec::new(),
        })
    }
}

fn classify_error(e: reqwest::Error, _model: &str) -> ProxyError {
    ProxyError::Network(e.to_string())
}

fn is_context_exceeded_error(body: &str) -> bool {
    let lower = body.to_lowercase();
    lower.contains("context_length_exceeded")
        || lower.contains("context length exceeded")
        || lower.contains("maximum context length")
        || lower.contains("string too long")
        || lower.contains("reduce the length of the messages")
        || lower.contains("too many tokens")
}

impl UpstreamClient {
    pub fn _unused_model_context_marker(&self) -> Option<ModelContextExceeded> {
        None
    }
}
