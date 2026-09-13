import React, { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertCircle, Download, Pencil, Plus, RefreshCw, Search, Sparkles, Trash2 } from "lucide-react";
import { api, ModelMetadata } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import { useToast } from "../lib/toast";
import Button from "../components/ui/Button";
import Dialog from "../components/ui/Dialog";
import Select from "../components/ui/Select";
import Switch from "../components/ui/Switch";
import DataTable, { DataTableColumn } from "../components/ui/DataTable";
import Pagination from "../components/ui/Pagination";
import Tooltip from "../components/ui/Tooltip";
import Menu from "../components/ui/Menu";

const NAME_RULES = [
  { value: 0, en: "Exact", zh: "精确匹配" },
  { value: 1, en: "Prefix", zh: "前缀匹配" },
  { value: 2, en: "Contains", zh: "包含匹配" },
  { value: 3, en: "Suffix", zh: "后缀匹配" },
];

function MetadataDialog({ editing, onClose }: { editing: ModelMetadata | null; onClose: () => void }) {
  const { t } = useI18n();
  const toast = useToast();
  const qc = useQueryClient();
  const [form, setForm] = useState<Partial<ModelMetadata>>(
    editing ?? {
      model_name: "",
      description: "",
      icon: "",
      tags: "",
      vendor: "",
      endpoints: [],
      name_rule: 0,
      status: 1,
      sync_official: 1,
    },
  );
  const mut = useMutation({
    mutationFn: (data: Partial<ModelMetadata>) =>
      editing ? api.modelMetadata.update(editing.id, data) : api.modelMetadata.create(data),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["modelMetadata"] });
      toast.success(editing ? t.modelsMeta.updated : t.modelsMeta.created);
      onClose();
    },
    onError: (error: unknown) => toast.error(t.common.notFound, String((error as Error)?.message ?? error)),
  });

  return (
    <Dialog
      open
      onClose={onClose}
      title={editing ? t.modelsMeta.edit : t.modelsMeta.create}
      size="md"
      footer={
        <>
          <Button variant="outlined" onClick={onClose}>{t.common.cancel}</Button>
          <Button
            variant="filled"
            loading={mut.isPending}
            disabled={!form.model_name}
            onClick={() => mut.mutate(form)}
          >
            {editing ? t.common.save : t.common.create}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <div>
          <label className="field-label">{t.modelsMeta.modelName}</label>
          <input
            className="input font-mono text-xs"
            value={form.model_name ?? ""}
            onChange={(e) => setForm({ ...form, model_name: e.target.value })}
            placeholder="gpt-4o"
          />
        </div>
        <div>
          <label className="field-label">{t.modelsMeta.description}</label>
          <input
            className="input text-xs"
            value={form.description ?? ""}
            onChange={(e) => setForm({ ...form, description: e.target.value })}
          />
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="field-label">{t.modelsMeta.vendor}</label>
            <input
              className="input text-xs"
              value={form.vendor ?? ""}
              onChange={(e) => setForm({ ...form, vendor: e.target.value })}
              placeholder="OpenAI"
            />
          </div>
          <div>
            <label className="field-label">{t.modelsMeta.icon}</label>
            <input
              className="input text-xs"
              value={form.icon ?? ""}
              onChange={(e) => setForm({ ...form, icon: e.target.value })}
              placeholder="🤖 / url"
            />
          </div>
        </div>
        <div>
          <label className="field-label">{t.modelsMeta.tags}</label>
          <input
            className="input text-xs"
            value={form.tags ?? ""}
            onChange={(e) => setForm({ ...form, tags: e.target.value })}
            placeholder="chat,vision"
          />
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="field-label">{t.modelsMeta.nameRule}</label>
            <Select
              value={String(form.name_rule ?? 0)}
              onChange={(value) => setForm({ ...form, name_rule: Number(value) })}
              fullWidth
              options={NAME_RULES.map((rule) => ({ value: String(rule.value), label: t.app.brand === "OxygenRouter" && rule.en ? rule.en : rule.en }))}
            />
          </div>
          <div>
            <label className="field-label">{t.modelsMeta.endpoints}</label>
            <input
              className="input font-mono text-xs"
              value={(form.endpoints ?? []).join(", ")}
              onChange={(e) =>
                setForm({ ...form, endpoints: e.target.value.split(",").map((v) => v.trim()).filter(Boolean) })
              }
              placeholder="/v1/chat/completions"
            />
          </div>
        </div>
        <div className="flex items-center gap-4">
          <Switch
            checked={(form.status ?? 1) === 1}
            onChange={(checked) => setForm({ ...form, status: checked ? 1 : 0 })}
            label={t.modelsMeta.statusEnabled}
          />
          <Switch
            checked={(form.sync_official ?? 1) === 1}
            onChange={(checked) => setForm({ ...form, sync_official: checked ? 1 : 0 })}
            label={t.modelsMeta.syncOfficial}
          />
        </div>
      </div>
    </Dialog>
  );
}

