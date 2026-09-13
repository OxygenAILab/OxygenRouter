import React from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, CheckCircle, Eye, EyeOff, RefreshCw, Save, Trash2 } from "lucide-react";
import { api, AppSettings } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import { useTheme } from "../lib/themeContext";
import { useToast } from "../lib/toast";
import Button from "../components/ui/Button";
import Input from "../components/ui/Input";
import Select from "../components/ui/Select";
import Switch from "../components/ui/Switch";
import Dialog from "../components/ui/Dialog";
import SegmentedControl from "../components/ui/SegmentedControl";

type Tab = "general" | "routing" | "upstream" | "retention" | "danger";

function SettingRow({ label, description, children }: { label: string; description?: string; children: React.ReactNode }) {
  return (
    <div className="settings-row">
      <div className="flex-1 pr-6">
        <div className="text-sm font-medium">{label}</div>
        {description && (
          <div className="text-xs mt-0.5" style={{ color: "var(--md-on-surface-variant)" }}>
            {description}
          </div>
        )}
      </div>
      <div className="flex-shrink-0">{children}</div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="settings-section">
      <div className="section-heading">
        <h2>{title}</h2>
      </div>
      <div className="card">{children}</div>
    </section>
  );
}

export default function SettingsPage() {
  const { t, locale, setLocale } = useI18n();
  const { theme, setTheme } = useTheme();
  const toast = useToast();
  const [tab, setTab] = React.useState<Tab>("general");
  const qc = useQueryClient();
  const { data: settings, isLoading } = useQuery({ queryKey: ["settings"], queryFn: api.settings.get });
  const [form, setForm] = React.useState<AppSettings | null>(null);
  const [showToken, setShowToken] = React.useState(false);
  const [saved, setSaved] = React.useState(false);
  const [confirmAction, setConfirmAction] = React.useState<null | "resetLogs" | "rotateToken">(null);

  React.useEffect(() => {
    if (settings && !form) setForm(settings);
  }, [settings, form]);

  const mut = useMutation({
    mutationFn: (data: Partial<AppSettings>) => api.settings.update(data),
    onSuccess: (updated) => {
      qc.setQueryData(["settings"], updated);
      setForm(updated);
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    },
  });

  if (isLoading || !form) {
    return (
      <div>
        <PageHeader title={t.settings.title} description={t.settings.description} />
        <div className="flex items-center justify-center py-20">
          <RefreshCw size={20} className="animate-spin" />
        </div>
      </div>
    );
  }

  const update = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) =>
    setForm((prev) => (prev ? { ...prev, [key]: value } : prev));

  const handleRotateToken = () => {
    const next = crypto.randomUUID();
    update("local_api_token", next);
    mut.mutate({ local_api_token: next });
    toast.success(t.settings.rotateToken, t.settings.rotateTokenDesc);
    setConfirmAction(null);
  };

  const handleResetLogs = () => {
    fetch("/api/logs?limit=0", { method: "DELETE" }).then(() => {
      qc.invalidateQueries({ queryKey: ["logs"] });
      toast.success(t.settings.resetLogsBtn, t.settings.resetLogsDesc);
    });
    setConfirmAction(null);
  };

  const tabs: { id: Tab; label: string }[] = [
    { id: "general", label: t.settings.tabGeneral },
    { id: "routing", label: t.settings.routing },
    { id: "upstream", label: t.settings.upstream },
    { id: "retention", label: t.settings.retention },
    { id: "danger", label: t.settings.tabDanger },
  ];

  return (
    <div className="page-fade-enter">
      <PageHeader title={t.settings.title} description={t.settings.description} />

      <div className="tabs" role="tablist">
        {tabs.map((tb) => (
          <button key={tb.id} className={`tab ${tab === tb.id ? "is-active" : ""}`} onClick={() => setTab(tb.id)} role="tab" aria-selected={tab === tb.id}>
            {tb.label}
          </button>
        ))}
      </div>

      {tab === "general" && (
        <div className="space-y-6">
          <Section title={t.settings.ui}>
            <SettingRow label={t.settings.language} description={t.settings.languageDesc}>
              <Select
                value={locale}
                onChange={(v) => setLocale(v as "en" | "zh")}
                ariaLabel={t.settings.language}
                options={[
                  { value: "en", label: "English" },
                  { value: "zh", label: "中文" },
                ]}
              />
            </SettingRow>
            <SettingRow label={t.settings.theme} description={t.settings.themeDesc}>
              <SegmentedControl
                value={theme}
                onChange={(v) => setTheme(v as "dark" | "light")}
                ariaLabel={t.settings.theme}
                options={[
                  { value: "dark", label: t.settings.themeDark },
                  { value: "light", label: t.settings.themeLight },
                ]}
              />
            </SettingRow>
            <SettingRow label={t.settings.openBrowser} description={t.settings.openBrowserDesc}>
              <Switch checked={form.open_browser_on_start} onChange={(v) => update("open_browser_on_start", v)} aria-label={t.settings.openBrowser} />
            </SettingRow>
          </Section>
          <Section title={t.settings.server}>
            <SettingRow label={t.settings.listenHost} description={t.settings.listenHostDesc}>
              <Input className="w-48" value={form.listen_host} onChange={(e) => update("listen_host", e.target.value)} />
            </SettingRow>
            <SettingRow label={t.settings.listenPort} description={t.settings.listenPortDesc}>
              <input
                className="input w-24"
                type="number"
                value={form.listen_port}
                onChange={(e) => update("listen_port", Number(e.target.value))}
              />
            </SettingRow>
            <SettingRow label={t.settings.localToken} description={t.settings.localTokenDesc}>
              <div className="flex gap-2 items-center">
                <input
                  className="input w-64 font-mono text-xs"
                  type={showToken ? "text" : "password"}
                  value={form.local_api_token}
                  onChange={(e) => update("local_api_token", e.target.value)}
                />
                <Button variant="ghost" size="sm" aria-label={showToken ? t.settings.hideToken : t.settings.showToken} onClick={() => setShowToken((v) => !v)}>
                  {showToken ? <EyeOff size={13} /> : <Eye size={13} />}
                </Button>
                <Button variant="outlined" size="sm" onClick={() => setConfirmAction("rotateToken")}>
                  {t.common.refresh}
                </Button>
              </div>
            </SettingRow>
          </Section>
        </div>
      )}

      {tab === "routing" && (
        <Section title={t.settings.routing}>
          <SettingRow label={t.settings.maxRetries} description={t.settings.maxRetriesDesc}>
            <input
              className="input w-20"
              type="number"
              min={0}
              max={10}
              value={form.max_retries}
              onChange={(e) => update("max_retries", Number(e.target.value))}
            />
          </SettingRow>
          <SettingRow label={t.settings.retryDelay} description={t.settings.retryDelayDesc}>
            <input
              className="input w-28"
              type="number"
              min={100}
              step={50}
              value={form.retry_delay_ms}
              onChange={(e) => update("retry_delay_ms", Number(e.target.value))}
            />
          </SettingRow>
          <SettingRow label={t.settings.retryBackoff} description={t.settings.retryBackoffDesc}>
            <Select
              value={form.retry_backoff}
              onChange={(v) => update("retry_backoff", v)}
              ariaLabel={t.settings.retryBackoff}
              options={[
                { value: "fixed", label: t.settings.backoffFixed },
                { value: "linear", label: t.settings.backoffLinear },
                { value: "exponential", label: t.settings.backoffExp },
              ]}
            />
          </SettingRow>
        </Section>
      )}

      {tab === "upstream" && (
        <Section title={t.settings.upstream}>
          <SettingRow label={t.settings.upstreamTimeout} description={t.settings.upstreamTimeoutDesc}>
            <input
              className="input w-28"
              type="number"
              min={1000}
              step={1000}
              value={form.upstream_timeout_ms}
              onChange={(e) => update("upstream_timeout_ms", Number(e.target.value))}
            />
          </SettingRow>
          <SettingRow label={t.settings.userAgent} description={t.settings.userAgentDesc}>
            <input className="input w-72" value={form.user_agent} onChange={(e) => update("user_agent", e.target.value)} />
          </SettingRow>
          <SettingRow label={t.settings.maxConcurrent} description={t.settings.maxConcurrentDesc}>
            <input
              className="input w-24"
              type="number"
              min={1}
              max={1024}
              value={form.max_concurrent_requests}
              onChange={(e) => update("max_concurrent_requests", Number(e.target.value))}
            />
          </SettingRow>
        </Section>
      )}

      {tab === "retention" && (
        <Section title={t.settings.retention}>
          <SettingRow label={t.settings.logRetention} description={t.settings.logRetentionDesc}>
            <input
              className="input w-24"
              type="number"
              min={0}
              step={1}
              value={form.request_log_retention_days}
              onChange={(e) => update("request_log_retention_days", Number(e.target.value))}
            />
          </SettingRow>
          <SettingRow label={t.settings.logLevel} description="">
            <Select
              value={form.log_level}
              onChange={(v) => update("log_level", v)}
              ariaLabel={t.settings.logLevel}
              options={[
                { value: "trace", label: "trace" },
                { value: "debug", label: "debug" },
                { value: "info", label: "info" },
                { value: "warn", label: "warn" },
                { value: "error", label: "error" },
              ]}
            />
          </SettingRow>
        </Section>
      )}

      {tab === "danger" && (
        <section className="danger-zone">
          <div className="flex items-center gap-2 mb-3">
            <AlertTriangle size={18} className="text-[var(--md-error)]" />
            <h2 className="text-sm font-semibold">{t.settings.dangerZoneTitle}</h2>
          </div>
          <p className="text-xs mb-4 text-[var(--md-on-surface-variant)]">{t.settings.dangerZoneSub}</p>
          <div className="space-y-3">
            <div className="flex items-center justify-between gap-3">
              <div>
                <div className="text-sm font-medium">{t.settings.resetLogs}</div>
                <div className="text-xs text-[var(--md-on-surface-variant)]">{t.settings.resetLogsDesc}</div>
              </div>
              <button className="danger-button" onClick={() => setConfirmAction("resetLogs")}>
                <Trash2 size={12} /> {t.settings.resetLogsBtn}
              </button>
            </div>
            <div className="flex items-center justify-between gap-3">
              <div>
                <div className="text-sm font-medium">{t.settings.rotateToken}</div>
                <div className="text-xs text-[var(--md-on-surface-variant)]">{t.settings.rotateTokenDesc}</div>
              </div>
              <button className="danger-button" onClick={() => setConfirmAction("rotateToken")}>
                <RefreshCw size={12} /> {t.common.refresh}
              </button>
            </div>
          </div>
        </section>
      )}

      <Dialog
        open={confirmAction === "resetLogs"}
        onClose={() => setConfirmAction(null)}
        title={t.settings.resetLogs}
        description={t.settings.resetLogsConfirm}
        size="sm"
        footer={
          <>
            <Button variant="outlined" onClick={() => setConfirmAction(null)}>{t.common.cancel}</Button>
            <Button variant="filled" className="danger-button" onClick={handleResetLogs} leftIcon={<Trash2 size={13} />}>
              {t.settings.resetLogsBtn}
            </Button>
          </>
        }
      >
        <span />
      </Dialog>

      <Dialog
        open={confirmAction === "rotateToken"}
        onClose={() => setConfirmAction(null)}
        title={t.settings.rotateToken}
        description={t.settings.rotateTokenConfirm}
        size="sm"
        footer={
          <>
            <Button variant="outlined" onClick={() => setConfirmAction(null)}>{t.common.cancel}</Button>
            <Button variant="filled" className="danger-button" onClick={handleRotateToken} leftIcon={<RefreshCw size={13} />}>
              {t.common.confirm}
            </Button>
          </>
        }
      >
        <span />
      </Dialog>

      {tab !== "danger" && (
        <div className="settings-sticky-footer">
          <div className="flex items-center justify-end gap-3">
            {saved && (
              <span className="text-xs text-[var(--md-success)] font-medium inline-flex items-center gap-1.5">
                <CheckCircle size={13} /> {t.common.saved}
              </span>
            )}
            <Button
              onClick={() => mut.mutate(form)}
              disabled={mut.isPending}
              loading={mut.isPending}
              leftIcon={saved ? <CheckCircle size={14} /> : <Save size={14} />}
            >
              {saved ? t.common.saved : mut.isPending ? t.common.saving : t.common.save}
            </Button>
          </div>
        </div>
      )}
      {mut.isError && <div className="error-banner mt-3">{String(mut.error)}</div>}
    </div>
  );
}
