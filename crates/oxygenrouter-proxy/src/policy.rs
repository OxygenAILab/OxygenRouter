//! The request policy: how a failed attempt is retried, when a channel is
//! disabled, and how the health test brings it back.
//!
//! This mirrors NewAPI's `RequestPolicySnapshot`
//! (`model/request_policy.go`): the option values are read once into a snapshot
//! and the running decisions come from that snapshot, so the console's write and
//! the proxy's read never disagree about which rules are in force. The snapshot
//! is rebuilt by `AppState::reload_request_policy` after an option write.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use oxygenrouter_core::{
    matches_status_ranges, parse_http_status_ranges, Database, ALWAYS_SKIP_RETRY_STATUS,
    DEFAULT_DISABLE_STATUS_RANGES, DEFAULT_RETRY_STATUS_RANGES,
};

use crate::upstream::ProxyError;

/// The reference's three health-test modes (`monitor_setting.go:19`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelTestMode {
    /// Test every channel each round; failures disable when auto-ban is on.
    ScheduledAll,
    /// Only test channels that the automatic flow has already disabled.
    AutoBanOnly,
    /// Test nothing proactively; disabled channels recover on real traffic.
    PassiveRecovery,
}

impl ChannelTestMode {
    pub fn from_setting(raw: &str) -> Self {
        match raw.trim() {
            "auto_ban_only" => ChannelTestMode::AutoBanOnly,
            "passive_recovery" => ChannelTestMode::PassiveRecovery,
            _ => ChannelTestMode::ScheduledAll,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ChannelTestMode::ScheduledAll => "scheduled_all",
            ChannelTestMode::AutoBanOnly => "auto_ban_only",
            ChannelTestMode::PassiveRecovery => "passive_recovery",
        }
    }
}

/// The `monitor_setting.*` group plus the response-time threshold.
#[derive(Debug, Clone)]
pub struct HealthTestPolicy {
    pub enabled: bool,
    pub interval_minutes: f64,
    /// 1..=32, clamped like `NormalizeChannelTestConcurrency`.
    pub concurrency: usize,
    pub mode: ChannelTestMode,
    /// Seconds; a test slower than this counts as a failure. `0` disables the
    /// rule, reproducing the reference's "an impossible threshold".
    pub response_time_threshold_seconds: f64,
}

impl Default for HealthTestPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_minutes: 10.0,
            concurrency: 1,
            mode: ChannelTestMode::ScheduledAll,
            response_time_threshold_seconds: 5.0,
        }
    }
}

/// A validated snapshot of every request-policy option.
#[derive(Debug, Clone)]
pub struct RequestPolicy {
    /// Additional attempts after the first.
    pub max_retries: u32,
    /// `0` retries immediately, matching the reference; anything else sleeps.
    pub backoff_base_ms: u64,
    pub backoff_cap_ms: u64,
    pub backoff_mode: BackoffMode,
    pub retry_ranges: Vec<(u16, u16)>,
    pub auto_disable_enabled: bool,
    pub disable_ranges: Vec<(u16, u16)>,
    /// Lowercased, trimmed, non-empty.
    pub disable_keywords: Vec<String>,
    pub auto_enable_enabled: bool,
    /// Consecutive failures before the scheduler stops selecting a channel.
    /// An OxygenRouter extension (the reference disables immediately on a
    /// matched failure instead of counting); `0` disables the rule.
    pub failure_streak_threshold: u32,
    pub health: HealthTestPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackoffMode {
    None,
    Fixed,
    Linear,
    Exponential,
}

impl BackoffMode {
    pub fn from_setting(raw: &str) -> Self {
        match raw.trim() {
            "none" => BackoffMode::None,
            "fixed" => BackoffMode::Fixed,
            "linear" => BackoffMode::Linear,
            _ => BackoffMode::Exponential,
        }
    }
}

impl Default for RequestPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            backoff_base_ms: 500,
            backoff_cap_ms: 30_000,
            backoff_mode: BackoffMode::Exponential,
            retry_ranges: parse_http_status_ranges(DEFAULT_RETRY_STATUS_RANGES)
                .expect("shipped retry ranges parse"),
            auto_disable_enabled: false,
            disable_ranges: parse_http_status_ranges(DEFAULT_DISABLE_STATUS_RANGES)
                .expect("shipped disable range parses"),
            disable_keywords: Vec::new(),
            auto_enable_enabled: false,
            failure_streak_threshold: 3,
            health: HealthTestPolicy::default(),
        }
    }
}

