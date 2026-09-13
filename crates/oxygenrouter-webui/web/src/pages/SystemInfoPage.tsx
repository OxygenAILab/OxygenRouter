import React from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Activity,
  AlertTriangle,
  Cpu,
  Database,
  Download,
  HardDrive,
  Key,
  Map as MapIcon,
  RefreshCw,
  Route as RouteIcon,
  Server,
  ServerCrash,
  ShieldCheck,
  Zap,
} from "lucide-react";
import { api, SystemInfo } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

function formatUptime(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  if (seconds < 86400) {
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    return `${h}h ${m}m`;
  }
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  return `${d}d ${h}h`;
}

function InfoTile({ icon: Icon, label, value, sub, tone }: { icon: React.ElementType; label: string; value: string; sub?: string; tone?: "default" | "success" | "warning" | "error" }) {
  return (
    <div className={`info-tile ${tone ? `is-${tone}` : ""}`}>
      <div className="info-tile-icon">
        <Icon size={16} />
      </div>
      <div className="info-tile-label">{label}</div>
      <div className="info-tile-value font-mono">{value}</div>
      {sub && <div className="info-tile-sub">{sub}</div>}
    </div>
  );
}

function SectionCard({ icon: Icon, title, description, children }: { icon: React.ElementType; title: string; description?: string; children: React.ReactNode }) {
  return (
    <section className="dashboard-section">
      <div className="section-heading">
        <div className="flex items-center gap-2">
          <Icon size={14} style={{ color: "var(--md-on-surface-variant)" }} />
          <h2>{title}</h2>
        </div>
        {description && <span className="section-meta">{description}</span>}
      </div>
      <div className="p-4">{children}</div>
    </section>
  );
}

