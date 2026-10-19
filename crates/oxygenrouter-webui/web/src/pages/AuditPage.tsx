import React, { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Eraser, RefreshCw, Search, ShieldAlert } from "lucide-react";
import { api, AuditLog } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import { useToast } from "../lib/toast";
import Button from "../components/ui/Button";
import Dialog from "../components/ui/Dialog";
import DataTable, { DataTableColumn } from "../components/ui/DataTable";
import Pagination from "../components/ui/Pagination";

const PAGE_SIZE = 25;

function timestamp(value: string): string {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString();
}

/** The stored body, pretty-printed when it is JSON and truncated when it is long. */
function pretty(detail: string): string {
  try {
    return JSON.stringify(JSON.parse(detail), null, 2);
  } catch {
    return detail;
  }
}

export default function AuditPage() {
  const { t } = useI18n();
  const toast = useToast();
  const qc = useQueryClient();
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);
  const [inspecting, setInspecting] = useState<AuditLog | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);

  const { data, isLoading, isFetching, refetch } = useQuery({
    queryKey: ["auditLogs"],
    queryFn: () => api.audit.list(500),
  });

  const clear = useMutation({
    mutationFn: api.audit.clear,
    onSuccess: (removed) => {
      setConfirmClear(false);
      qc.invalidateQueries({ queryKey: ["auditLogs"] });
      toast.success(t.audit.cleared.replace("{n}", String(removed)));
    },
    onError: (error: unknown) => toast.error(t.audit.clearFailed, String((error as Error)?.message ?? error)),
  });

  const rows = useMemo(() => {
    const entries = data ?? [];
    const needle = search.trim().toLowerCase();
    if (!needle) return entries;
    return entries.filter((entry) =>
      [entry.actor_name, entry.actor_role, entry.method, entry.path, entry.detail ?? ""]
        .join(" ")
        .toLowerCase()
        .includes(needle),
    );
  }, [data, search]);
  const pageCount = Math.max(1, Math.ceil(rows.length / PAGE_SIZE));
  const visible = rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE);

  const columns: DataTableColumn<AuditLog>[] = [
    { key: "when", header: t.audit.when, width: "18%", render: (row) => <span className="font-mono text-xs">{timestamp(row.created_at)}</span> },
    {
      key: "actor",
      header: t.audit.actor,
      width: "20%",
      render: (row) => (
        <span className="audit-actor">
          <span>{row.actor_name || "—"}</span>
          <small>{row.actor_role}</small>
        </span>
      ),
    },
    { key: "method", header: t.audit.method, width: "8%", render: (row) => <span className="pricing-badge">{row.method}</span> },
    { key: "path", header: t.audit.path, width: "34%", render: (row) => <span className="font-mono text-xs">{row.path}</span> },
    {
      key: "status",
      header: t.audit.status,
      width: "10%",
      render: (row) => (
        <span className={`font-mono text-xs ${row.status_code && row.status_code >= 400 ? "audit-status-error" : ""}`}>
          {row.status_code ?? "—"}
        </span>
      ),
    },
    {
      key: "detail",
      header: "",
      width: "10%",
      align: "right",
      render: (row) =>
        row.detail ? (
          <Button variant="ghost" size="sm" onClick={() => setInspecting(row)}>
            {t.audit.detail}
          </Button>
        ) : (
          <span className="pricing-muted">—</span>
        ),
    },
  ];

  return (
    <div className="page-fade-enter space-y-4">
      <PageHeader title={t.audit.title} description={t.audit.description} />

      <div className="pricing-toolbar">
        <div className="pricing-search">
          <Search size={14} />
          <input
            className="input text-xs"
            value={search}
            onChange={(event) => {
              setSearch(event.target.value);
              setPage(1);
            }}
            placeholder={t.audit.searchPlaceholder}
          />
        </div>
        <div className="flex items-center gap-2">
          <Button variant="ghost" size="sm" loading={isFetching} onClick={() => refetch()}>
            <RefreshCw size={14} /> {t.common.refresh}
          </Button>
          <Button variant="outlined" size="sm" onClick={() => setConfirmClear(true)}>
            <Eraser size={14} /> {t.audit.clear}
          </Button>
        </div>
      </div>

      {isLoading ? (
        <div className="skeleton h-64 w-full" />
      ) : (
        <>
          <section className="dashboard-section">
            <div className="section-heading">
              <h2>
                <ShieldAlert size={14} /> {t.audit.trail}
              </h2>
              <span className="section-meta">
                {rows.length} {t.audit.entries}
              </span>
            </div>
            <DataTable
              columns={columns}
              rows={visible}
              rowKey={(row) => row.id}
              empty={<span className="pricing-muted">{t.audit.empty}</span>}
            />
          </section>
          {pageCount > 1 && (
            <Pagination page={page} pageSize={PAGE_SIZE} total={rows.length} onPageChange={setPage} />
          )}
        </>
      )}

      {inspecting && (
        <Dialog
          open
          onClose={() => setInspecting(null)}
          title={`${inspecting.method} ${inspecting.path}`}
          description={`${inspecting.actor_name} · ${timestamp(inspecting.created_at)}${inspecting.client_ip ? ` · ${inspecting.client_ip}` : ""}`}
          size="lg"
          footer={
            <Button variant="outlined" onClick={() => setInspecting(null)}>
              {t.common.cancel}
            </Button>
          }
        >
          <pre className="plugin-result">{pretty(inspecting.detail ?? "")}</pre>
        </Dialog>
      )}

      {confirmClear && (
        <Dialog
          open
          onClose={() => setConfirmClear(false)}
          title={t.audit.clear}
          description={t.audit.clearHint}
          size="sm"
          footer={
            <>
              <Button variant="outlined" onClick={() => setConfirmClear(false)}>
                {t.common.cancel}
              </Button>
              <Button variant="filled" loading={clear.isPending} onClick={() => clear.mutate()}>
                {t.audit.clear}
              </Button>
            </>
          }
        >
          <p className="pricing-muted">{t.audit.clearBody}</p>
        </Dialog>
      )}
    </div>
  );
}
