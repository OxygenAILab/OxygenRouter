//! Adaptor registry: `ApiType` -> concrete adaptor.
//!
//! Adaptors are cheap; we construct one per request (matching NewAPI's
//! zero-sized `&XxxAdaptor{}` pattern) rather than holding shared mutable state.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use crate::adaptor::Adaptor;
use crate::adapters;
use crate::channel_type::ApiType;

/// Construct the adaptor for an internal API type.
///
/// Returns `None` for `ApiType::Unsupported`.
pub fn get_adaptor(api_type: ApiType) -> Option<Box<dyn Adaptor>> {
    match api_type {
        ApiType::OpenAi => Some(Box::new(adapters::openai::OpenAiAdaptor)),
        ApiType::Anthropic => Some(Box::new(adapters::anthropic::AnthropicAdaptor)),
        ApiType::Gemini => Some(Box::new(adapters::gemini::GeminiAdaptor)),
        ApiType::Aws => Some(Box::new(adapters::bedrock::BedrockAdaptor)),
        ApiType::VertexAi => Some(Box::new(adapters::vertex::VertexAdaptor)),
        ApiType::Ollama => Some(Box::new(adapters::ollama::OllamaAdaptor)),
        ApiType::Cohere => Some(Box::new(adapters::cohere::CohereAdaptor)),
        ApiType::Azure => Some(Box::new(adapters::azure::AzureAdaptor)),
        ApiType::AdvancedCustom => Some(Box::new(adapters::advanced_custom::AdvancedCustomAdaptor)),
        ApiType::Unsupported => None,
    }
}

/// Convenience: registry name listing, for diagnostics.
pub fn supported_names() -> Vec<&'static str> {
    vec![
        "openai",
        "anthropic",
        "gemini",
        "aws",
        "vertex",
        "ollama",
        "cohere",
        "azure",
        "advanced_custom",
    ]
}
