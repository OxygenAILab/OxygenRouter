//! Heuristic token estimation.
//!
//! Used in two places, matching NewAPI's roles:
//! * **pre-consume** — the request body must be priced before the upstream is
//!   called, so the prompt side is estimated rather than counted;
//! * **fallback** — when an upstream returns no `usage`, the prompt estimate is
//!   what gets billed (NewAPI: `GetEstimatePromptTokens`).
//!
//! The multipliers are per-provider because the same text tokenizes differently
//! in different vocabularies: CJK costs ~1.21 tokens/char for Claude but ~0.85
//! for OpenAI, and math symbols cost 4.52 for Claude versus 2.68 for OpenAI.
//!
//! When the `tiktoken` feature is on, [`TokenEstimator::count`] prefers the real
//! BPE encode for OpenAI-family models; the heuristic remains the fallback.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::collections::HashMap;

use parking_lot::RwLock;

/// A model vendor family, which selects the multiplier table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenEstimator {
    OpenAi,
    Gemini,
    Claude,
    /// Falls back to the OpenAI table.
    Unknown,
}

impl TokenEstimator {
    /// Pick the family from a model name, mirroring `EstimateTokenByModel`.
    pub fn for_model(model: &str) -> Self {
        let m = model.to_ascii_lowercase();
        if m.contains("gemini") {
            Self::Gemini
        } else if m.contains("claude") {
            Self::Claude
        } else if m.is_empty() {
            Self::Unknown
        } else {
            Self::OpenAi
        }
    }

    fn multiplier(self) -> &'static Multipliers {
        match self {
            Self::Gemini => &GEMINI,
            Self::Claude => &CLAUDE,
            Self::OpenAi | Self::Unknown => &OPENAI,
        }
    }

    /// Estimate the token count of `text` for this vendor family.
    pub fn count(self, text: &str) -> i64 {
        if text.is_empty() {
            return 0;
        }
        let m = self.multiplier();
        let mut count = 0.0f64;
        // Current run state: was the previous char a letter or a digit?
        let mut run = CharClass::None;

        for ch in text.chars() {
            if ch.is_whitespace() {
                run = CharClass::None;
                count += if ch == '\n' || ch == '\t' {
                    m.newline
                } else {
                    m.space
                };
                continue;
            }

            if is_cjk(ch) {
                run = CharClass::None;
                count += m.cjk;
                continue;
            }

            if is_emoji(ch) {
                run = CharClass::None;
                count += m.emoji;
                continue;
            }

            if ch.is_alphabetic() || ch.is_numeric() {
                let next = if ch.is_numeric() {
                    CharClass::Number
                } else {
                    CharClass::Latin
                };
                // A switch between letters and digits starts a new token; chars
                // inside a run are free.
                if run == CharClass::None || run != next {
                    count += if next == CharClass::Number {
                        m.number
                    } else {
                        m.word
                    };
                    run = next;
                }
                continue;
            }

            run = CharClass::None;
            if is_math_symbol(ch) {
                count += m.math_symbol;
            } else if ch == '@' {
                count += m.at_sign;
            } else if is_url_delim(ch) {
                count += m.url_delim;
            } else {
                count += m.symbol;
            }
        }

        count.ceil() as i64 + m.base_pad
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    None,
    Latin,
    Number,
}

struct Multipliers {
    word: f64,
    number: f64,
    cjk: f64,
    symbol: f64,
    math_symbol: f64,
    url_delim: f64,
    at_sign: f64,
    emoji: f64,
    newline: f64,
    space: f64,
    base_pad: i64,
}

const OPENAI: Multipliers = Multipliers {
    word: 1.02,
    number: 1.55,
    cjk: 0.85,
    symbol: 0.4,
    math_symbol: 2.68,
    url_delim: 1.0,
    at_sign: 2.0,
    emoji: 2.12,
    newline: 0.5,
    space: 0.42,
    base_pad: 0,
};

const CLAUDE: Multipliers = Multipliers {
    word: 1.13,
    number: 1.63,
    cjk: 1.21,
    symbol: 0.4,
    math_symbol: 4.52,
    url_delim: 1.26,
    at_sign: 2.82,
    emoji: 2.6,
    newline: 0.89,
    space: 0.39,
    base_pad: 0,
};

