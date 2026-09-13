//! Channel connectivity tester
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::time::Instant;

use oxygenrouter_core::{Channel, ChannelTestResult};

pub async fn test_channel(channel: &Channel) -> ChannelTestResult {
    let start = Instant::now();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

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
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as i64;
            let status = resp.status().as_u16();
            if resp.status().is_success() {
                ChannelTestResult {
                    channel_id: channel.id.clone(),
                    success: true,
                    latency_ms: Some(latency_ms),
                    error: None,
                    model: None,
                }
            } else {
                let err_text = resp.text().await.unwrap_or_default();
                ChannelTestResult {
                    channel_id: channel.id.clone(),
                    success: false,
                    latency_ms: Some(latency_ms),
                    error: Some(format!("HTTP {}: {}", status, err_text)),
                    model: None,
                }
            }
        }
        Err(e) => ChannelTestResult {
            channel_id: channel.id.clone(),
            success: false,
            latency_ms: None,
            error: Some(e.to_string()),
            model: None,
        },
    }
}
