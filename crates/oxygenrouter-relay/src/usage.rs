//! Usage extraction from non-streaming responses, per dialect.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use crate::value::Usage;

/// Extract usage from an OpenAI-compatible non-streaming response.
pub fn extract_openai_usage(body: &[u8]) -> Usage {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Usage::default();
    };
    let u = v.get("usage");
    let Some(u) = u else {
        return Usage {
            semantic: "openai".to_string(),
            ..Default::default()
        };
    };
    let mut usage = Usage {
        prompt_tokens: u.get("prompt_tokens").and_then(|x| x.as_i64()).unwrap_or(0),
        completion_tokens: u.get("completion_tokens").and_then(|x| x.as_i64()).unwrap_or(0),
        total_tokens: u.get("total_tokens").and_then(|x| x.as_i64()).unwrap_or(0),
        semantic: "openai".to_string(),
        ..Default::default()
    };
    if let Some(pd) = u.get("prompt_tokens_details") {
        usage.cached_tokens = pd.get("cached_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
    }
    if let Some(cd) = u.get("completion_tokens_details") {
        usage.reasoning_tokens = cd
            .get("reasoning_tokens")
            .and_then(|x| x.as_i64())
            .unwrap_or(0);
    }
    usage
}

/// Extract usage from an Anthropic non-streaming response.
pub fn extract_claude_usage(body: &[u8]) -> Usage {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Usage {
            semantic: "anthropic".to_string(),
            ..Default::default()
        };
    };
    let Some(u) = v.get("usage") else {
        return Usage {
            semantic: "anthropic".to_string(),
            ..Default::default()
        };
    };
    let input = u.get("input_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
    let output = u.get("output_tokens").and_then(|x| x.as_i64()).unwrap_or(0);
    let cache_read = u
        .get("cache_read_input_tokens")
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let cache_creation = u
        .get("cache_creation_input_tokens")
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    Usage {
        prompt_tokens: input + cache_read + cache_creation,
        completion_tokens: output,
        total_tokens: input + cache_read + cache_creation + output,
        semantic: "anthropic".to_string(),
        cached_tokens: cache_read,
        cache_creation_tokens: cache_creation,
        ..Default::default()
    }
}

/// Extract usage from a Gemini `generateContent` response.
pub fn extract_gemini_usage(body: &[u8]) -> Usage {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Usage {
            semantic: "gemini".to_string(),
            ..Default::default()
        };
    };
    let u = v.get("usageMetadata").cloned().unwrap_or_default();
    let prompt = u
        .get("promptTokenCount")
        .and_then(|x| x.as_i64())
        .unwrap_or(0)
        + u.get("toolUsePromptTokenCount")
            .and_then(|x| x.as_i64())
            .unwrap_or(0);
    let completion = u
        .get("candidatesTokenCount")
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let thoughts = u
        .get("thoughtsTokenCount")
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let cached = u
        .get("cachedContentTokenCount")
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion + thoughts,
        total_tokens: u.get("totalTokenCount").and_then(|x| x.as_i64()).unwrap_or(prompt + completion + thoughts),
        semantic: "gemini".to_string(),
        cached_tokens: cached,
        reasoning_tokens: thoughts,
        ..Default::default()
    }
}
