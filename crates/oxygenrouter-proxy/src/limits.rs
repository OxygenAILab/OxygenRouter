//! Rate limiting and a global concurrency ceiling.
//!
//! Two protections NewAPI splits across several middleware:
//!
//! * **Rate limiting** — fixed-window counters, matching NewAPI's in-memory
//!   fallback when Redis is absent. Scopes are addressed by name so the same
//!   limiter serves per-IP, per-token and per-model ceilings.
//! * **Concurrency ceiling** — a global in-flight limit. NewAPI does not enforce
//!   one; without it a slow upstream can pile up unbounded in-flight requests and
//!   exhaust memory or upstream quota. This is a deliberate improvement.
//!
//! Both are lock-based and dependency-free so they behave identically in tests
//! and in production.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// A fixed-window rate limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimit {
    /// Maximum accepted requests within one window. `0` disables the limit.
    pub max_requests: u32,
    /// Window length.
    pub window: Duration,
}

impl RateLimit {
    pub const fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            max_requests,
            window: Duration::from_secs(window_secs),
        }
    }

    /// A disabled limit, for callers that must pass one explicitly.
    pub const fn disabled() -> Self {
        Self {
            max_requests: 0,
            window: Duration::from_secs(60),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.max_requests > 0
    }
}

/// Why a request was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitError {
    /// The caller exceeded its rate limit; retry after this many seconds.
    RateLimited { retry_after_secs: u64 },
    /// The server is at its in-flight ceiling.
    ConcurrencyExceeded,
}

impl LimitError {
    pub fn status_code(self) -> u16 {
        match self {
            // NewAPI answers both with 429; a saturated server is the same
            // client-visible condition as a throttled one.
            Self::RateLimited { .. } | Self::ConcurrencyExceeded => 429,
        }
    }

    pub fn retry_after_secs(self) -> Option<u64> {
        match self {
            Self::RateLimited { retry_after_secs } => Some(retry_after_secs),
            Self::ConcurrencyExceeded => None,
        }
    }
}

/// One window's counter.
#[derive(Debug, Clone, Copy)]
struct Window {
    started: Instant,
    count: u32,
}

/// Fixed-window counters keyed by scope.
///
/// The clock is read once per check so a test can drive it; production passes
/// `Instant::now`.
pub struct RateLimiter {
    windows: Mutex<HashMap<String, Window>>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Record one request against `scope`, or refuse it.
    pub fn check(&self, scope: &str, limit: RateLimit) -> Result<(), LimitError> {
        self.check_at(scope, limit, Instant::now())
    }

    /// `check` with an explicit clock, so window expiry is testable.
    pub fn check_at(
        &self,
        scope: &str,
        limit: RateLimit,
        now: Instant,
    ) -> Result<(), LimitError> {
        if !limit.is_enabled() {
            return Ok(());
        }
        let mut windows = self.windows.lock();
        let entry = windows.entry(scope.to_string()).or_insert(Window {
            started: now,
            count: 0,
        });

        // A fixed window resets wholesale once it elapses; NewAPI's in-memory
        // fallback does the same rather than sliding.
        let elapsed = now.saturating_duration_since(entry.started);
        if elapsed >= limit.window {
            entry.started = now;
            entry.count = 0;
        }

        if entry.count >= limit.max_requests {
            let remaining = limit.window.saturating_sub(elapsed);
            return Err(LimitError::RateLimited {
                retry_after_secs: remaining.as_secs().max(1),
            });
        }
        entry.count += 1;
        Ok(())
    }

    /// How many seconds remain before `scope`'s window resets.
    ///
    /// Answers "how long must this caller wait before retrying", so it reports a
    /// duration **only when the caller is currently refused**. A scope that has
    /// accumulated failures but still has budget is not waiting for anything,
    /// and reporting the window there would lock a caller out one attempt early
    /// — which is exactly what a first draft of this did.
    ///
    /// Read-only by design: asking how much longer a lock has to run must not
    /// itself extend it, so this cannot be expressed through `check`.
    pub fn retry_after_secs(&self, scope: &str, limit: RateLimit, now: Instant) -> Option<u64> {
        if !limit.is_enabled() {
            return None;
        }
        let windows = self.windows.lock();
        let entry = windows.get(scope)?;
        if entry.count < limit.max_requests {
            // Under the limit: this caller may retry now.
            return None;
        }
        let elapsed = now.saturating_duration_since(entry.started);
        if elapsed >= limit.window {
            // The window has already lapsed; nothing to wait for.
            return None;
        }
        Some(limit.window.saturating_sub(elapsed).as_secs().max(1))
    }

    /// Forget a scope's window.
    ///
    /// Used to clear a failure count once the caller proves it is legitimate;
    /// without it a user who mistyped a few times would stay one attempt away
    /// from a lock for the rest of the window.
    pub fn reset(&self, scope: &str) {
        self.windows.lock().remove(scope);
    }

    /// Drop windows that have elapsed, so the map does not grow without bound
    /// when scopes are high-cardinality (e.g. one per client IP).
    pub fn prune(&self, now: Instant, longest: Duration) -> usize {
        let mut windows = self.windows.lock();
        let before = windows.len();
        windows.retain(|_, window| now.saturating_duration_since(window.started) < longest);
        before - windows.len()
    }

