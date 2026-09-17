//! Two-tier channel identity, mirroring NewAPI.
//!
//! * `ChannelType` — the DB/UI-level provider identity. Sparse numbering,
//!   NewAPI-compatible so imported databases keep working.
//! * `ApiType` — the contiguous internal key that selects an adaptor.
//!
//! The bridge is `channel_type_to_api_type`, which **falls back to OpenAI** for
//! unknown types. That fallback is why arbitrary OpenAI-compatible providers
//! work out of the box without a dedicated adaptor.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use serde::{Deserialize, Serialize};

/// DB-level provider identity (NewAPI-compatible numbering).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelType {
    OpenAI = 1,
    Azure = 3,
    Ollama = 4,
    Custom = 8,
    Anthropic = 14,
    Baidu = 15,
    Zhipu = 16,
    Ali = 17,
    Xunfei = 18,
    OpenRouter = 20,
    Tencent = 23,
    Gemini = 24,
    Moonshot = 25,
    Perplexity = 27,
    Aws = 33,
    Cohere = 34,
    MiniMax = 35,
    Dify = 37,
    Jina = 38,
    Cloudflare = 39,
    SiliconFlow = 40,
    VertexAi = 41,
    Mistral = 42,
    DeepSeek = 43,
    MokaAI = 44,
    VolcEngine = 45,
    Xinference = 47,
    Xai = 48,
    Coze = 49,
    Replicate = 56,
    Codex = 57,
    AdvancedCustom = 58,
    Sub2Api = 59,
    NewApi = 60,
    TaskPlugin = 61,
    Vllm = 62,
    Sglang = 63,
}

impl ChannelType {
    pub fn from_i64(v: i64) -> Self {
        match v {
            1 => Self::OpenAI,
            3 => Self::Azure,
            4 => Self::Ollama,
            8 => Self::Custom,
            14 => Self::Anthropic,
            15 => Self::Baidu,
            16 => Self::Zhipu,
            17 => Self::Ali,
            18 => Self::Xunfei,
            20 => Self::OpenRouter,
            23 => Self::Tencent,
            24 => Self::Gemini,
            25 => Self::Moonshot,
            27 => Self::Perplexity,
            33 => Self::Aws,
            34 => Self::Cohere,
            35 => Self::MiniMax,
            37 => Self::Dify,
            38 => Self::Jina,
            39 => Self::Cloudflare,
            40 => Self::SiliconFlow,
            41 => Self::VertexAi,
            42 => Self::Mistral,
            43 => Self::DeepSeek,
            44 => Self::MokaAI,
            45 => Self::VolcEngine,
            47 => Self::Xinference,
            48 => Self::Xai,
            49 => Self::Coze,
            56 => Self::Replicate,
            57 => Self::Codex,
            58 => Self::AdvancedCustom,
            59 => Self::Sub2Api,
            60 => Self::NewApi,
            61 => Self::TaskPlugin,
            62 => Self::Vllm,
            63 => Self::Sglang,
            _ => Self::OpenAI,
        }
    }

    pub fn as_i64(self) -> i64 {
        self as i64
    }

    /// Human name, used in logs and the channel list UI.
    pub fn name(self) -> &'static str {
        match self {
            Self::OpenAI => "OpenAI",
            Self::Azure => "Azure",
            Self::Ollama => "Ollama",
            Self::Custom => "Custom",
            Self::Anthropic => "Anthropic",
            Self::Baidu => "Baidu",
            Self::Zhipu => "Zhipu",
            Self::Ali => "Alibaba",
            Self::Xunfei => "Xunfei",
            Self::OpenRouter => "OpenRouter",
            Self::Tencent => "Tencent",
            Self::Gemini => "Google Gemini",
            Self::Moonshot => "Moonshot",
            Self::Perplexity => "Perplexity",
            Self::Aws => "AWS Bedrock",
            Self::Cohere => "Cohere",
            Self::MiniMax => "MiniMax",
            Self::Dify => "Dify",
            Self::Jina => "Jina",
            Self::Cloudflare => "Cloudflare",
            Self::SiliconFlow => "SiliconFlow",
            Self::VertexAi => "Google Vertex AI",
            Self::Mistral => "Mistral",
            Self::DeepSeek => "DeepSeek",
            Self::MokaAI => "MokaAI",
            Self::VolcEngine => "VolcEngine",
            Self::Xinference => "Xinference",
            Self::Xai => "xAI",
            Self::Coze => "Coze",
            Self::Replicate => "Replicate",
            Self::Codex => "Codex",
            Self::AdvancedCustom => "Advanced Custom",
            Self::Sub2Api => "Sub2API",
            Self::NewApi => "NewAPI",
            Self::TaskPlugin => "Task Plugin",
            Self::Vllm => "vLLM",
            Self::Sglang => "SGLang",
        }
    }

    /// Default base URL for a channel type (empty when the operator must supply one).
    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::OpenAI => "https://api.openai.com",
            Self::Anthropic => "https://api.anthropic.com",
            Self::Gemini => "https://generativelanguage.googleapis.com",
            Self::VertexAi => "https://us-central1-aiplatform.googleapis.com",
            Self::Aws => "https://bedrock-runtime.us-east-1.amazonaws.com",
            Self::Azure => "https://YOUR-RESOURCE.openai.azure.com",
            Self::Ollama => "http://localhost:11434",
            Self::Cohere => "https://api.cohere.ai",
            Self::DeepSeek => "https://api.deepseek.com",
            Self::Moonshot => "https://api.moonshot.cn",
            Self::Zhipu => "https://open.bigmodel.cn",
            Self::Ali => "https://dashscope.aliyuncs.com",
            Self::Baidu => "https://aip.baidubce.com",
            Self::Xunfei => "https://spark-api-open.xf-yun.com",
            Self::Tencent => "https://api.hunyuan.cloud.tencent.com",
            Self::OpenRouter => "https://openrouter.ai/api",
            Self::Perplexity => "https://api.perplexity.ai",
            Self::Mistral => "https://api.mistral.ai",
            Self::SiliconFlow => "https://api.siliconflow.cn",
            Self::VolcEngine => "https://ark.cn-beijing.volces.com",
            Self::MiniMax => "https://api.minimax.chat",
            Self::Xai => "https://api.x.ai",
            Self::Cloudflare => "https://api.cloudflare.com",
            Self::Jina => "https://api.jina.ai",
            Self::Replicate => "https://api.replicate.com",
            Self::Xinference => "http://localhost:9997",
            Self::Vllm => "http://localhost:8000",
            Self::Sglang => "http://localhost:30000",
            _ => "",
        }
    }
}

