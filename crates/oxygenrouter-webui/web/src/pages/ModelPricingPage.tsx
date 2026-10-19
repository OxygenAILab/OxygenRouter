import React, { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Calculator, Pencil, RefreshCw, RotateCcw, Search, Wand2 } from "lucide-react";
import {
  api,
  ModelPricingConversion,
  ModelPricingEntry,
  ModelPricingPreview,
  ModelPricingSnapshot,
} from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import { useToast } from "../lib/toast";
import Button from "../components/ui/Button";
import Dialog from "../components/ui/Dialog";
import Select from "../components/ui/Select";
import DataTable, { DataTableColumn } from "../components/ui/DataTable";
import Pagination from "../components/ui/Pagination";
import Tooltip from "../components/ui/Tooltip";

const PAGE_SIZE = 20;

/** A stored value rendered for a table cell, short enough not to wrap. */
function valueText(value: unknown): string {
  if (value === undefined || value === null) return "—";
  if (typeof value === "string") return value;
  if (typeof value === "number") return value.toLocaleString(undefined, { maximumFractionDigits: 6 });
  return JSON.stringify(value);
}

/** The pricing mode a row is actually on, read from its effective values. */
function modeOf(entry: ModelPricingEntry): string {
  const mode = entry.effective.billing_mode;
  return typeof mode === "string" ? mode : "ratio";
}

/** USD per million tokens, or per call, as the engine resolved it. */
function priceText(entry: ModelPricingEntry): string {
  const price = entry.effective.ModelPrice;
  if (typeof price === "number") return `$${price} / call`;
  if (modeOf(entry) === "tiered_expr") {
    const expr = entry.effective.billing_expr;
    return typeof expr === "string" ? expr : "—";
  }
  const ratio = entry.effective.ModelRatio;
  if (typeof ratio !== "number") return "—";
  // One ratio unit is $2 per million tokens at the shipped QuotaPerUnit, which
  // is the same arithmetic the preview's billing details report.
  const perMillion = ratio * 2;
  return `$${perMillion.toLocaleString(undefined, { maximumFractionDigits: 4 })} / M`;
}

/**
 * The editor for one model's price.
 *
 * The draft is the whole configuration, matching the API: switching to an
 * expression clears the ratio rather than leaving a stale multiplier behind it.
 */
