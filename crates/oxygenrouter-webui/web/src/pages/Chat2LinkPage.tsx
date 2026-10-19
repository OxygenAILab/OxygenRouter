import React from "react";
import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import { Check, Copy, ExternalLink, KeyRound, Link2, Settings } from "lucide-react";
import { api, ChatPresetEntry } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";

type ChatPreset = {
  id: string;
  name: string;
  template: string;
  web: boolean;
};

const KEY_PLACEHOLDERS = ["{key}", "{cherryConfig}", "{aionuiConfig}", "{deepchatConfig}", "{aqbotConfig}"];

function parsePresets(entries: ChatPresetEntry[] | undefined): ChatPreset[] {
  return (entries ?? []).flatMap((entry, index) => {
    const pair = Object.entries(entry)[0];
    if (!pair || !pair[0].trim() || typeof pair[1] !== "string" || !pair[1].trim()) return [];
    const template = pair[1].trim();
    return [{
      id: `${index}-${pair[0]}`,
      name: pair[0],
      template,
      web: /^https?:\/\//i.test(template),
    }];
  });
}

function base64(value: string): string {
  const bytes = new TextEncoder().encode(value);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function replaceAll(source: string, token: string, value: string): string {
  return source.split(token).join(value);
}

async function writeClipboard(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch {
      // Fall through: some local browser profiles deny the async API.
    }
  }
  const field = document.createElement("textarea");
  field.value = text;
  field.style.position = "fixed";
  field.style.opacity = "0";
  document.body.appendChild(field);
  field.select();
  document.execCommand("copy");
  document.body.removeChild(field);
}

/**
 * Resolve one operator-defined client template without exposing the API key
 * until the user explicitly opens or copies that preset.
 */
export function resolveChatTemplate(template: string, address: string, key: string): string {
  let result = template;
  const encodedAddress = encodeURIComponent(address);
  const normalizedKey = key.trim() && !key.trim().startsWith("sk-") ? `sk-${key.trim()}` : key.trim();

  const configPayload = (id: string) =>
    encodeURIComponent(base64(JSON.stringify({ id, baseUrl: address, apiKey: normalizedKey })));

  if (result.includes("{cherryConfig}")) {
    return replaceAll(result, "{cherryConfig}", configPayload("oxygenrouter"));
  }
  if (result.includes("{aionuiConfig}")) {
    return replaceAll(
      result,
      "{aionuiConfig}",
      encodeURIComponent(base64(JSON.stringify({ platform: "oxygenrouter", baseUrl: address, apiKey: normalizedKey }))),
    );
  }
  if (result.includes("{deepchatConfig}")) {
    return replaceAll(result, "{deepchatConfig}", configPayload("oxygenrouter"));
  }
  if (result.includes("{aqbotConfig}")) {
    const query = [
      `name=${encodeURIComponent("OxygenRouter")}`,
      `baseurl=${encodeURIComponent(address)}`,
      `apikey=${encodeURIComponent(normalizedKey)}`,
      "type=openai",
    ].join("&");
    return replaceAll(result, "{aqbotConfig}", query);
  }
  if (normalizedKey) result = replaceAll(result, "{key}", normalizedKey);
  if (address) result = replaceAll(result, "{address}", encodedAddress);
  return result;
}

export default function Chat2LinkPage() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const [busy, setBusy] = React.useState<string | null>(null);
  const [copied, setCopied] = React.useState<string | null>(null);
  const { data: site } = useQuery({ queryKey: ["site"], queryFn: api.site.get });
  const { data: keys } = useQuery({ queryKey: ["keys"], queryFn: api.keys.list });
  const presets = parsePresets(site?.chats);
  const activeKey = keys?.find((key) => key.enabled);
  const keysLoaded = keys !== undefined;
  const address = site?.server_address || window.location.origin;

  const resolved = async (preset: ChatPreset): Promise<string | null> => {
    let secret = "";
    if (KEY_PLACEHOLDERS.some((token) => preset.template.includes(token))) {
      if (!activeKey) {
        if (keysLoaded) navigate("/ui/keys");
        return null;
      }
      secret = (await api.keys.reveal(activeKey.id)).key;
    }
    return resolveChatTemplate(preset.template, address, secret);
  };

  const open = async (preset: ChatPreset) => {
    setBusy(preset.id);
    try {
      const url = await resolved(preset);
      if (url) window.location.href = url;
    } finally {
      setBusy(null);
    }
  };

  const copy = async (preset: ChatPreset) => {
    setBusy(preset.id);
    try {
      const url = await resolved(preset);
      if (!url) return;
      await writeClipboard(url);
      setCopied(preset.id);
      window.setTimeout(() => setCopied(null), 1200);
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="page-fade-enter space-y-6">
      <PageHeader title={t.chatLinks.title} description={t.chatLinks.description} />

      {keysLoaded && !activeKey && (
        <section className="dashboard-section">
          <div className="chat-link-notice">
            <KeyRound size={16} />
            <span>{t.chatLinks.needKey}</span>
            <button className="btn-outlined btn-sm" onClick={() => navigate("/ui/keys")}>
              {t.chatLinks.goKeys}
            </button>
          </div>
        </section>
      )}

      {presets.length === 0 ? (
        <section className="dashboard-section">
          <div className="chat-link-empty">
            <Link2 size={28} />
            <p>{t.chatLinks.empty}</p>
            <button className="btn-outlined btn-sm" onClick={() => navigate("/ui/system-settings")}>
              <Settings size={13} /> {t.chatLinks.openSettings}
            </button>
          </div>
        </section>
      ) : (
        <div className="chat-link-grid">
          {presets.map((preset) => (
            <article className="chat-link-card" key={preset.id}>
              <div className="chat-link-card-head">
                <div>
                  <h2>{preset.name}</h2>
                  <span className="chat-link-kind">{preset.web ? t.chatLinks.web : t.chatLinks.app}</span>
                </div>
                <ExternalLink size={16} />
              </div>
              <code>{preset.template}</code>
              <div className="chat-link-actions">
                <button className="btn-filled btn-sm" disabled={busy === preset.id} onClick={() => void open(preset)}>
                  <ExternalLink size={13} /> {t.chatLinks.open}
                </button>
                <button className="btn-outlined btn-sm" disabled={busy === preset.id} onClick={() => void copy(preset)}>
                  {copied === preset.id ? <Check size={13} /> : <Copy size={13} />}
                  {copied === preset.id ? t.common.copied : t.common.copy}
                </button>
              </div>
            </article>
          ))}
        </div>
      )}
    </div>
  );
}
