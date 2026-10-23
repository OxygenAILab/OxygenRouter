//! Channel scheduler: weighted pick, retry on retryable errors, switch on
//! context exceeded, adaptor-driven dispatch for non-OpenAI providers.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::time::Duration;

use crate::affinity::{AffinityStore, SessionMode};
use crate::dispatch::{relay_format_for_path, RelayClient};
use crate::policy::RequestPolicy;
use crate::selection::{
    apply_model_map, select_tiered_excluding, tiers_exhausted_excluding, Selection,
};
use crate::upstream::{ProxyError, ProxyRequest, ProxyResult};
use oxygenrouter_core::{Channel, ChannelSelector, ModelMap};

pub struct ChannelScheduler {
    selector: ChannelSelector,
    relay: RelayClient,
    model_maps: Vec<ModelMap>,
    /// The live request policy, replaced whole whenever an option changes.
    policy: RequestPolicy,
    /// Consecutive failures per channel, for the streak exclusion extension.
    failures: parking_lot::Mutex<std::collections::HashMap<String, u32>>,
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
            policy: RequestPolicy::default(),
            failures: parking_lot::Mutex::new(std::collections::HashMap::new()),
            affinity: std::sync::Arc::new(AffinityStore::new()),
        }
    }

    /// The affinity store, so the console can load a setting into it.
    pub fn affinity(&self) -> std::sync::Arc<AffinityStore> {
        std::sync::Arc::clone(&self.affinity)
    }

    /// Install a new policy snapshot.
    ///
    /// The console writes options and then pushes the rebuilt snapshot here, so
    /// an edit reaches the very next dispatch without a restart — the reference
    /// does the same through its `RequestPolicySnapshot`.
    pub fn set_policy(&mut self, policy: RequestPolicy) {
        self.policy = policy;
    }

    pub fn policy(&self) -> &RequestPolicy {
        &self.policy
    }

    pub fn set_model_maps(&mut self, maps: Vec<ModelMap>) {
        self.model_maps = maps;
    }

    /// Apply channel-specific model map rewriting.
    ///
    /// Refuses rather than guesses when a chain cycles: sending the original name
    /// would silently bill and route something the operator did not configure,
    /// and sending a half-resolved name would be worse. The error names the chain
    /// so the rule can be fixed.
    fn rewrite_model(
        &self,
        channel: &Channel,
        model: &str,
    ) -> Result<String, crate::selection::ModelMapError> {
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
        if self.policy.failure_streak_threshold == 0 {
            return false;
        }
        let mut failures = self.failures.lock();
        let count = failures.entry(channel_id.to_string()).or_insert(0);
        *count += 1;
        *count >= self.policy.failure_streak_threshold
    }

    /// Number of consecutive failures recorded for a channel.
    pub fn failure_count(&self, channel_id: &str) -> u32 {
        self.failures.lock().get(channel_id).copied().unwrap_or(0)
    }

    /// Record an attempt the caller performed itself.
    ///
    /// The task bridge shapes its own requests through a plugin, so it cannot
    /// go through `dispatch`; without this the streak that protects every other
    /// request would ignore every task attempt.
    pub fn note_attempt_success(&self, channel_id: &str) {
        self.note_success(channel_id);
    }

    /// Record a failed attempt and report whether the channel has now crossed
    /// the auto-disable threshold.
    pub fn note_attempt_failure(&self, channel_id: &str) -> bool {
        self.note_failure(channel_id)
    }

    /// Whether the channel has crossed the auto-disable threshold.
    ///
    /// A caller that selects channels itself has to apply the same exclusion
    /// `candidates` applies for the relay path.
    pub fn is_temporarily_disabled(&self, channel_id: &str) -> bool {
        self.policy.failure_streak_threshold > 0
            && self.failure_count(channel_id) >= self.policy.failure_streak_threshold
    }

    /// Persist an automatic disable when the policy says this failure deserves
    /// one, and drop the streak that stood in for it.
    ///
    /// The two mechanisms answer different questions: the streak keeps traffic
    /// away from a flapping channel in-process, while the persisted disable is
    /// the operator-visible state a health test can lift. Once the persisted
    /// disable exists, keeping the streak would leave the channel excluded even
    /// after the console shows it enabled again.
    fn maybe_auto_disable(&self, channel: &Channel, error: &ProxyError) {
        if !channel.auto_ban || !self.policy.auto_disable_enabled {
            return;
        }
        if !self.policy.should_disable(error) {
            return;
        }
        let reason = self.policy.disable_reason(error);
        match self
            .selector
            .database()
            .set_channel_auto_disabled(&channel.id, &reason)
        {
            Ok(true) => {
                self.failures.lock().remove(&channel.id);
                eprintln!(
                    "[OxygenRouter] channel {} ({}) auto-disabled: {}",
                    channel.name, channel.id, reason
                );
            }
            Ok(false) => {}
            Err(error) => eprintln!(
                "[OxygenRouter] failed to auto-disable channel {}: {error}",
                channel.id
            ),
        }
    }

    /// Record a failed attempt the caller performed itself, and apply the
    /// auto-disable rule to it.
    ///
    /// The task bridge shapes its own requests through a plugin, so it cannot
    /// go through `dispatch`; without this the policy that protects the relay
    /// path would not see task failures at all.
    pub fn record_task_failure(&self, channel: &Channel, status: u16, message: &str) -> bool {
        let crossed = self.note_failure(&channel.id);
        let error = ProxyError::Upstream {
            status,
            body: message.to_string(),
        };
        self.maybe_auto_disable(channel, &error);
        crossed
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
        // `RetryTimes` counts the attempts *after* the first, matching the
        // reference's `for retry <= RetryTimes` loop: 0 means try once.
        let limit = self.policy.max_retries;

        // Resolved once, before any attempt: the identity a rule derives must not
        // change because a retry rewrote the model, or a failover would pin the
        // wrong session.
        let affinity = self.affinity.resolve(req, model, group_name);

        for attempt in 0..=limit {
            // The requested model, not the possibly-rewritten one: a channel's
            // declaration is keyed by what the caller asked for, exactly as the
            // reference keys its ability rows by the origin model
            // (`model/ability.go:217`).
            let candidates = self.candidates(
                &tried,
                model,
                group_name,
                cross_group_retry && attempt > 0,
            );
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
                let actual_model = match self.rewrite_model(&channel, &current_model) {
                    Ok(name) => name,
                    Err(error) => return Err(ProxyError::InvalidRequest(error.to_string())),
                };
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
                        self.maybe_auto_disable(&channel, &error);
                        return Err(error);
                    }
                    Err(error) => {
                        // Prefer mode: fall through to ordinary selection, which
                        // will also remember the channel it ultimately chooses.
                        self.note_failure(&channel.id);
                        self.maybe_auto_disable(&channel, &error);
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

            let actual_model = match self.rewrite_model(&channel, &current_model) {
                Ok(name) => name,
                Err(error) => return Err(ProxyError::InvalidRequest(error.to_string())),
            };
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
                    let retryable = self.policy.should_retry(&e, limit - attempt);
                    if retryable {
                        let crossed = self.note_failure(&channel.id);
                        if crossed {
                            eprintln!(
                                "[OxygenRouter] channel {} hit {} consecutive failures; excluding it from selection",
                                channel.id, self.policy.failure_streak_threshold
                            );
                        }
                    }
                    // The persisted disable is decided from the policy's own
                    // rules and applies even to failures that are not retried:
                    // a keyword like "exceeded your current quota" arrives with
                    // a status the retry ranges leave alone, yet it is exactly
                    // the state that should take the channel out of service.
                    self.maybe_auto_disable(&channel, &e);
                    if retryable && attempt < limit {
                        last_err = Some(e);
                        let delay = self.policy.retry_delay_ms(attempt);
                        if delay > 0 {
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
        model: &str,
        group_name: Option<&str>,
        allow_cross_group: bool,
    ) -> Vec<Channel> {
        let channels = match self.selector.list_enabled() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let threshold = self.policy.failure_streak_threshold;
        let failures = self.failures.lock();
        let mut candidates: Vec<Channel> = channels
            .into_iter()
            .filter(|c| c.serves_model(model))
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

#[cfg(test)]
mod tests {
    use super::*;
    use oxygenrouter_core::Database;
    use std::sync::Arc;

    fn channel(name: &str, priority: i32, models: &[&str]) -> Channel {
        let mut channel = Channel::new(
            name.to_string(),
            "http://127.0.0.1:1".to_string(),
            "k".to_string(),
            priority,
            1,
            "test".to_string(),
        );
        channel.model_list = models.iter().map(|model| model.to_string()).collect();
        channel
    }

    fn scheduler_with(channels: Vec<Channel>) -> ChannelScheduler {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("scheduler-test.db");
        // `ChannelScheduler` holds only the selector, and the file must outlive
        // this function for the selector to keep reading it.
        std::mem::forget(dir);
        let db = Arc::new(Database::new(&path).expect("database"));
        for channel in channels {
            db.upsert_channel(&channel).expect("channel");
        }
        ChannelScheduler::new(
            ChannelSelector::new(db),
            RelayClient::new("test".to_string(), 5_000),
        )
    }

    /// A candidate serves only what it declares; an empty declaration is not a
    /// wildcard (`model/ability.go:217`), and `*` is our explicit extension.
    #[test]
    fn candidates_only_include_the_channels_that_declare_the_model() {
        let scheduler = scheduler_with(vec![
            channel("undeclared", 100, &["other-model"]),
            channel("declared", 1, &["wanted-model"]),
            channel("wildcard", 0, &["*"]),
            channel("empty", 50, &[]),
        ]);

        let candidates = scheduler.candidates(&[], "wanted-model", None, true);
        let mut names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["declared", "wildcard"]);
    }

    #[test]
    fn a_model_no_channel_declares_has_no_candidates() {
        let scheduler = scheduler_with(vec![
            channel("empty", 10, &[]),
            channel("other", 5, &["x"]),
        ]);
        assert!(scheduler
            .candidates(&[], "missing-model", None, true)
            .is_empty());
    }

    #[test]
    fn the_failure_streak_exclusion_reads_the_policy() {
        let candidate = channel("only", 1, &["wanted"]);
        let id = candidate.id.clone();
        let mut scheduler = scheduler_with(vec![candidate]);
        let mut policy = RequestPolicy::default();
        policy.failure_streak_threshold = 2;
        scheduler.set_policy(policy);

        assert!(!scheduler.note_attempt_failure(&id));
        assert!(!scheduler.is_temporarily_disabled(&id));
        assert!(scheduler.note_attempt_failure(&id));
        assert!(scheduler.is_temporarily_disabled(&id));
        assert!(scheduler.candidates(&[], "wanted", None, true).is_empty());

        scheduler.note_attempt_success(&id);
        assert_eq!(scheduler.candidates(&[], "wanted", None, true).len(), 1);
    }
}