impl Default for ChannelType {
    fn default() -> Self {
        Self::OpenAI
    }
}

impl Serialize for ChannelType {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(self.as_i64())
    }
}

impl<'de> Deserialize<'de> for ChannelType {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = i64::deserialize(d)?;
        Ok(Self::from_i64(v))
    }
}

/// Internal adaptor-selection key (contiguous).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiType {
    OpenAi,
    Anthropic,
    Gemini,
    Aws,
    VertexAi,
    Ollama,
    Cohere,
    Azure,
    AdvancedCustom,
    /// No adaptor — reject the request.
    Unsupported,
}

/// Map a DB channel type to the internal adaptor key.
///
/// Unknown types **fall back to OpenAI** so OpenAI-compatible providers work
/// without a dedicated adaptor. Task-plugin channels are explicitly unsupported
/// here (they use the separate task adaptor path).
pub fn channel_type_to_api_type(ct: ChannelType) -> ApiType {
    match ct {
        ChannelType::OpenAI
        | ChannelType::Custom
        | ChannelType::OpenRouter
        | ChannelType::Xinference
        | ChannelType::Vllm
        | ChannelType::Sglang
        | ChannelType::SiliconFlow
        | ChannelType::DeepSeek
        | ChannelType::Moonshot
        | ChannelType::Mistral
        | ChannelType::Xai
        | ChannelType::Perplexity
        | ChannelType::Tencent
        | ChannelType::VolcEngine
        | ChannelType::MiniMax
        | ChannelType::MokaAI
        | ChannelType::Cloudflare
        | ChannelType::Jina
        | ChannelType::Replicate
        | ChannelType::Dify
        | ChannelType::NewApi
        | ChannelType::Sub2Api => ApiType::OpenAi,

        ChannelType::Anthropic => ApiType::Anthropic,
        ChannelType::Gemini => ApiType::Gemini,
        ChannelType::Aws => ApiType::Aws,
        ChannelType::VertexAi => ApiType::VertexAi,
        ChannelType::Ollama => ApiType::Ollama,
        ChannelType::Cohere => ApiType::Cohere,
        ChannelType::Azure => ApiType::Azure,
        ChannelType::AdvancedCustom => ApiType::AdvancedCustom,
        ChannelType::Baidu
        | ChannelType::Zhipu
        | ChannelType::Ali
        | ChannelType::Xunfei
        | ChannelType::Codex
        | ChannelType::Coze => ApiType::OpenAi, // OpenAI-compatible shims
        ChannelType::TaskPlugin => ApiType::Unsupported,
    }
}

/// Best-effort parse of a provider *string* (as stored on `Channel.provider`)
/// into a `ChannelType`. Accepts both the NewAPI slug and numeric string.
pub fn provider_str_to_channel_type(provider: &str) -> ChannelType {
    let p = provider.trim().to_lowercase();
    if let Ok(n) = p.parse::<i64>() {
        return ChannelType::from_i64(n);
    }
    match p.as_str() {
        "anthropic" | "claude" => ChannelType::Anthropic,
        "gemini" | "google" => ChannelType::Gemini,
        "vertex" | "vertexai" | "vertex_ai" => ChannelType::VertexAi,
        "aws" | "bedrock" => ChannelType::Aws,
        "ollama" => ChannelType::Ollama,
        "cohere" => ChannelType::Cohere,
        "azure" | "azure_openai" => ChannelType::Azure,
        "deepseek" => ChannelType::DeepSeek,
        "moonshot" | "kimi" => ChannelType::Moonshot,
        "zhipu" | "glm" => ChannelType::Zhipu,
        "ali" | "qwen" | "dashscope" => ChannelType::Ali,
        "openrouter" => ChannelType::OpenRouter,
        "mistral" => ChannelType::Mistral,
        "siliconflow" => ChannelType::SiliconFlow,
        "xai" | "grok" => ChannelType::Xai,
        "perplexity" => ChannelType::Perplexity,
        "volcengine" | "doubao" => ChannelType::VolcEngine,
        "minimax" => ChannelType::MiniMax,
        "vllm" => ChannelType::Vllm,
        "sglang" => ChannelType::Sglang,
        "xinference" => ChannelType::Xinference,
        "cloudflare" => ChannelType::Cloudflare,
        "jina" => ChannelType::Jina,
        "replicate" => ChannelType::Replicate,
        "custom" => ChannelType::Custom,
        _ => ChannelType::OpenAI,
    }
}