export default function ModelMetadataPage() {
  const { t, locale } = useI18n();
  const toast = useToast();
  const qc = useQueryClient();
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [search, setSearch] = useState("");
  const [dialog, setDialog] = useState<{ editing: ModelMetadata | null } | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<ModelMetadata | null>(null);

  const { data, isLoading } = useQuery({
    queryKey: ["modelMetadata", page, pageSize, search],
    queryFn: () => api.modelMetadata.list({ page, pageSize, search: search || undefined }),
  });

  const { data: missing } = useQuery({ queryKey: ["modelMetadataMissing"], queryFn: api.modelMetadata.missing });

  const syncMut = useMutation({
    mutationFn: () => api.modelMetadata.sync(),
    onSuccess: (count) => {
      qc.invalidateQueries({ queryKey: ["modelMetadata"] });
      qc.invalidateQueries({ queryKey: ["modelMetadataMissing"] });
      toast.success(t.modelsMeta.syncDone, `${count} ${t.modelsMeta.syncCreated}`);
    },
    onError: (error: unknown) => toast.error(t.modelsMeta.syncFailed, String((error as Error)?.message ?? error)),
  });

  const deleteMut = useMutation({
    mutationFn: (id: string) => api.modelMetadata.delete(id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["modelMetadata"] });
      toast.success(t.modelsMeta.deleted);
    },
  });

  const ruleLabel = (rule: number) => {
    const found = NAME_RULES.find((entry) => entry.value === rule);
    if (!found) return String(rule);
    return locale === "zh" ? found.zh : found.en;
  };

  const columns: DataTableColumn<ModelMetadata>[] = useMemo(
    () => [
      {
        key: "model_name",
        header: t.modelsMeta.modelName,
        render: (row) => (
          <div className="flex items-center gap-2">
            <span className="text-sm">{row.icon || "🤖"}</span>
            <span className="font-mono text-xs">{row.model_name}</span>
          </div>
        ),
      },
      { key: "vendor", header: t.modelsMeta.vendor, render: (row) => <span className="text-xs">{row.vendor || "—"}</span> },
      { key: "tags", header: t.modelsMeta.tags, render: (row) => <span className="text-xs">{row.tags || "—"}</span> },
      { key: "name_rule", header: t.modelsMeta.nameRule, render: (row) => <span className="badge badge-neutral">{ruleLabel(row.name_rule)}</span> },
      {
        key: "endpoints",
        header: t.modelsMeta.endpoints,
        render: (row) => (
          <span className="font-mono text-[11px] text-[var(--md-on-surface-variant)]">
            {row.endpoints.length > 0 ? row.endpoints.join(", ") : "—"}
          </span>
        ),
      },
      {
        key: "status",
        header: t.logs.status,
        render: (row) =>
          row.status === 1 ? (
            <span className="badge badge-success">{t.common.enabled}</span>
          ) : (
            <span className="badge badge-neutral">{t.common.disabled}</span>
          ),
      },
      {
        key: "actions",
        header: "",
        align: "right",
        render: (row) => (
          <Menu
            align="end"
            ariaLabel={row.model_name}
            trigger={({ toggle }) => (
              <button className="btn-ghost p-1.5" onClick={toggle} aria-label="More">
                <Sparkles size={13} />
              </button>
            )}
            items={[
              { id: "edit", label: t.common.edit, icon: Pencil, onSelect: () => setDialog({ editing: row }) },
              { id: "delete", label: t.common.delete, icon: Trash2, danger: true, onSelect: () => setConfirmDelete(row) },
            ]}
          />
        ),
      },
    ],
    [t, locale, ruleLabel],
  );

  return (
    <div className="page-fade-enter">
      <PageHeader
        title={t.modelsMeta.title}
        description={t.modelsMeta.description}
        action={
          <div className="flex items-center gap-2">
            <Tooltip content={t.modelsMeta.syncHint}>
              <button className="btn-outlined btn-sm" onClick={() => syncMut.mutate()} disabled={syncMut.isPending}>
                <Download size={13} /> {t.modelsMeta.sync}
              </button>
            </Tooltip>
            <button className="btn-filled" onClick={() => setDialog({ editing: null })}>
              <Plus size={14} /> {t.modelsMeta.create}
            </button>
          </div>
        }
      />

      {missing && missing.length > 0 && (
        <div className="error-banner mb-4" style={{ background: "var(--md-warning-container)", color: "var(--md-on-warning-container)" }}>
          <AlertCircle size={16} />
          <span>
            {missing.length} {t.modelsMeta.missingHint}
          </span>
        </div>
      )}

      <div className="log-filters">
        <div className="log-filter">
          <Search size={12} style={{ color: "var(--md-on-surface-variant)" }} />
          <input
            placeholder={t.modelsMeta.search}
            value={search}
            onChange={(e) => {
              setSearch(e.target.value);
              setPage(1);
            }}
            className="w-56"
          />
        </div>
        <span className="count-label">{data?.total ?? 0} {t.modelsMeta.entries}</span>
      </div>

      <section className="dashboard-section">
        <DataTable
          columns={columns}
          rows={data?.items ?? []}
          rowKey={(row) => row.id}
          empty={<span>{isLoading ? t.common.saving : t.modelsMeta.empty}</span>}
        />
        <Pagination
          page={page}
          pageSize={pageSize}
          total={data?.total ?? 0}
          onPageChange={setPage}
          onPageSizeChange={setPageSize}
          labels={{ total: t.keys.totalLabel, perPage: t.logs.limit }}
        />
      </section>

      {dialog && <MetadataDialog editing={dialog.editing} onClose={() => setDialog(null)} />}

      <Dialog
        open={confirmDelete !== null}
        onClose={() => setConfirmDelete(null)}
        title={t.common.delete}
        description={confirmDelete ? `${confirmDelete.model_name}?` : ""}
        size="sm"
        footer={
          <>
            <Button variant="outlined" onClick={() => setConfirmDelete(null)}>{t.common.cancel}</Button>
            <Button
              variant="filled"
              className="danger-button"
              leftIcon={<Trash2 size={13} />}
              onClick={() => {
                if (confirmDelete) deleteMut.mutate(confirmDelete.id);
                setConfirmDelete(null);
              }}
            >
              {t.common.delete}
            </Button>
          </>
        }
      >
        <span />
      </Dialog>
    </div>
  );
}
