import React, { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import { ArrowRight, Download, Link2Off, RefreshCw } from "lucide-react";
import { api, RatioSyncReport, SyncableChannel } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import { useToast } from "../lib/toast";
import Button from "../components/ui/Button";
import DataTable, { DataTableColumn } from "../components/ui/DataTable";

type DifferenceRow = {
  upstream: string;
  model_name: string;
  field: string;
  local?: number;
  upstream_value?: number;
};

/**
 * Compare this instance's price table with another instance's.
 *
 * The comparison is a preview: nothing here writes a price. Applying a change is
 * the pricing editor's job, where it is versioned, conflict-checked and audited,
 * so a sync can never silently reprice traffic.
 */
export default function RatioSyncPage() {
  const { t } = useI18n();
  const toast = useToast();
  const [selected, setSelected] = useState<string[]>([]);
  const [manualUrl, setManualUrl] = useState("");
  const [report, setReport] = useState<RatioSyncReport | null>(null);

  const { data: channels, refetch, isFetching } = useQuery<SyncableChannel[]>({
    queryKey: ["ratioSyncChannels"],
    queryFn: api.ratioSync.channels,
  });

  const compare = useMutation({
    mutationFn: () =>
      api.ratioSync.fetch({
        channel_ids: selected,
        upstreams: manualUrl.trim() ? [{ name: manualUrl.trim(), base_url: manualUrl.trim() }] : [],
      }),
    onSuccess: (result) => {
      setReport(result);
      toast.success(t.ratioSync.compared.replace("{n}", String(result.upstreams.length)));
    },
    onError: (error: unknown) => toast.error(t.ratioSync.compareFailed, String((error as Error)?.message ?? error)),
  });

  const rows: DifferenceRow[] = (report?.upstreams ?? []).flatMap((upstream) =>
    upstream.models.flatMap((model) =>
      model.differences.map((difference) => ({
        upstream: upstream.name,
        model_name: model.model_name,
        field: difference.field,
        local: difference.local,
        upstream_value: difference.upstream,
      })),
    ),
  );

  const columns: DataTableColumn<DifferenceRow>[] = [
    { key: "source", header: t.ratioSync.source, width: "20%", render: (row) => <span className="font-mono text-xs">{row.upstream}</span> },
    { key: "model", header: t.ratioSync.model, width: "30%", render: (row) => <span className="font-mono text-xs">{row.model_name}</span> },
    { key: "field", header: t.ratioSync.field, width: "20%", render: (row) => <span className="pricing-badge">{row.field}</span> },
    { key: "local", header: t.ratioSync.local, width: "15%", align: "right", render: (row) => <span className="font-mono text-xs">{row.local ?? "—"}</span> },
    {
      key: "upstream",
      header: t.ratioSync.upstream,
      width: "15%",
      align: "right",
      render: (row) => <span className="font-mono text-xs audit-status-error">{row.upstream_value ?? "—"}</span>,
    },
  ];

  return (
    <div className="page-fade-enter space-y-4">
      <PageHeader
        title={t.ratioSync.title}
        description={t.ratioSync.description}
        action={
          <Link className="btn-outlined btn-sm" to="/ui/model-pricing">
            {t.ratioSync.openEditor} <ArrowRight size={14} />
          </Link>
        }
      />

      <section className="dashboard-section">
        <div className="section-heading">
          <h2>{t.ratioSync.sources}</h2>
          <span className="section-meta">{t.ratioSync.previewNote}</span>
        </div>
        <div className="security-body">
          {(channels ?? []).length === 0 ? (
            <span className="pricing-muted">
              <Link2Off size={12} /> {t.ratioSync.noChannels}
            </span>
          ) : (
            <div className="ratio-sources">
              {channels?.map((channel) => {
                const checked = selected.includes(channel.id);
                return (
                  <label className={`ratio-source ${checked ? "is-selected" : ""}`} key={channel.id}>
                    <input
                      type="checkbox"
                      checked={checked}
                      onChange={(event) =>
                        setSelected((previous) =>
                          event.target.checked
                            ? [...previous, channel.id]
                            : previous.filter((id) => id !== channel.id),
                        )
                      }
                    />
                    <span className="ratio-source-text">
                      <span>{channel.name}</span>
                      <small className="font-mono">
                        {channel.base_url}
                        {channel.endpoint}
                      </small>
                    </span>
                  </label>
                );
              })}
            </div>
          )}

          <div className="security-inline">
            <input
              className="input font-mono text-xs"
              value={manualUrl}
              onChange={(event) => setManualUrl(event.target.value)}
              placeholder={t.ratioSync.manualPlaceholder}
            />
            <Button
              variant="filled"
              size="sm"
              loading={compare.isPending}
              disabled={selected.length === 0 && !manualUrl.trim()}
              onClick={() => compare.mutate()}
            >
              <Download size={14} /> {t.ratioSync.compare}
            </Button>
            <Button variant="ghost" size="sm" loading={isFetching} onClick={() => refetch()}>
              <RefreshCw size={14} /> {t.common.refresh}
            </Button>
          </div>
        </div>
      </section>

      {report && (
        <>
          <section className="dashboard-section">
            <div className="section-heading">
              <h2>{t.ratioSync.results}</h2>
              <span className="section-meta">
                {rows.length} {t.ratioSync.differences}
              </span>
            </div>
            <div className="ratio-report">
              {report.upstreams.map((upstream) => (
                <div className="ratio-report-row" key={`${upstream.name}-${upstream.base_url}`}>
                  <span className="font-mono text-xs">{upstream.name}</span>
                  {upstream.ok ? (
                    <span className="pricing-muted">
                      {upstream.upstream_models} {t.ratioSync.models} · {upstream.models.length} {t.ratioSync.changed}
                    </span>
                  ) : (
                    <span className="audit-status-error text-xs">{upstream.error}</span>
                  )}
                </div>
              ))}
            </div>
            <DataTable
              columns={columns}
              rows={rows}
              rowKey={(row) => `${row.upstream}-${row.model_name}-${row.field}`}
              empty={<span className="pricing-muted">{t.ratioSync.noDifferences}</span>}
            />
          </section>

          {report.new_models.length > 0 && (
            <section className="dashboard-section">
              <div className="section-heading">
                <h2>{t.ratioSync.newModels}</h2>
                <span className="section-meta">{t.ratioSync.newModelsHint}</span>
              </div>
              <div className="ratio-new-models">
                {report.new_models.map((model) => (
                  <span className="pricing-badge font-mono" key={model}>
                    {model}
                  </span>
                ))}
              </div>
            </section>
          )}
        </>
      )}
    </div>
  );
}
