import React, { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CheckCircle2, Copy, KeyRound, RefreshCw, ShieldCheck, ShieldOff, Smartphone } from "lucide-react";
import { api, SessionView, TwoFactorSetup } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import { useAuth } from "../lib/auth";
import { useToast } from "../lib/toast";
import Button from "../components/ui/Button";
import Dialog from "../components/ui/Dialog";
import DataTable, { DataTableColumn } from "../components/ui/DataTable";

function timestamp(value: string): string {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString();
}

/** A value shown once, with the copy button that makes "once" survivable. */
function Once({ value, onDone }: { value: string; onDone?: () => void }) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);
  return (
    <div className="secret-once">
      <code>{value}</code>
      <Button
        variant="outlined"
        size="sm"
        onClick={() => {
          navigator.clipboard?.writeText(value);
          setCopied(true);
          setTimeout(() => setCopied(false), 1200);
          onDone?.();
        }}
      >
        {copied ? <CheckCircle2 size={14} /> : <Copy size={14} />}
        {copied ? t.common.copied : t.common.copy}
      </Button>
    </div>
  );
}

export default function SecurityPage() {
  const { t } = useI18n();
  const toast = useToast();
  const qc = useQueryClient();
  const { user } = useAuth();

  // Nothing here is readable without a session, so the queries are held back
  // rather than fired into a 401 and rendered as empty panels.
  const signedIn = Boolean(user);
  const { data: twoFa } = useQuery({ queryKey: ["twoFa"], queryFn: api.account.twoFactor.status, enabled: signedIn });
  const { data: sessions } = useQuery({ queryKey: ["sessions"], queryFn: api.account.sessions.list, enabled: signedIn });
  const { data: token } = useQuery({ queryKey: ["accessToken"], queryFn: api.account.accessToken.status, enabled: signedIn });

  const [setup, setSetup] = useState<TwoFactorSetup | null>(null);
  const [enableCode, setEnableCode] = useState("");
  const [recovery, setRecovery] = useState<string[] | null>(null);
  const [confirmDisable, setConfirmDisable] = useState(false);
  const [disableCode, setDisableCode] = useState("");
  const [rotateCode, setRotateCode] = useState("");
  const [tokenValue, setTokenValue] = useState<string | null>(null);

  const startSetup = useMutation({
    mutationFn: api.account.twoFactor.setup,
    onSuccess: setSetup,
    onError: (error: unknown) => toast.error(t.security.actionFailed, String((error as Error)?.message ?? error)),
  });
  const enableTwoFa = useMutation({
    mutationFn: () => api.account.twoFactor.enable(enableCode.trim()),
    onSuccess: (result) => {
      setRecovery(result.backup_codes);
      setSetup(null);
      setEnableCode("");
      qc.invalidateQueries({ queryKey: ["twoFa"] });
      toast.success(t.security.twoFactorEnabled);
    },
    onError: (error: unknown) => toast.error(t.security.badCode, String((error as Error)?.message ?? error)),
  });
  const disableTwoFa = useMutation({
    mutationFn: () => api.account.twoFactor.disable(disableCode.trim()),
    onSuccess: () => {
      setConfirmDisable(false);
      setDisableCode("");
      setRecovery(null);
      qc.invalidateQueries({ queryKey: ["twoFa"] });
      toast.success(t.security.twoFactorDisabled);
    },
    onError: (error: unknown) => toast.error(t.security.badCode, String((error as Error)?.message ?? error)),
  });
  const regenerate = useMutation({
    mutationFn: () => api.account.twoFactor.backupCodes(rotateCode.trim()),
    onSuccess: (result) => {
      setRecovery(result.backup_codes);
      setRotateCode("");
      qc.invalidateQueries({ queryKey: ["twoFa"] });
      toast.success(t.security.codesReissued);
    },
    onError: (error: unknown) => toast.error(t.security.badCode, String((error as Error)?.message ?? error)),
  });

  const revokeSession = useMutation({
    mutationFn: (id: string) => api.account.sessions.revoke(id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["sessions"] });
      toast.success(t.security.sessionRevoked);
    },
    onError: (error: unknown) => toast.error(t.security.actionFailed, String((error as Error)?.message ?? error)),
  });
  const revokeOthers = useMutation({
    mutationFn: api.account.sessions.revokeOthers,
    onSuccess: (count) => {
      qc.invalidateQueries({ queryKey: ["sessions"] });
      toast.success(t.security.sessionsRevoked.replace("{n}", String(count)));
    },
    onError: (error: unknown) => toast.error(t.security.actionFailed, String((error as Error)?.message ?? error)),
  });

  const generateToken = useMutation({
    mutationFn: api.account.accessToken.generate,
    onSuccess: (value) => {
      setTokenValue(value);
      qc.invalidateQueries({ queryKey: ["accessToken"] });
    },
    onError: (error: unknown) => toast.error(t.security.actionFailed, String((error as Error)?.message ?? error)),
  });
  const revokeToken = useMutation({
    mutationFn: api.account.accessToken.revoke,
    onSuccess: () => {
      setTokenValue(null);
      qc.invalidateQueries({ queryKey: ["accessToken"] });
      toast.success(t.security.tokenRevoked);
    },
    onError: (error: unknown) => toast.error(t.security.actionFailed, String((error as Error)?.message ?? error)),
  });

  const sessionColumns: DataTableColumn<SessionView>[] = [
    {
      key: "created",
      header: t.security.created,
      width: "34%",
      render: (row) => (
        <span className="font-mono text-xs">
          {timestamp(row.created_at)}
          {row.current && <span className="pricing-badge security-current">{t.security.thisDevice}</span>}
        </span>
      ),
    },
    { key: "expires", header: t.security.expires, width: "30%", render: (row) => <span className="font-mono text-xs">{timestamp(row.expires_at)}</span> },
    {
      key: "actions",
      header: "",
      width: "36%",
      align: "right",
      render: (row) =>
        row.current ? (
          <span className="pricing-muted">{t.security.currentSession}</span>
        ) : (
          <Button variant="ghost" size="sm" onClick={() => revokeSession.mutate(row.id)}>
            {t.security.revoke}
          </Button>
        ),
    },
  ];

  return (
    <div className="page-fade-enter space-y-5">
      <PageHeader title={t.security.title} description={t.security.description} />

      {!signedIn && (
        <section className="dashboard-section">
          <div className="security-body">
            <p className="pricing-muted">{t.security.signInFirst}</p>
          </div>
        </section>
      )}

      {signedIn && (
      <div className="security-grid">
        <section className="dashboard-section">
          <div className="section-heading">
            <h2>
              <ShieldCheck size={14} /> {t.security.twoFactor}
            </h2>
            <span className="section-meta">
              {twoFa?.enabled ? t.security.enabled : t.security.disabled}
              {twoFa?.enabled ? ` · ${twoFa.backup_codes_remaining} ${t.security.codesLeft}` : ""}
            </span>
          </div>
          <div className="security-body">
            {!twoFa?.enabled && !setup && (
              <>
                <p className="pricing-muted">{t.security.twoFactorHint}</p>
                <Button variant="filled" size="sm" loading={startSetup.isPending} onClick={() => startSetup.mutate()}>
                  <Smartphone size={14} /> {t.security.startSetup}
                </Button>
              </>
            )}

            {setup && (
              <>
                <p className="pricing-muted">{t.security.scanHint}</p>
                <Once value={setup.secret} />
                <code className="pricing-preview-code">{setup.otpauth_uri}</code>
                <div className="security-inline">
                  <input
                    className="input font-mono text-xs"
                    value={enableCode}
                    onChange={(event) => setEnableCode(event.target.value)}
                    placeholder={t.security.codePlaceholder}
                    inputMode="numeric"
                  />
                  <Button variant="filled" size="sm" loading={enableTwoFa.isPending} disabled={!enableCode.trim()} onClick={() => enableTwoFa.mutate()}>
                    {t.security.enable}
                  </Button>
                </div>
              </>
            )}

            {twoFa?.enabled && !confirmDisable && (
              <>
                <p className="pricing-muted">{t.security.enabledHint}</p>
                <div className="security-inline">
                  <input
                    className="input font-mono text-xs"
                    value={rotateCode}
                    onChange={(event) => setRotateCode(event.target.value)}
                    placeholder={t.security.codePlaceholder}
                    inputMode="numeric"
                  />
                  <Button variant="outlined" size="sm" loading={regenerate.isPending} disabled={!rotateCode.trim()} onClick={() => regenerate.mutate()}>
                    <RefreshCw size={14} /> {t.security.reissueCodes}
                  </Button>
                </div>
                <Button variant="ghost" size="sm" onClick={() => setConfirmDisable(true)}>
                  <ShieldOff size={14} /> {t.security.disable}
                </Button>
              </>
            )}

            {recovery && (
              <div className="security-recovery">
                <div className="field-label">{t.security.recoveryCodes}</div>
                <p className="pricing-muted">{t.security.recoveryOnce}</p>
                <div className="security-codes">
                  {recovery.map((code) => (
                    <code key={code} className="font-mono">
                      {code}
                    </code>
                  ))}
                </div>
              </div>
            )}
          </div>
        </section>

        <section className="dashboard-section">
          <div className="section-heading">
            <h2>
              <KeyRound size={14} /> {t.security.accessToken}
            </h2>
            <span className="section-meta">{token?.enabled ? t.security.enabled : t.security.disabled}</span>
          </div>
          <div className="security-body">
            <p className="pricing-muted">{t.security.tokenHint}</p>
            {token?.created_at && <span className="pricing-muted font-mono">{timestamp(token.created_at)}</span>}
            {tokenValue && <Once value={tokenValue} />}
            <div className="security-inline">
              <Button variant="filled" size="sm" loading={generateToken.isPending} onClick={() => generateToken.mutate()}>
                {token?.enabled ? t.security.rotateToken : t.security.createToken}
              </Button>
              {token?.enabled && (
                <Button variant="ghost" size="sm" loading={revokeToken.isPending} onClick={() => revokeToken.mutate()}>
                  {t.security.revokeToken}
                </Button>
              )}
            </div>
          </div>
        </section>
      </div>
      )}

      {signedIn && (
      <section className="dashboard-section">
        <div className="section-heading">
          <h2>{t.security.sessions}</h2>
          <span className="section-meta">
            {(sessions?.length ?? 0)} {t.security.active}
          </span>
        </div>
        <DataTable
          columns={sessionColumns}
          rows={sessions ?? []}
          rowKey={(row) => row.id}
          empty={<span className="pricing-muted">{t.security.noSessions}</span>}
        />
        <div className="security-footer">
          <Button variant="outlined" size="sm" loading={revokeOthers.isPending} onClick={() => revokeOthers.mutate()}>
            {t.security.revokeOthers}
          </Button>
        </div>
      </section>
      )}

      {signedIn && confirmDisable && (
        <Dialog
          open
          onClose={() => setConfirmDisable(false)}
          title={t.security.disable}
          description={t.security.disableHint}
          size="sm"
          footer={
            <>
              <Button variant="outlined" onClick={() => setConfirmDisable(false)}>
                {t.common.cancel}
              </Button>
              <Button variant="filled" loading={disableTwoFa.isPending} disabled={!disableCode.trim()} onClick={() => disableTwoFa.mutate()}>
                {t.security.disable}
              </Button>
            </>
          }
        >
          <input
            className="input font-mono text-xs"
            value={disableCode}
            onChange={(event) => setDisableCode(event.target.value)}
            placeholder={t.security.codePlaceholder}
            inputMode="numeric"
          />
        </Dialog>
      )}
    </div>
  );
}