const GEMINI: Multipliers = Multipliers {
    word: 1.15,
    number: 2.8,
    cjk: 0.68,
    symbol: 0.38,
    math_symbol: 1.05,
    url_delim: 1.2,
    at_sign: 2.5,
    emoji: 1.08,
    newline: 1.15,
    space: 0.2,
    base_pad: 0,
};

fn is_cjk(ch: char) -> bool {
    let c = ch as u32;
    // Han
    (0x4E00..=0x9FFF).contains(&c)
        || (0x3400..=0x4DBF).contains(&c)
        || (0xF900..=0xFAFF).contains(&c)
        // Hiragana/Katakana
        || (0x3040..=0x30FF).contains(&c)
        // Hangul
        || (0xAC00..=0xD7A3).contains(&c)
}

fn is_emoji(ch: char) -> bool {
    let c = ch as u32;
    (0x1F300..=0x1F9FF).contains(&c)
        || (0x2600..=0x26FF).contains(&c)
        || (0x2700..=0x27BF).contains(&c)
        || (0x1F600..=0x1F64F).contains(&c)
        || (0x1F900..=0x1F9FF).contains(&c)
        || (0x1FA00..=0x1FAFF).contains(&c)
}

fn is_url_delim(ch: char) -> bool {
    matches!(ch, '/' | ':' | '?' | '&' | '=' | ';' | '#' | '%')
}

fn is_math_symbol(ch: char) -> bool {
    let c = ch as u32;
    matches!(
        ch,
        '∑' | '∫'
            | '∂'
            | '√'
            | '∞'
            | '≤'
            | '≥'
            | '≠'
            | '≈'
            | '±'
            | '×'
            | '÷'
            | '∈'
            | '∉'
            | '∋'
            | '∌'
            | '⊂'
            | '⊃'
            | '⊆'
            | '⊇'
            | '∪'
            | '∩'
            | '∧'
            | '∨'
            | '¬'
            | '∀'
            | '∃'
            | '∄'
            | '∅'
            | '∆'
            | '∇'
            | '∝'
            | '∟'
            | '∠'
            | '∡'
            | '∢'
            | '°'
            | '²'
            | '³'
            | '¹'
            | '⁴'
            | '⁵'
            | '⁶'
            | '⁷'
            | '⁸'
            | '⁹'
            | '⁰'
            | '₀'
            | '₁'
            | '₂'
            | '₃'
            | '₄'
            | '₅'
            | '₆'
            | '₇'
            | '₈'
            | '₉'
    ) || (0x2200..=0x22FF).contains(&c)
        || (0x2A00..=0x2AFF).contains(&c)
        || (0x1D400..=0x1D7FF).contains(&c)
}

/// Estimate the prompt token count for a request body.
///
/// Walks the message contents (including multimodal text parts), the `system`
/// prompt, and any tool definitions, so a tools-heavy request is not priced as
/// if it were a bare string.
pub fn estimate_prompt_tokens(model: &str, body: &serde_json::Value) -> i64 {
    let estimator = TokenEstimator::for_model(model);
    let mut total: i64 = 0;

    if let Some(system) = body.get("system").and_then(|v| v.as_str()) {
        total += estimator.count(system);
    }

    // A chat body carries its input in `messages`; an image or task body carries
    // it in `prompt`, which is a different request shape rather than a chat
    // without messages. The reference makes the same distinction explicitly -- its
    // image request reports `CombineText: i.Prompt` for counting
    // (`relaykit/dto/openai_image.go:162,172`) -- and without it an image prompt
    // priced as zero and its task rode free.
    if let Some(prompt) = body.get("prompt").and_then(|v| v.as_str()) {
        total += estimator.count(prompt);
    }
    // An image edit or a video request may carry the input as text parts instead.
    if let Some(input) = body.get("input").and_then(|v| v.as_str()) {
        total += estimator.count(input);
    }

    if let Some(messages) = body.get("messages").and_then(|v| v.as_array()) {
        for message in messages {
            total += estimate_content(estimator, message.get("content"));
            // Tool/function call arguments are sent upstream and therefore billed.
            if let Some(calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
                for call in calls {
                    if let Some(args) = call
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| a.as_str())
                    {
                        total += estimator.count(args);
                    }
                }
            }
        }
    }

    if let Some(tools) = body.get("tools").and_then(|v| v.as_array()) {
        for tool in tools {
            total += estimator.count(&tool.to_string());
        }
    }

    if let Some(prompt) = body.get("prompt").and_then(|v| v.as_str()) {
        // Legacy /v1/completions.
        total += estimator.count(prompt);
    }

    if let Some(input) = body.get("input").and_then(|v| v.as_str()) {
        // /v1/responses.
        total += estimator.count(input);
    }

    total
}