function PricingDialog({ entry, onClose }: { entry: ModelPricingEntry; onClose: () => void }) {
  const { t } = useI18n();
  const toast = useToast();
  const qc = useQueryClient();

  const configuredRatio = entry.configured.ModelRatio;
  const configuredExpr = entry.configured.billing_expr;
  const [mode, setMode] = useState<string>(modeOf(entry) === "tiered_expr" ? "tiered_expr" : "ratio");
  const [ratio, setRatio] = useState<string>(
    typeof configuredRatio === "number" ? String(configuredRatio) : "",
  );
  const [expression, setExpression] = useState<string>(
    typeof configuredExpr === "string" ? configuredExpr : "",
  );
  const [preview, setPreview] = useState<ModelPricingPreview | null>(null);
  const [conversion, setConversion] = useState<ModelPricingConversion | null>(null);

  const draft = useMemo<Record<string, unknown>>(() => {
    if (mode === "tiered_expr") {
      return { billing_mode: "tiered_expr", billing_expr: expression };
    }
    const parsed = Number(ratio);
    return Number.isFinite(parsed) ? { billing_mode: "ratio", ModelRatio: parsed } : { billing_mode: "ratio" };
  }, [mode, ratio, expression]);

  const runPreview = useMutation({
    mutationFn: () => api.modelPricing.preview(entry.model_name, draft),
    onSuccess: (data) => {
      setPreview(data);
      setConversion(null);
    },
    onError: (error: unknown) => toast.error(t.modelPricing.previewFailed, String((error as Error)?.message ?? error)),
  });

  const runConvert = useMutation({
    mutationFn: () => api.modelPricing.convert(entry.model_name, draft),
    onSuccess: (data) => {
      setConversion(data);
      if (data.expression) {
        setMode("tiered_expr");
        setExpression(data.expression);
        setPreview(null);
      }
    },
    onError: (error: unknown) => toast.error(t.modelPricing.previewFailed, String((error as Error)?.message ?? error)),
  });

  const save = useMutation({
    mutationFn: () =>
      api.modelPricing.update([
        { model_name: entry.model_name, expected_version: entry.version, pricing: draft },
      ]),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["modelPricing"] });
      toast.success(t.modelPricing.saved);
      onClose();
    },
    // A conflict means another tab changed this model, so the honest move is to
    // reload the row rather than retry with a version that is no longer current.
    onError: (error: unknown) => {
      const message = String((error as Error)?.message ?? error);
      toast.error(t.modelPricing.saveFailed, message);
      qc.invalidateQueries({ queryKey: ["modelPricing"] });
    },
  });

  return (
    <Dialog
      open
      onClose={onClose}
      title={`${t.modelPricing.edit}: ${entry.model_name}`}
      size="md"
      footer={
        <>
          <Button variant="outlined" onClick={onClose}>
            {t.common.cancel}
          </Button>
          <Button variant="filled" loading={save.isPending} onClick={() => save.mutate()}>
            {t.common.save}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <div>
          <label className="field-label">{t.modelPricing.mode}</label>
          <Select
            value={mode}
            onChange={(value) => setMode(value)}
            options={[
              { value: "ratio", label: t.modelPricing.modeRatio },
              { value: "tiered_expr", label: t.modelPricing.modeExpr },
            ]}
          />
        </div>

        {mode === "ratio" ? (
          <div>
            <label className="field-label">{t.modelPricing.ratio}</label>
            <input
              className="input font-mono text-xs"
              value={ratio}
              onChange={(e) => setRatio(e.target.value)}
              placeholder="1.0"
            />
            <p className="field-hint">{t.modelPricing.ratioHint}</p>
          </div>
        ) : (
          <div>
            <label className="field-label">{t.modelPricing.expression}</label>
            <textarea
              className="input font-mono text-xs"
              rows={3}
              value={expression}
              onChange={(e) => setExpression(e.target.value)}
              placeholder='tier("base", p * 2 + c * 6)'
            />
            <p className="field-hint">{t.modelPricing.expressionHint}</p>
          </div>
        )}

        <div className="flex flex-wrap items-center gap-2">
          <Button variant="outlined" size="sm" loading={runPreview.isPending} onClick={() => runPreview.mutate()}>
            <Calculator size={14} /> {t.modelPricing.preview}
          </Button>
          <Button variant="ghost" size="sm" loading={runConvert.isPending} onClick={() => runConvert.mutate()}>
            <Wand2 size={14} /> {t.modelPricing.convert}
          </Button>
        </div>

        {preview && (
          <div className="pricing-preview" role="status">
            <div className="pricing-preview-row">
              <span>{t.modelPricing.inputPrice}</span>
              <span className="font-mono">
                {preview.billing_details.input_price_per_million !== undefined
                  ? `$${preview.billing_details.input_price_per_million.toFixed(4)} / M`
                  : "—"}
              </span>
            </div>
            <div className="pricing-preview-row">
              <span>{t.modelPricing.outputPrice}</span>
              <span className="font-mono">
                {preview.billing_details.output_price_per_million !== undefined
                  ? `$${preview.billing_details.output_price_per_million.toFixed(4)} / M`
                  : "—"}
              </span>
            </div>
            {preview.billing_details.per_call_price !== undefined && (
              <div className="pricing-preview-row">
                <span>{t.modelPricing.perCall}</span>
                <span className="font-mono">${preview.billing_details.per_call_price}</span>
              </div>
            )}
          </div>
        )}

        {conversion && (
          <div className="pricing-preview" role="status">
            {conversion.expression ? (
              <>
                <div className="pricing-preview-row">
                  <span>{t.modelPricing.converted}</span>
                </div>
                <code className="pricing-preview-code">{conversion.expression}</code>
              </>
            ) : (
              <div className="pricing-preview-row">
                <span>{conversion.unsupported_reason}</span>
              </div>
            )}
          </div>
        )}
      </div>
    </Dialog>
  );
}

