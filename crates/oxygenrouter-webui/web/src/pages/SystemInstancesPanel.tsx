import React from "react";
import { useQuery } from "@tanstack/react-query";
import { AlertTriangle, Loader2, RefreshCw, ServerCog, Trash2 } from "lucide-react";
import { api, SystemInstance } from "../lib/api";
import { useI18n } from "../lib/i18nContext";

function formatBytes(bytes?: number): string {
  if (typeof bytes !== "number" || !Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** index).toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

function formatPercent(value?: number): string {
  if (typeof value !== "number" || !Number.isFinite(value)) return "—";
  return `${value.toFixed(1)}%`;
}

function formatTimestamp(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString();
}

function instanceName(instance: SystemInstance): string {
  return instance.info?.node?.name || instance.node_name;
}

function roleLabel(instance: SystemInstance, t: ReturnType<typeof useI18n>["t"]): string {
  return instance.info?.role?.is_master === false ? t.systemInfo.worker : t.systemInfo.master;
}

export default function SystemInstancesPanel() {
  const { t } = useI18n();
  const [busy, setBusy] = React.useState<string | null>(null);
  const query = useQuery({
    queryKey: ["system-instances"],
    queryFn: api.system.instances,
    refetchInterval: 30_000,
    staleTime: 15_000,
  });
  const instances = query.data ?? [];
  const stale = instances.filter((instance) => instance.status === "stale");
  const refreshing = query.isFetching && !query.isLoading;

  const run = async (key: string, action: () => Promise<unknown>) => {
    setBusy(key);
    try {
      await action();
      await query.refetch();
    } catch (error) {
      alert(`${t.systemInfo.deleteFailed}: ${String((error as Error)?.message ?? error)}`);
    } finally {
      setBusy(null);
    }
  };

  const deleteAll = () => {
    if (!stale.length) return;
    if (!confirm(t.systemInfo.deleteAllConfirm.replace("{n}", String(stale.length)))) return;
    void run("all", api.system.deleteStaleInstances);
  };

  const deleteOne = (instance: SystemInstance) => {
    const name = instanceName(instance);
    if (!confirm(t.systemInfo.deleteOneConfirm.replace("{name}", name))) return;
    void run(instance.node_name, () => api.system.deleteStaleInstance(instance.node_name));
  };

  return (
    <section className="dashboard-section">
      <div className="section-heading">
        <div className="flex items-center gap-2">
          <ServerCog size={14} style={{ color: "var(--md-on-surface-variant)" }} />
          <h2>{t.systemInfo.instances}</h2>
        </div>
        <span className="section-meta">{t.systemInfo.instancesDesc}</span>
      </div>

      <div className="instance-panel-toolbar">
        <span className="pricing-muted">{t.systemInfo.autoRefresh}</span>
        <div className="flex items-center gap-2">
          {stale.length > 0 && (
            <button
              className="btn-outlined btn-sm"
              type="button"
              onClick={deleteAll}
              disabled={busy !== null}
            >
              {busy === "all" ? <Loader2 size={13} className="animate-spin" /> : <Trash2 size={13} />}
              {t.systemInfo.deleteAllStale}
            </button>
          )}
          <button
            className="btn-outlined btn-sm"
            type="button"
            onClick={() => void query.refetch()}
            disabled={query.isFetching}
          >
            <RefreshCw size={13} className={refreshing ? "animate-spin" : ""} />
            {t.common.refresh}
          </button>
        </div>
      </div>

      {query.isLoading ? (
        <div className="instance-panel-state">{t.systemInfo.loading}</div>
      ) : query.isError ? (
        <div className="error-banner">
          <AlertTriangle size={16} />
          <span>{t.systemInfo.loadError}</span>
        </div>
      ) : instances.length === 0 ? (
        <div className="instance-panel-state">{t.systemInfo.noInstances}</div>
      ) : (
        <div className="ui-datatable-wrap">
          <table className="ui-datatable instance-table">
            <thead>
              <tr>
                <th>{t.systemInfo.instance}</th>
                <th>{t.systemInfo.status}</th>
                <th>{t.systemInfo.role}</th>
                <th>{t.systemInfo.cpu}</th>
                <th>{t.systemInfo.memory}</th>
                <th>{t.systemInfo.storage}</th>
                <th>{t.systemInfo.version}</th>
                <th>{t.systemInfo.runtime}</th>
                <th>{t.systemInfo.started}</th>
                <th>{t.systemInfo.lastSeen}</th>
                <th>{t.systemInfo.actions}</th>
              </tr>
            </thead>
            <tbody>
              {instances.map((instance) => {
                const resources = instance.info?.resources;
                const storage = resources?.storage;
                return (
                  <tr key={instance.node_name}>
                    <td>
                      <div className="instance-name">
                        <span
                          className={`status-dot ${instance.status === "stale" ? "is-error" : ""}`}
                          aria-hidden="true"
                        />
                        <div>
                          <div className="instance-node">
                            {instanceName(instance)}
                            {instance.info?.node?.should_configure_manually && (
                              <span title={t.systemInfo.nodeAutoWarning}>
                                <AlertTriangle size={12} />
                              </span>
                            )}
                          </div>
                          <div className="instance-host">{instance.info?.host?.hostname || "—"}</div>
                        </div>
                      </div>
                    </td>
                    <td>
                      <span className={`badge ${instance.status === "stale" ? "badge-warning" : "badge-success"}`}>
                        {instance.status === "stale" ? t.systemInfo.stale : t.systemInfo.online}
                      </span>
                    </td>
                    <td>{roleLabel(instance, t)}</td>
                    <td className="font-mono">{formatPercent(resources?.cpu?.usage_percent)}</td>
                    <td className="font-mono">{formatPercent(resources?.memory?.usage_percent)}</td>
                    <td className="font-mono" title={`${formatBytes(storage?.used_bytes)} / ${formatBytes(storage?.total_bytes)}`}>
                      {formatPercent(storage?.used_percent)}
                    </td>
                    <td className="font-mono">{instance.info?.runtime?.version || "—"}</td>
                    <td className="font-mono">
                      {[instance.info?.runtime?.goos, instance.info?.runtime?.goarch]
                        .filter(Boolean)
                        .join("/") || "—"}
                    </td>
                    <td>{formatTimestamp(instance.started_at)}</td>
                    <td>{formatTimestamp(instance.last_seen_at)}</td>
                    <td>
                      {instance.status === "stale" ? (
                        <button
                          className="icon-button"
                          type="button"
                          title={t.systemInfo.deleteStale}
                          aria-label={t.systemInfo.deleteStale}
                          disabled={busy !== null}
                          onClick={() => deleteOne(instance)}
                        >
                          {busy === instance.node_name ? (
                            <Loader2 size={14} className="animate-spin" />
                          ) : (
                            <Trash2 size={14} />
                          )}
                        </button>
                      ) : (
                        <span className="pricing-muted">—</span>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