impl RequestPolicy {
    /// Read the current option values into one snapshot.
    ///
    /// A malformed rule string is not something the console can produce (writes
    /// are validated with the same parser), but a hand-edited database can: it
    /// falls back to the shipped default and says so, rather than silently
    /// retrying nothing.
    pub fn from_db(db: &Database) -> Self {
        // Every value is read through `typed_setting`, so an instance that has
        // never written a key runs on the schema's shipped default. Reading the
        // raw row instead would silently start with an empty rule -- the first
        // live run of this policy found exactly that on the keyword list, whose
        // default was declared but never applied.
        let retry_raw = db
            .typed_setting("AutomaticRetryStatusCodes", DEFAULT_RETRY_STATUS_RANGES.to_string());
        let retry_ranges = match parse_http_status_ranges(&retry_raw) {
            Ok(ranges) => ranges,
            Err(error) => {
                eprintln!(
                    "[OxygenRouter] AutomaticRetryStatusCodes ignored ({error}); using the shipped ranges"
                );
                parse_http_status_ranges(DEFAULT_RETRY_STATUS_RANGES).unwrap_or_default()
            }
        };
        let disable_raw = db.typed_setting(
            "AutomaticDisableStatusCodes",
            DEFAULT_DISABLE_STATUS_RANGES.to_string(),
        );
        let disable_ranges = match parse_http_status_ranges(&disable_raw) {
            Ok(ranges) => ranges,
            Err(error) => {
                eprintln!(
                    "[OxygenRouter] AutomaticDisableStatusCodes ignored ({error}); using the shipped range"
                );
                parse_http_status_ranges(DEFAULT_DISABLE_STATUS_RANGES).unwrap_or_default()
            }
        };
        let keywords = db
            .typed_setting("AutomaticDisableKeywords", String::new())
            .lines()
            .map(|line| line.trim().to_ascii_lowercase())
            .filter(|line| !line.is_empty())
            .collect();

        let max_retries = db.typed_setting("RetryTimes", 3_i64).max(0) as u32;
        let backoff_base_ms = db.typed_setting("RetryIntervalMs", 500_i64).max(0) as u64;
        let backoff_mode =
            BackoffMode::from_setting(&db.typed_setting("RetryBackoff", "exponential".to_string()));
        let failure_streak_threshold = db
            .typed_setting("ChannelFailureStreakThreshold", 3_i64)
            .max(0) as u32;
        let response_time_threshold_seconds = db
            .typed_setting("ChannelDisableThreshold", 5.0_f64)
            .max(0.0);
        let interval_minutes = db
            .typed_setting("monitor_setting.auto_test_channel_minutes", 10.0_f64)
            .max(0.0);
        let concurrency = db
            .typed_setting("monitor_setting.channel_test_concurrency", 1_i64)
            .clamp(1, 32) as usize;
        let mode = ChannelTestMode::from_setting(&db.typed_setting(
            "monitor_setting.channel_test_mode",
            "scheduled_all".to_string(),
        ));

        Self {
            max_retries,
            backoff_base_ms,
            backoff_cap_ms: backoff_base_ms.saturating_mul(60).max(30_000),
            backoff_mode,
            retry_ranges,
            auto_disable_enabled: db.typed_setting("AutomaticDisableChannelEnabled", false),
            disable_ranges,
            disable_keywords: keywords,
            auto_enable_enabled: db.typed_setting("AutomaticEnableChannelEnabled", false),
            failure_streak_threshold,
            health: HealthTestPolicy {
                enabled: db.typed_setting("monitor_setting.auto_test_channel_enabled", false),
                interval_minutes,
                concurrency,
                mode,
                response_time_threshold_seconds,
            },
        }
    }

