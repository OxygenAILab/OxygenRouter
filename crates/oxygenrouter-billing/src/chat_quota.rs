//! The chat/text quota formula.
//!
//! A direct port of NewAPI's `calculateTextQuotaSummary` (`service/text_quota.go`)
//! minus the tiered-expression and tool-surcharge extensions, which are additive
//! and land separately.
//!
//! ```text
//! ratio           = model_ratio * group_ratio
//! prompt_parts    = base + cache_read*cache_ratio
//!                        + image_tokens*image_ratio
//!                        + cache_write_tokens*cache_creation_ratio
//! completion_part = completion_tokens * completion_ratio
//! quota           = (prompt_parts + completion_part) * ratio
//! ```
//!
//! Two details decide parity and are easy to get wrong:
//!
//! 1. **Cache tokens are subtracted from the prompt base only for OpenAI
//!    semantics.** Anthropic reports `input_tokens` *excluding* cache reads, so
//!    subtracting there would under-bill.
//! 2. **The base is clamped at zero.** OpenAI cache-write usage reports
//!    unadjusted prefix counts, so `cached + cache_write` can exceed
//!    `prompt_tokens` and the remainder can go negative.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use rust_decimal::Decimal;

use crate::price::{to_dec, PriceData};
use crate::quota_math::{quota_from_decimal_checked, QuotaClamp};
use crate::usage::{BillingUsage, UsageSemantic};

/// The inputs to a charge, plus the numbers needed to explain it in a log.
#[derive(Debug, Clone)]
pub struct ChatQuotaRequest {
    pub usage: BillingUsage,
    pub price: PriceData,
    /// True when the client asked for streaming (recorded, not priced).
    pub is_stream: bool,
}

/// A computed charge together with its audit trail.
#[derive(Debug, Clone)]
pub struct ChatQuotaResult {
    /// Final quota, already saturated to the int32 single-request domain.
    pub quota: i64,
    /// Prompt-side charge before the combined ratio (None for per-call pricing).
    pub prompt_quota: Option<Decimal>,
    /// Completion-side charge before the combined ratio.
    pub completion_quota: Option<Decimal>,
    /// `base + cached*ratio + image*ratio + cache_write*ratio`.
    pub prompt_parts: Option<Decimal>,
    /// The combined `model_ratio * group_ratio`.
    pub ratio: Decimal,
    /// Set when a conversion had to saturate.
    pub clamp: Option<QuotaClamp>,
    /// True when the request was forced to a 1-quota minimum.
    pub minimum_applied: bool,
    /// True when nothing billable was reported, so the charge is zero.
    pub no_billable_usage: bool,
}

/// Compute the charge for one chat/text request.
pub fn compute_chat_quota(req: &ChatQuotaRequest) -> ChatQuotaResult {
    let usage = &req.usage;
    let price = &req.price;

    let ratio = price.combined_ratio();
    let is_anthropic = usage.semantic == UsageSemantic::Anthropic;

    // --- per-call (fixed price) billing -------------------------------------
    if price.use_price {
        let mut total = to_dec(price.model_price) * to_dec(crate::price::QUOTA_PER_UNIT)
            * to_dec(price.group_ratio);
        total = price.apply_other_ratios(total);
        let (quota, clamp) = quota_from_decimal_checked(total);

        return finalize(req, quota, None, None, None, ratio, clamp);
    }

    // --- per-token billing ---------------------------------------------------
    let mut base = to_dec(usage.prompt_tokens as f64);
    let mut prompt_parts = Decimal::ZERO;

    // Cache reads.
    if usage.cached_tokens != 0 {
        if !is_anthropic {
            base -= to_dec(usage.cached_tokens as f64);
        }
        prompt_parts += to_dec(usage.cached_tokens as f64) * to_dec(price.cache_ratio);
    }

    // Cache writes.
    let cache_write_total = usage.cache_write_tokens();
    if cache_write_total != 0 {
        if !is_anthropic {
            base -= to_dec(cache_write_total as f64);
            prompt_parts += to_dec(cache_write_total as f64) * to_dec(price.cache_creation_ratio);
        } else {
            // Anthropic splits cache writes by TTL; each tier has its own ratio.
            let split = usage.cache_creation_5m_tokens + usage.cache_creation_1h_tokens;
            let unsplit = (cache_write_total - split).max(0);
            prompt_parts += to_dec(unsplit as f64) * to_dec(price.cache_creation_ratio);
            prompt_parts +=
                to_dec(usage.cache_creation_5m_tokens as f64) * to_dec(price.cache_creation_5m_ratio);
            prompt_parts +=
                to_dec(usage.cache_creation_1h_tokens as f64) * to_dec(price.cache_creation_1h_ratio);
        }
    }

    // Image input tokens are billed at their own ratio.
    if usage.image_tokens != 0 {
        base -= to_dec(usage.image_tokens as f64);
        prompt_parts += to_dec(usage.image_tokens as f64) * to_dec(price.image_ratio);
    }

    // OpenAI cache-write usage can make the remainder negative; never let that
    // turn into a credit.
    if base < Decimal::ZERO {
        base = Decimal::ZERO;
    }

    prompt_parts += base;
    let completion = to_dec(usage.completion_tokens as f64) * to_dec(price.completion_ratio);

    let mut total = (prompt_parts + completion) * ratio;
    total = price.apply_other_ratios(total);

    // Preserve the "ratio is non-zero but the product rounds to zero" rule.
    if !ratio.is_zero() && total <= Decimal::ZERO {
        total = Decimal::ONE;
    }

    let (quota, clamp) = quota_from_decimal_checked(total);

    finalize(
        req,
        quota,
        Some(prompt_parts),
        Some(completion),
        Some(prompt_parts),
        ratio,
        clamp,
    )
}

