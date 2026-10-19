import React, { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Activity, Clock, Gauge, Search, TrendingUp } from "lucide-react";
import { Area, AreaChart, CartesianGrid, Line, LineChart, ResponsiveContainer, Tooltip as ChartTooltip, XAxis, YAxis } from "recharts";
import { api, PerfModelQuery, PerfModelSummary, PerfSummaryAll } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import DataTable, { DataTableColumn } from "../components/ui/DataTable";

type Range = { id: string; hours: number };
const RANGES: Range[] = [
  { id: "1h", hours: 1 },
  { id: "24h", hours: 24 },
  { id: "7d", hours: 24 * 7 },
  { id: "30d", hours: 24 * 30 },
];

/** Bucket labels are local hours, so a chart reads like the operator's clock. */
function hourLabel(ts: number): string {
  return new Date(ts * 1000).toLocaleString(undefined, { month: "numeric", day: "numeric", hour: "2-digit" });
}

function Metric({ icon: Icon, label, value, detail }: { icon: React.ElementType; label: string; value: string; detail: string }) {
  return (
    <div className="metric-card analytics-metric">
      <div className="metric-card-top">
        <span className="metric-label">{label}</span>
        <Icon size={16} />
      </div>
      <div className="metric-value">{value}</div>
      <div className="metric-detail">{detail}</div>
    </div>
  );
}

