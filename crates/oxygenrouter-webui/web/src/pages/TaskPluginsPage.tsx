import React, { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Boxes, Play, Trash2, Upload } from "lucide-react";
import { api, TaskPlugin } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import { useToast } from "../lib/toast";
import Button from "../components/ui/Button";
import Dialog from "../components/ui/Dialog";
import DataTable, { DataTableColumn } from "../components/ui/DataTable";
import Switch from "../components/ui/Switch";

function timestamp(value: string): string {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString();
}

/**
 * Uploading a build.
 *
 * The source is pasted rather than chosen from disk: the file picker would need
 * the browser's filesystem APIs, and what an operator has is usually a build
 * they just produced. The server runs it once before storing it, so a syntax
 * error is refused here rather than surfacing as a routing failure later.
 */
function UploadDialog({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const toast = useToast();
  const qc = useQueryClient();
  const [form, setForm] = useState({ key: "", version: "", source: "" });
  const upload = useMutation({
    mutationFn: () => api.plugins.upload(form),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["taskPlugins"] });
      toast.success(t.plugins.uploaded);
      onClose();
    },
    onError: (error: unknown) => toast.error(t.plugins.uploadFailed, String((error as Error)?.message ?? error)),
  });

  return (
    <Dialog
      open
      onClose={onClose}
      title={t.plugins.upload}
      size="lg"
      footer={
        <>
          <Button variant="outlined" onClick={onClose}>
            {t.common.cancel}
          </Button>
          <Button
            variant="filled"
            loading={upload.isPending}
            disabled={!form.key.trim() || !form.version.trim() || !form.source.trim()}
            onClick={() => upload.mutate()}
          >
            {t.common.create}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="field-label">{t.plugins.key}</label>
            <input
              className="input font-mono text-xs"
              value={form.key}
              onChange={(event) => setForm({ ...form, key: event.target.value })}
              placeholder="my-plugin"
            />
          </div>
          <div>
            <label className="field-label">{t.plugins.version}</label>
            <input
              className="input font-mono text-xs"
              value={form.version}
              onChange={(event) => setForm({ ...form, version: event.target.value })}
              placeholder="1.0.0"
            />
          </div>
        </div>
        <div>
          <label className="field-label">{t.plugins.source}</label>
          <textarea
            className="input font-mono text-xs"
            rows={12}
            value={form.source}
            onChange={(event) => setForm({ ...form, source: event.target.value })}
            placeholder='{"key":"my-plugin","name":"My plugin","protocols":["openai_video"]}'
          />
          <p className="field-hint">{t.plugins.sourceHint}</p>
        </div>
      </div>
    </Dialog>
  );
}

