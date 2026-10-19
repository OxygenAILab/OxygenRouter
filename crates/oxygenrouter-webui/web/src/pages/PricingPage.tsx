import React, { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Calculator, Search, Tags } from "lucide-react";
import { api, PricingCatalogue, PublicPricing } from "../lib/api";
import { PageHeader } from "../App";
import { useI18n } from "../lib/i18nContext";
import DataTable, { DataTableColumn } from "../components/ui/DataTable";
import Dialog from "../components/ui/Dialog";
import Select from "../components/ui/Select";

/** USD amounts stay readable at these magnitudes without lying about precision. */
function usd(value: number): string {
  if (!Number.isFinite(value)) return "—";
  if (value === 0) return "$0";
  if (value < 0.01) return `$${value.toFixed(4)}`;
  return `$${value.toFixed(2)}`;
}

/**
 * What one caller pays for this model in the selected group.
 *
 * The catalogue reports the model's own price; the group ratio is the
 * multiplier this instance applies on top, which is why it is applied here
 * rather than baked into the reported figure.
 */
function priceFor(row: PublicPricing, groupRatio: number): string {
  if (row.quota_type === 1) return `${usd(row.model_price * groupRatio)} / call`;
  return `${usd(row.input_price_per_million * groupRatio)} / M`;
}

function Detail({ row, catalogue, onClose }: { row: PublicPricing; catalogue: PricingCatalogue; onClose: () => void }) {
  const { t } = useI18n();
  const ratio = catalogue.group_ratio[t.pricing.defaultGroup] ?? 1;
  const facts: Array<[string, string]> = [
    [t.pricing.mode, row.billing_mode ?? "ratio"],
    [t.pricing.modelRatio, String(row.model_ratio)],
    [t.pricing.completionRatio, String(row.completion_ratio)],
    [t.pricing.inputPrice, `${usd(row.input_price_per_million * ratio)} / M`],
    [t.pricing.outputPrice, `${usd(row.output_price_per_million * ratio)} / M`],
  ];
  if (row.cache_ratio !== undefined) facts.push([t.pricing.cacheRatio, String(row.cache_ratio)]);
  if (row.create_cache_ratio !== undefined) facts.push([t.pricing.createCacheRatio, String(row.create_cache_ratio)]);
  if (row.image_ratio !== undefined) facts.push([t.pricing.imageRatio, String(row.image_ratio)]);
  if (row.quota_type === 1) facts.push([t.pricing.perCall, usd(row.model_price * ratio)]);

  return (
    <Dialog open onClose={onClose} title={row.model_name} size="md" footer={<button className="btn-filled" onClick={onClose}>{t.common.cancel}</button>}>
      <div className="space-y-3">
        {row.description && <p className="pricing-muted">{row.description}</p>}
        <div className="pricing-facts">
          {facts.map(([label, value]) => (
            <div className="pricing-fact" key={label}>
              <span>{label}</span>
              <b className="font-mono">{value}</b>
            </div>
          ))}
        </div>
        {row.billing_expr && (
          <div>
            <div className="field-label">{t.pricing.expression}</div>
            <code className="pricing-preview-code">{row.billing_expr}</code>
          </div>
        )}
        <div>
          <div className="field-label">{t.pricing.groups}</div>
          <div className="pricing-chips">
            {row.enable_groups.map((group) => (
              <span className="pricing-badge" key={group}>
                {group} × {catalogue.group_ratio[group] ?? 1}
              </span>
            ))}
          </div>
        </div>
        <div>
          <div className="field-label">{t.pricing.endpoints}</div>
          <div className="pricing-chips">
            {row.supported_endpoint_types.length === 0 ? (
              <span className="pricing-muted">{t.pricing.noEndpoints}</span>
            ) : (
              row.supported_endpoint_types.map((provider) => (
                <span className="pricing-badge" key={provider}>
                  {provider}
                </span>
              ))
            )}
          </div>
        </div>
        <div className="pricing-muted font-mono">pricing_version {row.pricing_version.slice(0, 12)}</div>
      </div>
    </Dialog>
  );
}

