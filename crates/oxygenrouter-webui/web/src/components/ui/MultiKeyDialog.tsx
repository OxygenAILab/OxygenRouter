import React, { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound, Power, PowerOff, Plus, RefreshCw, Trash2 } from "lucide-react";
import { api, Channel, ChannelKeyStatus } from "../../lib/api";
import { useI18n } from "../../lib/i18nContext";
import { useToast } from "../../lib/toast";
import Dialog from "./Dialog";
import Button from "./Button";
import Select from "./Select";
import SegmentedControl from "./SegmentedControl";

/** Multi-key manager — mirrors NewAPI's per-key status/enable/disable/delete workflows. */
export default function MultiKeyDialog({ channel, onClose }: { channel: Channel; onClose: () => void }) {
  const { t } = useI18n();
  const toast = useToast();
  const qc = useQueryClient();
  const [mode, setMode] = useState<"random" | "polling">(
    channel.info?.multi_key_mode === "polling" ? "polling" : "random",
  );
  const [newKeys, setNewKeys] = useState("");

  const { data: keys, isLoading } = useQuery<ChannelKeyStatus[]>({
    queryKey: ["channelKeys", channel.id],
    queryFn: () => api.channels.listKeys(channel.id),
  });

  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ["channelKeys", channel.id] });
    qc.invalidateQueries({ queryKey: ["channels"] });
  };

  const mut = useMutation({
    mutationFn: (input: { action: string; index?: number; reason?: string; keys?: string }) =>
      api.channels.manageKeys(channel.id, input),
    onSuccess: () => invalidate(),
    onError: (error: unknown) => toast.error(t.channels.keyActionFailed, String((error as Error)?.message ?? error)),
  });

  const enabled = keys?.filter((k) => k.status === 1).length ?? 0;
  const total = keys?.length ?? 0;

  return (
    <Dialog
      open
      onClose={onClose}
      title={`${t.channels.multiKeys} · ${channel.name}`}
      description={t.channels.multiKeysDesc}
      size="lg"
      footer={
        <>
          <Button variant="outlined" onClick={onClose}>{t.common.cancel}</Button>
          <Button
            variant="outlined"
            leftIcon={<Power size={13} />}
            onClick={() => mut.mutate({ action: "enable_all_keys" })}
          >
            {t.channels.enableAllKeys}
          </Button>
          <Button
            variant="outlined"
            leftIcon={<PowerOff size={13} />}
            onClick={() => mut.mutate({ action: "disable_all_keys" })}
          >
            {t.channels.disableAllKeys}
          </Button>
        </>
      }
    >
      <div className="flex items-center justify-between gap-3 mb-3 flex-wrap">
        <span className="text-xs text-[var(--md-on-surface-variant)]">
          {enabled} / {total} {t.channels.keysEnabled}
        </span>
        <SegmentedControl
          size="sm"
          value={mode}
          onChange={(value) => {
            setMode(value);
            mut.mutate({ action: "set_mode", reason: value });
          }}
          ariaLabel={t.channels.keyMode}
          options={[
            { value: "random", label: t.channels.keyModeRandom },
            { value: "polling", label: t.channels.keyModePolling },
          ]}
        />
      </div>

      {isLoading ? (
        <div className="space-y-2">
          {[0, 1, 2].map((i) => (
            <div key={i} className="skeleton h-10 w-full" />
          ))}
        </div>
      ) : (
        <div className="multikey-list">
          {(keys ?? []).map((key) => (
            <div key={key.index} className={`multikey-row ${key.status !== 1 ? "is-disabled" : ""}`}>
              <span className={`status-dot ${key.status === 1 ? "is-pulse" : "is-error"}`} />
              <span className="multikey-index font-mono">#{key.index + 1}</span>
              <span className="multikey-preview font-mono">{key.preview}</span>
              <span className={`badge ${key.status === 1 ? "badge-success" : key.status === 2 ? "badge-neutral" : "badge-error"}`}>
                {key.status === 1 ? t.common.enabled : key.status === 2 ? t.common.disabled : t.channels.autoDisabled}
              </span>
              <div className="multikey-actions">
                <Button
                  variant="ghost"
                  size="sm"
                  leftIcon={<KeyRound size={13} />}
                  onClick={() => mut.mutate({ action: key.status === 1 ? "disable_key" : "enable_key", index: key.index })}
                >
                  {key.status === 1 ? t.channels.disableKey : t.channels.enableKey}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  leftIcon={<Trash2 size={13} />}
                  onClick={() => mut.mutate({ action: "delete_key", index: key.index })}
                >
                  {t.common.delete}
                </Button>
              </div>
              {key.disabled_reason && <span className="multikey-reason">{key.disabled_reason}</span>}
            </div>
          ))}
        </div>
      )}

      <div className="mt-4 flex flex-col gap-2">
        <label className="field-label">{t.channels.appendKeys}</label>
        <textarea
          className="input font-mono text-xs"
          rows={3}
          value={newKeys}
          onChange={(e) => setNewKeys(e.target.value)}
          placeholder={t.channels.batchKeyPlaceholder}
        />
        <div className="flex gap-2">
          <Button
            variant="tonal"
            size="sm"
            leftIcon={<Plus size={13} />}
            disabled={!newKeys.trim()}
            onClick={() => {
              mut.mutate({ action: "append_keys", keys: newKeys });
              setNewKeys("");
            }}
          >
            {t.channels.appendKeysBtn}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            leftIcon={<RefreshCw size={13} />}
            onClick={() => mut.mutate({ action: "delete_disabled_keys" })}
          >
            {t.channels.deleteDisabledKeys}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}
