//! Model router: heuristic for `auto`, type-based routing, keyword matching
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RequestKind {
    ChatCompletion,
    TextCompletion,
    Embedding,
    ImageGeneration,
    Audio,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ModelFeature {
    Vision,
    Image,
    Audio,
    Tools,
    LongContext,
    Code,
    Chat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub kind: RequestKind,
    pub requested_model: String,
    pub resolved_model: String,
    pub required_features: Vec<ModelFeature>,
    pub is_auto: bool,
    pub long_context_fallback: Option<String>,
}

pub struct ModelRouter {
    pub long_context_fallbacks: Vec<(String, String)>,
}

impl Default for ModelRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelRouter {
    pub fn new() -> Self {
        Self {
            long_context_fallbacks: vec![
                ("gpt-3.5-turbo".to_string(), "gpt-3.5-turbo-16k".to_string()),
                ("gpt-4".to_string(), "gpt-4-32k".to_string()),
                ("claude-3-haiku".to_string(), "claude-3-sonnet".to_string()),
                ("claude-3-sonnet".to_string(), "claude-3-opus".to_string()),
            ],
        }
    }

    pub fn classify(&self, path: &str) -> RequestKind {
        let p = path.to_lowercase();
        if p.contains("/v1/chat/completions") {
            RequestKind::ChatCompletion
        } else if p.contains("/v1/completions") {
            RequestKind::TextCompletion
        } else if p.contains("/v1/embeddings") {
            RequestKind::Embedding
        } else if p.contains("/v1/images/") {
            RequestKind::ImageGeneration
        } else if p.contains("/v1/audio/") {
            RequestKind::Audio
        } else {
            RequestKind::Unknown
        }
    }

    pub fn detect_features(&self, body: &Value) -> Vec<ModelFeature> {
        let mut feats = Vec::new();
        if let Some(msgs) = body.get("messages").and_then(|m| m.as_array()) {
            for msg in msgs {
                if let Some(content) = msg.get("content") {
                    if let Some(arr) = content.as_array() {
                        for part in arr {
                            if let Some(t) = part.get("type").and_then(|t| t.as_str()) {
                                match t {
                                    "image_url" | "image" => feats.push(ModelFeature::Vision),
                                    "input_audio" | "audio" => feats.push(ModelFeature::Audio),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }
        if body.get("tools").is_some() || body.get("functions").is_some() {
            feats.push(ModelFeature::Tools);
        }
        if let Some(prompt) = body.get("prompt").and_then(|p| p.as_str()) {
            if prompt.len() > 8000 {
                feats.push(ModelFeature::LongContext);
            }
        }
        if let Some(msgs) = body.get("messages").and_then(|m| m.as_array()) {
            let total: usize = msgs
                .iter()
                .filter_map(|m| m.get("content").and_then(|c| c.as_str()).map(|s| s.len()))
                .sum();
            if total > 16000 {
                feats.push(ModelFeature::LongContext);
            }
        }
        feats.sort_by_key(|f| format!("{:?}", f));
        feats.dedup();
        feats
    }

    pub fn resolve_auto(&self, body: &Value, path: &str) -> RouteDecision {
        let kind = self.classify(path);
        let feats = self.detect_features(body);
        let requested = body
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("auto")
            .to_string();
        let is_auto = requested.eq_ignore_ascii_case("auto");

        let resolved = if is_auto {
            let lower_path = path.to_lowercase();
            let target = if lower_path.contains("/v1/images/") {
                "dall-e-3"
            } else if lower_path.contains("/v1/audio/") {
                "tts-1"
            } else if lower_path.contains("/v1/embeddings") {
                "text-embedding-3-small"
            } else if feats.iter().any(|f| matches!(f, ModelFeature::Vision)) {
                "gpt-4o"
            } else if feats.iter().any(|f| matches!(f, ModelFeature::LongContext)) {
                "gpt-3.5-turbo-16k"
            } else if feats.iter().any(|f| matches!(f, ModelFeature::Tools)) {
                "gpt-4o-mini"
            } else {
                "gpt-3.5-turbo"
            };
            target.to_string()
        } else {
            requested.clone()
        };

        let lc_fallback = self
            .long_context_fallbacks
            .iter()
            .find(|(from, _)| from.eq_ignore_ascii_case(&resolved))
            .map(|(_, to)| to.clone());

        RouteDecision {
            kind,
            requested_model: requested,
            resolved_model: resolved,
            required_features: feats,
            is_auto,
            long_context_fallback: lc_fallback,
        }
    }

    pub fn fallback_for_context(&self, model: &str) -> Option<String> {
        self.long_context_fallbacks
            .iter()
            .find(|(from, _)| from.eq_ignore_ascii_case(model))
            .map(|(_, to)| to.clone())
    }
}