export default function PricingPage() {
  const { t } = useI18n();
  const [search, setSearch] = useState("");
  const [group, setGroup] = useState("default");
  const [selected, setSelected] = useState<PublicPricing | null>(null);

  const { data, isLoading } = useQuery<PricingCatalogue>({
    queryKey: ["pricing-catalogue"],
    queryFn: api.pricing.get,
    staleTime: 60_000,
  });

  const ratio = data?.group_ratio[group] ?? 1;
  const rows = useMemo(() => {
    const entries = data?.data ?? [];
    const needle = search.trim().toLowerCase();
    if (!needle) return entries;
    return entries.filter(
      (entry) =>
        entry.model_name.toLowerCase().includes(needle) ||
        (entry.vendor ?? "").toLowerCase().includes(needle) ||
        (entry.tags ?? "").toLowerCase().includes(needle),
    );
  }, [data, search]);

  const columns: DataTableColumn<PublicPricing>[] = [
    {
      key: "model",
      header: t.pricing.model,
      width: "34%",
      render: (row) => (
        <span className="pricing-model-cell">
          <span className="font-mono text-xs">{row.model_name}</span>
          {row.tags && (
            <small>
              <Tags size={10} /> {row.tags}
            </small>
          )}
        </span>
      ),
    },
    { key: "vendor", header: t.pricing.vendor, width: "16%", render: (row) => <span className="text-xs">{row.vendor ?? "—"}</span> },
    { key: "mode", header: t.pricing.mode, width: "14%", render: (row) => <span className="pricing-badge">{row.billing_mode ?? "ratio"}</span> },
    { key: "input", header: t.pricing.inputPrice, width: "14%", align: "right", render: (row) => <span className="font-mono text-xs">{row.quota_type === 1 ? "—" : `${usd(row.input_price_per_million * ratio)} / M`}</span> },
    { key: "output", header: t.pricing.outputPrice, width: "14%", align: "right", render: (row) => <span className="font-mono text-xs">{row.quota_type === 1 ? `${usd(row.model_price * ratio)} / ${t.pricing.call}` : `${usd(row.output_price_per_million * ratio)} / M`}</span> },
  ];

  return (
    <div className="page-fade-enter space-y-4">
      <PageHeader title={t.pricing.title} description={t.pricing.description} />

      <div className="pricing-toolbar">
        <div className="pricing-search">
          <Search size={14} />
          <input
            className="input text-xs"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder={t.pricing.searchPlaceholder}
          />
        </div>
        <div className="pricing-group-picker">
          <Calculator size={14} />
          <Select
            value={group}
            onChange={setGroup}
            size="sm"
            ariaLabel={t.pricing.group}
            options={Object.keys(data?.group_ratio ?? { default: 1 }).map((name) => ({
              value: name,
              label: `${data?.usable_group[name] ?? name} × ${data?.group_ratio[name] ?? 1}`,
            }))}
          />
        </div>
      </div>

      {isLoading ? (
        <div className="skeleton h-64 w-full" />
      ) : (
        <section className="dashboard-section">
          <div className="section-heading">
            <h2>{t.pricing.catalogue}</h2>
            <span className="section-meta">
              {rows.length} {t.pricing.models} · {t.pricing.version} {(data?.pricing_version ?? "").slice(0, 8)}
            </span>
          </div>
          <DataTable
            columns={columns}
            rows={rows}
            rowKey={(row) => row.model_name}
            onRowClick={setSelected}
            empty={<span className="pricing-muted">{t.pricing.empty}</span>}
          />
        </section>
      )}

      {selected && data && <Detail row={selected} catalogue={data} onClose={() => setSelected(null)} />}
    </div>
  );
}