/** Call one hook with a hand-written input, without routing live traffic. */
function DryRunDialog({ plugin, onClose }: { plugin: TaskPlugin; onClose: () => void }) {
  const { t } = useI18n();
  const toast = useToast();
  const [hook, setHook] = useState(plugin.protocols[0] ? "submit" : "submit");
  const [body, setBody] = useState("{}");
  const [result, setResult] = useState<string | null>(null);
  const run = useMutation({
    mutationFn: () => api.plugins.dryRun(plugin.key, hook.trim(), JSON.parse(body || "{}")),
    onSuccess: (value) => setResult(JSON.stringify(value, null, 2)),
    onError: (error: unknown) => toast.error(t.plugins.dryRunFailed, String((error as Error)?.message ?? error)),
  });

  return (
    <Dialog
      open
      onClose={onClose}
      title={`${t.plugins.dryRun}: ${plugin.key}`}
      size="lg"
      footer={
        <>
          <Button variant="outlined" onClick={onClose}>
            {t.common.cancel}
          </Button>
          <Button variant="filled" loading={run.isPending} onClick={() => run.mutate()}>
            <Play size={14} /> {t.plugins.run}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <div>
          <label className="field-label">{t.plugins.hook}</label>
          <input
            className="input font-mono text-xs"
            value={hook}
            onChange={(event) => setHook(event.target.value)}
            placeholder="submit"
          />
        </div>
        <div>
          <label className="field-label">{t.plugins.input}</label>
          <textarea
            className="input font-mono text-xs"
            rows={6}
            value={body}
            onChange={(event) => setBody(event.target.value)}
          />
        </div>
        {result && (
          <div>
            <div className="field-label">{t.plugins.result}</div>
            <pre className="plugin-result">{result}</pre>
          </div>
        )}
      </div>
    </Dialog>
  );
}

export default function TaskPluginsPage() {
  const { t } = useI18n();
  const toast = useToast();
  const qc = useQueryClient();
  const [uploading, setUploading] = useState(false);
  const [dryRunning, setDryRunning] = useState<TaskPlugin | null>(null);

  const { data, isLoading } = useQuery({ queryKey: ["taskPlugins"], queryFn: api.plugins.list });

  const after = (message: string) => {
    qc.invalidateQueries({ queryKey: ["taskPlugins"] });
    toast.success(message);
  };
  const fail = (title: string) => (error: unknown) =>
    toast.error(title, String((error as Error)?.message ?? error));

  const activate = useMutation({
    mutationFn: ({ key, version }: { key: string; version: string }) => api.plugins.activate(key, version),
    onSuccess: () => after(t.plugins.activated),
    onError: fail(t.plugins.actionFailed),
  });
  const toggle = useMutation({
    mutationFn: ({ key, enabled }: { key: string; enabled: boolean }) => api.plugins.setEnabled(key, enabled),
    onSuccess: () => after(t.plugins.updated),
    onError: fail(t.plugins.actionFailed),
  });
  const remove = useMutation({
    mutationFn: ({ key, version }: { key: string; version: string }) => api.plugins.removeVersion(key, version),
    onSuccess: () => after(t.plugins.versionRemoved),
    onError: fail(t.plugins.actionFailed),
  });

  const columns: DataTableColumn<TaskPlugin>[] = [
    {
      key: "plugin",
      header: t.plugins.plugin,
      width: "26%",
      render: (row) => (
        <span className="plugin-cell">
          <span className="font-mono text-xs">{row.key}</span>
          <small>{row.name || row.description || "—"}</small>
        </span>
      ),
    },
    {
      key: "enabled",
      header: t.plugins.enabled,
      width: "12%",
      render: (row) => (
        <Switch
          checked={row.enabled}
          onChange={(next) => toggle.mutate({ key: row.key, enabled: next })}
          label={t.plugins.enabled}
        />
      ),
    },
    {
      key: "active",
      header: t.plugins.active,
      width: "12%",
      render: (row) => <span className="font-mono text-xs">{row.active_version ?? "—"}</span>,
    },
    {
      key: "versions",
      header: t.plugins.versions,
      width: "30%",
      render: (row) => (
        <span className="plugin-versions">
          {row.versions.map((version) => (
            <span className={`plugin-version ${version === row.active_version ? "is-active" : ""}`} key={version}>
              {version !== row.active_version && (
                <button
                  type="button"
                  className="plugin-version-activate"
                  onClick={() => activate.mutate({ key: row.key, version })}
                >
                  {version}
                </button>
              )}
              {version === row.active_version && <b>{version}</b>}
              <button
                type="button"
                className="plugin-version-remove"
                aria-label={`${t.plugins.removeVersion} ${version}`}
                title={t.plugins.removeVersion}
                disabled={version === row.active_version}
                onClick={() => remove.mutate({ key: row.key, version })}
              >
                <Trash2 size={11} />
              </button>
            </span>
          ))}
        </span>
      ),
    },
    {
      key: "protocols",
      header: t.plugins.protocols,
      width: "14%",
      render: (row) => (
        <span className="plugin-protocols">
          {row.protocols.length === 0 ? "—" : row.protocols.map((protocol) => (
            <span className="pricing-badge" key={protocol}>
              {protocol}
            </span>
          ))}
        </span>
      ),
    },
    {
      key: "actions",
      header: "",
      width: "6%",
      align: "right",
      render: (row) => (
        <Button variant="ghost" size="sm" onClick={() => setDryRunning(row)} title={t.plugins.dryRun}>
          <Play size={14} />
        </Button>
      ),
    },
  ];

  return (
    <div className="page-fade-enter space-y-4">
      <div className="pricing-toolbar">
        <PageHeader title={t.plugins.title} description={t.plugins.description} />
        <Button variant="filled" size="sm" onClick={() => setUploading(true)}>
          <Upload size={14} /> {t.plugins.upload}
        </Button>
      </div>

      {isLoading ? (
        <div className="skeleton h-64 w-full" />
      ) : (
        <section className="dashboard-section">
          <div className="section-heading">
            <h2>
              <Boxes size={14} /> {t.plugins.title}
            </h2>
            <span className="section-meta">
              {(data?.length ?? 0)} {t.plugins.installed}
            </span>
          </div>
          <DataTable
            columns={columns}
            rows={data ?? []}
            rowKey={(row) => row.key}
            empty={<span className="pricing-muted">{t.plugins.empty}</span>}
          />
          <div className="security-footer">
            <span className="pricing-muted">{t.plugins.lastUpdated}: {data?.length ? timestamp(data[0].updated_at) : "—"}</span>
          </div>
        </section>
      )}

      {uploading && <UploadDialog onClose={() => setUploading(false)} />}
      {dryRunning && <DryRunDialog plugin={dryRunning} onClose={() => setDryRunning(null)} />}
    </div>
  );
}
