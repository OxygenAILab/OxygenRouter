//! Channel scheduler: weighted pick, retry on retryable errors, switch on
//! context exceeded, adaptor-driven dispatch for non-OpenAI providers.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::time::Duration;

use crate::affinity::{AffinityStore, SessionMode};
use crate::dispatch::{relay_format_for_path, RelayClient};
use crate::selection::{
    apply_model_map, select_tiered_excluding, tiers_exhausted_excluding, Selection,
};
use crate::upstream::{ProxyError, ProxyRequest, ProxyResult};
use oxygenrouter_core::{Channel, ChannelSelector, ModelMap};
use oxygenrouter_relay::retry::backoff_ms;

pub struct ChannelScheduler {
    selector: ChannelSelector,
    relay: RelayClient,
    model_maps: Vec<ModelMap>,
    max_retries: u32,
    backoff_base_ms: u64,
    backoff_cap_ms: u64,
    /// Consecutive failures per channel, for auto-disable.
    failures: parking_lot::Mutex<std::collections::HashMap<String, u32>>,
    /// Consecutive failures before a channel is disabled. `0` disables the rule.
    disable_threshold: u32,
    /// Session stickiness, shared with the console so one edit reaches every
    /// in-flight dispatch.
    affinity: std::sync::Arc<AffinityStore>,
}

impl ChannelScheduler {
    pub fn new(selector: ChannelSelector, relay: RelayClient) -> Self {
        Self {
            selector,
            relay,
            model_maps: Vec::new(),
            max_retries: 3,
            backoff_base_ms: 300,
            backoff_cap_ms: 30_000,
            failures: parking_lot::Mutex::new(std::collections::HashMap::new()),
            disable_threshold: 5,
            affinity: std::sync::Arc::new(AffinityStore::new()),
        }
    }

    /// The affinity store, so the console can load a setting into it.
    pub fn affinity(&self) -> std::sync::Arc<AffinityStore> {
        std::sync::Arc::clone(&self.affinity)
    }

    /// Configure auto-disable. `threshold == 0` turns the rule off; NewAPI's
    /// default behaviour is to never auto-disable unless configured.
    pub fn with_disable_threshold(mut self, threshold: u32) -> Self {
        self.disable_threshold = threshold;
        self
    }

    pub fn with_max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    /// Configure retry backoff. `base_ms == 0` disables sleeping (immediate
    /// retry, matching NewAPI's behaviour).
    pub fn with_backoff(mut self, base_ms: u64, cap_ms: u64) -> Self {
        self.backoff_base_ms = base_ms;
        self.backoff_cap_ms = cap_ms.max(base_ms);
        self
    }

    pub fn set_model_maps(&mut self, maps: Vec<ModelMap>) {
        self.model_maps = maps;
    }

    /// Apply channel-specific model map rewriting.
    fn rewrite_model(&self, channel: &Channel, model: &str) -> String {
        apply_model_map(&self.model_maps, &channel.id, model)
    }

    /// Record a successful attempt: the channel's failure streak resets.
    fn note_success(&self, channel_id: &str) {
        self.failures.lock().remove(channel_id);
    }

    /// Record a failed attempt and report whether the channel has now crossed
    /// the auto-disable threshold.
    ///
    /// This is an improvement over NewAPI in one respect and deliberately
    /// identical in another: identical in that only *consecutive* failures
    /// count, and an improvement in that the threshold is enforced in-process
    /// so a dead channel stops being selected immediately.
    fn note_failure(&self, channel_id: &str) -> bool {
        if self.disable_threshold == 0 {
            return false;
        }
        let mut failures = self.failures.lock();
        let count = failures.entry(channel_id.to_string()).or_insert(0);
        *count += 1;
        *count >= self.disable_threshold
    }

