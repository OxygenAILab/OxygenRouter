//! The anonymous artifact-access limiter.
//!
//! Ported from `middleware/task_artifact_access.go`: a capability URL is
//! bearer access to one object, so the anonymous path gets a bounded number of
//! concurrent fetches (globally, per address, per object) and a bounded number
//! of failed-capability attempts per address per minute. A caller over the
//! invalid-attempt rate is answered 429 rather than 404, so probing costs the
//! prober its allowance instead of giving it an oracle.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// The reference's defaults (`setting/system_setting/task_artifact.go:6-9`).
const DEFAULT_INVALID_PER_MINUTE: i64 = 60;
const DEFAULT_GLOBAL: i64 = 128;
const DEFAULT_PER_IP: i64 = 64;
const DEFAULT_PER_OBJECT: i64 = 16;
const INVALID_WINDOW: Duration = Duration::from_secs(60);
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactLimits {
    pub invalid_per_minute: u32,
    pub global: usize,
    pub per_ip: usize,
    pub per_object: usize,
}

impl Default for ArtifactLimits {
    fn default() -> Self {
        ArtifactLimits {
            invalid_per_minute: DEFAULT_INVALID_PER_MINUTE as u32,
            global: DEFAULT_GLOBAL as usize,
            per_ip: DEFAULT_PER_IP as usize,
            per_object: DEFAULT_PER_OBJECT as usize,
        }
    }
}

impl ArtifactLimits {
    /// Read the four deployment variables. Anything missing, non-numeric, or
    /// non-positive keeps the default, so a typo cannot disable a limit.
    pub fn from_env() -> Self {
        ArtifactLimits {
            invalid_per_minute: env_positive(
                "TASK_ARTIFACT_INVALID_RATE_LIMIT_PER_MINUTE",
                DEFAULT_INVALID_PER_MINUTE,
            ) as u32,
            global: env_positive("TASK_ARTIFACT_GLOBAL_CONCURRENCY", DEFAULT_GLOBAL) as usize,
            per_ip: env_positive("TASK_ARTIFACT_IP_CONCURRENCY", DEFAULT_PER_IP) as usize,
            per_object: env_positive(
                "TASK_ARTIFACT_OBJECT_CONCURRENCY",
                DEFAULT_PER_OBJECT,
            ) as usize,
        }
    }
}

