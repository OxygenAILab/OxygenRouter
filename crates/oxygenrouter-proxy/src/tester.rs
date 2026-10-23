//! Channel connectivity tester
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::time::Instant;

use oxygenrouter_core::{Channel, ChannelTestResult};

/// The raw outcome of one probe, before the caller decides what it means.
///
/// The console's "test" button needs only a verdict, but the scheduled health
/// test has to apply the request policy's disable rules, and those are written
/// against the upstream's status and body. Returning those here keeps one HTTP
/// implementation for both: a second prober would drift from this one and the
/// health test would then judge failures the relay path never sees.
#[derive(Debug, Clone)]
pub struct ChannelProbe {
    pub channel_id: String,
    pub latency_ms: i64,
    /// `None` means the request never reached an upstream (transport failure).
    pub status: Option<u16>,
    pub body: String,
}

impl ChannelProbe {
    pub fn succeeded(&self) -> bool {
        matches!(self.status, Some(status) if (200..300).contains(&status))
    }

    /// The error the request policy should judge, if this probe failed.
    pub fn error(&self) -> Option<crate::upstream::ProxyError> {
        match self.status {
            Some(status) if (200..300).contains(&status) => None,
            Some(status) => Some(crate::upstream::ProxyError::Upstream {
                status,
                body: self.body.clone(),
            }),
            None => Some(crate::upstream::ProxyError::Network(self.body.clone())),
        }
    }
}

/// Probe one channel and report exactly what the upstream answered.
pub async fn probe_channel(channel: &Channel) -> ChannelProbe {
    let start = Instant::now();
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return ChannelProbe {
                channel_id: channel.id.clone(),
                latency_ms: start.elapsed().as_millis() as i64,
                status: None,
                body: format!("could not build the test client: {error}"),
            };
        }
    };

    let url = format!(
        "{}/v1/chat/completions",
        channel.base_url.trim_end_matches('/')
    );
    let body = serde_json::json!({
        "model": channel.test_model,
        "messages": [{"role":"user","content":"ping"}],
        "max_tokens": 5
    });

    match client
        .post(&url)
        .header("Authorization", format!("Bearer {}", channel.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(response) => {
            let latency_ms = start.elapsed().as_millis() as i64;
            let status = response.status().as_u16();
            // The body is always drained: on success it is discarded, but a
            // response that is not read leaves the connection unusable for the
            // next probe.
            let text = response.text().await.unwrap_or_default();
            ChannelProbe {
                channel_id: channel.id.clone(),
                latency_ms,
                status: Some(status),
                body: text,
            }
        }
        Err(error) => ChannelProbe {
            channel_id: channel.id.clone(),
            latency_ms: start.elapsed().as_millis() as i64,
            status: None,
            body: error.to_string(),
        },
    }
}

pub async fn test_channel(channel: &Channel) -> ChannelTestResult {
    let probe = probe_channel(channel).await;
    let success = probe.succeeded();
    ChannelTestResult {
        channel_id: probe.channel_id,
        success,
        latency_ms: Some(probe.latency_ms),
        error: (!success).then(|| match probe.status {
            Some(status) => format!("HTTP {status}: {}", probe.body),
            None => probe.body.clone(),
        }),
        model: None,
    }
}
