import React from "react";
import { Link } from "react-router-dom";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { PageHeader } from "../App";
import Button from "../components/ui/Button";
import Dialog from "../components/ui/Dialog";
import { api, SubscriptionPlan, SystemStatus } from "../lib/api";
import { useAuth } from "../lib/auth";
import { useToast } from "../lib/toast";
import { useI18n } from "../lib/i18nContext";

const money = (n: number, currency = "USD") =>
  new Intl.NumberFormat("en-US", { style: "currency", currency }).format(n / 1_000_000);

export default function PlansPage() {
  const { t } = useI18n(); const { user } = useAuth(); const toast = useToast(); const qc = useQueryClient();
  const { data, isLoading, error } = useQuery({ queryKey: ["plans"], queryFn: api.plans.list });
  const { data: status } = useQuery<SystemStatus>({ queryKey: ["status"], queryFn: api.status.get });
  const currency = status?.currency ?? "USD";
  const [selected, setSelected] = React.useState<SubscriptionPlan | null>(null); const [purchasing, setPurchasing] = React.useState(false);
  const purchase = async () => { if (!selected) return; setPurchasing(true); try { const result = await api.subscriptions.subscribe(selected.id); await Promise.all([qc.invalidateQueries({ queryKey: ["wallet"] }), qc.invalidateQueries({ queryKey: ["subscriptions"] })]); toast.success(t.saas.purchaseSuccess, `${selected.name} · ${money(result.balance_micros, currency)} remaining`); setSelected(null); } catch (err) { toast.error(t.saas.purchaseFailed, err instanceof Error ? err.message : undefined); } finally { setPurchasing(false); } };
  return <><PageHeader title={t.saas.plans} description={t.saas.plansDescription} />{!user && <div className="empty-state"><p>{t.saas.signInToPurchase}</p><Link className="btn-outlined" to="/ui/sign-in">{t.saas.signIn}</Link></div>}{isLoading ? <div className="empty-state">{t.saas.loadingPlans}</div> : error ? <div className="error-banner">{(error as Error).message}</div> : data?.length ? <div className="plan-grid">{data.filter((plan) => plan.enabled).map((plan) => <article className="plan-card" key={plan.id}><div><h2>{plan.name}</h2><p>{plan.description}</p></div><strong>{money(plan.price_micros, currency)}</strong><dl><div><dt>{t.saas.quota}</dt><dd>{money(plan.quota_micros, currency)}</dd></div><div><dt>{t.saas.duration}</dt><dd>{plan.duration_days} {t.common.days}</dd></div></dl>{user ? <Button onClick={() => setSelected(plan)}>{t.saas.subscribe}</Button> : <Link className="btn-outlined" to="/ui/sign-in">{t.saas.signIn}</Link>}</article>)}</div> : <div className="empty-state">{t.saas.noPlans}</div>}<Dialog open={!!selected} onClose={() => !purchasing && setSelected(null)} title={t.saas.confirmPurchase} description={selected?.name} footer={<><Button variant="outlined" onClick={() => setSelected(null)} disabled={purchasing}>{t.common.cancel}</Button><Button onClick={purchase} loading={purchasing}>{t.common.confirm}</Button></>}>{selected && <p>{money(selected.price_micros, currency)} {t.saas.forTerm} {selected.duration_days} {t.common.days}. {t.saas.purchasePrompt}</p>}</Dialog></>;
}
