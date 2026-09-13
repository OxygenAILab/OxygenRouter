//! Channel scheduler: weighted pick, retry on retryable errors, switch on context exceeded
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::time::Duration;

use crate::client::UpstreamClient;
use crate::upstream::{ProxyError, ProxyRequest, ProxyResult};
use oxygenrouter_core::{Channel, ChannelSelector};

pub struct ChannelScheduler {
    selector: ChannelSelector,
    upstream: UpstreamClient,
    model_maps: Vec<(String, String, String)>,
    max_retries: u32,
}

impl ChannelScheduler {
    pub fn new(selector: ChannelSelector, upstream: UpstreamClient) -> Self {
        Self {
            selector,
            upstream,
            model_maps: Vec::new(),
            max_retries: 3,
        }
    }

    pub fn with_max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    pub fn set_model_maps(&mut self, maps: Vec<oxygenrouter_core::ModelMap>) {
        self.model_maps = maps
            .into_iter()
            .filter(|m| m.enabled)
            .map(|m| (m.channel_id, m.pattern, m.target_model))
            .collect();
    }

    /// Apply channel-specific model map rewriting
    fn apply_model_map(&self, channel: &Channel, model: &str) -> String {
        for (ch_id, pattern, target) in &self.model_maps {
            if ch_id == &channel.id {
                if let Ok(re) = regex_lite_match(pattern, model) {
                    if re {
                        return target.clone();
                    }
                }
            }
        }
        model.to_string()
    }

    pub async fn dispatch(
        &self,
        req: &ProxyRequest,
        model: &str,
    ) -> Result<ProxyResult, ProxyError> {
        self.dispatch_for_group(req, model, None, true).await
    }

    pub async fn dispatch_for_group(
        &self,
        req: &ProxyRequest,
        model: &str,
        group_name: Option<&str>,
        cross_group_retry: bool,
    ) -> Result<ProxyResult, ProxyError> {
        let mut tried: Vec<String> = Vec::new();
        let mut last_err: Option<ProxyError> = None;
        let mut current_model = model.to_string();

        for attempt in 0..=self.max_retries {
            let channel = self.pick_channel(&tried, group_name, cross_group_retry && attempt > 0);
            let channel = match channel {
                Some(c) => c,
                None => {
                    if let Some(e) = last_err {
                        return Err(e);
                    }
                    return Err(ProxyError::NoChannel);
                }
            };
            tried.push(channel.id.clone());

            let actual_model = self.apply_model_map(&channel, &current_model);
            match self.upstream.send(&channel, &actual_model, req).await {
                Ok(r) => return Ok(r),
                Err(e) => {
                    let retryable = e.is_retryable();
                    let ctx_exceeded = e.is_context_exceeded();
                    if ctx_exceeded {
                        if let Some(fb) = self.selector.db_fallback(&channel, &current_model) {
                            current_model = fb;
                            continue;
                        }
                    }
                    if retryable && attempt < self.max_retries {
                        last_err = Some(e);
                        tokio::time::sleep(Duration::from_millis(300 * (attempt as u64 + 1))).await;
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_err.unwrap_or(ProxyError::NoChannel))
    }

    fn pick_channel(
        &self,
        exclude: &[String],
        group_name: Option<&str>,
        allow_cross_group: bool,
    ) -> Option<Channel> {
        let channels = match self.selector.list_enabled() {
            Ok(c) => c,
            Err(_) => return None,
        };
        let mut candidates: Vec<Channel> = channels
            .into_iter()
            .filter(|c| !exclude.contains(&c.id))
            .collect();
        if let Some(group_name) = group_name.filter(|group| !group.is_empty()) {
            let grouped: Vec<Channel> = candidates
                .iter()
                .filter(|c| c.group_name == group_name)
                .cloned()
                .collect();
            if !grouped.is_empty() || !allow_cross_group {
                candidates = grouped;
            }
        }
        if candidates.is_empty() {
            return None;
        }
        let total_weight: i32 = candidates.iter().map(|c| c.weight.max(1)).sum();
        let target = (rand_u32() as i32) % total_weight.max(1);
        let mut acc = 0;
        for c in &candidates {
            acc += c.weight.max(1);
            if acc > target {
                return Some(c.clone());
            }
        }
        candidates.into_iter().next()
    }
}

fn regex_lite_match(pattern: &str, input: &str) -> Result<bool, ()> {
    let pat = pattern.trim();
    if pat.is_empty() {
        return Ok(false);
    }
    if pat == "*" {
        return Ok(true);
    }
    if pat.starts_with('*') && pat.ends_with('*') {
        let mid = &pat[1..pat.len() - 1];
        return Ok(input.contains(mid));
    }
    if let Some(rest) = pat.strip_prefix('*') {
        return Ok(input.ends_with(rest));
    }
    if let Some(rest) = pat.strip_suffix('*') {
        return Ok(input.starts_with(rest));
    }
    Ok(pat == input)
}

fn rand_u32() -> u32 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::SystemTime;
    let mut h = DefaultHasher::new();
    SystemTime::now().hash(&mut h);
    h.finish() as u32
}
