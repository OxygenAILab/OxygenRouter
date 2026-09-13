import React, { useMemo, useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  AlertCircle,
  ArrowRight,
  Copy,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  XCircle,
} from "lucide-react";
import { api, ModelMap, RouteRule } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import Select from "../components/ui/Select";

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
      {copied ? <span className="text-[var(--md-success)]">✓</span> : <Copy size={14} />}
    </button>
  );
}

function ModelMapModal({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const [form, setForm] = useState({ channel_id: "", pattern: "", target_model: "" });
  const qc = useQueryClient();
  const { data: channels } = useQuery({ queryKey: ["channels"], queryFn: api.channels.list });
  const mut = useMutation({
    mutationFn: (data: Partial<ModelMap>) => api.modelMaps.create(data),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["modelMaps"] });
      onClose();
    },
  });
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-content modal-md" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2>{t.routes.newMap}</h2>
          <button className="btn-ghost p-1" onClick={onClose} aria-label="Close">
            <XCircle size={16} />
          </button>
        </div>
        <div className="space-y-3">
          <div>
            <label className="field-label">{t.channels.name}</label>
            <Select
              value={form.channel_id}
              onChange={(value) => setForm({ ...form, channel_id: value })}
              placeholder={t.routes.selectChannel}
              ariaLabel={t.channels.name}
              fullWidth
              options={[
                { value: "", label: t.routes.selectChannel },
                ...(channels ?? []).map((c) => ({ value: c.id, label: c.name })),
              ]}
            />
          </div>
          <div>
            <label className="field-label">{t.routes.pattern}</label>
            <input
              className="input font-mono"
              value={form.pattern}
              onChange={(e) => setForm({ ...form, pattern: e.target.value })}
              placeholder={t.routes.patternPlaceholder}
            />
          </div>
          <div>
            <label className="field-label">{t.routes.target}</label>
            <input
              className="input font-mono"
              value={form.target_model}
              onChange={(e) => setForm({ ...form, target_model: e.target.value })}
              placeholder={t.routes.targetPlaceholder}
            />
          </div>
        </div>
        <div className="modal-footer">
          <button className="btn-outlined" onClick={onClose}>{t.common.cancel}</button>
          <button
            className="btn-filled"
            onClick={() => mut.mutate(form)}
            disabled={mut.isPending || !form.channel_id || !form.pattern || !form.target_model}
          >
            {mut.isPending ? t.common.saving : t.common.create}
          </button>
        </div>
      </div>
    </div>
  );
}

function RuleModal({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const [form, setForm] = useState({ name: "", rule_type: "keyword_match", priority: 0, config: {} });
  const qc = useQueryClient();
  const mut = useMutation({
    mutationFn: (data: Partial<RouteRule>) => api.rules.create(data as any),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["rules"] });
      onClose();
    },
  });
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-content modal-md" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2>{t.routes.newRule}</h2>
          <button className="btn-ghost p-1" onClick={onClose} aria-label="Close">
            <XCircle size={16} />
          </button>
        </div>
        <div className="space-y-3">
          <div>
            <label className="field-label">{t.channels.name}</label>
            <input className="input" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder={t.routes.ruleName} />
          </div>
          <div>
            <label className="field-label">{t.routes.ruleType}</label>
            <Select
              value={form.rule_type}
              onChange={(value) => setForm({ ...form, rule_type: value })}
              ariaLabel={t.routes.ruleType}
              fullWidth
              options={[
                { value: "auto_heuristic", label: t.routes.autoHeuristic },
                { value: "keyword_match", label: t.routes.keywordMatch },
                { value: "type_routing", label: t.routes.typeRouting },
              ]}
            />
          </div>
          <div>
            <label className="field-label">{t.channels.priority}</label>
            <input className="input" type="number" value={form.priority} onChange={(e) => setForm({ ...form, priority: Number(e.target.value) })} />
          </div>
        </div>
        <div className="modal-footer">
          <button className="btn-outlined" onClick={onClose}>{t.common.cancel}</button>
          <button className="btn-filled" onClick={() => mut.mutate(form)} disabled={mut.isPending || !form.name}>
            {mut.isPending ? t.common.saving : t.common.create}
          </button>
        </div>
      </div>
    </div>
  );
}

