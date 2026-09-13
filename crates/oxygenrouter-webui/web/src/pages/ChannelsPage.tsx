import React, { useMemo, useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  AlertCircle,
  CheckCircle,
  Copy,
  Edit,
  Eye,
  EyeOff,
  KeyRound,
  MoreVertical,
  Plus,
  Power,
  RefreshCw,
  Search,
  TestTube2,
  Trash2,
  XCircle,
} from "lucide-react";
import { api, Channel, ChannelTestResult, PROVIDER_PRESETS } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import { useToast } from "../lib/toast";
import Button from "../components/ui/Button";
import Dialog from "../components/ui/Dialog";
import Select from "../components/ui/Select";
import Switch from "../components/ui/Switch";
import Checkbox from "../components/ui/Checkbox";
import Menu from "../components/ui/Menu";
import Tooltip from "../components/ui/Tooltip";
import MultiKeyDialog from "../components/ui/MultiKeyDialog";

function BulkActionBar({ selected, onEnable, onDisable, onDelete, t }: { selected: Set<string>; onEnable: () => void; onDisable: () => void; onDelete: () => void; t: ReturnType<typeof useI18n>['t'] }) {
  if (selected.size === 0) return null;
  return (
    <div className="bulk-action-bar">
      <span className="bulk-selected-count">{t.channels.selected.replace("X", String(selected.size))}</span>
      <div className="flex gap-2">
        <button className="btn-outlined btn-sm" onClick={onEnable}>
          <Power size={12} /> {t.channels.enableSelected}
        </button>
        <button className="btn-outlined btn-sm" onClick={onDisable}>
          <XCircle size={12} /> {t.channels.disableSelected}
        </button>
        <button className="btn-outlined btn-sm" style={{ color: "var(--md-error)" }} onClick={onDelete}>
          <Trash2 size={12} /> {t.channels.deleteSelected}
        </button>
      </div>
    </div>
  );
}

function StatusDot({ status }: { status: "ok" | "error" | "pulse" | "off" }) {
  return <span className={`status-dot ${status === "error" ? "is-error" : ""} ${status === "pulse" ? "is-pulse" : ""}`} />;
}

function CopyInline({ value, label }: { value: string; label: string }) {
  const [copied, setCopied] = React.useState(false);
  return (
    <button
      className="icon-button"
      title={label}
      onClick={() => {
        navigator.clipboard?.writeText(value);
        setCopied(true);
        setTimeout(() => setCopied(false), 1200);
      }}
    >
      {copied ? <CheckCircle size={14} /> : <Copy size={14} />}
    </button>
  );
}