fn estimate_content(estimator: TokenEstimator, content: Option<&serde_json::Value>) -> i64 {
    match content {
        Some(serde_json::Value::String(text)) => estimator.count(text),
        Some(serde_json::Value::Array(parts)) => {
            let mut total = 0;
            for part in parts {
                match part {
                    serde_json::Value::String(text) => total += estimator.count(text),
                    serde_json::Value::Object(obj) => {
                        // OpenAI parts: {"type":"text","text":"..."}
                        if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                            total += estimator.count(text);
                        }
                        if let Some(text) = obj.get("content").and_then(|v| v.as_str()) {
                            total += estimator.count(text);
                        }
                        // Anthropic image blocks carry no text but still cost
                        // tokens; NewAPI bills a flat allowance per image.
                        if obj.get("type").and_then(|v| v.as_str()) == Some("image")
                            || obj.get("image_url").is_some()
                            || obj.get("source").is_some()
                        {
                            total += IMAGE_TOKEN_ALLOWANCE;
                        }
                    }
                    _ => {}
                }
            }
            total
        }
        _ => 0,
    }
}

/// Conservative per-image allowance used by the pre-consume estimate.
///
/// NewAPI's `getImageToken` computes tile-based counts (85 base + 170/tile and
/// friends); the exact figure cannot be known before the image is decoded, so a
/// fixed mid-range value keeps the reservation from under-shooting.
pub const IMAGE_TOKEN_ALLOWANCE: i64 = 1_047;

// ---------------------------------------------------------------------------
// BPE-backed counting
// ---------------------------------------------------------------------------

/// Count text tokens with a real BPE encoder when available.
///
/// Falls back to the heuristic per model when the tokenizer cannot be built
/// (unknown model, offline build, feature disabled).
pub fn estimate_tokens(model: &str, text: &str) -> i64 {
    if text.is_empty() {
        return 0;
    }
    #[cfg(feature = "tiktoken")]
    {
        if let Some(count) = bpe_count(model, text) {
            return count;
        }
    }
    TokenEstimator::for_model(model).count(text)
}

#[cfg(feature = "tiktoken")]
fn bpe_count(model: &str, text: &str) -> Option<i64> {
    use once_cell::sync::Lazy;

    static CACHE: Lazy<RwLock<HashMap<String, Option<tiktoken_rs::CoreBPE>>>> =
        Lazy::new(|| RwLock::new(HashMap::new()));

    let key = model.to_ascii_lowercase();
    {
        let cache = CACHE.read();
        if let Some(entry) = cache.get(&key) {
            return entry.as_ref().map(|bpe| bpe.encode_ordinary(text).len() as i64);
        }
    }

    // `cl100k_base` is the OpenAI default and what NewAPI falls back to when
    // `tokenizer.ForModel` has no specific codec.
    let bpe = tiktoken_rs::cl100k_base().ok();
    let count = bpe.as_ref().map(|b| b.encode_ordinary(text).len() as i64);
    CACHE.write().insert(key, bpe);
    count
}

/// The assumed completion length an image request reserves, mirroring the
/// reference's `MaxTokens: 1584` (`relaykit/dto/openai_image.go:173`).
///
/// An image has no `max_tokens` to read, so a reservation that used the chat
/// default would hold back a chat-sized amount for one picture. The reference
/// states its own figure, so this one is stated rather than inherited.
pub const IMAGE_ASSUMED_TOKENS: i64 = 1584;

