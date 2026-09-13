import React, { useMemo, useState } from "react";
import { Copy, Search, CheckCircle } from "lucide-react";
import { PROVIDER_PRESETS } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";

type ModelType = "chat" | "embedding" | "image" | "audio";

interface ModelEntry {
  name: string;
  type: ModelType;
  context?: number;
  provider: string;
}

const PROVIDER_COLORS: Record<string, string> = {
  openai: "#10a37f",
  anthropic: "#c67c53",
  azure: "#0078d4",
  deepseek: "#697fe6",
  moonshot: "#7c3aed",
  zhipu: "#2d6cdf",
  ollama: "#4b5563",
  custom: "#6b7280",
};

const MODELS: ModelEntry[] = [
  // OpenAI
  { name: "gpt-4o", type: "chat", context: 128000, provider: "openai" },
  { name: "gpt-4-turbo", type: "chat", context: 128000, provider: "openai" },
  { name: "gpt-4", type: "chat", context: 8192, provider: "openai" },
  { name: "gpt-3.5-turbo", type: "chat", context: 16385, provider: "openai" },
  { name: "o1", type: "chat", context: 65536, provider: "openai" },
  { name: "o1-mini", type: "chat", context: 65536, provider: "openai" },
  { name: "dall-e-3", type: "image", provider: "openai" },
  { name: "tts-1", type: "audio", provider: "openai" },
  { name: "whisper-1", type: "audio", provider: "openai" },
  { name: "text-embedding-3-small", type: "embedding", context: 8191, provider: "openai" },
  { name: "text-embedding-3-large", type: "embedding", context: 8191, provider: "openai" },
  // Anthropic
  { name: "claude-3-5-sonnet", type: "chat", context: 200000, provider: "anthropic" },
  { name: "claude-3-5-haiku", type: "chat", context: 200000, provider: "anthropic" },
  { name: "claude-3-opus", type: "chat", context: 200000, provider: "anthropic" },
  { name: "claude-3-sonnet", type: "chat", context: 200000, provider: "anthropic" },
  { name: "claude-3-haiku", type: "chat", context: 200000, provider: "anthropic" },
  // Azure
  { name: "gpt-4o", type: "chat", context: 128000, provider: "azure" },
  { name: "gpt-35-turbo", type: "chat", context: 16385, provider: "azure" },
  { name: "text-embedding-3-small", type: "embedding", context: 8191, provider: "azure" },
  // DeepSeek
  { name: "deepseek-chat", type: "chat", context: 64000, provider: "deepseek" },
  { name: "deepseek-coder", type: "chat", context: 160000, provider: "deepseek" },
  { name: "deepseek-reasoner", type: "chat", context: 64000, provider: "deepseek" },
  // Moonshot
  { name: "moonshot-v1-8k", type: "chat", context: 8000, provider: "moonshot" },
  { name: "moonshot-v1-32k", type: "chat", context: 32000, provider: "moonshot" },
  { name: "moonshot-v1-128k", type: "chat", context: 128000, provider: "moonshot" },
  { name: "vision-preview", type: "chat", context: 32000, provider: "moonshot" },
  // Zhipu
  { name: "glm-4", type: "chat", context: 128000, provider: "zhipu" },
  { name: "glm-4-flash", type: "chat", context: 128000, provider: "zhipu" },
  { name: "glm-4v-plus", type: "chat", context: 128000, provider: "zhipu" },
  // Ollama
  { name: "llama3", type: "chat", context: 8192, provider: "ollama" },
  { name: "llama2", type: "chat", context: 4096, provider: "ollama" },
  { name: "mistral", type: "chat", context: 8192, provider: "ollama" },
  { name: "mixtral", type: "chat", context: 32000, provider: "ollama" },
  { name: "codellama", type: "chat", context: 16384, provider: "ollama" },
];

const TYPE_LABELS: Record<ModelType, (t: ReturnType<typeof useI18n>["t"]) => string> = {
  chat: (t) => t.models.typeChat,
  embedding: (t) => t.models.typeEmbedding,
  image: (t) => t.models.typeImage,
  audio: (t) => t.models.typeAudio,
};

const TYPE_BADGE: Record<ModelType, string> = {
  chat: "badge-info",
  embedding: "badge-neutral",
  image: "badge-warning",
  audio: "badge-success",
};

function ProviderBadge({ provider }: { provider: string }) {
  const color = PROVIDER_COLORS[provider] ?? "#6b7280";
  const preset = PROVIDER_PRESETS.find((p) => p.id === provider);
  return (
    <span className="provider-badge">
      <span className="provider-badge-dot" style={{ background: color }} />
      {preset?.name ?? provider}
    </span>
  );
}

function ModelCard({ model, t }: { model: ModelEntry; t: ReturnType<typeof useI18n>["t"] }) {
  const [copied, setCopied] = useState(false);
  const handleCopy = () => {
    navigator.clipboard?.writeText(model.name);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };
  return (
    <div className="model-card">
      <div className="model-card-header">
        <span className="model-name font-mono">{model.name}</span>
        <button className="icon-button" onClick={handleCopy} title={t.models.copyName}>
          {copied ? <CheckCircle size={14} /> : <Copy size={14} />}
        </button>
      </div>
      <div className="model-card-meta">
        <ProviderBadge provider={model.provider} />
        <span className={`badge ${TYPE_BADGE[model.type]} text-[10px]`}>
          {TYPE_LABELS[model.type](t)}
        </span>
      </div>
      {model.context && (
        <div className="model-card-context">
          <span className="text-[var(--md-on-surface-variant)] text-[10px]">{t.models.context}:</span>
          <span className="font-mono text-[10px]">{(model.context / 1000).toFixed(0)}k</span>
        </div>
      )}
    </div>
  );
}

export default function ModelsPage() {
  const { t } = useI18n();
  const [providerFilter, setProviderFilter] = useState<string>("all");
  const [search, setSearch] = useState("");

  const filtered = useMemo(() => {
    return MODELS.filter((m) => {
      if (providerFilter !== "all" && m.provider !== providerFilter) return false;
      if (search.trim()) {
        const q = search.toLowerCase();
        if (!m.name.toLowerCase().includes(q)) return false;
      }
      return true;
    });
  }, [providerFilter, search]);

  return (
    <div className="page-fade-enter">
      <PageHeader
        title={t.models.title}
        description={t.models.description}
      />

      <div className="log-filters">
        <div className="log-filter">
          <Search size={12} style={{ color: "var(--md-on-surface-variant)" }} />
          <input
            placeholder={t.models.search}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="w-72"
          />
        </div>
        <div className="provider-grid" style={{ gap: "4px" }}>
          <button
            className={`provider-pill ${providerFilter === "all" ? "is-active" : ""}`}
            onClick={() => setProviderFilter("all")}
          >
            {t.models.allProviders}
          </button>
          {PROVIDER_PRESETS.map((p) => (
            <button
              key={p.id}
              className={`provider-pill ${providerFilter === p.id ? "is-active" : ""}`}
              onClick={() => setProviderFilter(p.id)}
            >
              {p.name}
            </button>
          ))}
        </div>
        <span className="text-xs text-[var(--md-on-surface-variant)] ml-auto font-mono">
          {filtered.length}
        </span>
      </div>

      <div className="model-grid">
        {filtered.map((m) => (
          <ModelCard key={m.name} model={m} t={t} />
        ))}
      </div>
    </div>
  );
}
