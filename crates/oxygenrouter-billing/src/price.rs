//! Per-request pricing snapshot.
//!
//! Mirrors NewAPI's `types.PriceData`: the resolved ratios for one request,
//! captured once so tiered billing, settlement and logging all see identical
//! numbers even if the operator edits ratios mid-request.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use rust_decimal::Decimal;

/// Quota units per unit of currency. NewAPI: `500 * 1000`, i.e. `$0.002 / 1K`.
pub const QUOTA_PER_UNIT: f64 = 500_000.0;

/// Ratios and prices resolved for a single request.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceData {
    /// Per-token model multiplier. `1.0` means `$0.002 / 1K` tokens.
    pub model_ratio: f64,
    /// Completion (output) multiplier relative to `model_ratio`.
    pub completion_ratio: f64,
    /// Cached-input multiplier relative to `model_ratio`.
    pub cache_ratio: f64,
    /// Cache-write (cache creation) multiplier.
    pub cache_creation_ratio: f64,
    /// Cache-write multiplier for the 5-minute TTL tier.
    pub cache_creation_5m_ratio: f64,
    /// Cache-write multiplier for the 1-hour TTL tier.
    pub cache_creation_1h_ratio: f64,
    pub image_ratio: f64,
    /// Group multiplier applied to the whole charge.
    pub group_ratio: f64,
    /// When set, the request is billed per call rather than per token.
    pub model_price: f64,
    pub use_price: bool,
    /// Additional multipliers (prompt-cache variants, vendor margins, …).
    pub other_ratios: Vec<f64>,
}

impl Default for PriceData {
    fn default() -> Self {
        Self {
            model_ratio: 1.0,
            completion_ratio: 1.0,
            cache_ratio: 1.0,
            cache_creation_ratio: 1.0,
            cache_creation_5m_ratio: 1.0,
            cache_creation_1h_ratio: 1.0,
            image_ratio: 1.0,
            group_ratio: 1.0,
            model_price: 0.0,
            use_price: false,
            other_ratios: Vec::new(),
        }
    }
}

impl PriceData {
    /// `model_ratio * group_ratio` — the combined per-token multiplier.
    pub fn combined_ratio(&self) -> Decimal {
        to_dec(self.model_ratio) * to_dec(self.group_ratio)
    }

    /// Apply every additional ratio, skipping the identity multiplier.
    ///
    /// Non-finite and non-positive ratios are treated as "not set", matching
    /// NewAPI's `isValidOtherRatio`.
    pub fn apply_other_ratios(&self, value: Decimal) -> Decimal {
        let mut out = value;
        for ratio in &self.other_ratios {
            if ratio.is_finite() && *ratio > 0.0 && *ratio != 1.0 {
                out *= to_dec(*ratio);
            }
        }
        out
    }

    /// True when the request is free (a zero model ratio and not per-call).
    pub fn is_free(&self) -> bool {
        !self.use_price && self.model_ratio == 0.0
    }
}

/// Convert an f64 to `Decimal` through its shortest round-trip representation.
///
/// `Decimal::from_f64` keeps the exact binary value (so `0.1` becomes
/// `0.1000000000000000055511151231257827`), whereas NewAPI's
/// `decimal.NewFromFloat` rounds via `strconv` and therefore works with `0.1`.
/// Going through the string form reproduces Go's behaviour.
pub fn to_dec(value: f64) -> Decimal {
    if !value.is_finite() {
        return Decimal::ZERO;
    }
    Decimal::from_str_exact(&shortest_repr(value)).unwrap_or_else(|_| {
        Decimal::from_f64_retain(value).unwrap_or(Decimal::ZERO)
    })
}

/// The shortest decimal string that round-trips to `value`.
fn shortest_repr(value: f64) -> String {
    // Rust's `{}` for f64 produces the shortest representation that round-trips.
    let mut s = format!("{}", value);
    if s.contains('e') || s.contains('E') {
        // Decimal::from_str_exact does not accept exponent notation.
        s = format!("{:.10}", value);
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn decimal_conversion_matches_go_shortest_form() {
        // Go: decimal.NewFromFloat(0.1) -> 0.1, NOT 0.1000000000000000055...
        assert_eq!(to_dec(0.1), Decimal::from_str("0.1").unwrap());
        assert_eq!(to_dec(0.25), Decimal::from_str("0.25").unwrap());
        // The value NewAPI's live DB actually stores for glm-5.3-flash.
        assert_eq!(to_dec(0.2), Decimal::from_str("0.2").unwrap());
    }

    #[test]
    fn non_finite_becomes_zero() {
        assert_eq!(to_dec(f64::NAN), Decimal::ZERO);
        assert_eq!(to_dec(f64::INFINITY), Decimal::ZERO);
    }

    #[test]
    fn other_ratios_skip_identity_and_invalid() {
        let price = PriceData {
            other_ratios: vec![1.0, 2.0, 0.0, -1.0, f64::NAN],
            ..Default::default()
        };
        assert_eq!(price.apply_other_ratios(Decimal::from(10)), Decimal::from(20));
    }

    #[test]
    fn combined_ratio_multiplies_model_and_group() {
        let price = PriceData {
            model_ratio: 0.2,
            group_ratio: 10.0,
            ..Default::default()
        };
        assert_eq!(price.combined_ratio(), Decimal::from(2));
    }

    #[test]
    fn zero_model_ratio_is_free_unless_per_call() {
        let free = PriceData {
            model_ratio: 0.0,
            ..Default::default()
        };
        assert!(free.is_free());

        let per_call = PriceData {
            model_ratio: 0.0,
            use_price: true,
            model_price: 0.04,
            ..Default::default()
        };
        assert!(!per_call.is_free());
    }
}