    /// Number of tracked scopes, for diagnostics.
    pub fn tracked_scopes(&self) -> usize {
        self.windows.lock().len()
    }
}

/// A global in-flight ceiling.
///
/// Deliberately not a queue: a caller that cannot be admitted is refused
/// immediately, because queueing would hide the overload as latency and let the
/// backlog grow anyway.
pub struct ConcurrencyGuard {
    limit: usize,
    in_flight: Mutex<usize>,
}

impl ConcurrencyGuard {
    /// `limit == 0` means unlimited.
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            in_flight: Mutex::new(0),
        }
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn in_flight(&self) -> usize {
        *self.in_flight.lock()
    }

    pub fn is_enabled(&self) -> bool {
        self.limit > 0
    }

    /// Try to admit one request. On success the returned permit frees the slot
    /// when dropped, so no path can leak a slot by returning early.
    pub fn try_acquire(&self) -> Result<Permit<'_>, LimitError> {
        if self.limit == 0 {
            return Ok(Permit { guard: None });
        }
        let mut in_flight = self.in_flight.lock();
        if *in_flight >= self.limit {
            return Err(LimitError::ConcurrencyExceeded);
        }
        *in_flight += 1;
        Ok(Permit {
            guard: Some(self),
        })
    }
}

/// Holds one concurrency slot for its lifetime.
pub struct Permit<'a> {
    guard: Option<&'a ConcurrencyGuard>,
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        if let Some(guard) = self.guard {
            let mut in_flight = guard.in_flight.lock();
            *in_flight = in_flight.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disabled_limit_accepts_everything() {
        let limiter = RateLimiter::new();
        let limit = RateLimit::disabled();
        for _ in 0..1000 {
            assert!(limiter.check("scope", limit).is_ok());
        }
        assert_eq!(limiter.tracked_scopes(), 0);
    }

    #[test]
    fn the_limit_is_enforced_within_a_window() {
        let limiter = RateLimiter::new();
        let limit = RateLimit::new(3, 60);
        let now = Instant::now();
        for i in 0..3 {
            assert!(limiter.check_at("s", limit, now).is_ok(), "request {i}");
        }
        assert!(matches!(
            limiter.check_at("s", limit, now),
            Err(LimitError::RateLimited { .. })
        ));
    }

    #[test]
    fn the_window_resets_after_it_elapses() {
        let limiter = RateLimiter::new();
        let limit = RateLimit::new(2, 60);
        let start = Instant::now();
        assert!(limiter.check_at("s", limit, start).is_ok());
        assert!(limiter.check_at("s", limit, start).is_ok());
        assert!(limiter.check_at("s", limit, start).is_err());

        let later = start + Duration::from_secs(61);
        assert!(
            limiter.check_at("s", limit, later).is_ok(),
            "a new window must accept again"
        );
    }

    #[test]
    fn scopes_are_independent() {
        let limiter = RateLimiter::new();
        let limit = RateLimit::new(1, 60);
        let now = Instant::now();
        assert!(limiter.check_at("ip-a", limit, now).is_ok());
        // A different scope has its own budget.
        assert!(limiter.check_at("ip-b", limit, now).is_ok());
        // But the first is exhausted.
        assert!(limiter.check_at("ip-a", limit, now).is_err());
    }

    #[test]
    fn the_remaining_window_is_reported_without_extending_it() {
        let limiter = RateLimiter::new();
        let limit = RateLimit::new(1, 60);
        let start = Instant::now();

        // Nothing has failed yet, so there is no window to wait out.
        assert_eq!(limiter.retry_after_secs("s", limit, start), None);

        assert!(limiter.check_at("s", limit, start).is_ok());
        assert_eq!(
            limiter.retry_after_secs("s", limit, start),
            Some(60),
            "a fresh window reports the full window"
        );

        // Reading the remaining time must not itself consume budget: were the
        // query implemented as a `check`, this call would trip the limit.
        let midway = start + Duration::from_secs(20);
        assert_eq!(limiter.retry_after_secs("s", limit, midway), Some(40));
        assert!(limiter.check_at("s", limit, midway).is_err());
        assert_eq!(
            limiter.retry_after_secs("s", limit, midway),
            Some(40),
            "the window start did not move"
        );

        // Once it elapses there is nothing left to wait for.
        let after = start + Duration::from_secs(61);
        assert_eq!(limiter.retry_after_secs("s", limit, after), None);

        // A disabled limit never reports a wait, because it never refuses.
        assert_eq!(
            limiter.retry_after_secs("s", RateLimit::disabled(), start),
            None
        );
    }

    #[test]
    fn failures_below_the_ceiling_are_not_reported_as_a_lock() {
        // The distinction that a first draft of this guard got wrong: a caller
        // who has *failed* is not the same as a caller who is *refused*. The
        // three-failure limit must still accept the second and third attempt.
        let limiter = RateLimiter::new();
        let limit = RateLimit::new(3, 900);
        let now = Instant::now();
        for attempt in 1..=2 {
            assert!(
                limiter.check_at("caller", limit, now).is_ok(),
                "attempt {attempt} of 3 must be admitted"
            );
            assert_eq!(
                limiter.retry_after_secs("caller", limit, now),
                None,
                "attempt {attempt} of 3 is not yet a lock"
            );
        }
        // The third exhausts the budget, and only now is there a wait to report.
        assert!(limiter.check_at("caller", limit, now).is_ok());
        assert_eq!(limiter.retry_after_secs("caller", limit, now), Some(900));
    }

    #[test]
    fn resetting_a_scope_gives_it_a_clean_window() {
        let limiter = RateLimiter::new();
        let limit = RateLimit::new(1, 60);
        let now = Instant::now();
        assert!(limiter.check_at("s", limit, now).is_ok());
        assert!(limiter.check_at("s", limit, now).is_err());
        assert_eq!(limiter.retry_after_secs("s", limit, now), Some(60));

        limiter.reset("s");
        assert_eq!(limiter.retry_after_secs("s", limit, now), None);
        assert!(
            limiter.check_at("s", limit, now).is_ok(),
            "a reset scope must accept again"
        );
        // Resetting an unknown scope is a no-op, not a panic.
        limiter.reset("never-seen");
    }

    #[test]
    fn retry_after_shrinks_as_the_window_progresses() {
        let limiter = RateLimiter::new();
        let limit = RateLimit::new(1, 60);
        let start = Instant::now();
        assert!(limiter.check_at("s", limit, start).is_ok());

        let midway = start + Duration::from_secs(50);
        match limiter.check_at("s", limit, midway) {
            Err(LimitError::RateLimited { retry_after_secs }) => {
                assert!(retry_after_secs <= 10, "got {retry_after_secs}");
            }
            other => panic!("expected rate limiting, got {other:?}"),
        }
    }

    #[test]
    fn limit_errors_map_to_429() {
        assert_eq!(
            LimitError::RateLimited {
                retry_after_secs: 5
            }
            .status_code(),
            429
        );
        assert_eq!(LimitError::ConcurrencyExceeded.status_code(), 429);
        assert_eq!(
            LimitError::RateLimited {
                retry_after_secs: 5
            }
            .retry_after_secs(),
            Some(5)
        );
        assert_eq!(LimitError::ConcurrencyExceeded.retry_after_secs(), None);
    }

    #[test]
    fn prune_drops_only_elapsed_windows() {
        let limiter = RateLimiter::new();
        let limit = RateLimit::new(10, 60);
        let start = Instant::now();
        limiter.check_at("old", limit, start).unwrap();

        let later = start + Duration::from_secs(120);
        limiter.check_at("new", limit, later).unwrap();
        assert_eq!(limiter.tracked_scopes(), 2);

        let removed = limiter.prune(later, Duration::from_secs(60));
        assert_eq!(removed, 1, "only the older scope should be dropped");
        assert_eq!(limiter.tracked_scopes(), 1);
    }

    #[test]
    fn an_unlimited_concurrency_guard_admits_everything() {
        let guard = ConcurrencyGuard::new(0);
        assert!(!guard.is_enabled());
        let permits: Vec<_> = (0..100).map(|_| guard.try_acquire().unwrap()).collect();
        assert_eq!(permits.len(), 100);
    }

    #[test]
    fn the_concurrency_ceiling_is_enforced() {
        let guard = ConcurrencyGuard::new(3);
        let a = guard.try_acquire().unwrap();
        let b = guard.try_acquire().unwrap();
        let c = guard.try_acquire().unwrap();
        assert_eq!(guard.in_flight(), 3);
        assert!(matches!(
            guard.try_acquire(),
            Err(LimitError::ConcurrencyExceeded)
        ));
        drop(a);
        assert_eq!(guard.in_flight(), 2);
        assert!(guard.try_acquire().is_ok(), "a freed slot must be reusable");
        drop((b, c));
    }

    #[test]
    fn a_dropped_permit_always_frees_its_slot() {
        // The guard exists so no early return can leak a slot.
        let guard = ConcurrencyGuard::new(1);
        for _ in 0..1000 {
            let permit = guard.try_acquire().unwrap();
            drop(permit);
        }
        assert_eq!(guard.in_flight(), 0);
    }

    #[test]
    fn concurrency_is_thread_safe() {
        use std::sync::Arc;
        use std::thread;

        let guard = Arc::new(ConcurrencyGuard::new(5));
        let peak = Arc::new(Mutex::new(0usize));
        let mut handles = Vec::new();
        for _ in 0..50 {
            let guard = Arc::clone(&guard);
            let peak = Arc::clone(&peak);
            handles.push(thread::spawn(move || {
                let Ok(permit) = guard.try_acquire() else {
                    return 0usize;
                };
                let current = guard.in_flight();
                let mut peak = peak.lock();
                if current > *peak {
                    *peak = current;
                }
                drop(permit);
                1usize
            }));
        }
        let admitted: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(guard.in_flight(), 0, "every slot must be returned");
        assert!(admitted > 0);
        assert!(*peak.lock() <= 5, "the ceiling must never be exceeded");
    }
}