/// Whether a body is an image or task request rather than a chat request.
///
/// Decided by shape, because that is what the two differ in: a chat request
/// carries `messages`, while an image or task request carries its input in
/// `prompt`. The reference branches on the decoded request *type*
/// (`relay/request_billing.go:38`), which is the same distinction reaching the
/// same answer.
pub fn is_image_request(body: &serde_json::Value) -> bool {
    body.get("prompt").map(|v| !v.is_null()).unwrap_or(false)
        && body.get("messages").is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_zero() {
        assert_eq!(TokenEstimator::OpenAi.count(""), 0);
        assert_eq!(estimate_tokens("gpt-4o", ""), 0);
    }

    #[test]
    fn model_family_routing() {
        assert_eq!(
            TokenEstimator::for_model("claude-sonnet-4-20250514"),
            TokenEstimator::Claude
        );
        assert_eq!(
            TokenEstimator::for_model("gemini-2.0-flash"),
            TokenEstimator::Gemini
        );
        assert_eq!(TokenEstimator::for_model("gpt-4o"), TokenEstimator::OpenAi);
    }

    #[test]
    fn cjk_is_charged_per_character() {
        // 4 Han characters at 0.85 each = 3.4 -> ceil 4
        let n = TokenEstimator::OpenAi.count("你好世界");
        assert_eq!(n, 4);
        // Claude charges more per CJK char.
        let c = TokenEstimator::Claude.count("你好世界");
        assert!(c >= n, "claude ({c}) should be >= openai ({n}) for CJK");
    }

    #[test]
    fn word_runs_are_charged_once_per_run() {
        // "hello" is one run => 1.02 -> ceil 2
        assert_eq!(TokenEstimator::OpenAi.count("hello"), 2);
        // A single short run is dominated by the ceil, so compare many runs:
        // digits cost 1.55 per run versus 1.02 for letters.
        let words = TokenEstimator::OpenAi.count("a a a a a");
        let digits = TokenEstimator::OpenAi.count("1 1 1 1 1");
        assert!(
            digits > words,
            "digits ({digits}) should exceed letters ({words}) across runs"
        );
    }

    #[test]
    fn a_letter_digit_boundary_starts_a_new_run() {
        // "abc" is one run (1.02 -> 2); "abc123" is a letter run plus a digit
        // run (1.02 + 1.55 = 2.57 -> 3).
        assert_eq!(TokenEstimator::OpenAi.count("abc"), 2);
        assert_eq!(TokenEstimator::OpenAi.count("abc123"), 3);
    }

    #[test]
    fn whitespace_is_charged_cheaply() {
        // A space costs 0.42 for OpenAI, a newline 0.5.
        assert_eq!(TokenEstimator::OpenAi.count(" "), 1);
        let newline = TokenEstimator::OpenAi.count("\n");
        assert!(newline >= 1);
    }

    #[test]
    fn math_symbols_are_expensive_for_claude() {
        let claude = TokenEstimator::Claude.count("∑");
        let openai = TokenEstimator::OpenAi.count("∑");
        assert!(claude > openai, "claude={claude} openai={openai}");
        assert_eq!(claude, 5); // 4.52 -> ceil 5
    }

    #[test]
    fn prompt_estimate_walks_messages_and_tools() {
        let body = serde_json::json!({
            "messages": [
                {"role": "user", "content": "hello"}
            ]
        });
        let n = estimate_prompt_tokens("gpt-4o", &body);
        assert!(n > 0, "estimated {n}");
    }

    #[test]
    fn prompt_estimate_counts_multimodal_parts() {
        let body = serde_json::json!({
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "describe this"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAA"}}
                ]
            }]
        });
        let n = estimate_prompt_tokens("gpt-4o", &body);
        assert!(n >= IMAGE_TOKEN_ALLOWANCE, "estimated {n}");
    }

    #[test]
    fn prompt_estimate_includes_tool_definitions() {
        let bare = serde_json::json!({"messages":[{"role":"user","content":"hi"}]});
        let with_tools = serde_json::json!({
            "messages":[{"role":"user","content":"hi"}],
            "tools":[{"type":"function","function":{"name":"get_weather","description":"Get the weather","parameters":{"type":"object"}}}]
        });
        assert!(
            estimate_prompt_tokens("gpt-4o", &with_tools)
                > estimate_prompt_tokens("gpt-4o", &bare)
        );
    }

    #[test]
    fn estimate_is_monotonic_in_length() {
        let short = estimate_prompt_tokens(
            "gpt-4o",
            &serde_json::json!({"messages":[{"role":"user","content":"hi"}]}),
        );
        let long = estimate_prompt_tokens(
            "gpt-4o",
            &serde_json::json!({"messages":[{"role":"user","content":"hi ".repeat(200)}]}),
        );
        assert!(long > short);
    }
}
