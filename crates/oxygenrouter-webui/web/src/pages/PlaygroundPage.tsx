import React, { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  AlertCircle,
  ArrowRight,
  CheckCircle,
  ChevronDown,
  Image as ImageIcon,
  Loader2,
  MessageSquare,
  Play,
  Send,
  Sparkles,
  Type,
  Wand2,
  X,
} from "lucide-react";
import { api, Channel, RequestLog } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import { useToast } from "../lib/toast";
import Select from "../components/ui/Select";
import Slider from "../components/ui/Slider";
import SegmentedControl from "../components/ui/SegmentedControl";

type Mode = "chat" | "embeddings" | "image" | "audio";

interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

const SYSTEM_PROMPTS: Record<string, string> = {
  helpful: "You are a helpful assistant.",
  coder: "You are an expert programmer. Provide concise, correct code with brief explanations.",
  translator: "You are a professional translator. Translate the user's input accurately and naturally.",
  writer: "You are a creative writing assistant. Help the user with their writing in a clear, engaging style.",
  analyst: "You are a data analyst. Provide clear, structured analysis with actionable insights.",
  custom: "",
};

export default function PlaygroundPage() {
  const { t } = useI18n();
  const toast = useToast();
  const { data: channels } = useQuery({ queryKey: ["channels"], queryFn: api.channels.list });
  const enabledChannels = channels?.filter((c) => c.enabled) ?? [];

  const [mode, setMode] = useState<Mode>("chat");
  const [model, setModel] = useState("");
  const [channelId, setChannelId] = useState<string>("");
  const [temperature, setTemperature] = useState(0.7);
  const [maxTokens, setMaxTokens] = useState(1024);
  const [systemPrompt, setSystemPrompt] = useState("helpful");
  const [customSystem, setCustomSystem] = useState("");
  const [messages, setMessages] = useState<ChatMessage[]>([
    { role: "system", content: SYSTEM_PROMPTS.helpful },
    { role: "user", content: "Hello! What can you do?" },
  ]);
  const [input, setInput] = useState("");
  const [response, setResponse] = useState("");
  const [loading, setLoading] = useState(false);
  const [streamMode, setStreamMode] = useState<"stream" | "buffered">("stream");
  const [streamingContent, setStreamingContent] = useState("");
  const abortRef = React.useRef<AbortController | null>(null);
  const [embeddingResult, setEmbeddingResult] = useState<number[] | null>(null);
  const [imagePrompt, setImagePrompt] = useState("");
  const [imageSize, setImageSize] = useState("1024x1024");
  const [imageResult, setImageResult] = useState<string | null>(null);
  const [latencyMs, setLatencyMs] = useState<number | null>(null);
  const [tokenUsage, setTokenUsage] = useState<{ prompt?: number; completion?: number; total?: number } | null>(null);
  const [showSettings, setShowSettings] = useState(true);

  // Auto-select first enabled channel
  React.useEffect(() => {
    if (!channelId && enabledChannels.length > 0) {
      setChannelId(enabledChannels[0].id);
      setModel(enabledChannels[0].test_model || "gpt-3.5-turbo");
    }
  }, [enabledChannels, channelId]);

  const handleCancel = () => {
    abortRef.current?.abort();
    abortRef.current = null;
    setLoading(false);
  };

  const handleSend = async () => {
    if (!input.trim() || loading || !channelId) return;
    const userMsg: ChatMessage = { role: "user", content: input };
    const newMessages = [...messages, userMsg];
    setMessages(newMessages);
    setInput("");
    setResponse("");
    setStreamingContent("");
    setLoading(true);
    setLatencyMs(null);
    setTokenUsage(null);

    const controller = new AbortController();
    abortRef.current = controller;
    const t0 = performance.now();
    const sysContent = systemPrompt === "custom" ? customSystem : SYSTEM_PROMPTS[systemPrompt];
    const apiMessages = sysContent
      ? [{ role: "system" as const, content: sysContent }, ...newMessages.filter((m) => m.role !== "system")]
      : newMessages;

    const useStream = streamMode === "stream";
    try {
      const res = await fetch("/v1/chat/completions", {
        method: "POST",
        signal: controller.signal,
        headers: { "Content-Type": "application/json", Authorization: `Bearer ${"local-test"}` },
        body: JSON.stringify({
          model,
          messages: apiMessages,
          temperature,
          max_tokens: maxTokens,
          stream: useStream,
        }),
      });

      if (!useStream) {
        const data = await res.json();
        setLatencyMs(Math.round(performance.now() - t0));
        if (data.error) {
          toast.error(t.playground.requestFailed, data.error.message ?? "Unknown error");
          return;
        }
        const content = data.choices?.[0]?.message?.content ?? "";
        setResponse(content);
        setMessages((prev) => [...prev, { role: "assistant", content }]);
        if (data.usage) {
          setTokenUsage({
            prompt: data.usage.prompt_tokens,
            completion: data.usage.completion_tokens,
            total: data.usage.total_tokens,
          });
        }
        return;
      }

      // Streaming SSE delta parse
      const reader = res.body?.getReader();
      if (!reader) {
        toast.error(t.playground.requestFailed, "response body is not readable");
        return;
      }
      const decoder = new TextDecoder();
      let buffer = "";
      let full = "";
      let usageData: { prompt?: number; completion?: number; total?: number } | null = null;
      let streamError: string | null = null;

      // seed an empty assistant message for the typing cursor
      setMessages((prev) => [...prev, { role: "assistant", content: "" }]);

      const updateLastAssistant = (content: string) => {
        setMessages((prev) => {
          const next = [...prev];
          if (next.length > 0 && next[next.length - 1].role === "assistant") {
            next[next.length - 1] = { ...next[next.length - 1], content };
          }
          return next;
        });
      };

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split("\n");
        buffer = lines.pop() ?? "";
        for (const line of lines) {
          const trimmed = line.trim();
          if (trimmed.startsWith("data:")) {
            const payload = trimmed.slice(5).trim();
            if (payload === "[DONE]") continue;
            try {
              const parsed = JSON.parse(payload);
              if (parsed.error) {
                streamError = parsed.error.message ?? JSON.stringify(parsed.error);
                continue;
              }
              const delta = parsed.choices?.[0]?.delta?.content;
              if (delta) {
                full += delta;
                updateLastAssistant(full);
                setStreamingContent(full);
              }
              if (parsed.usage) {
                usageData = {
                  prompt: parsed.usage.prompt_tokens,
                  completion: parsed.usage.completion_tokens,
                  total: parsed.usage.total_tokens,
                };
              }
            } catch {
              // partial JSON, skip
            }
          }
        }
      }

      setLatencyMs(Math.round(performance.now() - t0));
      if (streamError) {
        toast.error(t.playground.requestFailed, streamError);
      }
      if (full) {
        setResponse(full);
        updateLastAssistant(full);
      } else if (!streamError) {
        // remove empty assistant message
        setMessages((prev) => prev.filter((m) => m.role !== "assistant" || m.content));
      }
      if (usageData) setTokenUsage(usageData);
    } catch (e: any) {
      if (e?.name === "AbortError") {
        // user cancelled; keep whatever partial content arrived
        const partial = streamingContent;
        if (partial) {
          setResponse(partial);
          setMessages((prev) => {
            const next = [...prev];
            if (next.length > 0 && next[next.length - 1].role === "assistant") {
              next[next.length - 1] = { ...next[next.length - 1], content: partial };
            }
            return next;
          });
        } else {
          setMessages((prev) => prev.filter((m) => m.role !== "assistant" || m.content));
        }
      } else {
        toast.error(t.playground.requestFailed, e.message ?? String(e));
      }
    } finally {
      abortRef.current = null;
      setLoading(false);
      setStreamingContent("");
    }
  };

  const handleEmbed = async () => {
    if (!input.trim() || loading || !channelId) return;
    setLoading(true);
    setEmbeddingResult(null);
    setLatencyMs(null);
    const t0 = performance.now();
    try {
      const res = await fetch("/v1/embeddings", {
        method: "POST",
        headers: { "Content-Type": "application/json", Authorization: `Bearer ${"local-test"}` },
        body: JSON.stringify({ model, input }),
      });
      const data = await res.json();
      setLatencyMs(Math.round(performance.now() - t0));
      if (data.error) {
        toast.error("Embedding failed", data.error.message);
        return;
      }
      const vec = data.data?.[0]?.embedding;
      if (vec) {
        setEmbeddingResult(vec);
        setTokenUsage({ prompt: data.usage?.prompt_tokens, total: data.usage?.total_tokens });
      }
    } catch (e: any) {
      toast.error("Embedding failed", e.message);
    } finally {
      setLoading(false);
    }
  };

  const handleImageGen = async () => {
    if (!imagePrompt.trim() || loading || !channelId) return;
    setLoading(true);
    setImageResult(null);
    setLatencyMs(null);
    const t0 = performance.now();
    try {
      const res = await fetch("/v1/images/generations", {
        method: "POST",
        headers: { "Content-Type": "application/json", Authorization: `Bearer ${"local-test"}` },
        body: JSON.stringify({ model, prompt: imagePrompt, n: 1, size: imageSize }),
      });
      const data = await res.json();
      setLatencyMs(Math.round(performance.now() - t0));
      if (data.error) {
        toast.error("Image generation failed", data.error.message);
        return;
      }
      setImageResult(data.data?.[0]?.url ?? data.data?.[0]?.b64_json ?? null);
    } catch (e: any) {
      toast.error("Image generation failed", e.message);
    } finally {
      setLoading(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      if (mode === "chat") handleSend();
      else if (mode === "embeddings") handleEmbed();
      else if (mode === "image") handleImageGen();
    }
  };

  const clearChat = () => {
    setMessages([{ role: "system", content: SYSTEM_PROMPTS[systemPrompt] || "" }, { role: "user", content: "" }]);
    setResponse("");
    setTokenUsage(null);
    setLatencyMs(null);
  };

  return (
    <div className="page-fade-enter">
      <PageHeader
        title={t.playground.title}
        description={t.playground.description}
        action={
          <div className="flex items-center gap-2">
            <div className="mode-tabs">
              {[
                { id: "chat", label: t.playground.tabChat, icon: MessageSquare },
                { id: "embeddings", label: t.playground.tabEmbeddings, icon: Sparkles },
                { id: "image", label: t.playground.tabImage, icon: ImageIcon },
              ].map((opt) => (
                <button
                  key={opt.id}
                  className={`mode-tab ${mode === opt.id ? "is-active" : ""}`}
                  onClick={() => setMode(opt.id as Mode)}
                >
                  <opt.icon size={13} />
                  <span>{opt.label}</span>
                </button>
              ))}
            </div>
          </div>
        }
      />

      {enabledChannels.length === 0 ? (
        <div className="empty-state">
          <AlertCircle size={40} />
          <p className="text-sm">{t.playground.noChannel}</p>
          <a href="/ui/channels" className="btn-filled mt-4">
            {t.playground.goChannels}
          </a>
        </div>
      ) : (
        <div className="playground-layout">
          {/* Settings sidebar */}
          {showSettings && (
            <aside className="playground-settings">
              <div className="playground-settings-header">
                <h3>{t.playground.settings}</h3>
                <button className="btn-ghost p-1" onClick={() => setShowSettings(false)}>
                  <X size={14} />
                </button>
              </div>

              <div className="playground-settings-body">
                <div className="settings-section">
                  <label className="field-label">{t.playground.channel}</label>
                  <Select
                    value={channelId}
                    onChange={setChannelId}
                    ariaLabel={t.playground.channel}
                    fullWidth
                    options={enabledChannels.map((c) => ({
                      value: c.id,
                      label: `${c.name}${c.provider ? ` · ${c.provider}` : ""}`,
                    }))}
                  />
                </div>

                <div className="settings-section">
                  <label className="field-label">{t.playground.model}</label>
                  <input
                    className="input font-mono text-xs"
                    value={model}
                    onChange={(e) => setModel(e.target.value)}
                    placeholder="gpt-3.5-turbo"
                  />
                </div>

                {mode === "chat" && (
                  <>
                    <div className="settings-section">
                      <label className="field-label">{t.playground.streamMode}</label>
                      <SegmentedControl
                        size="sm"
                        value={streamMode}
                        onChange={setStreamMode}
                        ariaLabel={t.playground.streamMode}
                        options={[
                          { value: "stream", label: t.playground.stream },
                          { value: "buffered", label: t.playground.buffered },
                        ]}
                      />
                    </div>
                    <div className="settings-section">
                      <Slider
                        value={temperature}
                        onChange={setTemperature}
                        min={0}
                        max={2}
                        step={0.1}
                        label={t.playground.temperature}
                        formatValue={(value) => value.toFixed(2)}
                      />
                    </div>
                    <div className="settings-section">
                      <label className="field-label">{t.playground.maxTokens}</label>
                      <input
                        type="number"
                        className="input"
                        value={maxTokens}
                        onChange={(e) => setMaxTokens(Number(e.target.value))}
                        min={1}
                        max={32768}
                      />
                    </div>
                    <div className="settings-section">
                      <Select
                        value={systemPrompt}
                        onChange={setSystemPrompt}
                        label={t.playground.systemPrompt}
                        fullWidth
                        options={[
                          { value: "helpful", label: "Helpful assistant" },
                          { value: "coder", label: "Expert coder" },
                          { value: "translator", label: "Translator" },
                          { value: "writer", label: "Creative writer" },
                          { value: "analyst", label: "Data analyst" },
                          { value: "custom", label: "Custom…" },
                        ]}
                      />
                      {systemPrompt === "custom" && (
                        <textarea
                          className="input mt-2 font-mono text-xs"
                          rows={3}
                          value={customSystem}
                          onChange={(e) => setCustomSystem(e.target.value)}
                          placeholder="You are a helpful assistant."
                        />
                      )}
                    </div>
                  </>
                )}

                {mode === "image" && (
                  <div className="settings-section">
                    <Select
                      value={imageSize}
                      onChange={setImageSize}
                      label={t.playground.size}
                      fullWidth
                      options={[
                        { value: "256x256", label: "256×256" },
                        { value: "512x512", label: "512×512" },
                        { value: "1024x1024", label: "1024×1024" },
                        { value: "1792x1024", label: "1792×1024 (landscape)" },
                        { value: "1024x1792", label: "1024×1792 (portrait)" },
                      ]}
                    />
                  </div>
                )}
              </div>
            </aside>
          )}

          {!showSettings && (
            <button className="btn-outlined playground-show-settings" onClick={() => setShowSettings(true)}>
              <Wand2 size={13} /> {t.playground.settings}
            </button>
          )}

          {/* Main content area */}
          <div className="playground-main">
            {mode === "chat" && (
              <>
                <div className="chat-messages">
                  {messages.filter((m) => m.role !== "system" || m.content).length === 0 ? (
                    <div className="empty-state">
                      <MessageSquare size={32} />
                      <p className="text-sm">{t.playground.startChat}</p>
                    </div>
                  ) : (
                    messages.filter((m) => m.role !== "system" || m.content).map((msg, i) => (
                      <div key={i} className={`chat-message chat-message-${msg.role}`}>
                        <div className="chat-message-role">
                          {msg.role === "user" ? t.playground.you : t.playground.assistant}
                        </div>
                        <div className="chat-message-content">
                          {msg.content || <span className="text-[var(--md-on-surface-variant)]">…</span>}
                        </div>
                      </div>
                    ))
                  )}
                  {loading && (
                    <div className="chat-message chat-message-assistant">
                      <div className="chat-message-role">{t.playground.assistant}</div>
                      <div className="chat-message-content">
                        <Loader2 size={14} className="animate-spin inline" /> {t.playground.thinking}
                      </div>
                    </div>
                  )}
                </div>
                <div className="chat-input-bar">
                  <textarea
                    className="chat-input"
                    value={input}
                    onChange={(e) => setInput(e.target.value)}
                    onKeyDown={handleKeyDown}
                    placeholder={t.playground.inputPlaceholder}
                    rows={2}
                  />
                  <div className="chat-input-actions">
                    <button className="btn-ghost btn-sm" onClick={clearChat}>
                      {t.playground.clear}
                    </button>
                    {loading ? (
                      <button className="danger-button" onClick={handleCancel}>
                        <Loader2 size={14} className="animate-spin" />
                        {t.playground.cancel}
                      </button>
                    ) : (
                      <button className="btn-filled" onClick={handleSend} disabled={!input.trim()}>
                        <Send size={14} />
                        {t.playground.send}
                        <span className="kbd-hint">⌘↵</span>
                      </button>
                    )}
                  </div>
                </div>
                {(latencyMs !== null || tokenUsage) && (
                  <div className="response-meta">
                    {latencyMs !== null && (
                      <span className="response-meta-item">
                        <span className="response-meta-label">{t.playground.latency}:</span>
                        <span className="response-meta-value font-mono">{latencyMs}ms</span>
                      </span>
                    )}
                    {tokenUsage?.prompt !== undefined && (
                      <span className="response-meta-item">
                        <span className="response-meta-label">{t.playground.tokens}:</span>
                        <span className="response-meta-value font-mono">
                          {tokenUsage.prompt} → {tokenUsage.completion ?? "?"} (Σ {tokenUsage.total ?? tokenUsage.prompt})
                        </span>
                      </span>
                    )}
                    {tokenUsage && (
                      <span className="response-meta-item">
                        <span className="response-meta-label">{t.playground.estCost}:</span>
                        <span className="response-meta-value font-mono">
                          ${((tokenUsage.total ?? 0) * 0.000002).toFixed(6)}
                        </span>
                      </span>
                    )}
                  </div>
                )}
              </>
            )}

            {mode === "embeddings" && (
              <div className="embed-layout">
                <div className="embed-input-section">
                  <label className="field-label">{t.playground.textToEmbed}</label>
                  <textarea
                    className="input font-mono text-xs"
                    value={input}
                    onChange={(e) => setInput(e.target.value)}
                    onKeyDown={handleKeyDown}
                    placeholder="Enter text to embed..."
                    rows={6}
                  />
                  <button className="btn-filled mt-3" onClick={handleEmbed} disabled={loading || !input.trim()}>
                    {loading ? <Loader2 size={14} className="animate-spin" /> : <Sparkles size={14} />}
                    {t.playground.embed}
                    <span className="kbd-hint">⌘↵</span>
                  </button>
                </div>
                {embeddingResult && (
                  <div className="embed-result">
                    <div className="embed-result-header">
                      <span className="text-xs font-mono text-[var(--md-on-surface-variant)]">
                        [{embeddingResult.length} dimensions]
                      </span>
                      {latencyMs !== null && (
                        <span className="text-xs font-mono text-[var(--md-on-surface-variant)]">
                          {latencyMs}ms
                        </span>
                      )}
                    </div>
                    <div className="embed-vector font-mono">
                      [{embeddingResult.slice(0, 32).map((v) => v.toFixed(6)).join(", ")}
                      {embeddingResult.length > 32 ? `, … +${embeddingResult.length - 32} more` : ""}]
                    </div>
                    {tokenUsage?.total && (
                      <div className="text-xs text-[var(--md-on-surface-variant)] mt-2 font-mono">
                        tokens: {tokenUsage.total}
                      </div>
                    )}
                  </div>
                )}
              </div>
            )}

            {mode === "image" && (
              <div className="image-layout">
                <div className="image-input-section">
                  <label className="field-label">{t.playground.prompt}</label>
                  <textarea
                    className="input"
                    value={imagePrompt}
                    onChange={(e) => setImagePrompt(e.target.value)}
                    onKeyDown={handleKeyDown}
                    placeholder="A futuristic city at sunset, cyberpunk style, 8k, highly detailed..."
                    rows={4}
                  />
                  <button className="btn-filled mt-3" onClick={handleImageGen} disabled={loading || !imagePrompt.trim()}>
                    {loading ? <Loader2 size={14} className="animate-spin" /> : <Wand2 size={14} />}
                    {t.playground.generate}
                    <span className="kbd-hint">⌘↵</span>
                  </button>
                </div>
                {imageResult && (
                  <div className="image-result">
                    {imageResult.startsWith("data:") || imageResult.length > 200 ? (
                      <img src={imageResult} alt="Generated" className="image-output" />
                    ) : (
                      <a href={imageResult} target="_blank" rel="noreferrer" className="btn-outlined">
                        {t.playground.openImage}
                      </a>
                    )}
                    {latencyMs !== null && (
                      <div className="text-xs font-mono text-[var(--md-on-surface-variant)] mt-2">
                        {latencyMs}ms
                      </div>
                    )}
                  </div>
                )}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