    /// Whether a failed attempt may move to another channel.
    ///
    /// This is the reference's `DecideRelayRetry` (`service/relay_error.go`):
    /// 2xx never retries, 504/524 are always excluded even when a range would
    /// match, codes outside 100..=599 retry (an unrecognized status is treated
    /// as a transport problem), and everything else follows the configured
    /// ranges.
    pub fn should_retry(&self, error: &ProxyError, remaining_attempts: u32) -> bool {
        if remaining_attempts == 0 {
            return false;
        }
        match error {
            // No candidate is a routing result, not a channel fault; a retry
            // would look for the same empty set.
            ProxyError::NoChannel => false,
            // Context overflow is handled by the long-context fallback before
            // the retry decision, and stays retryable when no fallback exists.
            ProxyError::ContextExceeded(_) => true,
            ProxyError::Network(_) => true,
            ProxyError::Internal(_) | ProxyError::InvalidRequest(_) => false,
            ProxyError::Upstream { status, .. } => {
                let status = *status;
                if (200..300).contains(&status) {
                    return false;
                }
                if ALWAYS_SKIP_RETRY_STATUS.contains(&status) {
                    return false;
                }
                if !(100..=599).contains(&status) {
                    return true;
                }
                matches_status_ranges(&self.retry_ranges, status)
            }
        }
    }

    /// Whether this failure should stand the channel down.
    ///
    /// Mirrors `service.ShouldDisableChannel`: the switch gates everything, a
    /// matched status or keyword disables, and the caller applies both the
    /// switch and the per-channel auto-ban flag.
    pub fn should_disable(&self, error: &ProxyError) -> bool {
        if !self.auto_disable_enabled {
            return false;
        }
        match error {
            ProxyError::Upstream { status, body } => {
                matches_status_ranges(&self.disable_ranges, *status)
                    || self.matches_keyword(body)
            }
            ProxyError::Network(message)
            | ProxyError::Internal(message) => self.matches_keyword(message),
            // A context overflow is a property of the request, not a signal
            // about the channel; standing the channel down would be blaming it
            // for the caller's prompt.
            ProxyError::ContextExceeded(_) => false,
            ProxyError::NoChannel | ProxyError::InvalidRequest(_) => false,
        }
    }

    fn matches_keyword(&self, haystack: &str) -> bool {
        if self.disable_keywords.is_empty() {
            return false;
        }
        let haystack = haystack.to_ascii_lowercase();
        self.disable_keywords
            .iter()
            .any(|keyword| haystack.contains(keyword.as_str()))
    }

    /// The reason stored on the channel, matching the reference's use of the
    /// masked error text and its log-preview truncation.
    pub fn disable_reason(&self, error: &ProxyError) -> String {
        preview(&error.to_string(), 200)
    }

    /// Milliseconds to wait before retry `attempt` (0-based). `0` means retry
    /// immediately.
    pub fn retry_delay_ms(&self, attempt: u32) -> u64 {
        match self.backoff_mode {
            BackoffMode::None => 0,
            BackoffMode::Fixed => self.backoff_base_ms,
            BackoffMode::Linear => self
                .backoff_base_ms
                .saturating_mul(attempt as u64 + 1),
            BackoffMode::Exponential => {
                crate::retry::backoff_ms(attempt, self.backoff_base_ms, self.backoff_cap_ms)
            }
        }
    }
}

