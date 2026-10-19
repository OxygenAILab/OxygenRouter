import React, { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Eraser, Play, RefreshCw, Trash2 } from "lucide-react";
import { api, SystemTask } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import { useToast } from "../lib/toast";
import Button from "../components/ui/Button";
import DataTable, { DataTableColumn } from "../components/ui/DataTable";

const TASK_TYPE = "log_cleanup";

function timestamp(value: string): string {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString();
}

/** The stored JSON, rendered compactly for a table cell. */
function jsonField(raw: string): Record<string, unknown> {
  if (!raw) return {};
  try {
    const parsed = JSON.parse(raw);
    return typeof parsed === "object" && parsed !== null ? parsed : {};
  } catch {
    return {};
  }
}

function statusClass(status: string): string {
  if (status === "succeeded") return "is-done";
  if (status === "failed") return "is-failed";
  if (status === "running") return "is-running";
  return "";
}

export default function SystemTasksPage() {
  const { t } = useI18n();
  const toast = useToast();
  const qc = useQueryClient();
  const [retainDays, setRetainDays] = useState("30");

  const { data: current } = useQuery({
    queryKey: ["systemTaskCurrent"],
    queryFn: () => api.systemTasks.current(TASK_TYPE),
    // A running task advances on its own, so the card follows it until it stops.
    refetchInterval: (query) => (query.state.data ? 1000 : 5000),
  });
  const { data: history, isLoading, refetch, isFetching } = useQuery({
    queryKey: ["systemTaskHistory"],
    queryFn: () => api.systemTasks.list(TASK_TYPE, 50),
    refetchInterval: 10_000,
  });

  const start = useMutation({
    mutationFn: () => {
      const days = Math.max(1, Number(retainDays) || 30);
      const cutoff = Math.floor(Date.now() / 1000) - days * 86_400;
      return api.systemTasks.startLogCleanup(cutoff);
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["systemTaskCurrent"] });
      qc.invalidateQueries({ queryKey: ["systemTaskHistory"] });
      toast.success(t.systemTasks.started);
    },
    onError: (error: unknown) => toast.error(t.systemTasks.startFailed, String((error as Error)?.message ?? error)),
  });
  const clearHistory = useMutation({
    mutationFn: () => api.systemTasks.clearHistory(TASK_TYPE),
    onSuccess: (removed) => {
      qc.invalidateQueries({ queryKey: ["systemTaskHistory"] });
      toast.success(t.systemTasks.cleared.replace("{n}", String(removed)));
    },
    onError: (error: unknown) => toast.error(t.systemTasks.clearFailed, String((error as Error)?.message ?? error)),
  });

  const progress = jsonField(current?.state ?? "");
  const percent = typeof progress.progress === "number" ? progress.progress : 0;

  const columns: DataTableColumn<SystemTask>[] = [
    { key: "when", header: t.systemTasks.when, width: "22%", render: (row) => <span className="font-mono text-xs">{timestamp(row.created_at)}</span> },
    { key: "type", header: t.systemTasks.type, width: "16%", render: (row) => <span className="pricing-badge">{row.type}</span> },
    {
      key: "status",
      header: t.systemTasks.status,
      width: "14%",
      render: (row) => <span className={`task-status ${statusClass(row.status)}`}>{t.systemTasks[row.status as "pending"] ?? row.status}</span>,
    },
    {
      key: "result",
      header: t.systemTasks.result,
      width: "26%",
      render: (row) => {
        const result = jsonField(row.result);
        if (row.status === "failed") return <span className="audit-status-error text-xs">{row.error || t.systemTasks.failed}</span>;
        if (typeof result.deleted_count === "number") {
          return <span className="font-mono text-xs">{t.systemTasks.deleted.replace("{n}", String(result.deleted_count))}</span>;
        }
        return <span className="pricing-muted">—</span>;
      },
    },
    { key: "updated", header: t.systemTasks.updated, width: "22%", render: (row) => <span className="font-mono text-xs">{timestamp(row.updated_at)}</span> },
  ];

  return (
    <div className="page-fade-enter space-y-4">
      <PageHeader title={t.systemTasks.title} description={t.systemTasks.description} />

      <section className="dashboard-section">
        <div className="section-heading">
          <h2>{t.systemTasks.logCleanup}</h2>
          <span className="section-meta">{t.systemTasks.logCleanupHint}</span>
        </div>
        <div className="security-body">
          <div className="security-inline">
            <label className="field-label" htmlFor="retain-days">
              {t.systemTasks.retain}
            </label>
            <input
              id="retain-days"
              className="input font-mono text-xs task-days"
              type="number"
              min={1}
              value={retainDays}
              onChange={(event) => setRetainDays(event.target.value)}
            />
            <Button variant="filled" size="sm" loading={start.isPending} disabled={Boolean(current)} onClick={() => start.mutate()}>
              <Play size={14} /> {t.systemTasks.run}
            </Button>
          </div>

          {current ? (
            <div className="task-progress" role="status">
              <div className="task-progress-head">
                <span className={`task-status ${statusClass(current.status)}`}>
                  {t.systemTasks[current.status as "pending"] ?? current.status}
                </span>
                <span className="font-mono text-xs">
                  {typeof progress.processed === "number" ? progress.processed : 0}
                  {" / "}
                  {typeof progress.total === "number" ? progress.total : 0}
                </span>
              </div>
              <div className="task-progress-track">
                <span style={{ width: `${Math.min(Math.max(percent, 0), 100)}%` }} />
              </div>
              <span className="pricing-muted font-mono">{current.task_id}</span>
            </div>
          ) : (
            <span className="pricing-muted">{t.systemTasks.idle}</span>
          )}
        </div>
      </section>

      <section className="dashboard-section">
        <div className="section-heading">
          <h2>{t.systemTasks.history}</h2>
          <div className="flex items-center gap-2">
            <Button variant="ghost" size="sm" loading={isFetching} onClick={() => refetch()}>
              <RefreshCw size={14} /> {t.common.refresh}
            </Button>
            <Button variant="outlined" size="sm" loading={clearHistory.isPending} onClick={() => clearHistory.mutate()}>
              <Eraser size={14} /> {t.systemTasks.clearHistory}
            </Button>
          </div>
        </div>
        {isLoading ? (
          <div className="skeleton h-40 w-full" />
        ) : (
          <DataTable
            columns={columns}
            rows={history?.tasks ?? []}
            rowKey={(row) => row.task_id}
            empty={
              <span className="pricing-muted">
                <Trash2 size={12} /> {t.systemTasks.noHistory}
              </span>
            }
          />
        )}
      </section>
    </div>
  );
}
