import React, { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Activity, ArrowDownRight, ArrowUpRight, Boxes, Crown, Minus, Trophy } from "lucide-react";
import { Area, AreaChart, Bar, BarChart, ResponsiveContainer, Tooltip as ChartTooltip, XAxis, YAxis } from "recharts";
import { api, RankingsResponse } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import DataTable, { DataTableColumn } from "../components/ui/DataTable";
import { RankedModel } from "../lib/api";

type Period = "today" | "week" | "month";

const PERIODS: Period[] = ["today", "week", "month"];
const SERIES_COLORS = ["#60a5fa", "#34d399", "#fbbf24", "#f87171", "#a78bfa", "#22d3ee", "#fb7185", "#a3e635"];

/** Token counts reach millions quickly, so they are rendered compactly. */
function formatTokens(value: number): string {
  if (!Number.isFinite(value)) return "—";
  if (Math.abs(value) >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(2)}B`;
  if (Math.abs(value) >= 1_000_000) return `${(value / 1_000_000).toFixed(2)}M`;
  if (Math.abs(value) >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return String(value);
}

/**
 * The API reports a share as a fraction (`0.75` is three quarters), so every
 * place that renders it multiplies once, here, rather than after each call.
 */
function formatShare(fraction: number): string {
  return `${(fraction * 100).toFixed(1)}%`;
}

function Growth({ value }: { value: number }) {
  const flat = Math.abs(value) < 0.05;
  const up = value > 0;
  const Icon = flat ? Minus : up ? ArrowUpRight : ArrowDownRight;
  return (
    <span className={`ranking-growth ${flat ? "is-flat" : up ? "is-up" : "is-down"}`}>
      <Icon size={12} />
      {flat ? "0.0%" : `${Math.abs(value).toFixed(1)}%`}
    </span>
  );
}

function Metric({ icon: Icon, label, value }: { icon: React.ElementType; label: string; value: string }) {
  return (
    <div className="metric-card analytics-metric">
      <div className="metric-card-top">
        <span className="metric-label">{label}</span>
        <Icon size={16} />
      </div>
      <div className="metric-value">{value}</div>
    </div>
  );
}

export default function RankingsPage() {
  const { t } = useI18n();
  const [period, setPeriod] = useState<Period>("week");
  const { data, isLoading } = useQuery<RankingsResponse>({
    queryKey: ["rankings", period],
    queryFn: () => api.rankings.get(period),
    refetchInterval: 60_000,
  });

  // The API emits one point per (bucket, model); recharts wants one row per
  // bucket with a column per model, so the points are pivoted here.
  const history = useMemo(() => {
    const series = data?.models_history;
    if (!series || series.points.length === 0) return { rows: [] as Array<Record<string, number | string>>, names: [] as string[] };
    const names = series.models.slice(0, 6).map((model) => model.name);
    const byLabel = new Map<string, Record<string, number | string>>();
    for (const point of series.points) {
      if (!names.includes(point.model)) continue;
      const row = byLabel.get(point.label) ?? { label: point.label };
      row[point.model] = Number(row[point.model] ?? 0) + point.tokens;
      byLabel.set(point.label, row);
    }
    return { rows: Array.from(byLabel.values()), names };
  }, [data]);

  const vendorHistory = useMemo(() => {
    const series = data?.vendor_share_history;
    if (!series || series.points.length === 0) return { rows: [] as Array<Record<string, number | string>>, names: [] as string[] };
    const names = series.vendors.map((vendor) => vendor.name);
    const byLabel = new Map<string, Record<string, number | string>>();
    for (const point of series.points) {
      const row = byLabel.get(point.label) ?? { label: point.label };
      row[point.vendor] = Number(row[point.vendor] ?? 0) + point.share * 100;
      byLabel.set(point.label, row);
    }
    return { rows: Array.from(byLabel.values()), names };
  }, [data]);

  const columns: DataTableColumn<RankedModel>[] = [
    {
      key: "rank",
      header: t.rankings.rank,
      width: "10%",
      render: (row) => (
        <span className="ranking-rank">
          <b>{row.rank}</b>
          {row.previous_rank !== undefined && row.previous_rank !== row.rank && (
            <small className={row.previous_rank > row.rank ? "is-up" : "is-down"}>
              {row.previous_rank > row.rank ? "▲" : "▼"}
              {Math.abs(row.previous_rank - row.rank)}
            </small>
          )}
        </span>
      ),
    },
    { key: "model", header: t.rankings.model, width: "34%", render: (row) => <span className="font-mono text-xs">{row.model_name}</span> },
    { key: "vendor", header: t.rankings.vendor, width: "18%", render: (row) => <span className="text-xs">{row.vendor}</span> },
    { key: "tokens", header: t.rankings.tokens, width: "16%", align: "right", render: (row) => <span className="font-mono text-xs">{formatTokens(row.total_tokens)}</span> },
    {
      key: "share",
      header: t.rankings.share,
      width: "14%",
      render: (row) => (
        <span className="ranking-share">
          <span className="ranking-share-track">
            <span style={{ width: `${Math.min(Math.max(row.share * 100, 0), 100)}%` }} />
          </span>
          <b className="font-mono">{formatShare(row.share)}</b>
        </span>
      ),
    },
    { key: "growth", header: t.rankings.growth, width: "10%", align: "right", render: (row) => <Growth value={row.growth_pct} /> },
  ];

  if (isLoading) {
    return (
      <div className="page-fade-enter space-y-4">
        <PageHeader title={t.rankings.title} description={t.rankings.description} />
        <div className="skeleton h-64 w-full" />
      </div>
    );
  }

  const models = data?.models ?? [];
  const vendors = data?.vendors ?? [];
  const totalTokens = models.reduce((sum, model) => sum + model.total_tokens, 0);
  const leader = models[0];

  return (
    <div className="page-fade-enter space-y-5">
      <PageHeader title={t.rankings.title} description={t.rankings.description} />

      <div className="time-range-pill" role="group" aria-label={t.rankings.period}>
        {PERIODS.map((value) => (
          <button key={value} className={period === value ? "is-active" : ""} onClick={() => setPeriod(value)}>
            {value === "today" ? t.rankings.today : value === "week" ? t.rankings.week : t.rankings.month}
          </button>
        ))}
      </div>

      <div className="metric-grid">
        <Metric icon={Activity} label={t.rankings.totalTokens} value={formatTokens(totalTokens)} />
        <Metric icon={Boxes} label={t.rankings.modelsTracked} value={String(models.length)} />
        <Metric icon={Trophy} label={t.rankings.vendorsTracked} value={String(vendors.length)} />
        <Metric icon={Crown} label={t.rankings.leader} value={leader ? leader.model_name : "—"} />
      </div>

      <section className="dashboard-section">
        <div className="section-heading">
          <h2>{t.rankings.leaderboard}</h2>
          <span className="section-meta">{t.rankings.leaderboardMeta}</span>
        </div>
        <DataTable
          columns={columns}
          rows={models}
          rowKey={(row) => row.model_name}
          empty={<span className="pricing-muted">{t.rankings.empty}</span>}
        />
      </section>

      <div className="rankings-split">
        <section className="dashboard-section">
          <div className="section-heading">
            <h2>{t.rankings.vendorShare}</h2>
          </div>
          <div className="rankings-vendors">
            {vendors.length === 0 ? (
              <span className="pricing-muted">{t.rankings.empty}</span>
            ) : (
              vendors.map((vendor, index) => (
                <div key={vendor.vendor} className="rankings-vendor-row">
                  <span className="rankings-vendor-name" title={vendor.vendor}>
                    {vendor.vendor}
                    <small>{vendor.top_model}</small>
                  </span>
                  <span className="ranking-share-track">
                    <span style={{ width: `${Math.min(Math.max(vendor.share * 100, 0), 100)}%`, background: SERIES_COLORS[index % SERIES_COLORS.length] }} />
                  </span>
                  <b className="font-mono">{formatShare(vendor.share)}</b>
                  <span className="font-mono rankings-vendor-tokens">{formatTokens(vendor.total_tokens)}</span>
                </div>
              ))
            )}
          </div>
        </section>

        <section className="dashboard-section">
          <div className="section-heading">
            <h2>{t.rankings.movers}</h2>
          </div>
          <div className="rankings-movers">
            <div>
              <div className="rankings-movers-label is-up">{t.rankings.rising}</div>
              {(data?.top_movers ?? []).map((mover) => (
                <div key={mover.model_name} className="rankings-mover-row">
                  <span className="font-mono">{mover.model_name}</span>
                  <Growth value={mover.growth_pct} />
                </div>
              ))}
            </div>
            <div>
              <div className="rankings-movers-label is-down">{t.rankings.falling}</div>
              {(data?.top_droppers ?? []).map((mover) => (
                <div key={mover.model_name} className="rankings-mover-row">
                  <span className="font-mono">{mover.model_name}</span>
                  <Growth value={mover.growth_pct} />
                </div>
              ))}
            </div>
          </div>
        </section>
      </div>

      <section className="dashboard-section">
        <div className="section-heading">
          <h2>{t.rankings.modelHistory}</h2>
          <span className="section-meta">{t.rankings.modelHistoryMeta}</span>
        </div>
        <div className="rankings-chart">
          {history.rows.length === 0 ? (
            <span className="pricing-muted">{t.rankings.empty}</span>
          ) : (
            <ResponsiveContainer width="100%" height={260}>
              <BarChart data={history.rows} margin={{ top: 8, right: 8, left: -16 }}>
                <XAxis dataKey="label" tick={{ fontSize: 11 }} />
                <YAxis tick={{ fontSize: 11 }} tickFormatter={formatTokens} />
                <ChartTooltip formatter={(value) => formatTokens(Number(value))} />
                {history.names.map((name, index) => (
                  <Bar key={name} dataKey={name} stackId="tokens" fill={SERIES_COLORS[index % SERIES_COLORS.length]} />
                ))}
              </BarChart>
            </ResponsiveContainer>
          )}
        </div>
      </section>

      <section className="dashboard-section">
        <div className="section-heading">
          <h2>{t.rankings.vendorHistory}</h2>
        </div>
        <div className="rankings-chart">
          {vendorHistory.rows.length === 0 ? (
            <span className="pricing-muted">{t.rankings.empty}</span>
          ) : (
            <ResponsiveContainer width="100%" height={240}>
              <AreaChart data={vendorHistory.rows} margin={{ top: 8, right: 8, left: -16 }}>
                <XAxis dataKey="label" tick={{ fontSize: 11 }} />
                <YAxis tick={{ fontSize: 11 }} />
                <ChartTooltip formatter={(value) => `${Number(value).toFixed(1)}%`} />
                {vendorHistory.names.map((name, index) => (
                  <Area
                    key={name}
                    type="monotone"
                    dataKey={name}
                    stackId="share"
                    stroke={SERIES_COLORS[index % SERIES_COLORS.length]}
                    fill={SERIES_COLORS[index % SERIES_COLORS.length]}
                    fillOpacity={0.18}
                  />
                ))}
              </AreaChart>
            </ResponsiveContainer>
          )}
        </div>
      </section>
    </div>
  );
}
