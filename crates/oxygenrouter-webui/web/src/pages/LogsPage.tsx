import React, { useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useSearchParams } from "react-router-dom";
import {
  ChevronDown,
  ChevronUp,
  Download,
  FileJson,
  FileSpreadsheet,
  RefreshCw,
  Search,
  Wifi,
  WifiOff,
} from "lucide-react";
import { api, Channel, RequestLog, ApiKey } from "../lib/api";
import { useAuth } from "../lib/auth";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import Select from "../components/ui/Select";
import Menu from "../components/ui/Menu";
import Button from "../components/ui/Button";
import DataTable, { DataTableColumn } from "../components/ui/DataTable";
import Pagination from "../components/ui/Pagination";
import SegmentedControl from "../components/ui/SegmentedControl";

function timeAgo(iso: string) {
  const diff = (Date.now() - new Date(iso).getTime()) / 1000;
  if (diff < 60) return `${Math.floor(diff)}s`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
  return `${Math.floor(diff / 86400)}d`;
}

function StatusBadge({ code }: { code: number | null }) {
  if (!code) return <span className="badge badge-neutral">—</span>;
  if (code >= 200 && code < 300) return <span className="badge badge-success">{code}</span>;
  if (code >= 400 && code < 500) return <span className="badge badge-warning">{code}</span>;
  return <span className="badge badge-error">{code}</span>;
}

