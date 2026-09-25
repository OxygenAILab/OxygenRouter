//! Billing-facing usage: dialect-preserving token details plus the semantic that
//! produced them.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use serde::{Deserialize, Serialize};

/// Which provider dialect produced these numbers. This changes the *formula*,
/// not just the labels: OpenAI reports cache reads as a subset of the prompt
/// total, while Anthropic reports them alongside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageSemantic {
    #[default]
    OpenAi,
    Anthropic,
    Gemini,
    /// Numbers came from a local estimator, not from upstream.
    Estimate,
}

/// Token counts for one request, split by the modality the price tables use.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BillingUsage {
    /// Total input tokens as reported upstream (including any cached subset).
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    /// Cached (cache-read) input tokens.
    pub cached_tokens: i64,
    /// Cache-write tokens with an unspecified TTL.
    pub cache_creation_tokens: i64,
    /// Cache-write tokens with a 5-minute TTL.
    pub cache_creation_5m_tokens: i64,
    /// Cache-write tokens with a 1-hour TTL.
    pub cache_creation_1h_tokens: i64,
    pub image_tokens: i64,
    pub audio_tokens: i64,
    pub semantic: UsageSemantic,
}

impl BillingUsage {
    pub fn total_tokens(&self) -> i64 {
        self.prompt_tokens + self.completion_tokens
    }

    /// Normalized cache-write total.
    ///
    /// When split 5m/1h counts are present they are the source of truth; the
    /// unsplit field is only used when it exceeds their sum (some providers
    /// report a combined figure as well as a split).
    pub fn cache_write_tokens(&self) -> i64 {
        if self.cache_creation_5m_tokens > 0 || self.cache_creation_1h_tokens > 0 {
            let split = self.cache_creation_5m_tokens + self.cache_creation_1h_tokens;
            if self.cache_creation_tokens > split {
                return self.cache_creation_tokens;
            }
            return split;
        }
        self.cache_creation_tokens
    }

    /// True when this usage should incur a charge attributable to tokens.
    pub fn has_billable_tokens(&self) -> bool {
        self.total_tokens() > 0 || self.cache_write_tokens() > 0
    }
}

/// An OpenAI-style prompt-token-details object.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageDetails {
    #[serde(default)]
    pub cached_tokens: i64,
    #[serde(default)]
    pub cache_creation_tokens: i64,
    #[serde(default)]
    pub cache_creation_tokens_5m: i64,
    #[serde(default)]
    pub cache_creation_tokens_1h: i64,
    #[serde(default)]
    pub image_tokens: i64,
    #[serde(default)]
    pub audio_tokens: i64,
    #[serde(default)]
    pub text_tokens: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_write_prefers_split_when_present() {
        let usage = BillingUsage {
            cache_creation_tokens: 10,
            cache_creation_5m_tokens: 4,
            cache_creation_1h_tokens: 3,
            ..Default::default()
        };
        // split sum (7) < combined (10) => combined wins
        assert_eq!(usage.cache_write_tokens(), 10);
    }

    #[test]
    fn cache_write_uses_split_sum_when_larger() {
        let usage = BillingUsage {
            cache_creation_tokens: 0,
            cache_creation_5m_tokens: 40,
            cache_creation_1h_tokens: 30,
            ..Default::default()
        };
        assert_eq!(usage.cache_write_tokens(), 70);
    }

    #[test]
    fn cache_write_falls_back_to_unsplit() {
        let usage = BillingUsage {
            cache_creation_tokens: 12,
            ..Default::default()
        };
        assert_eq!(usage.cache_write_tokens(), 12);
    }

    #[test]
    fn zero_usage_is_not_billable() {
        assert!(!BillingUsage::default().has_billable_tokens());
    }
}