/// Apply the "no billable usage => zero, non-zero ratio & zero quota => 1" tail.
fn finalize(
    req: &ChatQuotaRequest,
    quota: i64,
    prompt_quota: Option<Decimal>,
    completion_quota: Option<Decimal>,
    prompt_parts: Option<Decimal>,
    ratio: Decimal,
    clamp: Option<QuotaClamp>,
) -> ChatQuotaResult {
    let billable_tokens = req.usage.has_billable_tokens();
    let billable = if req.price.use_price {
        true
    } else {
        billable_tokens
    };

    if !billable {
        return ChatQuotaResult {
            quota: 0,
            prompt_quota,
            completion_quota,
            prompt_parts,
            ratio,
            clamp,
            minimum_applied: false,
            no_billable_usage: true,
        };
    }

    if !ratio.is_zero() && quota == 0 {
        return ChatQuotaResult {
            quota: 1,
            prompt_quota,
            completion_quota,
            prompt_parts,
            ratio,
            clamp,
            minimum_applied: true,
            no_billable_usage: false,
        };
    }

    ChatQuotaResult {
        quota,
        prompt_quota,
        completion_quota,
        prompt_parts,
        ratio,
        clamp,
        minimum_applied: false,
        no_billable_usage: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::BillingUsage;

    fn price(model: f64, completion: f64, group: f64) -> PriceData {
        PriceData {
            model_ratio: model,
            completion_ratio: completion,
            group_ratio: group,
            ..Default::default()
        }
    }

    fn openai_usage(prompt: i64, completion: i64) -> BillingUsage {
        BillingUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            semantic: UsageSemantic::OpenAi,
            ..Default::default()
        }
    }

    #[test]
    fn plain_openai_charge_matches_hand_computation() {
        // glm-5.3-flash live ratios: model 0.2, completion 3, group 10.
        // (1000*1 + 500*3) * (0.2*10) = 2500 * 2 = 5000
        let req = ChatQuotaRequest {
            usage: openai_usage(1000, 500),
            price: price(0.2, 3.0, 10.0),
            is_stream: false,
        };
        let result = compute_chat_quota(&req);
        assert_eq!(result.quota, 5000);
        assert_eq!(result.ratio, Decimal::from(2));
        assert!(!result.minimum_applied);
    }

    #[test]
    fn cached_tokens_are_subtracted_from_base_for_openai() {
        // prompt=1000 of which 400 cached; cache ratio 0.2.
        // base 600 + cached 400*0.2=80 => 680; completion 0
        // 680 * (1*1) = 680
        let usage = BillingUsage {
            prompt_tokens: 1000,
            cached_tokens: 400,
            semantic: UsageSemantic::OpenAi,
            ..Default::default()
        };
        let req = ChatQuotaRequest {
            usage,
            price: PriceData {
                cache_ratio: 0.2,
                ..price(1.0, 1.0, 1.0)
            },
            is_stream: false,
        };
        assert_eq!(compute_chat_quota(&req).quota, 680);
    }

    #[test]
    fn anthropic_does_not_subtract_cached_tokens() {
        // Anthropic's input_tokens already exclude cache reads, so the whole
        // 1000 is base and 400 is billed additionally at the cache ratio.
        // 1000 + 400*0.2 = 1080
        let usage = BillingUsage {
            prompt_tokens: 1000,
            cached_tokens: 400,
            semantic: UsageSemantic::Anthropic,
            ..Default::default()
        };
        let req = ChatQuotaRequest {
            usage,
            price: PriceData {
                cache_ratio: 0.2,
                ..price(1.0, 1.0, 1.0)
            },
            is_stream: false,
        };
        assert_eq!(compute_chat_quota(&req).quota, 1080);
    }

    #[test]
    fn negative_base_is_clamped_to_zero() {
        // OpenAI cache-write reporting can push the remainder below zero.
        let usage = BillingUsage {
            prompt_tokens: 100,
            cache_creation_tokens: 300,
            semantic: UsageSemantic::OpenAi,
            ..Default::default()
        };
        let req = ChatQuotaRequest {
            usage,
            price: PriceData {
                cache_creation_ratio: 1.25,
                ..price(1.0, 1.0, 1.0)
            },
            is_stream: false,
        };
        // base clamps to 0, cache-write contributes 300*1.25 = 375
        assert_eq!(compute_chat_quota(&req).quota, 375);
    }

    #[test]
    fn anthropic_split_cache_write_uses_per_tier_ratios() {
        // 100 base input + 0 unsplit cache-write + 40*2.0 (5m) + 30*4.0 (1h)
        // = 100 + 80 + 120 = 300
        let usage = BillingUsage {
            prompt_tokens: 100,
            cache_creation_5m_tokens: 40,
            cache_creation_1h_tokens: 30,
            semantic: UsageSemantic::Anthropic,
            ..Default::default()
        };
        let req = ChatQuotaRequest {
            usage,
            price: PriceData {
                cache_creation_5m_ratio: 2.0,
                cache_creation_1h_ratio: 4.0,
                ..price(1.0, 1.0, 1.0)
            },
            is_stream: false,
        };
        assert_eq!(compute_chat_quota(&req).quota, 300);
    }

    #[test]
    fn zero_tokens_charge_nothing() {
        let req = ChatQuotaRequest {
            usage: openai_usage(0, 0),
            price: price(0.2, 3.0, 10.0),
            is_stream: false,
        };
        let result = compute_chat_quota(&req);
        assert_eq!(result.quota, 0);
        assert!(result.no_billable_usage);
    }

    #[test]
    fn tiny_charge_rounds_up_to_one_quota() {
        // A very small ratio makes the product round to 0 (< 0.5 quota).
        let req = ChatQuotaRequest {
            usage: openai_usage(1, 0),
            price: price(0.0001, 1.0, 1.0),
            is_stream: false,
        };
        let result = compute_chat_quota(&req);
        assert_eq!(result.quota, 1);
        assert!(result.minimum_applied);
    }

    #[test]
    fn zero_ratio_is_free_not_minimum() {
        let req = ChatQuotaRequest {
            usage: openai_usage(1000, 1000),
            price: price(0.0, 1.0, 10.0),
            is_stream: false,
        };
        let result = compute_chat_quota(&req);
        assert_eq!(result.quota, 0);
        assert!(!result.minimum_applied);
    }

    #[test]
    fn per_call_pricing_ignores_tokens() {
        // dall-e-3 style: $0.04 per call.
        let req = ChatQuotaRequest {
            usage: openai_usage(0, 0),
            price: PriceData {
                use_price: true,
                model_price: 0.04,
                group_ratio: 1.0,
                ..Default::default()
            },
            is_stream: false,
        };
        let result = compute_chat_quota(&req);
        assert_eq!(result.quota, 20_000); // 0.04 * 500_000
        assert!(!result.no_billable_usage);
    }

    #[test]
    fn image_tokens_use_the_image_ratio() {
        // base 1000-500=500, image 500*2=1000 => 1500
        let usage = BillingUsage {
            prompt_tokens: 1000,
            image_tokens: 500,
            semantic: UsageSemantic::OpenAi,
            ..Default::default()
        };
        let req = ChatQuotaRequest {
            usage,
            price: PriceData {
                image_ratio: 2.0,
                ..price(1.0, 1.0, 1.0)
            },
            is_stream: false,
        };
        assert_eq!(compute_chat_quota(&req).quota, 1500);
    }

    #[test]
    fn fractional_ratios_use_decimal_not_binary_float() {
        // 3 tokens at ratio 0.1*1 => 0.30000000000000004 in f64; must round to 0
        // with exact decimals, then to the 1-quota minimum since ratio != 0.
        let req = ChatQuotaRequest {
            usage: openai_usage(3, 0),
            price: price(0.1, 1.0, 1.0),
            is_stream: false,
        };
        assert_eq!(compute_chat_quota(&req).quota, 1);
    }

    #[test]
    fn large_prompt_accumulates_correctly() {
        // 100k prompt + 50k completion at model 1, completion 1, group 1 => 150k
        let req = ChatQuotaRequest {
            usage: openai_usage(100_000, 50_000),
            price: price(1.0, 1.0, 1.0),
            is_stream: false,
        };
        assert_eq!(compute_chat_quota(&req).quota, 150_000);
    }
}