function ChannelModal({ channel, onClose }: { channel?: Channel; onClose: () => void }) {
  const { t } = useI18n();
  const [providerId, setProviderId] = useState<string>(channel?.provider ?? "openai");
  const preset = PROVIDER_PRESETS.find((p) => p.id === providerId) ?? PROVIDER_PRESETS[0];
  const [form, setForm] = useState<Partial<Channel>>(
    channel ?? {
      name: "",
      provider: "openai",
      base_url: "https://api.openai.com",
      api_key: "",
      priority: 0,
      weight: 1,
      test_model: "gpt-3.5-turbo",
      enabled: true,
      config: {},
       models: [],
       model_mapping: "",
       system_prompt: "",
       group_name: "default", tags: [], model_list: [], balance_micros: 0,
       response_headers: {}, status_code_mapping: {}, override_parameters: {},
    },
  );
  const [showKey, setShowKey] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [jsonError, setJsonError] = useState("");
  const [batchMode, setBatchMode] = useState(false);
  const [modelSearch, setModelSearch] = useState("");
  const [customModel, setCustomModel] = useState("");
  const qc = useQueryClient();
  const mut = useMutation({
    mutationFn: (data: Partial<Channel>) =>
      channel ? api.channels.update(channel.id, data) : api.channels.create(data),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["channels"] });
      onClose();
    },
  });
  const submit = () => {
    for (const value of [form.response_headers, form.status_code_mapping, form.override_parameters]) {
      if (!value || Array.isArray(value) || typeof value !== "object") { setJsonError("Advanced JSON fields must be JSON objects."); return; }
    }
    setJsonError("");
    mut.mutate({ ...form, model_list: form.model_list ?? form.models ?? [] });
  };

  const handleProviderChange = (newId: string) => {
    setProviderId(newId);
    const newPreset = PROVIDER_PRESETS.find((p) => p.id === newId);
    if (newPreset && !channel) {
      setForm({
        ...form,
        provider: newId,
        base_url: newPreset.baseUrl,
        test_model: newPreset.testModel,
        config: {},
        models: newPreset.defaultModels ?? [],
      });
    } else {
      setForm({ ...form, provider: newId });
    }
  };

  const handleConfigChange = (key: string, value: string) => {
    setForm({ ...form, config: { ...form.config, [key]: value } });
  };

  const availableModels = (preset.defaultModels ?? []).filter(
    (m) => !modelSearch || m.toLowerCase().includes(modelSearch.toLowerCase()),
  );

  const toggleModel = (model: string) => {
    const current = form.models ?? [];
    const next = current.includes(model) ? current.filter((m) => m !== model) : [...current, model];
    setForm({ ...form, models: next });
  };

  const fillAllModels = () => {
    setForm({ ...form, models: [...(preset.defaultModels ?? [])] });
  };

  const clearModels = () => {
    setForm({ ...form, models: [] });
  };

  const addCustomModel = () => {
    if (!customModel.trim()) return;
    const current = form.models ?? [];
    if (!current.includes(customModel.trim())) {
      setForm({ ...form, models: [...current, customModel.trim()] });
    }
    setCustomModel("");
  };

  return (
    <Dialog open onClose={onClose} title={channel ? t.channels.edit : t.channels.new} size="lg" footer={null}>
        <div className="space-y-3">
          {/* Name */}
          <div>
            <label className="field-label">{t.channels.name}</label>
            <input
              className="input"
              value={form.name ?? ""}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder={t.channels.namePlaceholder}
            />
          </div>

          {/* Provider Grid */}
          <div>
            <label className="field-label">{t.channels.provider}</label>
            <div className="provider-grid">
              {PROVIDER_PRESETS.map((p) => (
                <button
                  key={p.id}
                  className={`provider-pill ${providerId === p.id ? "is-active" : ""}`}
                  onClick={() => handleProviderChange(p.id)}
                  type="button"
                >
                  {p.name}
                </button>
              ))}
            </div>
          </div>

          {/* Base URL */}
          <div>
            <label className="field-label">{t.channels.baseUrl}</label>
            <input
              className="input font-mono text-xs"
              value={form.base_url ?? ""}
              onChange={(e) => setForm({ ...form, base_url: e.target.value })}
              placeholder={preset?.baseUrl ?? t.channels.baseUrlPlaceholder}
            />
          </div>

          {/* Provider-specific fields */}
          {preset.fields?.map((field) => (
            <div key={field.key}>
              <label className="field-label">{field.label}</label>
              {field.type === "textarea" ? (
                <textarea
                  className="input font-mono text-xs"
                  rows={3}
                  value={(form.config ?? {})[field.key] ?? ""}
                  onChange={(e) => handleConfigChange(field.key, e.target.value)}
                  placeholder={field.placeholder}
                />
              ) : (
                <input
                  className="input font-mono text-xs"
                  type={field.type ?? "text"}
                  value={(form.config ?? {})[field.key] ?? ""}
                  onChange={(e) => handleConfigChange(field.key, e.target.value)}
                  placeholder={field.placeholder}
                />
              )}
            </div>
          ))}

          {/* API Key */}
          <div>
            <label className="field-label">{t.channels.apiKey}</label>
            <div className="flex gap-2">
              {batchMode ? (
                <textarea
                  className="input flex-1 font-mono text-xs"
                  rows={3}
                  value={form.api_key ?? ""}
                  onChange={(e) => setForm({ ...form, api_key: e.target.value })}
                  placeholder={t.channels.batchKeyPlaceholder}
                />
              ) : (
                <input
                  className="input flex-1 font-mono text-xs"
                  type={showKey ? "text" : "password"}
                  value={form.api_key ?? ""}
                  onChange={(e) => setForm({ ...form, api_key: e.target.value })}
                  placeholder="sk-..."
                />
              )}
              <div className="flex flex-col gap-1">
                <button className="btn-ghost p-2" onClick={() => setShowKey((v) => !v)}>
                  {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
                </button>
              </div>
            </div>
            {!channel && (
              <div className="mt-1.5">
                <Checkbox checked={batchMode} onChange={setBatchMode} label={t.channels.batchMode} />
              </div>
            )}
          </div>

          {/* Priority + Weight */}
           <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="field-label">{t.channels.priority}</label>
              <input
                className="input"
                type="number"
                value={form.priority ?? 0}
                onChange={(e) => setForm({ ...form, priority: Number(e.target.value) })}
              />
           </div>
           <div className="grid grid-cols-2 gap-3"><div><label className="field-label">Group</label><input className="input" value={form.group_name ?? "default"} onChange={(e) => setForm({ ...form, group_name: e.target.value || "default" })} /></div><div><label className="field-label">Balance (micros)</label><input className="input" type="number" value={form.balance_micros ?? 0} onChange={(e) => setForm({ ...form, balance_micros: Number(e.target.value) })} /></div></div>
           <div><label className="field-label">Tags (CSV)</label><input className="input" value={(form.tags ?? []).join(", ")} onChange={(e) => setForm({ ...form, tags: e.target.value.split(",").map((v) => v.trim()).filter(Boolean) })} /></div>
           <div><label className="field-label">Declared models (CSV)</label><input className="input" value={(form.model_list ?? form.models ?? []).join(", ")} onChange={(e) => setForm({ ...form, model_list: e.target.value.split(",").map((v) => v.trim()).filter(Boolean) })} /></div>
            <div>
              <label className="field-label">{t.channels.weight}</label>
              <input
                className="input"
                type="number"
                value={form.weight ?? 1}
                onChange={(e) => setForm({ ...form, weight: Number(e.target.value) })}
              />
            </div>
          </div>

          {/* Test Model */}
          <div>
            <label className="field-label">{t.channels.testModel}</label>
            <input
              className="input font-mono text-xs"
              value={form.test_model ?? preset?.testModel ?? "gpt-3.5-turbo"}
              onChange={(e) => setForm({ ...form, test_model: e.target.value })}
            />
          </div>

          {/* Models Selection */}
          {preset.defaultModels && preset.defaultModels.length > 0 && (
            <div>
              <label className="field-label">{t.channels.models} ({(form.models ?? []).length})</label>
              <div className="model-select-box">
                <div className="model-select-toolbar">
                  <div className="flex items-center gap-1 flex-1">
                    <Search size={12} style={{ color: "var(--md-on-surface-variant)" }} />
                    <input
                      className="model-search-input"
                      placeholder={t.channels.modelSearchPlaceholder}
                      value={modelSearch}
                      onChange={(e) => setModelSearch(e.target.value)}
                    />
                  </div>
                  <button className="model-action-btn" onClick={fillAllModels} type="button">
                    {t.channels.fillModels}
                  </button>
                  <button className="model-action-btn" onClick={clearModels} type="button">
                    {t.channels.clearModels}
                  </button>
                </div>
                <div className="model-checkbox-grid">
                  {availableModels.map((m) => (
                    <div key={m} className="model-checkbox-item">
                      <Checkbox
                        checked={(form.models ?? []).includes(m)}
                        onChange={() => toggleModel(m)}
                        label={m}
                      />
                    </div>
                  ))}
                </div>
                <div className="model-add-custom">
                  <input
                    className="model-search-input flex-1"
                    placeholder={t.channels.customModelPlaceholder}
                    value={customModel}
                    onChange={(e) => setCustomModel(e.target.value)}
                    onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); addCustomModel(); } }}
                  />
                  <button className="model-action-btn" onClick={addCustomModel} type="button">
                    <Plus size={11} />
                  </button>
                </div>
              </div>
            </div>
          )}

          {/* Advanced Settings Toggle */}
          <button
            className="btn-ghost text-xs flex items-center gap-1.5 w-full justify-start"
            onClick={() => setShowAdvanced((v) => !v)}
            type="button"
          >
            {t.channels.advancedSettings}
            <span className={`transition-transform ${showAdvanced ? "rotate-90" : ""}`} style={{ display: "inline-flex" }}>▶</span>
          </button>

          {showAdvanced && (
            <div className="space-y-3 animate-slide-down">
              {/* Model Mapping */}
              {preset.supportsModelMapping && (
                <div>
                  <label className="field-label">{t.channels.modelMapping}</label>
                  <textarea
                    className="input font-mono text-xs"
                    rows={4}
                    value={form.model_mapping ?? ""}
                    onChange={(e) => setForm({ ...form, model_mapping: e.target.value })}
                    placeholder={`{\n  "gpt-4o": "gpt-4-turbo"\n}`}
                  />
                </div>
              )}

              {/* System Prompt */}
               {preset.supportsSystemPrompt && (
                <div>
                  <label className="field-label">{t.channels.systemPrompt}</label>
                  <textarea
                    className="input text-xs"
                    rows={3}
                    value={form.system_prompt ?? ""}
                    onChange={(e) => setForm({ ...form, system_prompt: e.target.value })}
                    placeholder={t.channels.systemPromptPlaceholder}
                  />
                </div>
               )}
               {(["response_headers", "status_code_mapping", "override_parameters"] as const).map((field) => <div key={field}><label className="field-label">{field.replace(/_/g, " ")}</label><textarea className="input font-mono text-xs" rows={3} value={JSON.stringify(form[field] ?? {}, null, 2)} onChange={(e) => { try { const value = JSON.parse(e.target.value); setForm({ ...form, [field]: value }); setJsonError(""); } catch { setJsonError(`${field} contains invalid JSON.`); } }} /></div>)}
            </div>
          )}
        </div>

        <div className="modal-footer">
          <Button variant="outlined" onClick={onClose}>{t.common.cancel}</Button>
          <Button loading={mut.isPending} onClick={submit}>{channel ? t.common.save : t.common.create}</Button>
        </div>
        {jsonError && <div className="error-banner mt-3 text-xs">{jsonError}</div>}
        {mut.isError && <div className="error-banner mt-3 text-xs">{String(mut.error)}</div>}
    </Dialog>
  );
}