export default function RoutesPage() {
  const { t } = useI18n();
  const { data: maps, isLoading: mapsLoading } = useQuery({ queryKey: ["modelMaps"], queryFn: api.modelMaps.list });
  const { data: rules, isLoading: rulesLoading } = useQuery({ queryKey: ["rules"], queryFn: api.rules.list });
  const { data: channels } = useQuery({ queryKey: ["channels"], queryFn: api.channels.list });
  const qc = useQueryClient();
  const [mapModal, setMapModal] = useState(false);
  const [ruleModal, setRuleModal] = useState(false);
  const [mapSearch, setMapSearch] = useState("");
  const [ruleSearch, setRuleSearch] = useState("");

  const deleteMap = useMutation({
    mutationFn: (id: string) => api.modelMaps.delete(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["modelMaps"] }),
  });
  const deleteRule = useMutation({
    mutationFn: (id: string) => api.rules.delete(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["rules"] }),
  });

  const channelName = (id: string) => channels?.find((c) => c.id === id)?.name ?? id.slice(0, 8);
  const isLoading = mapsLoading || rulesLoading;

  const filteredMaps = useMemo(() => {
    if (!maps) return [];
    const q = mapSearch.trim().toLowerCase();
    if (!q) return maps;
    return maps.filter((m) =>
      m.pattern.toLowerCase().includes(q) ||
      m.target_model.toLowerCase().includes(q) ||
      channelName(m.channel_id).toLowerCase().includes(q),
    );
  }, [maps, mapSearch, channels]);

  const filteredRules = useMemo(() => {
    if (!rules) return [];
    const q = ruleSearch.trim().toLowerCase();
    if (!q) return rules;
    return rules.filter((r) =>
      r.name.toLowerCase().includes(q) ||
      r.rule_type.toLowerCase().includes(q),
    );
  }, [rules, ruleSearch]);

  return (
    <div className="page-fade-enter">
      <PageHeader title={t.routes.title} description={t.routes.description} />
      {isLoading ? (
        <div className="space-y-3">
          {[0, 1].map((i) => (
            <div key={i} className="card p-4">
              <div className="skeleton h-3 w-40" />
            </div>
          ))}
        </div>
      ) : (
        <div className="space-y-6">
          <div>
            <div className="flex items-center justify-between mb-3">
              <div>
                <h2 className="text-sm font-semibold">{t.routes.modelMaps}</h2>
                <p className="text-xs mt-0.5 text-[var(--md-on-surface-variant)]">{t.routes.modelMapsDesc}</p>
              </div>
              <button className="btn-outlined text-xs" onClick={() => setMapModal(true)}>
                <Plus size={12} /> {t.routes.addMap}
              </button>
            </div>
            <div className="log-filters">
              <div className="log-filter">
                <Search size={12} style={{ color: "var(--md-on-surface-variant)" }} />
                <input
                  placeholder={t.routes.pattern}
                  value={mapSearch}
                  onChange={(e) => setMapSearch(e.target.value)}
                  className="w-56"
                />
              </div>
              <span className="text-xs text-[var(--md-on-surface-variant)] ml-auto font-mono">
                {filteredMaps.length} / {maps?.length ?? 0}
              </span>
            </div>
            {!filteredMaps.length ? (
              <div className="empty-state py-8">
                <AlertCircle size={32} />
                <p className="text-xs mt-2">{maps?.length ? "No matches" : t.routes.emptyMap}</p>
              </div>
            ) : (
              <div className="card-grid">
                {filteredMaps.map((m) => (
                  <div className="card channel-card click-ripple" key={m.id}>
                    <div className="flex items-center gap-2 font-mono text-sm">
                      <span className="truncate flex-1">{m.pattern}</span>
                      <ArrowRight size={14} className="text-[var(--md-on-surface-variant)]" />
                      <span className="truncate flex-1">{m.target_model}</span>
                      <CopyInline value={`${m.pattern} -> ${m.target_model}`} label="Copy mapping" />
                    </div>
                    <div className="text-xs text-[var(--md-on-surface-variant)]">Channel · {channelName(m.channel_id)}</div>
                    <div className="flex gap-2 mt-1">
                      <button className="btn-ghost p-1.5" style={{ color: "var(--md-error)" }} onClick={() => deleteMap.mutate(m.id)}>
                        <Trash2 size={13} />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>

          <div>
            <div className="flex items-center justify-between mb-3">
              <div>
                <h2 className="text-sm font-semibold">{t.routes.rules}</h2>
                <p className="text-xs mt-0.5 text-[var(--md-on-surface-variant)]">{t.routes.rulesDesc}</p>
              </div>
              <button className="btn-outlined text-xs" onClick={() => setRuleModal(true)}>
                <Plus size={12} /> {t.routes.addRule}
              </button>
            </div>
            <div className="log-filters">
              <div className="log-filter">
                <Search size={12} style={{ color: "var(--md-on-surface-variant)" }} />
                <input
                  placeholder={t.routes.rules}
                  value={ruleSearch}
                  onChange={(e) => setRuleSearch(e.target.value)}
                  className="w-56"
                />
              </div>
              <span className="text-xs text-[var(--md-on-surface-variant)] ml-auto font-mono">
                {filteredRules.length} / {rules?.length ?? 0}
              </span>
            </div>
            {!filteredRules.length ? (
              <div className="empty-state py-8">
                <AlertCircle size={32} />
                <p className="text-xs mt-2">{rules?.length ? "No matches" : t.routes.emptyRule}</p>
              </div>
            ) : (
              <div className="space-y-2">
                {filteredRules.map((r) => (
                  <div className="card p-3 flex items-center justify-between click-ripple" key={r.id}>
                    <div className="flex items-center gap-3">
                      <span className="badge badge-neutral text-xs">p {r.priority}</span>
                      <span className="text-sm font-medium">{r.name}</span>
                      <span className="badge badge-info text-xs">{r.rule_type.replace("_", " ")}</span>
                    </div>
                    <button className="btn-ghost p-1" style={{ color: "var(--md-error)" }} onClick={() => deleteRule.mutate(r.id)}>
                      <Trash2 size={12} />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      )}
      {mapModal && <ModelMapModal onClose={() => setMapModal(false)} />}
      {ruleModal && <RuleModal onClose={() => setRuleModal(false)} />}
    </div>
  );
}
