import React from "react";
import { Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import {
  Activity,
  ArrowRight,
  CheckCircle2,
  ChevronRight,
  Copy,
  Clock,
  Gauge,
  Key,
  KeyRound,
  Plus,
  Server,
  Settings,
  ShieldCheck,
  Sparkles,
  TrendingUp,
  Wallet,
  Zap,
} from "lucide-react";
import { api, SystemStatus, DashboardSnapshot } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";

function StatusDot({ status }: { status: "ok" | "pulse" | "warn" | "off" }) {
  return <span className={`status-dot ${status === "warn" ? "is-error" : ""} ${status === "pulse" ? "is-pulse" : ""}`} />;
}

function CopyInline({ value, label }: { value: string; label: string }) {
  const [copied, setCopied] = React.useState(false);
  return (
    <button
      className="icon-button"
      title={label}
      onClick={() => {
        navigator.clipboard?.writeText(value);
        setCopied(true);
        setTimeout(() => setCopied(false), 1200);
      }}
    >
      {copied ? <CheckCircle2 size={14} /> : <Copy size={14} />}
    </button>
  );
}

function QuickAction({ icon: Icon, label, description, href }: { icon: React.ElementType; label: string; description: string; href: string }) {
  return (
    <Link to={href} className="quick-action-card click-ripple">
      <div className="quick-action-icon">
        <Icon size={18} />
      </div>
      <div className="quick-action-text">
        <div className="quick-action-label">{label}</div>
        <div className="quick-action-desc">{description}</div>
      </div>
      <ArrowRight size={14} className="quick-action-arrow" />
    </Link>
  );
}

function StatCard({ icon: Icon, label, value, sub, color }: { icon: React.ElementType; label: string; value: string; sub?: string; color?: string }) {
  return (
    <div className="stat-card">
      <div className="stat-card-icon" style={color ? { color } : undefined}>
        <Icon size={18} />
      </div>
      <div className="stat-card-label">{label}</div>
      <div className="stat-card-value font-mono">{value}</div>
      {sub && <div className="stat-card-sub">{sub}</div>}
    </div>
  );
}

export default function OverviewPage() {
  const { t } = useI18n();
  const { data: status, isLoading } = useQuery<SystemStatus>({ queryKey: ["status"], queryFn: api.status.get, refetchInterval: 15_000 });
  const { data: channels } = useQuery({ queryKey: ["channels"], queryFn: api.channels.list });
  const { data: keys } = useQuery({ queryKey: ["keys"], queryFn: api.keys.list });
  const { data: maps } = useQuery({ queryKey: ["modelMaps"], queryFn: api.modelMaps.list });
  const { data: dash } = useQuery<DashboardSnapshot>({
    queryKey: ["dashboard", "24h"],
    queryFn: () => api.dashboard.get("24h"),
    refetchInterval: 30_000,
  });

  const baseUrl = status ? `http://${status.listen_host}:${status.listen_port}` : "http://127.0.0.1:3001";
  const enabledChannels = channels?.filter((c) => c.enabled).length ?? 0;
  const totalChannels = channels?.length ?? 0;
  const hasChannels = totalChannels > 0;
  const hasKeys = (keys?.length ?? 0) > 0;
  const hasMaps = (maps?.length ?? 0) > 0;

  const steps = [
    { label: t.dashboard.step1.title, desc: t.dashboard.step1.desc, done: hasChannels, href: "/ui/channels", icon: Server },
    { label: t.dashboard.step2.title, desc: t.dashboard.step2.desc, done: hasKeys, href: "/ui/keys", icon: Key },
    { label: t.dashboard.step3.title, desc: t.dashboard.step3.desc, done: false, href: "/ui/logs", icon: Activity },
  ];
  const completedSteps = steps.filter((s) => s.done).length;
  const ready = hasChannels && hasKeys;

  if (isLoading) {
    return (
      <div className="page-fade-enter">
        <PageHeader title={t.nav.overview} />
        <div className="space-y-4">
          <div className="skeleton h-32 w-full" />
          <div className="skeleton h-48 w-full" />
          <div className="grid grid-cols-3 gap-4">
            <div className="skeleton h-24" />
            <div className="skeleton h-24" />
            <div className="skeleton h-24" />
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="page-fade-enter space-y-6">
      <PageHeader title={t.nav.overview} description={t.dashboard.description} />

      {/* Setup Wizard Banner */}
      <section className="setup-banner">
        <div className="setup-banner-content">
          <div className="setup-banner-icon">
            <Sparkles size={20} />
          </div>
          <div className="setup-banner-text">
            <div className="setup-banner-title">
              {ready ? t.dashboard.bannerReady : t.dashboard.bannerTitle}
            </div>
            <div className="setup-banner-sub">
              {t.dashboard.bannerSub} {completedSteps}/{steps.length}
            </div>
          </div>
        </div>
        <div className="setup-banner-actions">
          <Link to="/ui/keys" className="btn-filled btn-sm">
            <KeyRound size={14} /> {t.dashboard.createApiKey}
          </Link>
          <Link to="/ui/channels" className="btn-outlined btn-sm">
            <Server size={14} /> {t.dashboard.addChannel}
          </Link>
          <Link to="/ui/logs" className="btn-ghost btn-sm">
            <Activity size={14} /> {t.dashboard.viewLogs}
          </Link>
        </div>
      </section>

      {/* Setup Steps */}
      <section className="setup-steps">
        <div className="section-heading">
          <h2>{t.dashboard.getStarted}</h2>
          <span className="section-meta">{completedSteps} / {steps.length}</span>
        </div>
        <div className="steps-list">
          {steps.map((step, idx) => (
            <Link key={step.label} to={step.href} className={`step-row click-ripple ${step.done ? "is-done" : ""}`}>
              <span className="step-number">
                {step.done ? <CheckCircle2 size={16} /> : <span>{idx + 1}</span>}
              </span>
              <span className="step-icon">
                <step.icon size={16} />
              </span>
              <div className="step-text">
                <div className="step-label">{step.label}</div>
                <div className="step-desc">{step.desc}</div>
              </div>
              <ArrowRight size={14} className="step-chevron" />
            </Link>
          ))}
        </div>
      </section>

      {/* Usage Overview + Balance */}
      <div className="overview-grid">
        <section className="usage-section">
          <div className="section-heading">
            <h2>{t.dashboard.usageOverview}</h2>
            <span className="section-meta">{t.dashboard.monitorBalance}</span>
          </div>
          <div className="usage-cards">
            <StatCard
              icon={Activity}
              label={t.dashboard.todayConsumption}
              value={`$${((dash?.today_errors ?? 0) * 0.001).toFixed(2)}`}
              sub={t.dashboard.last24h}
              color="var(--accent)"
            />
            <StatCard
              icon={TrendingUp}
              label={t.dashboard.totalUsage}
              value={`$${((dash?.total_requests ?? 0) * 0.001).toFixed(2)}`}
              sub={t.dashboard.totalConsumption}
              color="var(--accent-green)"
            />
            <StatCard
              icon={Zap}
              label={t.dashboard.requestCount}
              value={`${dash?.total_requests ?? 0}`}
              sub={t.dashboard.totalRequests}
              color="var(--accent-purple)"
            />
          </div>
        </section>

        <section className="balance-section">
          <div className="balance-panel">
            <div className="balance-header">
              <span className="balance-label">{t.dashboard.remainingBalance}</span>
              <span className="balance-status">
                <StatusDot status="ok" />
                {t.dashboard.statusNormal}
              </span>
            </div>
            <div className="balance-amount font-mono">$0.00</div>
            <div className="balance-details">
              <div className="balance-detail">
                <span>{t.dashboard.last24h}</span>
                <span className="font-mono">$0</span>
              </div>
              <div className="balance-detail">
                <span>{t.dashboard.availableTime}</span>
                <span>{t.dashboard.noUsage}</span>
              </div>
            </div>
            <Link to="/ui/settings" className="btn-filled btn-block">
              <Wallet size={14} /> {t.dashboard.wallet}
            </Link>
          </div>
        </section>
      </div>

      {/* Performance Health */}
      <section className="health-section">
        <div className="section-heading">
          <h2>{t.dashboard.performanceHealth}</h2>
          <span className="section-meta">{t.dashboard.last24hPerf}</span>
        </div>
        <div className="health-grid">
          <div className="health-metric">
            <div className="health-metric-icon">
              <CheckCircle2 size={16} />
            </div>
            <div className="health-metric-label">{t.dashboard.successRate}</div>
            <div className="health-metric-value font-mono">
              {dash ? `${(dash.success_rate * 100).toFixed(1)}%` : "—"}
            </div>
          </div>
          <div className="health-metric">
            <div className="health-metric-icon">
              <Clock size={16} />
            </div>
            <div className="health-metric-label">{t.dashboard.avgLatency}</div>
            <div className="health-metric-value font-mono">
              {dash ? `${Math.round(dash.average_latency_ms)}ms` : "—"}
            </div>
          </div>
          <div className="health-metric">
            <div className="health-metric-icon">
              <Activity size={16} />
            </div>
            <div className="health-metric-label">{t.dashboard.throughput}</div>
            <div className="health-metric-value font-mono">
              {dash ? `${dash.today_requests} ${t.dashboard.requestsToday}` : "—"}
            </div>
          </div>
        </div>
      </section>

      {/* Quick Actions + Info */}
      <div className="overview-bottom-grid">
        <section className="quick-actions-section">
          <div className="section-heading">
            <h2>{t.dashboard.quickActions}</h2>
          </div>
          <div className="quick-actions-list">
            <QuickAction icon={KeyRound} label={t.nav.keys} description={t.dashboard.createApiKeyDesc} href="/ui/keys" />
            <QuickAction icon={Activity} label={t.nav.logs} description={t.dashboard.viewLogsDesc} href="/ui/logs" />
            <QuickAction icon={Gauge} label={t.nav.analytics} description={t.dashboard.viewAnalyticsDesc} href="/ui/analytics" />
            <QuickAction icon={Settings} label={t.nav.settings} description={t.dashboard.configureDesc} href="/ui/settings" />
          </div>
        </section>

        <section className="info-section">
          <div className="info-panel">
            <div className="section-heading">
              <h2>{t.dashboard.apiInfo}</h2>
            </div>
            <div className="api-info-grid">
              <div className="api-info-item">
                <span className="api-info-label">{t.dashboard.routingEnabled}</span>
                <span className="api-info-value">
                  <StatusDot status="ok" />
                  {t.dashboard.currentDomain}
                </span>
              </div>
              <div className="api-info-item">
                <span className="api-info-label">{t.dashboard.authConfigured}</span>
                <span className="api-info-value">
                  <StatusDot status={hasKeys ? "ok" : "warn"} />
                  {hasKeys ? t.dashboard.requiresApiKey : t.dashboard.noKeys}
                </span>
              </div>
              <div className="api-info-item">
                <span className="api-info-label">{t.dashboard.modelSelected}</span>
                <span className="api-info-value">
                  <StatusDot status={hasMaps ? "ok" : "pulse"} />
                  {hasMaps ? `${maps?.length} ${t.dashboard.mappingsActive}` : t.dashboard.noMappings}
                </span>
              </div>
            </div>
            <div className="endpoint-box mt-3">
              <div className="endpoint-label">{t.dashboard.endpoint}</div>
              <div className="endpoint-value">
                <span className="font-mono">{baseUrl}/v1</span>
                <CopyInline value={`${baseUrl}/v1`} label={t.dashboard.copyAddress} />
              </div>
            </div>
          </div>

          <div className="info-panel">
            <div className="section-heading">
              <h2>{t.dashboard.announcements}</h2>
            </div>
            <div className="announcements-empty">
              <Sparkles size={24} className="announcements-icon" />
              <span>{t.dashboard.noAnnouncements}</span>
            </div>
          </div>
        </section>
      </div>

      {/* Watermark footer */}
      <div className="watermark-footer">
        <span>GitHub@OxygenAILab | OxygenAILab@StarsailsClover</span>
      </div>
    </div>
  );
}