function TestStatus({ result }: { result?: ChannelTestResult }) {
  const { t } = useI18n();
  if (!result) return null;
  if (result.success) {
    return <span className="badge badge-success">{t.channels.ok} {result.latency_ms}{t.dashboard.ms}</span>;
  }
  return <span className="badge badge-error" title={result.error ?? ""}>{t.channels.err}</span>;
}

function ProviderBadge({ provider }: { provider?: string }) {
  const preset = PROVIDER_PRESETS.find((p) => p.id === provider);
  return (
    <span className="provider-badge">
      <span className="provider-badge-dot" />
      {preset?.name ?? provider ?? "OpenAI"}
    </span>
  );
}

export default function ChannelsPage() {
  const { t } = useI18n();
  const { data: channels, isLoading } = useQuery({ queryKey: ["channels"], queryFn: api.channels.list });
  const toast = useToast();
  const qc = useQueryClient();
  const [testResults, setTestResults] = useState<Record<string, ChannelTestResult>>({});
  const [testing, setTesting] = useState<Record<string, boolean>>({});
  const [testingAll, setTestingAll] = useState(false);
  const [modal, setModal] = useState<Channel | "new" | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<Channel | null>(null);
  const [confirmBulkDelete, setConfirmBulkDelete] = useState(false);
  const [keyChannel, setKeyChannel] = useState<Channel | null>(null);
  const [search, setSearch] = useState("");
  const [showDisabled, setShowDisabled] = useState(true);
  const [providerFilter, setProviderFilter] = useState<string>("all");
  const [groupFilter, setGroupFilter] = useState<string>("all");
  const [selected, setSelected] = useState<Set<string>>(new Set());

  const testMutation = useMutation({
    mutationFn: (id: string) => api.channels.test(id),
    onMutate: (id) => setTesting((prev) => ({ ...prev, [id]: true })),
    onSuccess: (result, id) => {
      setTestResults((prev) => ({ ...prev, [id]: result }));
      setTesting((prev) => ({ ...prev, [id]: false }));
    },
    onError: (_, id) => setTesting((prev) => ({ ...prev, [id]: false })),
  });

  const toggleMutation = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      api.channels.update(id, { enabled }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["channels"] }),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.channels.delete(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["channels"] }),
  });

  const fetchModelsMutation = useMutation({
    mutationFn: (id: string) => api.channels.fetchModels(id),
    onSuccess: (models) => {
      qc.invalidateQueries({ queryKey: ["channels"] });
      toast.success(t.channels.modelsSynced, `${models.length} ${t.channels.modelsAvailable}`);
    },
    onError: (error: unknown) => toast.error(t.channels.modelsSyncFailed, String((error as Error)?.message ?? error)),
  });

  const batchEnableMutation = useMutation({
    mutationFn: (ids: string[]) => api.channels.batchUpdate(ids, true),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["channels"] });
      setSelected(new Set());
    },
  });

  const batchDisableMutation = useMutation({
    mutationFn: (ids: string[]) => api.channels.batchUpdate(ids, false),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["channels"] });
      setSelected(new Set());
    },
  });

  const batchDeleteMutation = useMutation({
    mutationFn: (ids: string[]) => api.channels.batchDelete(ids),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["channels"] });
      setSelected(new Set());
    },
  });

  const testAll = async () => {
    if (!channels) return;
    setTestingAll(true);
    await Promise.all(
      channels.map(async (ch) => {
        setTesting((prev) => ({ ...prev, [ch.id]: true }));
        try {
          const result = await api.channels.test(ch.id);
          setTestResults((prev) => ({ ...prev, [ch.id]: result }));
        } catch {
          // ignore
        } finally {
          setTesting((prev) => ({ ...prev, [ch.id]: false }));
        }
      }),
    );
    setTestingAll(false);
  };

  const filtered = useMemo(() => {
    if (!channels) return [];
    const q = search.trim().toLowerCase();
    return channels
      .filter((c) => (showDisabled ? true : c.enabled))
       .filter((c) => (providerFilter === "all" ? true : c.provider === providerFilter))
       .filter((c) => (groupFilter === "all" ? true : c.group_name === groupFilter))
      .filter(
        (c) =>
          !q ||
          c.name.toLowerCase().includes(q) ||
          c.base_url.toLowerCase().includes(q) ||
          (c.provider ?? "").toLowerCase().includes(q) ||
          c.test_model.toLowerCase().includes(q),
      );
  }, [channels, search, showDisabled, providerFilter, groupFilter]);

  const enabledCount = channels?.filter((c) => c.enabled).length ?? 0;
  const totalCount = channels?.length ?? 0;

  return (
    <div className="page-fade-enter">
      <PageHeader
        title={t.channels.title}
        description={t.channels.description}
        action={
          <div className="flex items-center gap-2">
            {channels && channels.length > 0 && (
              <button className="btn-outlined btn-sm" onClick={testAll} disabled={testingAll}>
                <TestTube2 size={13} className={testingAll ? "animate-spin" : ""} />
                {testingAll ? t.channels.testing : t.channels.testAll}
              </button>
            )}
            <button className="btn-filled" onClick={() => setModal("new")}>
              <Plus size={14} /> {t.channels.add}
            </button>
          </div>
        }
      />

      <div className="log-filters">
         <div className="log-filter">
          <Search size={12} style={{ color: "var(--md-on-surface-variant)" }} />
          <input
            placeholder={t.channels.search}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="w-56"
          />
         </div>
         <div className="log-filter">
           <Select
             value={groupFilter}
             onChange={setGroupFilter}
             ariaLabel={t.channels.group}
             size="sm"
             options={[
               { value: "all", label: t.channels.allGroups },
               ...Array.from(new Set(channels?.map((channel) => channel.group_name || "default") ?? [])).map((group) => ({ value: group, label: group })),
             ]}
           />
         </div>
        <div className="log-filter">
          <Select
            value={providerFilter}
            onChange={setProviderFilter}
            ariaLabel={t.channels.allProviders}
            size="sm"
            options={[
              { value: "all", label: t.channels.allProviders },
              ...PROVIDER_PRESETS.map((p) => ({ value: p.id, label: p.name })),
            ]}
          />
        </div>
        <div className="log-filter">
          <Checkbox checked={showDisabled} onChange={setShowDisabled} label={t.channels.showDisabled} />
        </div>
        <span className="text-xs text-[var(--md-on-surface-variant)] ml-auto font-mono">
          {filtered.length} / {totalCount}
        </span>
      </div>

      {isLoading ? (
        <div className="card-grid">
          {[0, 1, 2, 3].map((i) => (
            <div key={i} className="card channel-card">
              <div className="skeleton h-3 w-24" />
              <div className="skeleton h-3 w-40" />
              <div className="skeleton h-3 w-32" />
            </div>
          ))}
        </div>
      ) : filtered.length === 0 ? (
        <div className="empty-state">
          <AlertCircle size={40} />
          <p className="text-sm">{t.channels.empty}</p>
          <button className="btn-filled mt-4" onClick={() => setModal("new")}>
            <Plus size={14} /> {t.channels.add}
          </button>
        </div>
      ) : (
        <>
        <div className="card-grid">
          {filtered.map((ch) => {
            const lastTest = testResults[ch.id];
            const isTesting = testing[ch.id];
            return (
              <div className={`card channel-card click-ripple ${!ch.enabled ? "is-disabled" : ""} ${selected.has(ch.id) ? "is-selected" : ""}`} key={ch.id}>
                <div className="channel-card-checkbox" onClick={(e) => e.stopPropagation()}>
                  <Checkbox
                    checked={selected.has(ch.id)}
                    onChange={(checked) => {
                      setSelected((prev) => {
                        const next = new Set(prev);
                        if (checked) next.add(ch.id);
                        else next.delete(ch.id);
                        return next;
                      });
                    }}
                  />
                </div>
                <div className="channel-card-head">
                  <StatusDot
                    status={
                      !ch.enabled
                        ? "off"
                        : isTesting
                        ? "pulse"
                        : lastTest?.success
                        ? "ok"
                        : lastTest?.success === false
                        ? "error"
                        : "pulse"
                    }
                  />
                  <span className="channel-card-name" title={ch.name}>{ch.name}</span>
                  <TestStatus result={lastTest} />
                </div>
                <div className="channel-card-provider">
                  <ProviderBadge provider={ch.provider} />
                </div>
                <div className="channel-card-base" title={ch.base_url}>
                  <span className="flex-1 truncate">{ch.base_url}</span>
                  <CopyInline value={ch.base_url} label={t.channels.copyUrl} />
                </div>
                <div className="channel-card-meta">
                  <span className="channel-meta-chip">p {ch.priority}</span>
                   <span className="channel-meta-chip">w {ch.weight}</span>
                   <span className="channel-meta-chip">{ch.group_name || "default"}</span>
                  <span className="channel-card-model font-mono">{ch.test_model}</span>
                </div>
                {lastTest?.error && <div className="channel-health-error" title={lastTest.error}>{lastTest.error}</div>}
                <div className="channel-card-actions">
                  <Tooltip content={ch.enabled ? t.channels.clickDisable : t.channels.clickEnable} placement="top">
                    <Switch
                      size="sm"
                      checked={ch.enabled}
                      onChange={(checked) => toggleMutation.mutate({ id: ch.id, enabled: checked })}
                    />
                  </Tooltip>
                  <div className="flex-1" />
                  <Tooltip content={t.channels.test}>
                    <button className="btn-ghost p-1.5" onClick={() => testMutation.mutate(ch.id)}>
                      {isTesting ? <RefreshCw size={13} className="animate-spin" /> : <TestTube2 size={13} />}
                    </button>
                  </Tooltip>
                  <Tooltip content={t.common.edit}>
                    <button className="btn-ghost p-1.5" onClick={() => setModal(ch)}>
                      <Edit size={13} />
                    </button>
                  </Tooltip>
                  <Menu
                    align="end"
                    ariaLabel={t.channels.title}
                    trigger={({ toggle }) => (
                      <button className="btn-ghost p-1.5" onClick={toggle} aria-label="More actions">
                        <MoreVertical size={13} />
                      </button>
                    )}
                    items={[
                      { id: "test", label: t.channels.test, icon: TestTube2, onSelect: () => testMutation.mutate(ch.id) },
                      { id: "edit", label: t.common.edit, icon: Edit, onSelect: () => setModal(ch) },
                      { id: "models", label: t.channels.syncModels, icon: RefreshCw, onSelect: () => fetchModelsMutation.mutate(ch.id) },
                      { id: "keys", label: t.channels.multiKeys, icon: KeyRound, onSelect: () => setKeyChannel(ch) },
                      { id: "delete", label: t.channels.delete, icon: Trash2, danger: true, onSelect: () => setConfirmDelete(ch) },
                    ]}
                  />
                </div>
              </div>
            );
          })}
        </div>
        <BulkActionBar
          selected={selected}
          onEnable={() => batchEnableMutation.mutate(Array.from(selected))}
          onDisable={() => batchDisableMutation.mutate(Array.from(selected))}
          onDelete={() => {
            if (selected.size > 0) setConfirmBulkDelete(true);
          }}
          t={t}
        />
        </>
      )}
      <Dialog
        open={confirmDelete !== null}
        onClose={() => setConfirmDelete(null)}
        title={t.channels.delete}
        description={confirmDelete ? t.channels.deleteConfirm(confirmDelete.name) : ""}
        size="sm"
        footer={
          <>
            <Button variant="outlined" onClick={() => setConfirmDelete(null)}>{t.common.cancel}</Button>
            <Button variant="filled" className="danger-button" leftIcon={<Trash2 size={13} />} onClick={() => { if (confirmDelete) { deleteMutation.mutate(confirmDelete.id); setConfirmDelete(null); } }}>
              {t.channels.delete}
            </Button>
          </>
        }
      >
        <span />
      </Dialog>
      <Dialog
        open={confirmBulkDelete}
        onClose={() => setConfirmBulkDelete(false)}
        title={t.channels.deleteSelected}
        description={t.channels.deleteSelectedConfirm.replace("X", String(selected.size))}
        size="sm"
        footer={
          <>
            <Button variant="outlined" onClick={() => setConfirmBulkDelete(false)}>{t.common.cancel}</Button>
            <Button variant="filled" className="danger-button" leftIcon={<Trash2 size={13} />} onClick={() => { batchDeleteMutation.mutate(Array.from(selected)); setConfirmBulkDelete(false); }}>
              {t.channels.deleteSelected}
            </Button>
          </>
        }
      >
        <span />
      </Dialog>
      {keyChannel && <MultiKeyDialog channel={keyChannel} onClose={() => setKeyChannel(null)} />}
      {modal !== null && (
        <ChannelModal channel={modal === "new" ? undefined : modal} onClose={() => setModal(null)} />
      )}
    </div>
  );
}
