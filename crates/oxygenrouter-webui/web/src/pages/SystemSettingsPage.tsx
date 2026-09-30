import React, { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CheckCircle, RefreshCw, Settings2, Undo2 } from "lucide-react";
import { api, OptionEntry } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import { useToast } from "../lib/toast";
import Button from "../components/ui/Button";
import Switch from "../components/ui/Switch";
import SegmentedControl from "../components/ui/SegmentedControl";

const SECTION_ORDER: OptionEntry["section"][] = [
  "site",
  "auth",
  "routing",
  "billing",
  "operations",
  "security",
  "models",
  "bootstrap",
];

function OptionRow({
  option,
  onSave,
  pending,
}: {
  option: OptionEntry;
  onSave: (key: string, value: string) => void;
  pending: boolean;
}) {
  const { t } = useI18n();
  const [value, setValue] = useState(option.value);
  const dirty = value !== option.value;

  React.useEffect(() => {
    setValue(option.value);
  }, [option.value]);

  return (
    <div className="settings-row">
      <div className="flex-1 pr-6">
        <div className="text-sm font-medium font-mono">{option.key}</div>
        <div className="text-xs mt-0.5" style={{ color: "var(--md-on-surface-variant)" }}>
          {option.description}
        </div>
      </div>
      <div className="flex-shrink-0 flex items-center gap-2">
        {option.kind === "bool" ? (
          <Switch
            checked={value.toLowerCase() === "true" || value === "1"}
            onChange={(checked) => {
              const next = checked ? "true" : "false";
              setValue(next);
              onSave(option.key, next);
            }}
            aria-label={option.key}
          />
        ) : (
          <>
            <input
              className={`input ${option.kind === "string" ? "w-56" : "w-28"} font-mono text-xs`}
              type={option.kind === "int" || option.kind === "float" ? "number" : "text"}
              value={value}
              onChange={(e) => setValue(e.target.value)}
            />
            {dirty && (
              <>
                <Button
                  variant="filled"
                  size="sm"
                  loading={pending}
                  leftIcon={<CheckCircle size={13} />}
                  onClick={() => onSave(option.key, value)}
                >
                  {t.common.save}
                </Button>
                <button className="btn-ghost p-1.5" onClick={() => setValue(option.value)} aria-label={t.common.cancel}>
                  <Undo2 size={13} />
                </button>
              </>
            )}
            {!dirty && value !== option.default && (
              <button
                className="btn-ghost p-1.5"
                title={t.systemSettings.resetDefault}
                onClick={() => {
                  setValue(option.default);
                  onSave(option.key, option.default);
                }}
              >
                <Undo2 size={13} />
              </button>
            )}
          </>
        )}
      </div>
    </div>
  );
}

export default function SystemSettingsPage() {
  const { t } = useI18n();
  const toast = useToast();
  const qc = useQueryClient();
  const [section, setSection] = useState<OptionEntry["section"]>("site");

  const { data: options, isLoading, refetch } = useQuery({ queryKey: ["options"], queryFn: api.options.list });

  const mut = useMutation({
    mutationFn: ({ key, value }: { key: string; value: string }) => api.options.update(key, value),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["options"] });
      toast.success(t.systemSettings.saved);
    },
    onError: (error: unknown) => toast.error(t.systemSettings.saveFailed, String((error as Error)?.message ?? error)),
  });

  const grouped = useMemo(() => {
    const map: Record<string, OptionEntry[]> = {};
    for (const option of options ?? []) {
      (map[option.section] ??= []).push(option);
    }
    return map;
  }, [options]);

  const counts = useMemo(() => {
    const map: Record<string, number> = {};
    for (const [key, list] of Object.entries(grouped)) map[key] = list.length;
    return map;
  }, [grouped]);

  const active = grouped[section] ?? [];

  return (
    <div className="page-fade-enter">
      <PageHeader
        title={t.systemSettings.title}
        description={t.systemSettings.description}
        action={
          <button className="btn-outlined btn-sm" onClick={() => refetch()} aria-label={t.common.refresh}>
            <RefreshCw size={13} /> {t.common.refresh}
          </button>
        }
      />

      <div className="log-filters">
        <SegmentedControl
          size="sm"
          value={section}
          onChange={setSection}
          ariaLabel={t.systemSettings.title}
          options={SECTION_ORDER.map((id) => ({
            value: id,
            label: `${t.systemSettings.sections[id]} (${counts[id] ?? 0})`,
          }))}
        />
      </div>

      {isLoading ? (
        <div className="settings-section">
          <div className="card">
            {[0, 1, 2, 3].map((i) => (
              <div key={i} className="settings-row">
                <div className="skeleton h-4 w-40" />
                <div className="skeleton h-6 w-28" />
              </div>
            ))}
          </div>
        </div>
      ) : active.length === 0 ? (
        <div className="empty-state">
          <Settings2 size={32} />
          <p className="text-sm">{t.systemSettings.empty}</p>
        </div>
      ) : (
        <section className="settings-section">
          <div className="section-heading">
            <h2>{t.systemSettings.sections[section]}</h2>
            <span className="section-meta">{active.length} {t.systemSettings.entries}</span>
          </div>
          <div className="card">
            {active.map((option) => (
              <OptionRow
                key={option.key}
                option={option}
                pending={mut.isPending}
                onSave={(key, value) => mut.mutate({ key, value })}
              />
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
