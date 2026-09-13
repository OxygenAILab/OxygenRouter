import React, { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  AlertCircle,
  CheckCircle,
  Copy,
  Eye,
  EyeOff,
  LayoutGrid,
  MoreVertical as MoreVerticalIcon,
  Plus,
  RefreshCw,
  Search,
  Table2,
  Trash2,
} from "lucide-react";
import { api, ApiKey, ApiKeyUsage } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import { useToast } from "../lib/toast";
import Button from "../components/ui/Button";
import Dialog from "../components/ui/Dialog";
import Select from "../components/ui/Select";
import Switch from "../components/ui/Switch";
import SegmentedControl from "../components/ui/SegmentedControl";
import DataTable, { DataTableColumn } from "../components/ui/DataTable";
import Pagination from "../components/ui/Pagination";
import Tooltip from "../components/ui/Tooltip";
import Menu from "../components/ui/Menu";
import Slider from "../components/ui/Slider";

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

function KeyModal({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const toast = useToast();
  const [form, setForm] = useState<Partial<ApiKey>>({ name: "", key: "", quota_micros: 0, allowed_models: [], ip_allowlist: [], group_name: "default", cross_group_retry: true });
  const [showKey, setShowKey] = useState(true);
  const [advanced, setAdvanced] = useState(false);
  const qc = useQueryClient();
  const mut = useMutation({
    mutationFn: (data: Partial<ApiKey>) => api.keys.create(data),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["keys"] });
      qc.invalidateQueries({ queryKey: ["keyUsage"] });
      toast.success(t.keys.created, t.keys.createdDesc);
      onClose();
    },
    onError: (e: unknown) => toast.error("Error", String((e as any)?.message ?? e)),
  });
  return (
    <Dialog open onClose={onClose} title={t.keys.new} size="md" footer={null}>
        <div className="space-y-3">
          <div>
            <label className="field-label">{t.keys.name}</label>
            <input
              className="input"
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              placeholder={t.keys.namePlaceholder}
            />
          </div>
          <button className="btn-ghost text-xs" type="button" onClick={() => setAdvanced((value) => !value)}>Advanced {advanced ? "▼" : "▶"}</button>
          {advanced && <div className="space-y-3 animate-slide-down">
            <div className="grid grid-cols-2 gap-3"><div><label className="field-label">Expiry</label><input className="input" type="text" placeholder="YYYY-MM-DD HH:mm" value={form.expires_at?.slice(0, 16).replace("T", " ") ?? ""} onChange={(e) => setForm({ ...form, expires_at: e.target.value ? new Date(e.target.value.replace(" ", "T")).toISOString() : null })} /></div><div><Slider value={form.quota_micros ?? 0} onChange={(value) => setForm({ ...form, quota_micros: value })} min={0} max={10000000} step={500000} label="Quota (USD)" formatValue={(value) => `$${(value / 1000000).toFixed(2)}`} /></div></div>
            <div><label className="field-label">Allowed models (CSV, empty = all)</label><input className="input" value={(form.allowed_models ?? []).join(", ")} onChange={(e) => setForm({ ...form, allowed_models: e.target.value.split(",").map((v) => v.trim()).filter(Boolean) })} /></div>
            <div><label className="field-label">IP allowlist (exact IP or *, empty = all)</label><input className="input" value={(form.ip_allowlist ?? []).join(", ")} onChange={(e) => setForm({ ...form, ip_allowlist: e.target.value.split(",").map((v) => v.trim()).filter(Boolean) })} /></div>
            <div><label className="field-label">Group</label><input className="input" value={form.group_name ?? "default"} onChange={(e) => setForm({ ...form, group_name: e.target.value || "default" })} /></div>
            <Switch checked={form.cross_group_retry ?? true} onChange={(checked) => setForm({ ...form, cross_group_retry: checked })} label="Allow cross-group retries" />
          </div>}
          <div>
            <label className="field-label">{t.keys.key}</label>
            <div className="flex gap-2">
              <input
                className="input font-mono flex-1"
                type={showKey ? "text" : "password"}
                value={form.key}
                onChange={(e) => setForm({ ...form, key: e.target.value })}
                placeholder={t.keys.keyPlaceholder}
              />
              <button className="btn-ghost p-2" onClick={() => setShowKey((v) => !v)}>
                {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
            </div>
          </div>
        </div>
        <div className="modal-footer">
          <Button variant="outlined" onClick={onClose}>{t.common.cancel}</Button>
          <Button loading={mut.isPending} onClick={() => mut.mutate(form)} disabled={!form.key}>{t.common.create}</Button>
        </div>
        {mut.isError && <div className="error-banner mt-3 text-xs">{String(mut.error)}</div>}
    </Dialog>
  );
}