export default function ModelPricingPage() {
  const { t } = useI18n();
  const toast = useToast();
  const qc = useQueryClient();
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);
  const [editing, setEditing] = useState<ModelPricingEntry | null>(null);
  // Models fetched by name. The catalogue is deliberately bounded (the shipped
  // pack prices thousands of models through its ratio table), so a name that is
  // not in it is loaded on demand instead of the API listing everything.
  const [extra, setExtra] = useState<ModelPricingEntry[]>([]);

  const { data, isLoading, refetch, isFetching } = useQuery<ModelPricingSnapshot>({
    queryKey: ["modelPricing"],
    queryFn: () => api.modelPricing.snapshot(),
  });

  const lookup = useMutation({
    mutationFn: (name: string) => api.modelPricing.snapshot([name]),
    onSuccess: (snapshot) => {
      setExtra((previous) => [...previous, ...snapshot.entries]);
    },
    onError: (error: unknown) => toast.error(t.modelPricing.lookupFailed, String((error as Error)?.message ?? error)),
  });

  const resetAll = useMutation({
    mutationFn: () => api.modelPricing.reset([]),
    onSuccess: (models) => {
      qc.invalidateQueries({ queryKey: ["modelPricing"] });
      toast.success(t.modelPricing.resetDone.replace("{n}", String(models.length)));
    },
    onError: (error: unknown) => toast.error(t.modelPricing.saveFailed, String((error as Error)?.message ?? error)),
  });

  const resetOne = useMutation({
    mutationFn: (model: string) => api.modelPricing.reset([model]),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["modelPricing"] });
      toast.success(t.modelPricing.saved);
    },
    onError: (error: unknown) => toast.error(t.modelPricing.saveFailed, String((error as Error)?.message ?? error)),
  });

  const filtered = useMemo(() => {
    const base = data?.entries ?? [];
    const known = new Set(base.map((entry) => entry.model_name));
    const entries = [...base, ...extra.filter((entry) => !known.has(entry.model_name))];
    const needle = search.trim().toLowerCase();
    if (!needle) return entries;
    return entries.filter((entry) => entry.model_name.toLowerCase().includes(needle));
  }, [data, extra, search]);
  const pageCount = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const visible = filtered.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE);

  const columns: DataTableColumn<ModelPricingEntry>[] = [
    { key: "model", header: t.modelPricing.model, width: "28%", render: (row) => <span className="font-mono text-xs">{row.model_name}</span> },
    {
      key: "mode",
      header: t.modelPricing.mode,
      width: "14%",
      render: (row) => (
        <span className="pricing-badge">{modeOf(row) === "tiered_expr" ? t.modelPricing.modeExpr : t.modelPricing.modeRatio}</span>
      ),
    },
    {
      key: "configured",
      header: t.modelPricing.configured,
      width: "24%",
      render: (row) => {
        const keys = Object.keys(row.configured);
        if (keys.length === 0) return <span className="pricing-muted">{t.modelPricing.inherited}</span>;
        return (
          <span className="font-mono text-xs">
            {keys.map((key) => `${key}=${valueText(row.configured[key])}`).join("  ")}
          </span>
        );
      },
    },
    { key: "effective", header: t.modelPricing.effective, width: "24%", render: (row) => <span className="font-mono text-xs">{priceText(row)}</span> },
    {
      key: "actions",
      header: "",
      width: "10%",
      align: "right",
      render: (row) => (
        <div className="flex items-center justify-end gap-1">
          <Button variant="ghost" size="sm" onClick={() => setEditing(row)} aria-label={t.common.edit}>
            <Pencil size={14} />
          </Button>
          <Tooltip content={t.modelPricing.resetOne}>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => resetOne.mutate(row.model_name)}
              aria-label={t.modelPricing.resetOne}
            >
              <RotateCcw size={14} />
            </Button>
          </Tooltip>
        </div>
      ),
    },
  ];

  return (
    <div className="page-fade-enter space-y-4">
      <PageHeader title={t.modelPricing.title} description={t.modelPricing.description} />

      <div className="pricing-toolbar">
        <div className="pricing-search">
          <Search size={14} />
          <input
            className="input text-xs"
            value={search}
            onChange={(e) => {
              setSearch(e.target.value);
              setPage(1);
            }}
            onKeyDown={(e) => {
              // Enter asks the API for that exact model, which is how a model
              // outside the catalogue becomes editable.
              if (e.key === "Enter" && search.trim()) lookup.mutate(search.trim());
            }}
            placeholder={t.modelPricing.searchPlaceholder}
          />
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            loading={isFetching}
            onClick={() => {
              setExtra([]);
              refetch();
            }}
          >
            <RefreshCw size={14} /> {t.common.refresh}
          </Button>
          <Button variant="outlined" size="sm" loading={resetAll.isPending} onClick={() => resetAll.mutate()}>
            <RotateCcw size={14} /> {t.modelPricing.resetAll}
          </Button>
        </div>
      </div>

      {isLoading ? (
        <div className="skeleton h-64 w-full" />
      ) : (
        <>
          <DataTable
            columns={columns}
            rows={visible}
            rowKey={(row) => row.model_name}
            empty={
              <span className="pricing-muted">
                {search.trim() ? t.modelPricing.emptyHint : t.modelPricing.empty}
              </span>
            }
          />
          {pageCount > 1 && (
            <Pagination page={page} pageSize={PAGE_SIZE} total={filtered.length} onPageChange={setPage} />
          )}
        </>
      )}

      {editing && <PricingDialog entry={editing} onClose={() => setEditing(null)} />}
    </div>
  );
}