export default function SystemInfoPage() {
  const { t } = useI18n();
  const { data: info, isLoading, isError, refetch, dataUpdatedAt } = useQuery<SystemInfo>({
    queryKey: ["systemInfo"],
    queryFn: api.system.info,
    refetchInterval: 10_000,
  });

  const [backupPending, setBackupPending] = React.useState(false);

  const handleBackup = async () => {
    if (!confirm(t.systemInfo.backupConfirm)) return;
    setBackupPending(true);
    try {
      await api.backup.create();
    } catch (e) {
      alert(`Backup failed: ${e}`);
    } finally {
      setBackupPending(false);
    }
  };

  if (isLoading) {
    return (
      <div className="page-fade-enter">
        <PageHeader title={t.systemInfo.title} description={t.systemInfo.description} />
        <div className="space-y-4">
          <div className="skeleton h-32 w-full" />
          <div className="skeleton h-48 w-full" />
        </div>
      </div>
    );
  }

  if (isError || !info) {
    return (
      <div className="page-fade-enter">
        <PageHeader title={t.systemInfo.title} description={t.systemInfo.description} />
        <div className="error-banner">
          <AlertTriangle size={16} />
          <span>{t.systemInfo.loadError}</span>
        </div>
      </div>
    );
  }

  return (
    <div className="page-fade-enter space-y-6">
      <PageHeader
        title={t.systemInfo.title}
        description={t.systemInfo.description}
        action={
          <div className="flex items-center gap-2">
            <span className="text-xs text-[var(--md-on-surface-variant)] font-mono">
              {dataUpdatedAt ? new Date(dataUpdatedAt).toLocaleTimeString() : "—"}
            </span>
            <button className="btn-outlined btn-sm" onClick={() => refetch()}>
              <RefreshCw size={13} />
            </button>
          </div>
        }
      />

      <div className="info-tile-grid">
        <InfoTile
          icon={Activity}
          label={t.systemInfo.uptime}
          value={formatUptime(info.uptime_seconds)}
          sub={t.systemInfo.sinceStart}
        />
        <InfoTile
          icon={HardDrive}
          label={t.systemInfo.dbSize}
          value={formatBytes(info.db_size_bytes)}
          sub={t.systemInfo.sqlite}
        />
        <InfoTile
          icon={Database}
          label={t.systemInfo.logCount}
          value={info.log_count.toLocaleString()}
          sub={t.systemInfo.stored}
        />
        <InfoTile
          icon={Server}
          label={t.systemInfo.channels}
          value={`${info.enabled_channel_count} / ${info.channel_count}`}
          sub={t.systemInfo.enabledTotal}
        />
        <InfoTile
          icon={Key}
          label={t.systemInfo.apiKeys}
          value={info.key_count.toString()}
          sub={t.systemInfo.configured}
        />
        <InfoTile
          icon={MapIcon}
          label={t.systemInfo.modelMaps}
          value={info.model_map_count.toString()}
          sub={t.systemInfo.active}
        />
      </div>

      <div className="overview-grid">
        <SectionCard icon={ShieldCheck} title={t.systemInfo.server} description={`v${info.version}`}>
          <dl className="info-list">
            <div className="info-list-row">
              <dt>{t.systemInfo.listenAddress}</dt>
              <dd className="font-mono text-xs">{info.listen_host}:{info.listen_port}</dd>
            </div>
            <div className="info-list-row">
              <dt>{t.systemInfo.localToken}</dt>
              <dd className="font-mono text-xs">{info.local_api_token}</dd>
            </div>
            <div className="info-list-row">
              <dt>{t.systemInfo.maxRetries}</dt>
              <dd className="font-mono text-xs">{info.max_retries}</dd>
            </div>
            <div className="info-list-row">
              <dt>{t.systemInfo.upstreamTimeout}</dt>
              <dd className="font-mono text-xs">{info.upstream_timeout_ms.toLocaleString()} ms</dd>
            </div>
            <div className="info-list-row">
              <dt>{t.systemInfo.maxConcurrent}</dt>
              <dd className="font-mono text-xs">{info.max_concurrent_requests}</dd>
            </div>
            <div className="info-list-row">
              <dt>{t.systemInfo.routeRules}</dt>
              <dd className="font-mono text-xs">{info.rule_count}</dd>
            </div>
          </dl>
        </SectionCard>

        <SectionCard icon={Cpu} title={t.systemInfo.runtime} description={info.build_profile}>
          <dl className="info-list">
            <div className="info-list-row">
              <dt>{t.systemInfo.version}</dt>
              <dd className="font-mono text-xs">v{info.version}</dd>
            </div>
            <div className="info-list-row">
              <dt>{t.systemInfo.platform}</dt>
              <dd className="font-mono text-xs">{info.platform}</dd>
            </div>
            <div className="info-list-row">
              <dt>{t.systemInfo.architecture}</dt>
              <dd className="font-mono text-xs">{info.arch}</dd>
            </div>
            <div className="info-list-row">
              <dt>{t.systemInfo.rustc}</dt>
              <dd className="font-mono text-xs">{info.rustc_version}</dd>
            </div>
            <div className="info-list-row">
              <dt>{t.systemInfo.buildProfile}</dt>
              <dd className="font-mono text-xs">{info.build_profile}</dd>
            </div>
            <div className="info-list-row">
              <dt>{t.systemInfo.startedAt}</dt>
              <dd className="font-mono text-xs">{new Date(info.started_at).toLocaleString()}</dd>
            </div>
          </dl>
        </SectionCard>
      </div>

      <SectionCard icon={HardDrive} title={t.systemInfo.backup} description={t.systemInfo.backupDesc}>
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="text-sm font-medium">{t.systemInfo.backupTitle}</div>
            <div className="text-xs text-[var(--md-on-surface-variant)] mt-1">{t.systemInfo.backupSub}</div>
          </div>
          <button className="btn-filled" onClick={handleBackup} disabled={backupPending}>
            {backupPending ? <RefreshCw size={14} className="animate-spin" /> : <Download size={14} />}
            {backupPending ? t.systemInfo.backupPending : t.systemInfo.backupBtn}
          </button>
        </div>
      </SectionCard>

      <div className="watermark-footer">
        <span>GitHub@OxygenAILab | OxygenAILab@StarsailsClover</span>
      </div>
    </div>
  );
}
