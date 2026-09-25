//! Quota conversion and saturation policy.
//!
//! Mirrors NewAPI's `common/quota_math.go`. Single-request charges are bounded to
//! the int32 domain; wallet and top-up values use a JavaScript-safe 64-bit domain
//! because they are also exactly representable as f64.
//!
//! The rounding rule matters for parity: `round` uses **half away from zero**
//! (Rust's `f64::round`), matching Go's `math.Round`, not banker's rounding.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

/// Upper bound for a single request's quota, mirroring `math.MaxInt32`.
pub const MAX_QUOTA: i64 = i32::MAX as i64;
/// Lower bound for a single request's quota, mirroring `math.MinInt32`.
pub const MIN_QUOTA: i64 = i32::MIN as i64;
/// Upper bound for wallet mutations: 2^53 - 1.
pub const MAX_WALLET_QUOTA: i64 = (1i64 << 53) - 1;

/// Why a quota conversion had to be saturated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaClampKind {
    Overflow,
    Underflow,
    NaN,
}

/// A saturation event, reported so callers can record it on the related log.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaClamp {
    pub op: &'static str,
    pub kind: QuotaClampKind,
    pub original: f64,
    pub clamped: i64,
}

impl std::fmt::Display for QuotaClamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "quota conversion ({}) {:?}: original={}, clamped={}",
            self.op, self.kind, self.original, self.clamped
        )
    }
}

/// Saturate an already-rounded single-request quota to the int32 domain.
fn saturate_bounded(value: f64, op: &'static str, max: i64, min: i64) -> (i64, Option<QuotaClamp>) {
    if value.is_nan() {
        return (
            0,
            Some(QuotaClamp {
                op,
                kind: QuotaClampKind::NaN,
                original: value,
                clamped: 0,
            }),
        );
    }
    if value > max as f64 {
        return (
            max,
            Some(QuotaClamp {
                op,
                kind: QuotaClampKind::Overflow,
                original: value,
                clamped: max,
            }),
        );
    }
    if value < min as f64 {
        return (
            min,
            Some(QuotaClamp {
                op,
                kind: QuotaClampKind::Underflow,
                original: value,
                clamped: min,
            }),
        );
    }

    // Go's `int(value)` truncates toward zero. Guard the i64 cast the same way.
    let truncated = value.trunc();
    if truncated >= i64::MIN as f64 && truncated <= i64::MAX as f64 {
        (truncated as i64, None)
    } else {
        let clamped = if truncated > 0.0 { max } else { min };
        (
            clamped,
            Some(QuotaClamp {
                op,
                kind: if truncated > 0.0 {
                    QuotaClampKind::Overflow
                } else {
                    QuotaClampKind::Underflow
                },
                original: value,
                clamped,
            }),
        )
    }
}

/// Truncate toward zero with saturation, in the single-request domain.
///
/// Use for products of prices, ratios and user-controlled multipliers.
pub fn quota_from_f64(value: f64) -> i64 {
    quota_from_f64_checked(value).0
}

pub fn quota_from_f64_checked(value: f64) -> (i64, Option<QuotaClamp>) {
    saturate_bounded(value, "QuotaFromFloat", MAX_QUOTA, MIN_QUOTA)
}

/// Round half away from zero with saturation, in the single-request domain.
///
/// Every tiered billing path must use this to avoid ±1 discrepancies.
pub fn quota_round(value: f64) -> i64 {
    quota_round_checked(value).0
}

pub fn quota_round_checked(value: f64) -> (i64, Option<QuotaClamp>) {
    let rounded = if value.is_nan() {
        f64::NAN
    } else {
        // f64::round is "round half away from zero", same as Go's math.Round.
        value.round()
    };
    saturate_bounded(rounded, "QuotaRound", MAX_QUOTA, MIN_QUOTA)
}

/// Convert a decimal quota with rounding, in the single-request domain.
pub fn quota_from_decimal(d: Decimal) -> i64 {
    quota_from_decimal_checked(d).0
}