    /// Number of consecutive failures recorded for a channel.
    pub fn failure_count(&self, channel_id: &str) -> u32 {
        self.failures.lock().get(channel_id).copied().unwrap_or(0)
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
        let limit = self.max_retries.max(1);

        // Resolved once, before any attempt: the identity a rule derives must not
        // change because a retry rewrote the model, or a failover would pin the
        // wrong session.
        let affinity = self.affinity.resolve(req, model, group_name);

        for attempt in 0..=limit {
            let candidates = self.candidates(&tried, group_name, cross_group_retry && attempt > 0);
            if candidates.is_empty() {
                return Err(last_err.unwrap_or(ProxyError::NoChannel));
            }
            // Walk priority tiers with the attempt counter so the first attempt
            // takes the best tier and each retry steps down. `tried` is passed in
            // so a tier whose members have all been attempted falls through to
            // the next one -- that fall-through is what lets a dead primary hand
            // off to its backup instead of failing the request.
            if tiers_exhausted_excluding(&candidates, &tried, attempt) {
                return Err(last_err.unwrap_or(ProxyError::NoChannel));
            }

            // On the first attempt only. A retry is the scheduler's reaction to
            // a failure, so repeating a choice that just failed would burn an
            // attempt; stickiness is a preference, not a policy that overrides
            // failover.
            let pinned = affinity.as_ref().and_then(|hit| {
                if attempt > 0 {
                    return None;
                }
                hit.pinned_channel
                    .as_ref()
                    .and_then(|id| candidates.iter().find(|c| &c.id == id).cloned())
            });
            if let Some(channel) = pinned {
                let actual_model = self.rewrite_model(&channel, &current_model);
                let format = relay_format_for_path(&req.path);
                match self.relay.send(&channel, &actual_model, req, format).await {
                    Ok(outcome) => {
                        self.note_success(&channel.id);
                        return Ok(outcome.result);
                    }
                    Err(error) if affinity.as_ref().map(|h| h.mode) == Some(SessionMode::Strict) => {
                        // The session must not move, so the failure is final for
                        // this request rather than a reason to pick another
                        // channel. A caller that asked for a stable upstream
                        // would rather see the error than be moved silently.
                        self.note_failure(&channel.id);
                        return Err(error);
                    }
                    Err(_) => {
                        // Prefer mode: fall through to ordinary selection, which
                        // will also remember the channel it ultimately chooses.
                        self.note_failure(&channel.id);
                        tried.push(channel.id.clone());
                    }
                }
            }

            let channel = match select_tiered_excluding(&candidates, &tried, attempt, rand_i64()) {
                Selection::Picked(channel) => channel,
                Selection::NoCandidate => {
                    return Err(last_err.unwrap_or(ProxyError::NoChannel));
                }
                Selection::TiersExhausted => {
                    return Err(last_err.unwrap_or(ProxyError::NoChannel));
                }
            };
            tried.push(channel.id.clone());

            let actual_model = self.rewrite_model(&channel, &current_model);
            let format = relay_format_for_path(&req.path);
            match self.relay.send(&channel, &actual_model, req, format).await {
                Ok(outcome) => {
                    self.note_success(&channel.id);
                    // Only now, once a channel has actually served the request:
                    // remembering a choice that failed would pin the session to a
                    // channel that cannot serve it.
                    if let Some(hit) = &affinity {
                        self.affinity.remember(&hit.key, &channel.id, hit.ttl);
                    }
                    let mut result = outcome.result;
                    // `tried` ends with the winner; everything before it is the
                    // failover trail, recorded so a retried request is auditable.
                    if tried.len() > 1 {
                        result.failed_channels = tried[..tried.len() - 1].to_vec();
                    }
                    return Ok(result);
                }
                Err(e) => {
                    let ctx_exceeded = e.is_context_exceeded();
                    if ctx_exceeded {
                        if let Some(fb) = self.selector.db_fallback(&channel, &current_model) {
                            current_model = fb;
                            continue;
                        }
                    }
                    let retryable = e.is_retryable();
                    if retryable {
                        let crossed = self.note_failure(&channel.id);
                        if crossed {
                            eprintln!(
                                "[OxygenRouter] channel {} hit {} consecutive failures; excluding it from selection",
                                channel.id, self.disable_threshold
                            );
                        }
                    }
                    if retryable && attempt < limit {
                        last_err = Some(e);
                        if self.backoff_base_ms > 0 {
                            let delay =
                                backoff_ms(attempt, self.backoff_base_ms, self.backoff_cap_ms);
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                        }
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_err.unwrap_or(ProxyError::NoChannel))
    }

    /// Eligible channels for this attempt.
    ///
    /// Excludes channels already tried in this request and any channel that has
    /// crossed the auto-disable threshold.
    fn candidates(
        &self,
        exclude: &[String],
        group_name: Option<&str>,
        allow_cross_group: bool,
    ) -> Vec<Channel> {
        let channels = match self.selector.list_enabled() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let threshold = self.disable_threshold;
        let failures = self.failures.lock();
        let mut candidates: Vec<Channel> = channels
            .into_iter()
            .filter(|c| !exclude.contains(&c.id))
            .filter(|c| {
                // A channel past the threshold is skipped until it succeeds again.
                threshold == 0 || failures.get(&c.id).copied().unwrap_or(0) < threshold
            })
            .collect();
        drop(failures);

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
        candidates
    }

}

/// A random value used as the weighted-draw roll.
///
/// Seeded from the clock and the thread id so concurrent requests do not draw
/// the same value; the distribution matters, not cryptographic quality.
fn rand_i64() -> i64 {
    use std::hash::{Hash, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        .hash(&mut hasher);
    std::thread::current().id().hash(&mut hasher);
    (hasher.finish() >> 1) as i64
}