fn env_positive(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

struct RateWindow {
    started: Instant,
    count: u32,
}

#[derive(Default)]
struct LimiterState {
    global: usize,
    by_ip: HashMap<String, usize>,
    by_object: HashMap<String, usize>,
    rates: HashMap<String, RateWindow>,
    next_cleanup: Option<Instant>,
}

impl LimiterState {
    fn cleanup(&mut self, now: Instant) {
        if self
            .next_cleanup
            .map(|next| now < next)
            .unwrap_or(false)
        {
            return;
        }
        self.rates.retain(|_, window| {
            now.saturating_duration_since(window.started) < INVALID_WINDOW
        });
        self.next_cleanup = Some(now + CLEANUP_INTERVAL);
    }
}

pub struct ArtifactAccessLimiter {
    state: Mutex<LimiterState>,
    limits: ArtifactLimits,
}

impl ArtifactAccessLimiter {
    pub fn new(limits: ArtifactLimits) -> Self {
        ArtifactAccessLimiter {
            state: Mutex::new(LimiterState::default()),
            limits,
        }
    }

    pub fn from_env() -> Self {
        Self::new(ArtifactLimits::from_env())
    }

    /// Record one invalid capability attempt.
    ///
    /// `false` means the address has spent its per-minute allowance and the
    /// caller must be answered 429 rather than 404.
    pub fn invalid_attempt(&self, ip: &str, now: Instant) -> bool {
        let mut state = self.state.lock();
        state.cleanup(now);
        let entry = state
            .rates
            .entry(ip.to_string())
            .or_insert_with(|| RateWindow {
                started: now,
                count: 0,
            });
        if now.saturating_duration_since(entry.started) >= INVALID_WINDOW {
            entry.started = now;
            entry.count = 0;
        }
        if entry.count >= self.limits.invalid_per_minute {
            return false;
        }
        entry.count += 1;
        true
    }

    /// Take one of the three concurrency slots for an anonymous fetch.
    ///
    /// The permit releases them on drop, which is when the response finishes.
    pub fn acquire(
        self: &Arc<Self>,
        ip: &str,
        task_id: &str,
        artifact_key: &str,
        now: Instant,
    ) -> Option<ArtifactPermit> {
        let mut state = self.state.lock();
        state.cleanup(now);
        let object = format!("{task_id}\u{0}{artifact_key}");
        if state.global >= self.limits.global
            || state.by_ip.get(ip).copied().unwrap_or(0) >= self.limits.per_ip
            || state.by_object.get(&object).copied().unwrap_or(0) >= self.limits.per_object
        {
            return None;
        }
        state.global += 1;
        *state.by_ip.entry(ip.to_string()).or_insert(0) += 1;
        *state.by_object.entry(object.clone()).or_insert(0) += 1;
        Some(ArtifactPermit {
            limiter: Arc::clone(self),
            ip: ip.to_string(),
            object,
        })
    }
}

/// One anonymous fetch's hold on the limiter; dropping it releases every slot.
pub struct ArtifactPermit {
    limiter: Arc<ArtifactAccessLimiter>,
    ip: String,
    object: String,
}

impl Drop for ArtifactPermit {
    fn drop(&mut self) {
        let mut state = self.limiter.state.lock();
        state.global = state.global.saturating_sub(1);
        let mut drop_ip = false;
        if let Some(count) = state.by_ip.get_mut(&self.ip) {
            *count -= 1;
            drop_ip = *count == 0;
        }
        if drop_ip {
            state.by_ip.remove(&self.ip);
        }
        let mut drop_object = false;
        if let Some(count) = state.by_object.get_mut(&self.object) {
            *count -= 1;
            drop_object = *count == 0;
        }
        if drop_object {
            state.by_object.remove(&self.object);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> ArtifactLimits {
        ArtifactLimits {
            invalid_per_minute: 3,
            global: 2,
            per_ip: 1,
            per_object: 1,
        }
    }

    #[test]
    fn invalid_attempts_are_capped_per_minute() {
        let limiter = ArtifactAccessLimiter::new(limits());
        let start = Instant::now();
        assert!(limiter.invalid_attempt("1.2.3.4", start));
        assert!(limiter.invalid_attempt("1.2.3.4", start));
        assert!(limiter.invalid_attempt("1.2.3.4", start));
        assert!(!limiter.invalid_attempt("1.2.3.4", start));
        // Another address has its own allowance.
        assert!(limiter.invalid_attempt("5.6.7.8", start));
        // A later minute resets it.
        assert!(limiter.invalid_attempt(
            "1.2.3.4",
            start + INVALID_WINDOW + Duration::from_secs(1)
        ));
    }

    #[test]
    fn concurrency_slots_release_on_drop() {
        let limiter = Arc::new(ArtifactAccessLimiter::new(limits()));
        let now = Instant::now();
        let first = limiter
            .acquire("ip", "t1", "video", now)
            .expect("first slot");
        // The address slot is taken.
        assert!(limiter.acquire("ip", "t2", "video", now).is_none());
        // Another address holds the second global slot.
        let second = limiter
            .acquire("ip2", "t2", "video", now)
            .expect("second slot");
        // Global is full.
        assert!(limiter.acquire("ip3", "t3", "video", now).is_none());
        drop(first);
        // A different object fits, and the first address is free again.
        assert!(limiter.acquire("ip", "t3", "video", now).is_some());
        drop(second);
    }

    #[test]
    fn object_slots_are_per_object() {
        let limiter = Arc::new(ArtifactAccessLimiter::new(ArtifactLimits {
            per_object: 1,
            ..limits()
        }));
        let now = Instant::now();
        let held = limiter.acquire("ip1", "t1", "video", now).expect("held");
        assert!(limiter.acquire("ip2", "t1", "video", now).is_none());
        assert!(limiter.acquire("ip2", "t1", "audio", now).is_some());
        drop(held);
        assert!(limiter.acquire("ip2", "t1", "video", now).is_some());
    }
}