export default function PerfMetricsPage() {
  const { t } = useI18n();
  const [hours, setHours] = useState(24);
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<string | null>(null);

  const { data, isLoading } = useQuery<PerfSummaryAll>({
    queryKey: ["perfSummary", hours],
    queryFn: () => api.perfMetrics.summary(hours),
      refetchInterval: 60_000,
  });
  const { data: model } = useQuery<PerfModelQuery>({
    queryKey: ["perfModel", selected, hours],
    queryFn: () => api.perfMetrics.model(selected as string, hours),
    enabled: Boolean(selected),
    refetchInterval: 60_000,
  });

  const models = (data?.models ?? []).filter((entry) =>
    entry.model_name.toLowerCase().includes(search.trim().toLowerCase()),
  );
  const series = (model?.series ?? []).map((point) => ({ ...point, label: hourLabel(point.ts) }));

  const columns: DataTableColumn<PerfModelSummary>[] = [
    { key: "model", header: t.perf.model, width: "34%", render: (row) => <span className="font-mono text-xs">{row.model_name}</span> },
    { key: "requests", header: t.perf.requests, width: "16%", align: "right", render: (row) => <span className="font-mono text-xs">{row.request_count}</span> },
    {
      key: "success",
      header: t.perf.successRate,
      width: "16%",
      align: "right",
      render: (row) => (
        <span className={`font-mono text-xs ${row.success_rate < 99 ? "audit-status-error" : ""}`}>{row.success_rate.toFixed(2)}%</span>
      ),
    },
    { key: "latency", header: t.perf.avgLatency, width: "16%", align: "right", render: (row) => <span className="font-mono text-xs">{row.avg_latency_ms} ms</span> },
    { key: "tps", header: t.perf.avgTps, width: "18%", align: "right", render: (row) => <span className="font-mono text-xs">{row.avg_tps.toFixed(2)}</span> },
  ];

  return (
    <div className="page-fade-enter space-y-4">
      <PageHeader title={t.perf.title} description={t.perf.description} />

      <div className="pricing-toolbar">
        <div className="time-range-pill" role="group" aria-label={t.perf.range}>
          {RANGES.map((range) => (
            <button key={range.id} className={hours === range.hours ? "is-active" : ""} onClick={() => setHours(range.hours)}>
              {range.id}
            </button>
          ))}
        </div>
        <div className="pricing-search">
          <Search size={14} />
          <input
            className="input text-xs"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder={t.perf.searchPlaceholder}
          />
        </div>
      </div>

      <div className="metric-grid">
        <Metric icon={Activity} label={t.perf.requests} value={String(data?.summary.request_count ?? 0)} detail={t.perf.window} />
        <Metric
          icon={TrendingUp}
          label={t.perf.successRate}
          value={`${(data?.summary.success_rate ?? 0).toFixed(2)}%`}
          detail={t.perf.successDetail}
        />
        <Metric icon={Clock} label={t.perf.avgLatency} value={`${data?.summary.avg_latency_ms ?? 0} ms`} detail={t.perf.latencyDetail} />
        <Metric icon={Gauge} label={t.perf.avgTps} value={(data?.summary.avg_tps ?? 0).toFixed(2)} detail={t.perf.tpsDetail} />
      </div>

      {isLoading ? (
        <div className="skeleton h-64 w-full" />
      ) : (
        <section className="dashboard-section">
          <div className="section-heading">
            <h2>{t.perf.byModel}</h2>
            <span className="section-meta">{t.perf.clickHint}</span>
          </div>
          <DataTable
            columns={columns}
            rows={models}
            rowKey={(row) => row.model_name}
            onRowClick={(row) => setSelected(row.model_name)}
            empty={<span className="pricing-muted">{t.perf.empty}</span>}
          />
        </section>
      )}

      {selected && (
        <section className="dashboard-section">
          <div className="section-heading">
            <h2 className="font-mono text-sm">{selected}</h2>
            <span className="section-meta">
              {t.perf.groups}: {(model?.groups ?? []).map((group) => group.group).join(", ") || "—"}
            </span>
          </div>
          <div className="perf-charts">
            <div className="perf-chart">
              <div className="field-label">{t.perf.latencySeries}</div>
              {series.length === 0 ? (
                <span className="pricing-muted">{t.perf.noSeries}</span>
              ) : (
                <ResponsiveContainer width="100%" height={200}>
                  <LineChart data={series} margin={{ top: 8, right: 8, left: -18 }}>
                    <CartesianGrid strokeDasharray="3 3" stroke="var(--hairline)" />
                    <XAxis dataKey="label" tick={{ fontSize: 10 }} />
                    <YAxis tick={{ fontSize: 10 }} />
                    <ChartTooltip />
                    <Line type="monotone" dataKey="avg_latency_ms" name={t.perf.avgLatency} stroke="#60a5fa" strokeWidth={2} dot={false} />
                  </LineChart>
                </ResponsiveContainer>
              )}
            </div>
            <div className="perf-chart">
              <div className="field-label">{t.perf.tpsSeries}</div>
              {series.length === 0 ? (
                <span className="pricing-muted">{t.perf.noSeries}</span>
              ) : (
                <ResponsiveContainer width="100%" height={200}>
                  <AreaChart data={series} margin={{ top: 8, right: 8, left: -18 }}>
                    <CartesianGrid strokeDasharray="3 3" stroke="var(--hairline)" />
                    <XAxis dataKey="label" tick={{ fontSize: 10 }} />
                    <YAxis tick={{ fontSize: 10 }} domain={[0, 100]} />
                    <ChartTooltip />
                    <Area type="monotone" dataKey="success_rate" name={t.perf.successRate} stroke="#34d399" fill="#34d399" fillOpacity={0.18} strokeWidth={2} />
                  </AreaChart>
                </ResponsiveContainer>
              )}
            </div>
          </div>
          {(model?.groups ?? []).length > 0 && (
            <div className="perf-group-list">
              {model?.groups.map((group) => (
                <div className="perf-group-row" key={group.group}>
                  <span className="font-mono text-xs">{group.group}</span>
                  <span className="font-mono text-xs">{group.request_count}</span>
                  <span className="font-mono text-xs">{group.success_rate.toFixed(2)}%</span>
                  <span className="font-mono text-xs">{group.avg_latency_ms} ms</span>
                  <span className="font-mono text-xs">{group.avg_tps.toFixed(2)}</span>
                </div>
              ))}
            </div>
          )}
        </section>
      )}
    </div>
  );
}