export default function KeysPage() {
  const { t } = useI18n();
  const toast = useToast();
  const { data: keys, isLoading } = useQuery({ queryKey: ["keys"], queryFn: api.keys.list });
  const { data: usage } = useQuery<ApiKeyUsage[]>({
    queryKey: ["keyUsage"],
    queryFn: api.keys.usage,
    refetchInterval: 15_000,
  });
  const qc = useQueryClient();
  const [showKeyId, setShowKeyId] = useState<string | null>(null);
  const [modal, setModal] = useState(false);
  const [search, setSearch] = useState("");

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.keys.delete(id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["keys"] });
      qc.invalidateQueries({ queryKey: ["keysPage"] });
      qc.invalidateQueries({ queryKey: ["keyUsage"] });
      toast.success(t.keys.deleted, "");
    },
    onError: (e: unknown) => toast.error("Error", String((e as any)?.message ?? e)),
  });

  const usageByKey = React.useMemo(() => {
    const map: Record<string, ApiKeyUsage> = {};
    usage?.forEach((u) => { map[u.api_key_id] = u; });
    return map;
  }, [usage]);

  const totalRequests = usage?.reduce((sum, u) => sum + u.total_requests, 0) ?? 0;
  const totalTokens = usage?.reduce((sum, u) => sum + u.total_tokens, 0) ?? 0;

  // NewAPI-style table view state (server-side pagination)
  const [viewMode, setViewMode] = useState<"cards" | "table">("table");
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const { data: pageData } = useQuery({
    queryKey: ["keysPage", page, pageSize, search],
    queryFn: () => api.keys.query({ page, pageSize, search: search || undefined }),
  });
  const paged = pageData?.items ?? [];
  const searchActive = search.trim().length > 0;
  const filtered = searchActive ? (keys ?? []).filter((k) => k.name.toLowerCase().includes(search.trim().toLowerCase()) || k.key.toLowerCase().includes(search.trim().toLowerCase())) : (keys ?? []);
  const shown = searchActive ? filtered.slice((page - 1) * pageSize, page * pageSize) : paged;
  const total = searchActive ? filtered.length : (pageData?.total ?? 0);
  React.useEffect(() => { setPage(1); }, [search]);

  const usd = (micros: number) => `$${(micros / 1_000_000).toFixed(2)}`;

  const columns: DataTableColumn<ApiKey>[] = [
    {
      key: "name",
      header: t.keys.colName,
      render: (k) => (
        <div className="flex items-center gap-2">
          <span className={`status-dot ${k.enabled ? "is-pulse" : ""}`} />
          <span className="font-medium">{k.name}</span>
        </div>
      ),
    },
    {
      key: "status",
      header: t.keys.colStatus,
      render: (k) => {
        const expired = !!k.expires_at && new Date(k.expires_at) <= new Date();
        if (expired) return <span className="badge badge-error">{t.keys.expired}</span>;
        return k.enabled
          ? <span className="badge badge-success">{t.common.enabled}</span>
          : <span className="badge badge-neutral">{t.common.disabled}</span>;
      },
    },
    {
      key: "key",
      header: t.keys.colKey,
      render: (k) => (
        <div className="flex items-center gap-1.5 font-mono text-xs">
          <span className="truncate" style={{ maxWidth: 150 }}>
            {k.key.slice(0, 7)}{"•".repeat(Math.max(0, Math.min(k.key.length - 10, 12)))}
          </span>
          <button className="icon-button" title={t.keys.copyKey} onClick={() => navigator.clipboard?.writeText(k.key)}>
            <Copy size={13} />
          </button>
        </div>
      ),
    },
    {
      key: "quota",
      header: t.keys.colQuota,
      render: (k) => (
        <span className="font-mono text-xs">
          {k.quota_micros > 0 ? `${usd(k.used_micros)} / ${usd(k.quota_micros)}` : "∞"}
        </span>
      ),
    },
    {
      key: "usage",
      header: t.keys.colUsage,
      render: (k) => {
        const u = usageByKey[k.id];
        if (!u) return <span className="text-[var(--md-on-surface-variant)]">—</span>;
        const rate = u.total_requests > 0 ? (u.successful_requests / u.total_requests) * 100 : 0;
        return (
          <div className="flex items-center gap-3 text-xs">
            <span className="font-mono">{u.total_requests.toLocaleString()}</span>
            <span className="font-mono text-[var(--md-on-surface-variant)]">{rate.toFixed(0)}%</span>
            <span className="font-mono text-[var(--md-on-surface-variant)]">{u.total_tokens.toLocaleString()}</span>
          </div>
        );
      },
    },
    {
      key: "models",
      header: t.keys.colModels,
      render: (k) => (
        <span className="font-mono text-xs text-[var(--md-on-surface-variant)]">
          {(k.allowed_models?.length ?? 0) > 0 ? k.allowed_models!.join(", ") : "—"}
        </span>
      ),
    },
    {
      key: "ip",
      header: t.keys.colIp,
      render: (k) => (
        <span className="font-mono text-xs text-[var(--md-on-surface-variant)]">
          {(k.ip_allowlist?.length ?? 0) > 0 ? k.ip_allowlist!.join(", ") : "—"}
        </span>
      ),
    },
    {
      key: "group",
      header: t.keys.colGroup,
      render: (k) => <span className="badge badge-neutral">{k.group_name || "default"}</span>,
    },
    {
      key: "created",
      header: t.keys.colCreated,
      render: (k) => <span className="font-mono text-xs text-[var(--md-on-surface-variant)]">{new Date(k.created_at).toLocaleDateString()}</span>,
    },
    {
      key: "actions",
      header: "",
      align: "right",
      render: (k) => (
        <div className="flex items-center justify-end gap-1">
          <Tooltip content={t.keys.reveal}>
            <button className="btn-ghost p-1.5" onClick={() => setShowKeyId((v) => (v === k.id ? null : k.id))}>
              <Eye size={13} />
            </button>
          </Tooltip>
          <Menu
            align="end"
            ariaLabel={k.name}
            trigger={({ toggle }) => (
              <button className="btn-ghost p-1.5" onClick={toggle} aria-label="More">
                <MoreVerticalIcon size={13} />
              </button>
            )}
            items={[
              { id: "copy", label: t.keys.copyKey, icon: Copy, onSelect: () => navigator.clipboard?.writeText(k.key) },
              { id: "del", label: t.keys.delete, icon: Trash2, danger: true, onSelect: () => { if (confirm(t.keys.deleteConfirm(k.name))) deleteMutation.mutate(k.id); } },
            ]}
          />
        </div>
      ),
    },
  ];

  return (
    <div className="page-fade-enter">
      <PageHeader
        title={t.keys.title}
        description={t.keys.description}
        action={
          <div className="flex items-center gap-2">
            <SegmentedControl
              size="sm"
              value={viewMode}
              onChange={setViewMode}
              ariaLabel={t.keys.view}
              options={[
                { value: "table", label: "", icon: Table2 },
                { value: "cards", label: "", icon: LayoutGrid },
              ]}
            />
            <button className="btn-filled" onClick={() => setModal(true)}>
              <Plus size={14} /> {t.keys.add}
            </button>
          </div>
        }
      />

      {/* Summary stats */}
      {usage && usage.length > 0 && (
        <div className="info-tile-grid mb-4">
          <div className="info-tile">
            <div className="info-tile-icon"><Copy size={16} /></div>
            <div className="info-tile-label">{t.keys.totalKeys}</div>
            <div className="info-tile-value font-mono">{keys?.length ?? 0}</div>
          </div>
          <div className="info-tile">
            <div className="info-tile-icon"><RefreshCw size={16} /></div>
            <div className="info-tile-label">{t.keys.totalRequests}</div>
            <div className="info-tile-value font-mono">{totalRequests.toLocaleString()}</div>
          </div>
          <div className="info-tile">
            <div className="info-tile-icon"><CheckCircle size={16} /></div>
            <div className="info-tile-label">{t.keys.successRate}</div>
            <div className="info-tile-value font-mono">
              {totalRequests > 0
                ? `${((usage.reduce((s, u) => s + u.successful_requests, 0) / totalRequests) * 100).toFixed(1)}%`
                : "—"}
            </div>
          </div>
          <div className="info-tile">
            <div className="info-tile-icon"><Eye size={16} /></div>
            <div className="info-tile-label">{t.keys.totalTokens}</div>
            <div className="info-tile-value font-mono">{totalTokens.toLocaleString()}</div>
          </div>
        </div>
      )}

      {keys && keys.length > 0 && (
        <div className="log-filters">
          <div className="log-filter">
            <Search size={12} style={{ color: "var(--md-on-surface-variant)" }} />
            <input
              placeholder={t.keys.search}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="w-56"
            />
          </div>
          <span className="text-xs text-[var(--md-on-surface-variant)] ml-auto font-mono">
            {total} / {keys.length}
          </span>
        </div>
      )}

      {isLoading ? (
        <div className="space-y-3">
          {[0, 1, 2].map((i) => (
            <div key={i} className="card p-4">
              <div className="skeleton h-3 w-32" />
              <div className="skeleton h-3 w-48 mt-3" />
            </div>
          ))}
        </div>
      ) : total === 0 ? (
        <div className="empty-state">
          <AlertCircle size={40} />
          <p className="text-sm">{search ? t.keys.noMatch : t.keys.empty}</p>
          <button className="btn-filled mt-4" onClick={() => setModal(true)}>
            <Plus size={14} /> {t.keys.add}
          </button>
        </div>
      ) : viewMode === "table" ? (
        <section className="dashboard-section">
          <DataTable
            columns={columns}
            rows={shown}
            rowKey={(k) => k.id}
            empty={<span>{search ? t.keys.noMatch : t.keys.empty}</span>}
          />
          <Pagination
            page={page}
            pageSize={pageSize}
            total={total}
            onPageChange={setPage}
            onPageSizeChange={setPageSize}
            labels={{ total: t.keys.totalLabel, perPage: t.logs.limit }}
          />
        </section>
      ) : (
        <div className="space-y-3">
          {shown.map((k) => {
            const u = usageByKey[k.id];
            const successRate = u && u.total_requests > 0 ? (u.successful_requests / u.total_requests) * 100 : 0;
            const expired = !!k.expires_at && new Date(k.expires_at) <= new Date();
            const quotaPercent = k.quota_micros > 0 ? Math.min(100, (k.used_micros / k.quota_micros) * 100) : 0;
            return (
              <div className="card p-4 click-ripple" key={k.id}>
                <div className="flex items-center gap-3 flex-wrap">
                  <span className={`status-dot ${k.enabled ? "is-pulse" : ""}`} />
                  <div className="min-w-0 flex-1">
                    <div className="font-medium text-sm flex items-center gap-2">
                      {k.name}
                      {k.enabled ? (
                        <span className="badge badge-success">{t.common.enabled}</span>
                      ) : (
                        <span className="badge badge-neutral">{t.common.disabled}</span>
                       )}
                       {expired && <span className="badge badge-error">Expired</span>}
                       {k.quota_micros > 0 && <span className="badge badge-neutral">Quota {quotaPercent.toFixed(0)}%</span>}
                    </div>
                    <div className="text-xs mt-1 text-[var(--md-on-surface-variant)]">Group: {k.group_name || "default"}{k.quota_micros > 0 ? ` · ${k.used_micros.toLocaleString()} / ${k.quota_micros.toLocaleString()} micros` : " · Unlimited quota"}</div>
                    <div className="text-xs font-mono mt-1 text-[var(--md-on-surface-variant)] flex items-center gap-2 flex-wrap">
                      {showKeyId === k.id ? (
                        <>
                          <span className="truncate">{k.key}</span>
                          <CopyInline value={k.key} label={t.keys.copyKey} />
                        </>
                      ) : (
                        <>
                          <span className="truncate">
                            {k.key.slice(0, 7)}
                            {"•".repeat(Math.max(0, k.key.length - 10))}
                          </span>
                          <button
                            className="icon-button"
                            title={t.keys.reveal}
                            onClick={() => setShowKeyId((v) => (v === k.id ? null : k.id))}
                          >
                            <Eye size={14} />
                          </button>
                          <CopyInline value={k.key} label={t.keys.copyKey} />
                        </>
                      )}
                    </div>
                  </div>

                  {u && (
                    <div className="key-usage-stats">
                      <div className="key-usage-stat">
                        <span className="key-usage-label">{t.keys.requests}</span>
                        <span className="key-usage-value font-mono">{u.total_requests.toLocaleString()}</span>
                      </div>
                      <div className="key-usage-stat">
                        <span className="key-usage-label">{t.keys.successRate}</span>
                        <span className="key-usage-value font-mono">{successRate.toFixed(0)}%</span>
                      </div>
                      <div className="key-usage-stat">
                        <span className="key-usage-label">{t.keys.tokens}</span>
                        <span className="key-usage-value font-mono">{u.total_tokens.toLocaleString()}</span>
                      </div>
                    </div>
                  )}

                  <button
                    className="btn-ghost p-2"
                    style={{ color: "var(--md-error)" }}
                    onClick={() => {
                      if (confirm(t.keys.deleteConfirm(k.name))) deleteMutation.mutate(k.id);
                    }}
                    title={t.keys.delete}
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}
      {modal && <KeyModal onClose={() => setModal(false)} />}
    </div>
  );
}