fn preview(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push('\u{2026}');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy_with_ranges(ranges: &str) -> RequestPolicy {
        RequestPolicy {
            retry_ranges: parse_http_status_ranges(ranges).unwrap(),
            ..Default::default()
        }
    }

    #[test]
    fn retry_decision_follows_the_configured_ranges() {
        let policy = policy_with_ranges("500-599");
        let upstream = |status| ProxyError::Upstream {
            status,
            body: String::new(),
        };
        assert!(policy.should_retry(&upstream(500), 1));
        assert!(!policy.should_retry(&upstream(429), 1));
        // 2xx and the two always-skip codes never retry, even inside a range.
        assert!(!policy.should_retry(&upstream(200), 1));
        assert!(!policy.should_retry(&upstream(504), 1));
        assert!(!policy.should_retry(&upstream(524), 1));
        // An unrecognized status is treated as transport trouble.
        assert!(policy.should_retry(&upstream(700), 1));
        // Budget and local refusals stop it.
        assert!(!policy.should_retry(&upstream(500), 0));
        assert!(!policy.should_retry(&ProxyError::InvalidRequest("x".into()), 3));
        assert!(policy.should_retry(&ProxyError::Network("reset".into()), 3));
    }

    #[test]
    fn the_shipped_defaults_match_the_reference() {
        let policy = RequestPolicy::default();
        let upstream = |status| ProxyError::Upstream {
            status,
            body: String::new(),
        };
        for status in [401, 403, 429, 500, 503, 505] {
            assert!(policy.should_retry(&upstream(status), 1), "{status} retries");
        }
        for status in [400, 408, 504, 524] {
            assert!(
                !policy.should_retry(&upstream(status), 1),
                "{status} does not retry"
            );
        }
    }

    #[test]
    fn disable_requires_the_switch_and_a_match() {
        let error = ProxyError::Upstream {
            status: 401,
            body: "no".into(),
        };
        let mut policy = RequestPolicy::default();
        assert!(!policy.should_disable(&error), "off by default");
        policy.auto_disable_enabled = true;
        assert!(policy.should_disable(&error), "401 is the shipped disable code");
        let ok = ProxyError::Upstream {
            status: 200,
            body: "no".into(),
        };
        assert!(!policy.should_disable(&ok));
    }

    #[test]
    fn disable_keywords_match_case_insensitively() {
        let mut policy = RequestPolicy::default();
        policy.auto_disable_enabled = true;
        policy.disable_keywords = vec!["your credit balance is too low".into()];
        let error = ProxyError::Upstream {
            status: 400,
            body: r#"{"error":{"message":"Your Credit Balance Is Too Low"}}"#.into(),
        };
        assert!(policy.should_disable(&error));
        let clean = ProxyError::Upstream {
            status: 400,
            body: r#"{"error":{"message":"bad request"}}"#.into(),
        };
        assert!(!policy.should_disable(&clean));
    }

    #[test]
    fn a_snapshot_reads_the_option_store() {
        let db = Database::new(":memory:").expect("db");
        db.set_setting("RetryTimes", "7").unwrap();
        db.set_setting("AutomaticRetryStatusCodes", "500-599").unwrap();
        db.set_setting("AutomaticDisableChannelEnabled", "true").unwrap();
        db.set_setting("AutomaticDisableKeywords", "Bad Key\n\nSecond Line").unwrap();
        db.set_setting("ChannelFailureStreakThreshold", "9").unwrap();
        db.set_setting("monitor_setting.channel_test_concurrency", "40").unwrap();

        let policy = RequestPolicy::from_db(&db);
        assert_eq!(policy.max_retries, 7);
        assert_eq!(policy.retry_ranges, vec![(500, 599)]);
        assert!(policy.auto_disable_enabled);
        assert_eq!(policy.disable_keywords, vec!["bad key", "second line"]);
        assert_eq!(policy.failure_streak_threshold, 9);
        // Out-of-range concurrency in a hand-edited row is clamped, not obeyed.
        assert_eq!(policy.health.concurrency, 32);
    }

    /// An instance that never wrote these keys must still run on the shipped
    /// defaults, not on empty rules.
    ///
    /// The first live run of this feature found the keyword list starting empty
    /// for exactly this reason: the default was declared in the schema but the
    /// snapshot read the raw row, so a channel answering "Your credit balance is
    /// too low" was never disabled until an operator happened to write the key.
    #[test]
    fn a_fresh_store_falls_back_to_the_shipped_defaults() {
        let db = Database::new(":memory:").expect("db");
        let policy = RequestPolicy::from_db(&db);

        assert_eq!(
            policy.retry_ranges,
            parse_http_status_ranges(DEFAULT_RETRY_STATUS_RANGES).unwrap()
        );
        assert_eq!(
            policy.disable_ranges,
            parse_http_status_ranges(DEFAULT_DISABLE_STATUS_RANGES).unwrap()
        );
        for keyword in [
            "your credit balance is too low",
            "you exceeded your current quota",
            "permission denied",
        ] {
            assert!(
                policy.disable_keywords.iter().any(|entry| entry == keyword),
                "the shipped keyword {keyword:?} must be in force on a fresh store"
            );
        }
    }

    #[test]
    fn backoff_modes_are_distinct() {
        let mut policy = RequestPolicy::default();
        policy.backoff_base_ms = 100;
        policy.backoff_cap_ms = 10_000;
        policy.backoff_mode = BackoffMode::None;
        assert_eq!(policy.retry_delay_ms(3), 0);
        policy.backoff_mode = BackoffMode::Fixed;
        assert_eq!(policy.retry_delay_ms(3), 100);
        policy.backoff_mode = BackoffMode::Linear;
        assert_eq!(policy.retry_delay_ms(3), 400);
        policy.backoff_mode = BackoffMode::Exponential;
        assert!(policy.retry_delay_ms(3) <= 800);
    }
}
