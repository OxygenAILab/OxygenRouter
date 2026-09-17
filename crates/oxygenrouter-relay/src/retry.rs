//! Retry classification, mirroring NewAPI's `ShouldRetryRelayError`.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

/// Status codes that are always retried.
///
/// Ranges (NewAPI `AutomaticRetryStatusCodeRanges`):
/// 1xx, 3xx, 401-407, 409-499, 500-503, 505-523, 525-599.
/// Notably NOT 400, 408, 504, 524.
pub fn is_retryable_status(status: u16) -> bool {
    match status {
        100..=199 => true,
        300..=399 => true,
        401..=407 => true,
        409..=499 => true,
        500..=503 => true,
        505..=523 => true,
        525..=599 => true,
        _ => false,
    }
}

/// Status codes that must never be retried even though they would otherwise
/// match a retryable range.
pub fn is_always_skip_retry_status(status: u16) -> bool {
    matches!(status, 504 | 524)
}

/// The single status that triggers automatic channel disable by default.
pub fn is_auto_disable_status(status: u16) -> bool {
    status == 401
}

/// Combined decision.
pub fn should_retry(status: u16, remaining_retries: u32) -> bool {
    if remaining_retries == 0 {
        return false;
    }
    if (200..300).contains(&status) {
        return false;
    }
    if is_always_skip_retry_status(status) {
        return false;
    }
    is_retryable_status(status)
}

/// Exponential backoff with full jitter, in milliseconds.
///
/// NewAPI retries immediately with no sleep; this is a deliberate improvement.
/// `attempt` is 0-based.
pub fn backoff_ms(attempt: u32, base_ms: u64, cap_ms: u64) -> u64 {
    let exp = base_ms.saturating_mul(1u64 << attempt.min(10));
    let capped = exp.min(cap_ms);
    if capped == 0 {
        return 0;
    }
    // Full jitter: random in [0, capped]. Deterministic-ish fallback if no rng.
    jitter_below(capped)
}

fn jitter_below(max: u64) -> u64 {
    if max == 0 {
        return 0;
    }
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
        .unwrap_or(0);
    nanos % max
}