pub fn quota_from_decimal_checked(d: Decimal) -> (i64, Option<QuotaClamp>) {
    let rounded = d.round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    match rounded.to_f64() {
        Some(f) => saturate_bounded(f, "QuotaFromDecimal", MAX_QUOTA, MIN_QUOTA),
        None => (
            0,
            Some(QuotaClamp {
                op: "QuotaFromDecimal",
                kind: QuotaClampKind::NaN,
                original: f64::NAN,
                clamped: 0,
            }),
        ),
    }
}

/// Convert a wallet/top-up decimal in the JavaScript-safe 64-bit domain.
pub fn wallet_quota_from_decimal(d: Decimal) -> Result<i64, QuotaClamp> {
    let rounded = d.round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    let value = rounded.to_f64().unwrap_or(f64::NAN);
    let (quota, clamp) = saturate_bounded(
        value,
        "WalletQuotaFromDecimal",
        MAX_WALLET_QUOTA,
        -MAX_WALLET_QUOTA,
    );
    match clamp {
        Some(c) => Err(c),
        None => Ok(quota),
    }
}

/// Reject a single-request quota that would be saturated rather than silently
/// accepting the clamped value (used by strict pre-consume paths).
pub fn quota_from_decimal_strict(d: Decimal) -> Result<i64, QuotaClamp> {
    match quota_from_decimal_checked(d) {
        (_, Some(c)) => Err(c),
        (quota, None) => Ok(quota),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).expect("valid decimal")
    }

    #[test]
    fn truncates_toward_zero_like_go_int() {
        // Go's int(2.9) == 2 and int(-2.9) == -2.
        assert_eq!(quota_from_f64(2.9), 2);
        assert_eq!(quota_from_f64(-2.9), -2);
    }

    #[test]
    fn rounds_half_away_from_zero_like_math_round() {
        // Go's math.Round(2.5) == 3 and math.Round(-2.5) == -3.
        assert_eq!(quota_round(2.5), 3);
        assert_eq!(quota_round(-2.5), -3);
        assert_eq!(quota_round(2.4), 2);
        assert_eq!(quota_round(-2.4), -2);
    }

    #[test]
    fn decimal_rounds_half_away_from_zero() {
        assert_eq!(quota_from_decimal(dec("2.5")), 3);
        assert_eq!(quota_from_decimal(dec("-2.5")), -3);
        assert_eq!(quota_from_decimal(dec("2.4999")), 2);
    }

    #[test]
    fn saturates_at_int32_bounds() {
        assert_eq!(quota_round(0.0), 0);

        let (q, clamp) = quota_round_checked(1e30);
        assert_eq!(q, MAX_QUOTA);
        assert_eq!(clamp.map(|c| c.kind), Some(QuotaClampKind::Overflow));

        let (q, clamp) = quota_round_checked(-1e30);
        assert_eq!(q, MIN_QUOTA);
        assert_eq!(clamp.map(|c| c.kind), Some(QuotaClampKind::Underflow));
    }

    #[test]
    fn nan_becomes_zero_and_is_reported() {
        let (q, clamp) = quota_round_checked(f64::NAN);
        assert_eq!(q, 0);
        assert_eq!(clamp.map(|c| c.kind), Some(QuotaClampKind::NaN));
    }

    #[test]
    fn in_range_values_report_no_clamp() {
        let (q, clamp) = quota_round_checked(1234.567);
        assert_eq!(q, 1235);
        assert!(clamp.is_none());
    }

    #[test]
    fn wallet_domain_is_wider_than_request_domain() {
        // 2^53 is beyond int32 but valid for wallets.
        let big = dec("9007199254740991");
        assert_eq!(wallet_quota_from_decimal(big).unwrap(), MAX_WALLET_QUOTA);
        // ...and the same magnitude saturates in the request domain.
        assert_eq!(quota_from_decimal(big), MAX_QUOTA);
    }

    #[test]
    fn wallet_rejects_beyond_javascript_safe_range() {
        let too_big = dec("9007199254740992");
        assert!(wallet_quota_from_decimal(too_big).is_err());
    }

    #[test]
    fn strict_rejects_saturation() {
        assert!(quota_from_decimal_strict(dec("100")).is_ok());
        assert!(quota_from_decimal_strict(dec("99999999999")).is_err());
    }
}