function toLocalInput(date: Date) {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export default function LogsPage() {
  const { t } = useI18n();
  const { user } = useAuth();
  // An admin sees the instance's logs; everyone else sees only their own, which
  // the server enforces by role regardless of what is asked for.
  const isAdmin = user?.role === "admin" || user?.role === "root";
  const [searchParams] = useSearchParams();
  const plan = searchParams.get("plan");

  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [search, setSearch] = useState(() => searchParams.get("q") ?? "");
  const [status, setStatus] = useState<"all" | "2xx" | "4xx" | "5xx" | "error">("all");
  const [model, setModel] = useState("all");
  const [channelId, setChannelId] = useState("all");
  const [apiKeyId, setApiKeyId] = useState("all");
  const [range, setRange] = useState<"1h" | "24h" | "7d" | "30d" | "custom">("24h");
  const [start, setStart] = useState(() => toLocalInput(new Date(Date.now() - 24 * 3600 * 1000)));
  const [end, setEnd] = useState(() => toLocalInput(new Date()));
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [live, setLive] = useState(false);
  const [liveStatus, setLiveStatus] = useState<"connecting" | "connected" | "disconnected">("disconnected");
  const liveRowsRef = useRef<RequestLog[]>([]);
  const [liveTick, setLiveTick] = useState(0);

  const rangeBounds = useMemo(() => {
    if (range === "custom") {
      return {
        start: start ? new Date(start).toISOString() : undefined,
        end: end ? new Date(end).toISOString() : undefined,
      };
    }
    const hours = range === "1h" ? 1 : range === "24h" ? 24 : range === "7d" ? 24 * 7 : 24 * 30;
    return {
      start: new Date(Date.now() - hours * 3600 * 1000).toISOString(),
      end: new Date().toISOString(),
    };
  }, [range, start, end]);

  const { data, isLoading, refetch } = useQuery({
    queryKey: ["logs", isAdmin, page, pageSize, search, status, model, channelId, apiKeyId, rangeBounds.start, rangeBounds.end],
    queryFn: () =>
      isAdmin
        ? api.logs.query({
            page,
            pageSize,
            search: search || undefined,
            status,
            model,
            channelId,
            apiKeyId,
            start: rangeBounds.start,
            end: rangeBounds.end,
          })
        : // Non-admins see only their own rows; the instance-wide route is
          // admin-gated on the server, so asking for it would just 403.
          api.logs.mine({
            page,
            pageSize,
            search: search || undefined,
            status,
            model,
          }),
    refetchInterval: live ? false : 10_000,
  });

  const { data: stats } = useQuery({ queryKey: ["logStats", range], queryFn: () => api.logs.stats(range === "custom" ? "24h" : range), refetchInterval: 30_000 });
  const { data: channels } = useQuery<Channel[]>({ queryKey: ["channels"], queryFn: api.channels.list });
  const { data: keys } = useQuery<ApiKey[]>({ queryKey: ["keys"], queryFn: api.keys.list });

  // Live stream overlay
  useEffect(() => {
    // The stream carries the instance-wide firehose, which is admin-only. Rather
    // than let a non-admin toggle it and watch an EventSource fail silently, the
    // control is disabled for them (see the toggle below).
    if (!live || !isAdmin) {
      setLiveStatus("disconnected");
      return;
    }
    setLiveStatus("connecting");
    const es = new EventSource("/api/logs/stream");
    es.onopen = () => setLiveStatus("connected");
    es.onerror = () => setLiveStatus("disconnected");
    es.onmessage = (event) => {
      try {
        const row = JSON.parse(event.data) as RequestLog;
        liveRowsRef.current = [row, ...liveRowsRef.current].slice(0, 200);
        setLiveTick((v) => v + 1);
      } catch {
        /* ignore */
      }
    };
    return () => {
      es.close();
      setLiveStatus("disconnected");
    };
  }, [live]);

  useEffect(() => { setPage(1); }, [search, status, model, channelId, apiKeyId, rangeBounds.start, rangeBounds.end, plan]);

  const exportMutation = useMutation({ mutationFn: (format: "csv" | "json") => api.logs.export(format) });

  const rows = useMemo(() => {
    const base = data?.items ?? [];
    if (!live || liveRowsRef.current.length === 0) return base;
    const seen = new Set(base.map((row) => row.id));
    return [...liveRowsRef.current.filter((row) => !seen.has(row.id)), ...base];
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data, live, liveTick]);

  const modelOptions = useMemo(() => {
    const names = new Set<string>();
    for (const row of rows) if (row.model) names.add(row.model);
    return [{ value: "all", label: t.logs.allModels }, ...[...names].sort().map((m) => ({ value: m, label: m }))];
  }, [rows, t]);

  const channelOptions = useMemo(
    () => [{ value: "all", label: t.logs.allChannels }, ...(channels ?? []).map((c) => ({ value: c.id, label: c.name }))],
    [channels, t],
  );
  const keyOptions = useMemo(
    () => [{ value: "all", label: t.logs.allKeys }, ...(keys ?? []).map((k) => ({ value: k.id, label: k.name }))],
    [keys, t],
  );

  const columns: DataTableColumn<RequestLog>[] = [
    {
      key: "time",
      header: t.logs.time,
      render: (row) => (
        <div className="flex flex-col">
          <span className="font-mono text-xs">{new Date(row.created_at).toLocaleString()}</span>
          <span className="text-[10px] text-[var(--md-on-surface-variant)]">{timeAgo(row.created_at)} {t.logs.ago}</span>
        </div>
      ),
    },
    { key: "method", header: t.logs.method, render: (row) => <span className="font-mono text-xs">{row.method}</span> },
    { key: "path", header: t.logs.path, render: (row) => <span className="font-mono text-xs">{row.path}</span> },
    { key: "model", header: t.logs.model, render: (row) => <span className="font-mono text-xs">{row.model ?? "—"}</span> },
    {
      key: "channel",
      header: t.logs.channel,
      render: (row) => {
        const name = channels?.find((c) => c.id === row.channel_id)?.name;
        return <span className="text-xs">{name ?? (row.channel_id ? row.channel_id.slice(0, 8) : "—")}</span>;
      },
    },
    {
      key: "key",
      header: t.keys.colName,
      render: (row) => <span className="text-xs">{keys?.find((k) => k.id === row.api_key_id)?.name ?? "—"}</span>,
    },
    { key: "duration", header: t.logs.duration, render: (row) => <span className="font-mono text-xs">{row.duration_ms}ms</span> },
    { key: "tokens", header: t.keys.tokens, render: (row) => <span className="font-mono text-xs">{row.tokens_used ?? "—"}</span> },
    { key: "status", header: t.logs.status, render: (row) => <StatusBadge code={row.status_code} /> },
    {
      key: "clientIp",
      header: t.logs.clientIp,
      // Blank until the instance is configured to store addresses; "—" keeps the
      // column readable rather than shifting the row.
      render: (row) => <span className="font-mono text-xs">{row.client_ip ?? "—"}</span>,
    },
    {
      key: "detail",
      header: "",
      align: "right",
      render: (row) => {
        const open = expanded.has(row.id);
        return (
          <button
            className="btn-ghost p-1.5"
            aria-label="Detail"
            onClick={() =>
              setExpanded((prev) => {
                const next = new Set(prev);
                if (next.has(row.id)) next.delete(row.id);
                else next.add(row.id);
                return next;
              })
            }
          >
            {open ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
          </button>
        );
      },
    },
  ];

  return (
    <div className="page-fade-enter">
      <PageHeader
        title={t.logs.title}
        description={t.logs.description}
        action={
          <div className="flex items-center gap-2 flex-wrap">
            <button
              className={`btn-outlined btn-sm ${live ? "is-active" : ""}`}
              onClick={() => setLive((v) => !v)}
              // The stream is the instance-wide firehose, so it is admin-only on
              // the server; disabling it here keeps the control honest instead of
              // letting it silently fail to connect.
              disabled={!isAdmin}
              title={isAdmin ? undefined : t.logs.allChannels}
            >
              <span className={`live-indicator ${liveStatus}`} />
              {live ? <Wifi size={13} /> : <WifiOff size={13} />}
              {t.logs.live}
            </button>
            <Menu
              align="end"
              ariaLabel={t.logs.export}
              trigger={({ toggle }) => (
                <button className="btn-outlined btn-sm" onClick={toggle}>
                  <Download size={13} /> {t.logs.export}
                </button>
              )}
              items={[
                { id: "csv", label: "CSV", icon: FileSpreadsheet, onSelect: () => exportMutation.mutate("csv") },
                { id: "json", label: "JSON", icon: FileJson, onSelect: () => exportMutation.mutate("json") },
              ]}
            />
            <button className="btn-ghost btn-sm" onClick={() => refetch()} aria-label={t.common.refresh}>
              <RefreshCw size={13} />
            </button>
          </div>
        }
      />

      <div className="info-tile-grid mb-4">
        <div className="info-tile">
          <div className="info-tile-label">{t.logs.statTotal}</div>
          <div className="info-tile-value font-mono">{(stats?.total ?? 0).toLocaleString()}</div>
        </div>
        <div className="info-tile">
          <div className="info-tile-label">{t.keys.successRate}</div>
          <div className="info-tile-value font-mono">{(stats?.success_rate ?? 0).toFixed(1)}%</div>
        </div>
        <div className="info-tile">
          <div className="info-tile-label">{t.dashboard.avgLatency}</div>
          <div className="info-tile-value font-mono">{Math.round(stats?.avg_latency_ms ?? 0)}ms</div>
        </div>
        <div className="info-tile">
          <div className="info-tile-label">{t.keys.totalTokens}</div>
          <div className="info-tile-value font-mono">{(stats?.total_tokens ?? 0).toLocaleString()}</div>
        </div>
      </div>

      <section className="dashboard-section">
        <div className="log-filters">
          <div className="log-filter">
            <Search size={12} style={{ color: "var(--md-on-surface-variant)" }} />
            <input
              placeholder={t.logs.searchPlaceholder}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="w-56"
            />
          </div>
          <div className="log-filter">
            <SegmentedControl
              size="sm"
              value={range}
              onChange={setRange}
              ariaLabel={t.analytics.range}
              options={[
                { value: "1h", label: "1h" },
                { value: "24h", label: "24h" },
                { value: "7d", label: "7d" },
                { value: "30d", label: "30d" },
                { value: "custom", label: t.logs.customRange },
              ]}
            />
          </div>
        </div>

        {range === "custom" && (
          <div className="log-filters">
            <div className="log-filter">
              <span className="text-xs text-[var(--md-on-surface-variant)]">{t.logs.startTime}</span>
              <input type="text" className="input" style={{ width: 168 }} value={start} onChange={(e) => setStart(e.target.value)} placeholder="YYYY-MM-DDTHH:mm" />
            </div>
            <div className="log-filter">
              <span className="text-xs text-[var(--md-on-surface-variant)]">{t.logs.endTime}</span>
              <input type="text" className="input" style={{ width: 168 }} value={end} onChange={(e) => setEnd(e.target.value)} placeholder="YYYY-MM-DDTHH:mm" />
            </div>
          </div>
        )}

        <div className="log-filters">
          <div className="log-filter">
            <Select
              size="sm"
              value={status}
              onChange={(v) => setStatus(v as typeof status)}
              ariaLabel={t.logs.status}
              options={[
                { value: "all", label: t.logs.filterAll },
                { value: "2xx", label: t.logs.filter2xx },
                { value: "4xx", label: t.logs.filter4xx },
                { value: "5xx", label: t.logs.filter5xx },
                { value: "error", label: t.logs.filterError },
              ]}
            />
          </div>
          <div className="log-filter">
            <Select size="sm" value={model} onChange={setModel} ariaLabel={t.logs.model} options={modelOptions} />
          </div>
          <div className="log-filter">
            <Select size="sm" value={channelId} onChange={setChannelId} ariaLabel={t.logs.channel} options={channelOptions} />
          </div>
          <div className="log-filter">
            <Select size="sm" value={apiKeyId} onChange={setApiKeyId} ariaLabel={t.keys.colName} options={keyOptions} />
          </div>
          <span className="count-label">{rows.length} / {data?.total ?? 0}</span>
        </div>

        <DataTable
          columns={columns}
          rows={rows}
          rowKey={(row) => row.id}
          empty={<span>{t.logs.empty}</span>}
        />
        {expanded.size > 0 && (
          <div className="logs-row-detail" style={{ borderTop: "1px solid var(--md-outline-variant)" }}>
            {rows
              .filter((row) => expanded.has(row.id))
              .map((row) => (
                <div key={row.id} className="mb-2">
                  <div className="font-mono text-[11px] text-[var(--md-on-surface-variant)]">{row.id}</div>
                  {row.error ? (
                    <div className="font-mono text-xs text-[var(--md-error)]">{row.error}</div>
                  ) : (
                    <div className="text-xs">{t.logs.noError}</div>
                  )}
                </div>
              ))}
          </div>
        )}
        <Pagination
          page={page}
          pageSize={pageSize}
          total={data?.total ?? 0}
          onPageChange={setPage}
          onPageSizeChange={setPageSize}
          labels={{ total: t.keys.totalLabel, perPage: t.logs.limit }}
        />
      </section>
    </div>
  );
}
