import React from "react";
import { Plus, Save, Trash2 } from "lucide-react";
import { useI18n } from "../lib/i18nContext";

type ChatPresetRow = {
  id: string;
  name: string;
  url: string;
};

function parsePresets(value: string): ChatPresetRow[] {
  try {
    const parsed = JSON.parse(value || "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.flatMap((entry, index) => {
      if (!entry || typeof entry !== "object" || Array.isArray(entry)) return [];
      const pair = Object.entries(entry)[0];
      if (!pair || typeof pair[0] !== "string" || typeof pair[1] !== "string") return [];
      return [{ id: `preset-${index}`, name: pair[0], url: pair[1] }];
    });
  } catch {
    return [];
  }
}

function serializePresets(rows: ChatPresetRow[]): string {
  return JSON.stringify(
    rows.map((row) => {
      const entry: Record<string, string> = {};
      entry[row.name.trim()] = row.url.trim();
      return entry;
    }),
  );
}

export default function ChatPresetsEditor({
  value,
  onSave,
  pending,
}: {
  value: string;
  onSave: (value: string) => void;
  pending: boolean;
}) {
  const { t } = useI18n();
  const [rows, setRows] = React.useState<ChatPresetRow[]>(() => parsePresets(value));
  const [dirty, setDirty] = React.useState(false);

  React.useEffect(() => {
    setRows(parsePresets(value));
    setDirty(false);
  }, [value]);

  const update = (id: string, patch: Partial<ChatPresetRow>) => {
    setRows((current) => current.map((row) => (row.id === id ? { ...row, ...patch } : row)));
    setDirty(true);
  };

  const add = () => {
    setRows((current) => [...current, { id: `new-${Date.now()}`, name: "", url: "" }]);
    setDirty(true);
  };

  const remove = (id: string) => {
    setRows((current) => current.filter((row) => row.id !== id));
    setDirty(true);
  };

  const valid = rows.every((row) => row.name.trim() && row.url.trim());

  return (
    <div className="settings-row chat-presets-editor">
      <div className="chat-presets-head">
        <div>
          <div className="text-sm font-medium">{t.chatLinks.presets}</div>
          <div className="text-xs mt-1" style={{ color: "var(--md-on-surface-variant)" }}>
            {t.chatLinks.editorHint}
          </div>
        </div>
        <button className="btn-outlined btn-sm" type="button" onClick={add}>
          <Plus size={13} /> {t.chatLinks.addPreset}
        </button>
      </div>

      <div className="chat-preset-rows">
        {rows.length === 0 ? (
          <div className="chat-preset-empty">{t.chatLinks.noPresets}</div>
        ) : (
          rows.map((row) => (
            <div className="chat-preset-row" key={row.id}>
              <input
                className="input font-mono"
                value={row.name}
                aria-label={t.chatLinks.presetName}
                placeholder={t.chatLinks.presetName}
                onChange={(event) => update(row.id, { name: event.target.value })}
              />
              <input
                className="input font-mono"
                value={row.url}
                aria-label={t.chatLinks.presetUrl}
                placeholder={t.chatLinks.presetUrl}
                onChange={(event) => update(row.id, { url: event.target.value })}
              />
              <button
                className="icon-button"
                type="button"
                aria-label={t.common.delete}
                title={t.common.delete}
                onClick={() => remove(row.id)}
              >
                <Trash2 size={14} />
              </button>
            </div>
          ))
        )}
      </div>

      {!valid && <div className="chat-preset-error">{t.chatLinks.invalidPreset}</div>}
      {dirty && (
        <div className="chat-presets-actions">
          <button
            className="btn-filled btn-sm"
            type="button"
            disabled={!valid || pending}
            onClick={() => onSave(serializePresets(rows))}
          >
            <Save size={13} /> {pending ? t.common.saving : t.common.save}
          </button>
        </div>
      )}
    </div>
  );
}
